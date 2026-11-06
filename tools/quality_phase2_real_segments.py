"""Exact exported-segment containment in the frozen raw raster foreground.

This is a global alpha/background diagnostic, not semantic per-colour truth.
The reference is the baseline trace mask BEFORE region area-floor omissions.
No source polygon tracer or engine segment predicate is used for containment.
"""
from __future__ import annotations

import argparse
from collections import Counter
import json
import math
from pathlib import Path
import sys
import time

import autodigitize_validation as av
from quality_phase2_sewout import circle_interval, interval_linear

ALLOWANCE = .075


class RasterUnion:
    def __init__(self, foreground, width, height, width_mm, height_mm, radius=ALLOWANCE):
        self.foreground, self.w, self.h = foreground, width, height
        self.sx, self.sy = width_mm/width, height_mm/height
        self.radius = radius
        self.pad_x, self.pad_y = math.ceil(radius/self.sx), math.ceil(radius/self.sy)

    def present(self, x, y):
        return 0 <= x < self.w and 0 <= y < self.h and self.foreground[y*self.w+x]

    def path_cells(self, a, b):
        # Split at EVERY grid crossing, not a distance-sampling grid. Each
        # open interval has constant cell membership; its closure is contained
        # in the corresponding closed pixel rectangle.
        cuts = {0., 1.}
        for axis, scale in ((0, self.sx), (1, self.sy)):
            rate = b[axis]-a[axis]
            if abs(rate) < 1e-14:
                continue
            lo, hi = sorted((a[axis]/scale, b[axis]/scale))
            for line in range(math.floor(lo)+1, math.ceil(hi)):
                t = (line*scale-a[axis])/rate
                if 0. < t < 1.:
                    cuts.add(t)
        cuts = sorted(cuts)
        return {(math.floor((a[0]+(b[0]-a[0])*(lo+hi)/2)/self.sx),
                 math.floor((a[1]+(b[1]-a[1])*(lo+hi)/2)/self.sy))
                for lo, hi in zip(cuts, cuts[1:])}

    @staticmethod
    def rectangle_interval(a, delta, x0, y0, x1, y1):
        xx = interval_linear(a[0], delta[0], x0, x1)
        yy = interval_linear(a[1], delta[1], y0, y1)
        if xx and yy:
            lo, hi = max(xx[0], yy[0]), min(xx[1], yy[1])
            if hi >= lo:
                return lo, hi
        return None

    def check(self, a, b):
        cells = self.path_cells(a, b)
        if all(self.present(x, y) for x, y in cells):
            return True, 0., True
        candidates = {(x+dx, y+dy) for x, y in cells
                      for dy in range(-self.pad_y, self.pad_y+1)
                      for dx in range(-self.pad_x, self.pad_x+1)
                      if self.present(x+dx, y+dy)}
        delta, intervals = (b[0]-a[0], b[1]-a[1]), []
        r = self.radius
        for x, y in sorted(candidates):
            x0, y0, x1, y1 = x*self.sx, y*self.sy, (x+1)*self.sx, (y+1)*self.sy
            # A buffered rectangle is the union of these two strips and four
            # corner disks. Square expanded bounding boxes would over-allow
            # diagonal corners; their exact circle intervals are necessary.
            for box in ((x0-r, y0, x1+r, y1), (x0, y0-r, x1, y1+r)):
                interval = self.rectangle_interval(a, delta, *box)
                if interval:
                    intervals.append(interval)
            for corner in ((x0, y0), (x1, y0), (x1, y1), (x0, y1)):
                interval = circle_interval(a, delta, corner, r)
                if interval and interval[1] >= interval[0]:
                    intervals.append(interval)
        cursor = covered = 0.
        for lo, hi in sorted(intervals):
            if hi > cursor:
                covered += hi-max(cursor, lo)
                cursor = hi
        missing = max(0., 1.-covered)
        return missing <= 1e-8, missing*math.dist(a, b), False


def self_checks():
    mask = [True]*25
    mask[12] = False
    ring = RasterUnion(mask, 5, 5, 5., 5.)
    passed, outside, _ = ring.check((.5, 2.5), (4.5, 2.5))
    pixel = RasterUnion([True], 1, 1, 1., 1.)
    tests = dict(interior_segment=ring.check((.1, .1), (4.9, .1))[0],
                 hole_crossing_inside_endpoints_rejected=not passed and abs(outside-.85) < 1e-8,
                 narrow_quantisation_band_allowed=pixel.check((-.07, .1), (-.07, .9))[0],
                 beyond_quantisation_band_rejected=not pixel.check((-.08, .1), (-.08, .9))[0],
                 rounded_corner_inside_allowed=pixel.check((-.05, -.05), (-.05, -.05))[0],
                 square_corner_false_allowance_rejected=not pixel.check((-.06, -.06), (-.06, -.06))[0],
                 zero_segment_in_hole_rejected=not ring.check((2.5, 2.5), (2.5, 2.5))[0])
    if not all(tests.values()):
        raise AssertionError(tests)
    return tests


def evaluate(folder, reference, pe):
    plan = json.loads((folder/'design.plan.json').read_text(encoding='utf-8'))
    needles = [{**s, **m} for s, m in zip(plan['stitches'], plan['metadata']) if s['command'] == 'Stitch']
    records = pe.read(str(folder/'design.dst')).stitches
    previous, index = (0., 0.), 0
    escaped, fast, outside, examples = 0, 0, 0., []
    roles, escaped_roles = Counter(), Counter()
    exact_coordinates = True
    begin = time.perf_counter()
    for x, y, code in records:
        command = pe.decode_embroidery_command(code)[0]
        current = x/10, y/10
        if command == pe.STITCH:
            source = needles[index]
            index += 1
            role = source['role']
            roles[role] += 1
            exact_coordinates &= math.dist(current, (av.quantize(source['x']), av.quantize(source['y']))) < 1e-8
            passed, length, quick = reference.check(previous, current)
            fast += quick
            if not passed:
                escaped += 1
                escaped_roles[role] += 1
                outside += length
                if len(examples) < 20:
                    examples.append(dict(needle_index=index-1, object_id=source['object_id'], role=role,
                                         start_mm=previous, end_mm=current, outside_length_mm=length))
        if command in (pe.STITCH, pe.JUMP):
            previous = current
    return dict(dst=str((folder/'design.dst').resolve()), dst_sha256=av.sha(folder/'design.dst'),
                checked_sewn_segments=index, planned_needles=len(needles), roles=dict(roles),
                exact_needle_coordinate_match=exact_coordinates and index == len(needles),
                exact_foreground_interval_fast_paths=fast, buffered_boundary_segments=index-fast,
                escaped_segments=escaped, escaped_by_role=dict(escaped_roles), outside_length_mm=round(outside, 9),
                first_escaped_segments=examples, seconds=round(time.perf_counter()-begin, 5),
                passed=escaped == 0 and exact_coordinates and index == len(needles))


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--comparison', type=Path, default=av.ROOT/'outputs/quality-phase2-20261106/comparison')
    parser.add_argument('--output', type=Path, default=av.ROOT/'outputs/quality-phase2-20261106/real-segment-checks')
    args = parser.parse_args()
    import pyembroidery as pe
    args.output.mkdir(parents=True, exist_ok=True)
    checks = self_checks()
    trace_path = args.comparison/'baseline/user_crest/design.trace.json'
    trace = json.loads(trace_path.read_text(encoding='utf-8'))
    project = json.loads(trace_path.with_name('design.elab').read_text(encoding='utf-8'))
    w, h = trace['trace_dimensions']
    width_mm, height_mm = project['width_mm'], project['source_image']['height_mm']
    foreground = [label != 255 for label in trace['labels_before_preparation']]
    if len(foreground) != w*h:
        raise ValueError('Invalid frozen raster-mask dimensions')
    reference = RasterUnion(foreground, w, h, width_mm, height_mm)
    evidence = dict(command_line=[sys.executable]+sys.argv, script_sha256=av.sha(Path(__file__)),
                    self_checks=checks, reference=str(trace_path.resolve()), reference_sha256=av.sha(trace_path),
                    trace_dimensions_px=[w, h], physical_dimensions_mm=[width_mm, height_mm],
                    foreground_pixels=sum(foreground), reference_stage='labels_before_preparation, prior to minimum-area omissions',
                    allowed_boundary_band_mm=ALLOWANCE, compensation_mm=0., semantic_per_colour_truth=False,
                    method='Exact interval partition of raster-grid crossings plus analytic rounded-rectangle union; no endpoint-only or pitch sampling',
                    scope='Global alpha/excluded-background diagnostic using frozen raw trace foreground; no per-colour detail or fabric claim')
    av.write_json(args.output/'acceptance-before-run.json', evidence)
    results = {}
    for label in ('baseline', 'candidate'):
        results[label] = evaluate(args.comparison/label/'user_crest', reference, pe)
        av.write_json(args.output/(label+'.json'), results[label])
        print(json.dumps({label: {k: results[label][k] for k in ('passed', 'checked_sewn_segments', 'escaped_segments', 'outside_length_mm', 'seconds')}}), flush=True)
    evidence['results'] = results
    evidence['passed'] = all(result['passed'] for result in results.values())
    av.write_json(args.output/'summary.json', evidence)
    return 0 if evidence['passed'] else 1


if __name__ == '__main__':
    raise SystemExit(main())
