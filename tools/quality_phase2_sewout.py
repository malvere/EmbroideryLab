"""Reproducible local trial coupons; no machine/fabric calibration is claimed.

Run with Python 3.12, Pillow 13.0.0 and pyembroidery 1.5.1. Only test tooling
uses Python. Whole-segment polygon checks below are independent of Rust.
"""
from __future__ import annotations

import argparse
from collections import Counter, defaultdict
import copy
import csv
import hashlib
import html
import importlib.metadata
import io
import json
import math
from pathlib import Path
import sys

import autodigitize_validation as av

ROOT = Path(__file__).resolve().parents[1]
ALLOWANCE_MM = .075  # sqrt(2)*0.05mm DST point quantisation, rounded upward.


def interval_linear(value, rate, low, high):
    if abs(rate) < 1e-14:
        return (0., 1.) if low-1e-12 <= value <= high+1e-12 else None
    a, b = (low-value)/rate, (high-value)/rate
    return max(0., min(a, b)), min(1., max(a, b))


def circle_interval(a, delta, center, radius):
    dx, dy = a[0]-center[0], a[1]-center[1]
    aa = delta[0]**2+delta[1]**2
    cc = dx*dx+dy*dy-radius*radius
    if aa < 1e-24:
        return (0., 1.) if cc <= 1e-12 else None
    bb = 2*(dx*delta[0]+dy*delta[1])
    disc = bb*bb-4*aa*cc
    if disc < -1e-12:
        return None
    disc = math.sqrt(max(0., disc))
    return max(0., (-bb-disc)/(2*aa)), min(1., (-bb+disc)/(2*aa))


def segment_containment(a, b, rings, allowance=ALLOWANCE_MM):
    """Cover ALL of segment a--b with polygon interior or boundary capsules.

    Polygon edge intersections split [0,1] into intervals of constant even-odd
    membership. A radius-r boundary capsule is its edge rectangle plus endpoint
    circles; line/capsule intersections are solved analytically. Union lengths
    therefore check the entire segment, including concave holes, without a
    sampling grid. The only permitted exterior is the explicit DST allowance.
    Returns (covered, exterior_length_mm). No pull compensation is implicit.
    """
    delta = b[0]-a[0], b[1]-a[1]
    cuts = [0., 1.]
    intervals = []
    for ring in rings:
        for start, end in zip(ring, ring[1:]+ring[:1]):
            c, d = (start['x'], start['y']), (end['x'], end['y'])
            edge = d[0]-c[0], d[1]-c[1]
            denom = delta[0]*edge[1]-delta[1]*edge[0]
            if abs(denom) > 1e-14:
                ac = c[0]-a[0], c[1]-a[1]
                t = (ac[0]*edge[1]-ac[1]*edge[0])/denom
                u = (ac[0]*delta[1]-ac[1]*delta[0])/denom
                if -1e-12 <= t <= 1+1e-12 and -1e-12 <= u <= 1+1e-12:
                    cuts.append(min(1., max(0., t)))
            length = math.hypot(*edge)
            if length > 1e-14:
                unit = edge[0]/length, edge[1]/length
                normal = -unit[1], unit[0]
                ac = a[0]-c[0], a[1]-c[1]
                along = interval_linear(ac[0]*unit[0]+ac[1]*unit[1], delta[0]*unit[0]+delta[1]*unit[1], 0., length)
                across = interval_linear(ac[0]*normal[0]+ac[1]*normal[1], delta[0]*normal[0]+delta[1]*normal[1], -allowance, allowance)
                if along and across:
                    intervals.append((max(along[0], across[0]), min(along[1], across[1])))
            for vertex in (c, d):
                interval = circle_interval(a, delta, vertex, allowance)
                if interval:
                    intervals.append(interval)
    cuts = sorted(set(cuts))
    for lo, hi in zip(cuts, cuts[1:]):
        t = (lo+hi)/2
        if av.inside((a[0]+t*delta[0], a[1]+t*delta[1]), rings):
            intervals.append((lo, hi))
    cursor = covered = 0.
    for lo, hi in sorted((max(0., lo), min(1., hi)) for lo, hi in intervals if hi >= lo):
        if hi > cursor:
            covered += hi-max(cursor, lo)
            cursor = hi
    missing = max(0., 1.-covered)
    return missing <= 1e-8, missing*math.hypot(*delta)


def geometry_self_checks():
    outer, hole = av.rect(0, 0, 10, 10), av.rect(4, 2, 2, 6)
    tests = {
        'inside': segment_containment((1, 1), (9, 1), [outer])[0],
        'hole_crossing_rejected': not segment_containment((1, 5), (9, 5), [outer, hole])[0],
        'outside_endpoints_rejected': not segment_containment((-1, 1), (-1, 9), [outer])[0],
        'quantisation_band_allowed': segment_containment((1, -.07), (9, -.07), [outer])[0],
        'beyond_quantisation_rejected': not segment_containment((1, -.08), (9, -.08), [outer])[0],
        'zero_inside': segment_containment((2, 2), (2, 2), [outer])[0],
        'zero_in_hole_rejected': not segment_containment((5, 5), (5, 5), [outer, hole])[0],
    }
    if not all(tests.values()):
        raise AssertionError(tests)
    return tests


def contours(item):
    if item.get('contours'):
        return item['contours']
    column = item.get('satin_column')
    if column:
        if column.get('closed', False):
            return [column['left'], column['right']]
        return [column['left']+list(reversed(column['right']))]
    return []


def whole_segment_checks(project, generated, records, pe):
    objects = {item['id']: item for item in project['objects']}
    needles = [s for s in generated if s['command'] == 'Stitch']
    previous, index = (0., 0.), 0
    violations, counts, escaped = [], Counter(), 0.
    maximum_role_length = defaultdict(float)
    grid = defaultdict(lambda: defaultdict(float))
    for x, y, flag in records:
        current = x, y
        if flag == pe.STITCH:
            source = needles[index]
            index += 1
            role = source.get('role', 'LegacyUnspecified')
            counts[role] += 1
            length = math.dist(previous, current)
            maximum_role_length[role] = max(maximum_role_length[role], length)
            rings = contours(objects[source['object_id']])
            if not rings:
                raise AssertionError('Coupon must expose explicit permitted geometry')
            passed, outside = segment_containment(previous, current, rings)
            escaped += outside
            if not passed:
                if len(violations) < 12:
                    violations.append(dict(needle_index=index-1, object_id=source['object_id'], role=role,
                                           start=previous, end=current, outside_length_mm=outside))
            # Split segment at every 1mm grid line; assign true traversed length.
            cuts = {0., 1.}
            for axis in (0, 1):
                rate = current[axis]-previous[axis]
                if abs(rate) > 1e-12:
                    for line in range(math.floor(min(current[axis], previous[axis]))+1,
                                      math.ceil(max(current[axis], previous[axis]))):
                        cuts.add((line-previous[axis])/rate)
            cuts = sorted(cuts)
            for lo, hi in zip(cuts, cuts[1:]):
                t = (lo+hi)/2
                cell = tuple(math.floor(previous[j]+t*(current[j]-previous[j])) for j in (0, 1))
                grid[role][cell] += length*(hi-lo)
        if flag in (pe.STITCH, pe.JUMP):
            previous = current
    return dict(method='analytic whole-segment interval union with even-odd polygon and boundary capsules',
                quantisation_allowance_mm=ALLOWANCE_MM, implicit_compensation_mm=0.,
                checked_sewn_segments=index, roles=dict(counts), violations=violations,
                exterior_length_mm=round(escaped, 9), passed=not violations,
                max_segment_mm_by_role=dict(maximum_role_length), density_cell_mm=1.,
                maximum_segment_mm_per_mm2_by_role={role: max(cells.values(), default=0.) for role, cells in grid.items()})


def protected_source_checks(project):
    recipe = project.get('raster_recipe')
    cleanup = recipe.get('options', {}).get('cleanup') if recipe else None
    if not cleanup:
        return dict(applicable=False, passed=True)
    scale = recipe['width_mm']/cleanup['width']
    black = [contours(o) for o in project['objects'] if o['color'] == [0, 0, 0]]
    foreground = [contours(o) for o in project['objects']]
    from PIL import Image
    embedded = project['source_image']['encoded']
    source = Image.open(io.BytesIO(bytes.fromhex(embedded) if isinstance(embedded, str) else bytes(embedded))).convert('RGBA')
    indices = [i for start, count in cleanup['protected_runs'] for i in range(start, start+count)]
    def retained(selected, shapes):
        return sum(any(av.inside(((i%cleanup['width']+.5)*scale, (i//cleanup['width']+.5)*scale), rings)
                       for rings in shapes) for i in selected)
    exact_black = [i for i in indices if source.getpixel((i%cleanup['width'], i//cleanup['width']))[:3] == (0, 0, 0)]
    kept, black_kept = retained(indices, foreground), retained(exact_black, black)
    return dict(applicable=True, source_pixel_centers=len(indices), protected_eye_centers_retained=kept,
                exact_black_source_centers=len(exact_black), pinned_black_centers_retained=black_kept,
                passed=bool(indices) and kept == len(indices) and black_kept == len(exact_black),
                exclusion_protection_overlap_restores_detail=bool(set(indices) & {i for start, count in cleanup['excluded_runs'] for i in range(start, start+count)}))


def observation_row(result, existing=None):
    """Refresh generated dimensions while keeping actual human observations."""
    row = dict(existing or {})
    row.setdefault('sample', result['name'])
    row.setdefault('profile', 'experimental KeepThread / hardware unconfirmed')
    row['expected_width_mm'], row['expected_height_mm'] = result['actual_stitched_dimensions_mm']
    return row


def make_object(name, rings, color=(26, 87, 155), **settings):
    result = av.obj(name, rings, underlay=True, tie_in=True, tie_off=True, **settings)
    result['color'] = list(color)
    return result


def prepare(output, source_project, cli, compare_preparation=False):
    fixtures = output/'sources'
    fixtures.mkdir(parents=True, exist_ok=True)
    generic = dict(max_stitch_mm=12., max_jump_mm=None, hoop_width_mm=None, hoop_height_mm=None,
                   trim_jump_mm=10., dst_trim_encoding='KeepThread')
    historical = json.loads(source_project.read_text(encoding='utf-8-sig'))
    original_profile = historical.get('machine_profile')
    # Actual unknown hoop and machine remain null. KeepThread is an explicit
    # coupon preparation policy, not a statement about the user's equipment.
    if original_profile:
        generic.update(original_profile)
        generic['dst_trim_encoding'] = 'KeepThread'
    profile = dict(source_project=str(source_project), source_sha256=av.sha(source_project),
                   original_saved_profile=original_profile, coupon_profile=generic,
                   hardware_identity=None, actual_hoop_confirmed=False,
                   machine_acceptance='PENDING: actual model/hoop/limits not supplied',
                   trim_policy='KeepThread: no automatic cut support assumed',
                   status='экспериментальный — требует отшивки')
    av.write_json(output/'machine-profile-review.json', profile)
    cases = []

    def vector(name, objects, description, comparison=None):
        for index, obj in enumerate(objects):
            obj['id'] = index+1
        project = dict(version=5, name=name, width_mm=40., objects=objects, machine_profile=copy.deepcopy(generic))
        path = fixtures/(name+'.elab')
        av.write_json(path, project)
        cases.append(dict(name=name, source=path, description=description, comparison=comparison,
                          actual_hoop_check='pending' if generic['hoop_width_mm'] is None or generic['hoop_height_mm'] is None else 'checked below'))

    vector('01_tatami_reference', [make_object('Tatami rectangle', [av.rect(3, 3, 24, 16)])], 'Tatami 24 × 16 мм; шаг 0,40 мм')
    vector('01b_tatami_spacing_045', [make_object('Tatami rectangle', [av.rect(3, 3, 24, 16)], spacing_mm=.45)],
           'Та же геометрия; изменён только шаг 0,40 → 0,45 мм',
           dict(reference='01_tatami_reference', changed_setting='spacing_mm', before=.4, after=.45))
    vector('02_ring_hole', [make_object('Ring with 10 × 8 mm opening', [av.rect(3, 3, 24, 18), av.rect(10, 8, 10, 8)])],
           'Кольцо 24 × 18 мм, отверстие 10 × 8 мм')
    vector('03_adjacent_colors', [make_object('Blue left', [av.rect(3, 3, 12, 16)]),
                                  make_object('Red right', [av.rect(15, 3, 12, 16)], color=(190, 30, 45))],
           'Две соседние контрастные заливки 12 × 16 мм, общая граница x=15 мм')
    column = make_object('Simple 3 mm Satin column', [], kind='Satin')
    column['satin_column'] = dict(left=[av.p(3, 3), av.p(25, 3)], right=[av.p(3, 6), av.p(25, 6)],
                                  left_handles=[], right_handles=[], rungs=[], closed=False, seam=0., first_right=False)
    vector('04_satin_column', [column], 'Простая Satin-колонка 22 × 3 мм; без автоматического ветвления')
    border = make_object('Narrow 1 mm closed Satin border', [], kind='Satin')
    border['satin_column'] = dict(left=av.rect(3, 3, 24, 16), right=av.rect(4, 4, 22, 14),
                                  left_handles=[], right_handles=[], rungs=[], closed=True, seam=0., first_right=False)
    vector('05_narrow_border', [border], 'Замкнутая Satin-окантовка шириной 1 мм, 24 × 16 мм')

    from PIL import Image
    embedded = historical['source_image']['encoded']
    source_bytes = bytes.fromhex(embedded) if isinstance(embedded, str) else bytes(embedded)
    pixels = Image.open(io.BytesIO(source_bytes)).convert('RGBA')
    # Deliberate, recorded crop of the real user's crest: horse head and mane.
    box = (289, 70, 454, 245)
    if pixels.size != (500, 669):
        raise ValueError('Recorded real-motif crop requires the exact 500 × 669 crest')
    image = pixels.crop(box)
    real_path = fixtures/'06_real_horse_head.png'
    image.save(real_path)
    # A protected eye test and source mask survive ELAB save/reopen. Explicit
    # protected pixel belongs to black eye; exclusion is a red corner pixel.
    def runs(indices):
        result = []
        for index in sorted(set(indices)):
            if result and result[-1][0]+result[-1][1] == index:
                result[-1][1] += 1
            else:
                result.append([index, 1])
        return result
    # This cropped motif contains black linework and white fill on an excluded
    # red exterior. Its physical coupon explicitly uses TWO intended threads;
    # learned grey compression shades are not additional thread colours. This
    # choice is identical for both preparation modes and is not an algorithm
    # quality comparison or a change to the full-crest palette.
    options = dict(max_dimension=256, palette_size=2,
                   background={'ConnectedColors': {'rgb': [[204, 0, 0]], 'tolerance': 24.}},
                   min_region_pixels=1, min_region_area_mm2=0., simplify_tolerance_mm=0.,
                   preserve_shared_boundaries=True, smoothing_iterations=0, denoise_passes=0,
                   pinned_colors=[[0, 0, 0], [255, 255, 255]])
    eye = [y*image.width+x for y in range(80, 108) for x in range(110, 132)
           if max(image.getpixel((x, y))[:3]) < 48]
    if not eye:
        raise AssertionError('Recorded eye protection must intersect a real black detail')
    options['cleanup'] = dict(width=image.width, height=image.height,
                              excluded_runs=runs([0]+eye), protected_runs=runs(eye))
    # Keep this independent package's settings explicit. No evolving automatic
    # preset is silently substituted into a physical sample.
    real_variants = [('06_real_horse_head', None)]
    if compare_preparation:
        real_variants.append(('06b_real_standard', 'Standard'))
    for variant_name, preparation in real_variants:
        selected_options = copy.deepcopy(options)
        if preparation:
            selected_options['preparation'] = preparation
        options_path = fixtures/(variant_name+'.options.json')
        av.write_json(options_path, selected_options)
        intermediate = output/'diagnostics'/variant_name/'import.dst'
        execution = av.run([cli, 'generate-with-options', real_path, intermediate, 22., options_path])
        av.write_json(fixtures/(variant_name+'.import-command.json'), execution)
        if execution['exit_code'] != 0:
            raise RuntimeError(execution)
        motif = json.loads(intermediate.with_suffix('.elab').read_text(encoding='utf-8-sig'))
        motif['machine_profile'] = copy.deepcopy(generic)
        # Retain Tatami tracing from the existing generator. Small real motif
        # is a controlled coupon, not evidence for automatic Satin quality.
        motif['name'] = variant_name
        motif_path = fixtures/(variant_name+'.elab')
        av.write_json(motif_path, motif)
        cases.append(dict(name=variant_name, source=motif_path,
                          description='Реальный фрагмент герба: голова коня, 22 × 23,333 мм'+(' — Standard' if preparation else ' — Legacy'),
                          comparison=dict(reference='06_real_horse_head', changed_setting='preparation', before='Legacy', after='Standard') if preparation else None,
                          actual_hoop_check='pending' if generic['hoop_width_mm'] is None or generic['hoop_height_mm'] is None else 'checked below'))
    av.write_json(fixtures/'06_provenance.json', dict(source_project=str(source_project),
                  source_project_sha256=av.sha(source_project), embedded_source_sha256=hashlib.sha256(source_bytes).hexdigest(),
                  crop_xyxy_px=box, cropped_source_sha256=av.sha(real_path), width_mm=22.,
                  original_dimensions_px=pixels.size, crop_dimensions_px=image.size,
                  note='Реальный фрагмент герба пользователя; хранится только локально'))
    return cases, profile


def preservation_checks(cli, output, baseline_cli=None):
    def original_fields_preserved(before, after):
        # A compatible save can materialise serde defaults absent from an old
        # file. Check every actual original decision, including array order.
        if isinstance(before, dict):
            return isinstance(after, dict) and all(k in after and original_fields_preserved(v, after[k]) for k, v in before.items())
        if isinstance(before, list):
            return isinstance(after, list) and len(before) == len(after) and all(original_fields_preserved(a, b) for a, b in zip(before, after))
        return before == after
    results = []
    for relative in ('outputs/shield-cleanup/shield-cleaned.elab',
                     'outputs/autodigitize/import-fix/final-real/after-autodigitize/design.elab'):
        source = ROOT/relative
        folder = output/'compatibility'/source.parent.name
        folder.mkdir(parents=True, exist_ok=True)
        before = json.loads(source.read_text(encoding='utf-8-sig'))
        saved = folder/'opened.elab'
        opened = av.run([cli, 'project', source, saved])
        av.write_json(folder/'open-save-command.json', opened)
        previous_reader = None
        if baseline_cli and opened['exit_code'] == 0:
            previous_reader = av.run([baseline_cli, 'project', saved, folder/'previous-reader-opened.elab'])
            av.write_json(folder/'previous-reader-command.json', previous_reader)
        a, b = folder/'first.dst', folder/'second.dst'
        first = av.run([cli, 'generate', source, a])
        av.write_json(folder/'first-command.json', first)
        second = av.run([cli, 'generate', a.with_suffix('.elab'), b]) if first['exit_code'] == 0 else None
        av.write_json(folder/'second-command.json', second)
        after = json.loads(saved.read_text(encoding='utf-8-sig')) if opened['exit_code'] == 0 else {}
        baseline = None
        if first['exit_code'] != 0 and baseline_cli:
            baseline = av.run([baseline_cli, 'generate', source, folder/'original-baseline.dst'])
            av.write_json(folder/'original-baseline-command.json', baseline)
        checks = dict(old_schema_version=before['version'], source=str(source), source_sha256=av.sha(source),
                      opens=opened['exit_code'] == 0,
                      previous_binary_opens_saved_legacy=previous_reader['exit_code'] == 0 if previous_reader else None,
                      manual_objects_identical=original_fields_preserved(before['objects'], after.get('objects')),
                      embedded_source_identical=before.get('source_image') == after.get('source_image'),
                      recipe_identical=original_fields_preserved(before.get('raster_recipe'), after.get('raster_recipe')),
                      comparison_semantics='Every original field and array order; newly materialised schema defaults allowed',
                      generation_succeeded=first['exit_code'] == 0,
                      baseline_same_rejection=bool(baseline and baseline['exit_code'] != 0 and first['stderr'] == baseline['stderr']),
                      generation_error=first['stderr'] if first['exit_code'] != 0 else None,
                      repeated_dst_identical=bool(second and second['exit_code'] == 0 and av.sha(a) == av.sha(b)))
        checks['passed'] = all(checks[k] for k in ('opens', 'manual_objects_identical', 'embedded_source_identical', 'recipe_identical')) and checks['previous_binary_opens_saved_legacy'] is not False and (checks['repeated_dst_identical'] or checks['baseline_same_rejection'])
        av.write_json(folder/'preservation.json', checks)
        results.append(checks)
    return results


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--cli', type=Path, required=True)
    parser.add_argument('--output', type=Path, default=ROOT/'outputs/quality-phase2-20261106/sewout')
    parser.add_argument('--real-source-project', type=Path,
                        default=ROOT/'outputs/autodigitize/import-fix/final-real/after-autodigitize/design.elab')
    parser.add_argument('--baseline-cli', type=Path)
    parser.add_argument('--compare-preparation', action='store_true',
                        help='Also prepare a Standard variant changing only real-motif segmentation')
    args = parser.parse_args()
    import pyembroidery as pe
    if importlib.metadata.version('pyembroidery') != '1.5.1':
        raise RuntimeError('Use pyembroidery==1.5.1 for reproducible independent reading')
    output, cli = args.output.resolve(), args.cli.resolve()
    output.mkdir(parents=True, exist_ok=True)
    initial_cli_sha256 = av.sha(cli)
    initial_source_sha256 = av.sha(args.real_source_project.resolve())
    self_checks = geometry_self_checks()
    # Acceptance tolerances are saved BEFORE coupon generation and measurement.
    av.write_json(output/'acceptance-before-run.json', dict(
        entire_segment_method='Analytic intervals against explicit polygon geometry',
        allowed_dst_boundary_band_mm=ALLOWANCE_MM, compensation_mm=0.,
        expected_exterior_intervals=0, exact_needle_coordinates=True, exact_color_controls=True,
        exact_end_record=True, actual_machine_fit='unknown until hardware profile is supplied',
        top_coverage=dict(effective_thread_width_mm=.30, sample_pitch_mm=.23, status='diagnostic, no fabric claim')))
    cases, profile = prepare(output, args.real_source_project.resolve(), cli, args.compare_preparation)
    results = []
    for case in cases:
        folder = output/case['name']
        folder.mkdir(parents=True, exist_ok=True)
        target = folder/'design.dst'
        execution = av.run([cli, 'generate', case['source'], target])
        av.write_json(folder/'generation-command.json', execution)
        if execution['exit_code'] != 0:
            raise RuntimeError(execution)
        project = json.loads(target.with_suffix('.elab').read_text(encoding='utf-8-sig'))
        plan = json.loads(target.with_suffix('.plan.json').read_text(encoding='utf-8-sig'))
        generated = [{**s, **m} for s, m in zip(plan['stitches'], plan['metadata'])]
        independent, records = av.inspect(target, generated, project, pe, {})
        segments = whole_segment_checks(project, generated, records, pe)
        reopen_target = folder/'reopened.dst'
        reopen = av.run([cli, 'generate', target.with_suffix('.elab'), reopen_target])
        av.write_json(folder/'reopen-command.json', reopen)
        reopened = json.loads(reopen_target.with_suffix('.elab').read_text(encoding='utf-8-sig'))
        b = independent['bounds_mm']
        width, height = b[2]-b[0], b[3]-b[1]
        mp = project['machine_profile']
        fit = None if mp['hoop_width_mm'] is None or mp['hoop_height_mm'] is None else width <= mp['hoop_width_mm'] and height <= mp['hoop_height_mm']
        coverage_project = copy.deepcopy(project)
        for item in coverage_project['objects']:
            item['contours'] = contours(item)
        result = dict(name=case['name'], description=case['description'], source=str(case['source']),
                      comparison=case['comparison'], independent_dst=independent, entire_segments=segments,
                      geometric_top_coverage=av.top_coverage(coverage_project, generated),
                      actual_stitched_dimensions_mm=[round(width, 3), round(height, 3)],
                      machine_hoop_fit=fit, machine_hardware_accepted=False,
                      project_settings=[dict(object_id=o['id'], color_rgb=o['color'], settings=o['settings']) for o in project['objects']],
                      plan_warnings=plan.get('warnings', []), seconds=execution['seconds'],
                      protected_details=protected_source_checks(project),
                      repeated_dst_identical=reopen['exit_code'] == 0 and av.sha(target) == av.sha(reopen_target),
                      source_preserved=project.get('source_image') == reopened.get('source_image'),
                      recipe_masks_palette_preserved=project.get('raster_recipe') == reopened.get('raster_recipe'))
        result['digital_checks_passed'] = independent['format_geometry_passed'] and segments['passed'] and result['protected_details']['passed'] and result['repeated_dst_identical'] and result['source_preserved'] and result['recipe_masks_palette_preserved'] and fit is not False
        av.write_json(folder/'verification.json', result)
        av.render(folder/'preview.png', project, generated, records, pe, case['description'])
        results.append(result)
    compatibility = preservation_checks(cli, output, args.baseline_cli.resolve() if args.baseline_cli else None)
    provenance_unchanged = initial_cli_sha256 == av.sha(cli) and initial_source_sha256 == av.sha(args.real_source_project.resolve())
    summary = dict(status='экспериментальный — требует отшивки', cli=str(cli), cli_sha256=initial_cli_sha256,
                   cli_and_original_source_unchanged_during_run=provenance_unchanged,
                   python_executable=sys.executable, reader='pyembroidery '+importlib.metadata.version('pyembroidery'),
                   command_line=[sys.executable]+sys.argv,
                   pillow_version=importlib.metadata.version('Pillow'), script_sha256=av.sha(Path(__file__)),
                   geometry_checker_self_checks=self_checks, machine_profile=profile,
                   digital_checks_passed=provenance_unchanged and all(r['digital_checks_passed'] for r in results) and all(r['passed'] for r in compatibility),
                   actual_machine_profile_verified=False, physical_sewout_performed=False,
                   all_old_projects_regenerate=all(r['generation_succeeded'] for r in compatibility),
                   old_project_generation_limitations=[dict(source=r['source'], error=r['generation_error'],
                                                            baseline_same_rejection=r['baseline_same_rejection'])
                                                       for r in compatibility if not r['generation_succeeded']],
                   cases=results, compatibility=compatibility)
    av.write_json(output/'summary.json', summary)
    with (output/'parameters.csv').open('w', newline='', encoding='utf-8-sig') as stream:
        fieldnames = ['sample', 'object_id', 'color_rgb', 'kind', 'spacing_mm', 'length_mm', 'angle_deg', 'auto_angle', 'underlay',
                      'edge_underlay', 'zigzag_underlay', 'underlay_angle_deg',
                      'underlay_inset_mm', 'underlay_spacing_mm', 'underlay_length_mm', 'pull_comp_mm', 'tie_in', 'tie_off']
        writer = csv.DictWriter(stream, fieldnames)
        writer.writeheader()
        for result in results:
            for item in result['project_settings']:
                writer.writerow({k: ({'sample': result['name'], 'object_id': item['object_id'], 'color_rgb': item['color_rgb']} | item['settings']).get(k) for k in fieldnames})
    observation_fields = ['sample', 'machine_model', 'fabric', 'stabilizer', 'thread', 'needle', 'hoop', 'speed',
                          'profile', 'expected_width_mm', 'expected_height_mm', 'measured_width_mm', 'measured_height_mm',
                          'gaps', 'puckering', 'outline_offset_mm', 'thread_breaks', 'reverse_side_notes', 'notes']
    observation_path = output/'observations.csv'
    old_observations = {}
    if observation_path.exists():
        with observation_path.open(newline='', encoding='utf-8-sig') as stream:
            old_observations = {row['sample']: row for row in csv.DictReader(stream)}
    with observation_path.open('w', newline='', encoding='utf-8-sig') as stream:
        writer = csv.DictWriter(stream, observation_fields)
        writer.writeheader()
        for result in results:
            row = observation_row(result, old_observations.pop(result['name'], None))
            writer.writerow({key: row.get(key, '') for key in observation_fields})
        for row in old_observations.values():
            writer.writerow({key: row.get(key, '') for key in observation_fields})
    observation_form = output/'observations.md'
    if not observation_form.exists():
        observation_form.write_text('# Наблюдения после реальной отшивки\n\n'
                                    'Заполните отдельно для каждого образца. Фактические результаты пока не получены.\n\n'
                                    '| Поле | Наблюдение |\n| --- | --- |\n'+''.join(f'| {label} | |\n' for label in (
                                        'Образец и дата', 'Модель машины', 'Ткань', 'Стабилизатор', 'Нить', 'Игла',
                                        'Пяльцы и размещение', 'Скорость', 'Применённый профиль', 'Фактическая ширина, мм',
                                        'Фактическая высота, мм', 'Щели', 'Стягивание', 'Смещение обводки, мм', 'Обрывы',
                                        'Изнанка и переходы', 'Заметки')), encoding='utf-8')
    cards, rows = [], []
    for r in results:
        n, measures = r['name'], r['independent_dst']
        dims = ' × '.join(str(x) for x in r['actual_stitched_dimensions_mm'])
        rows.append(f"| {n} | {dims} | {measures['needle_count']} | {measures['color_changes']} | {r['digital_checks_passed']} |")
        colors = list(dict.fromkeys(tuple(s['color_rgb']) for s in r['project_settings']))
        swatches = ' '.join(f'<span style="display:inline-block;padding:4px;border-left:20px solid rgb{color}">{color}</span>' for color in colors)
        cards.append(f'<article><h2>{html.escape(n)}</h2><p>{html.escape(r["description"])}; фактический размер стежков {dims} мм.</p><p>{swatches}</p><img src="{n}/preview.png"><p><a href="{n}/design.elab">ELAB</a> · <a href="{n}/design.dst">DST</a> · <a href="{n}/design.threads.json">Порядок цветов</a> · <a href="{n}/verification.json">Проверки и параметры</a></p></article>')
    text = ('# Пробная отшивка — экспериментальный пакет\n\n'
            'Физическая отшивка не проводилась. Модель машины, пяльцы, допустимые переходы и параметры нити/ткани не заданы. '
            'Профиль в исходном проекте — общая геометрическая граница 12 мм; размеры пялец и max_jump отсутствуют. '
            'Сначала подтвердите профиль и размещение в реальных пяльцах. Геометрические проверки не являются разрешением запуска на неизвестной машине.\n\n'
            'Для новых образцов явно выбран KeepThread: возможность автоматической обрезки не предполагается. Схема цветов хранится рядом с каждым DST. '
            'Все исходные ELAB, настройки и реальный локальный фрагмент сохранены в sources/.\n\n'
            'Фрагмент головы коня готовится как отдельный двухцветный мотив: закреплены чёрная и белая нити, красный внешний фон исключён явно. '
            'Это выбор схемы конкретного образца; уменьшение палитры не считается доказательством улучшения алгоритма и не применяется к сравнению полного герба.\n\n'
            'Сравнение 01 → 01b меняет только spacing_mm: 0,40 → 0,45 мм. Подложка, длины, компенсация, закрепки и геометрия одинаковы. '
            'При наличии 06b сравнение 06 → 06b меняет только режим подготовки Legacy → Standard; исходник, физический размер, маски, палитра и параметры стежка одинаковы. '
            'У остальных образцов разные геометрические задачи; это не многофакторное сравнение качества.\n\n'
            '| Образец | Стежковый размер, мм | Проколы | Смены цвета | Цифровые проверки |\n| --- | --- | --- | --- | --- |\n'
            +'\n'.join(rows)+'\n\n'
            'Проверены PyEmbroidery: координаты всех проколов, управляющие команды и их позиции, конечная запись, длины, размеры и повторная генерация. '
            'Каждый прошиваемый сегмент целиком проверен по явной геометрии; допускается только 0,075 мм для квантования DST, компенсация равна нулю. '
            'Покрытие верхним слоем и локальная длина нити приведены отдельно по ролям и являются геометрическими оценками.\n\n'
            'Таблица параметров всех объектов — parameters.csv. Совместимость старых проектов проверена отдельно в compatibility/. '
            'Если старый проект отвергается также исходной сборкой из-за самопересечения, это явно отмечено; такое поведение не считается успешно выполненной генерацией.\n\n'
            'Заполните observations.csv после отшивки: материал, стабилизатор, нить, игла, пяльцы, скорость, профиль, фактические размеры, щели, стягивание, смещение обводки, обрывы и заметки. '
            'Поля фактических наблюдений намеренно пусты.\n')
    (output/'README.md').write_text(text, encoding='utf-8')
    (output/'index.html').write_text('<!doctype html><meta charset="utf-8"><title>Пробная отшивка</title><style>body{font:16px/1.5 system-ui;max-width:1400px;margin:30px auto;padding:0 20px}article{border:1px solid #bbb;margin:20px 0;padding:20px}img{max-width:100%}</style><h1>Экспериментальный — требует отшивки</h1><p>Машина и пяльцы не подтверждены. Цифровая геометрия проверена; ткань не тестировалась.</p><a href="README.md">Параметры и порядок проверки</a> · <a href="observations.csv">Форма наблюдений</a>'+''.join(cards), encoding='utf-8')
    print(json.dumps(dict(digital_checks_passed=summary['digital_checks_passed'], cases=len(results),
                          machine_profile_verified=False, physical_sewout=False, output=str(output)), ensure_ascii=False))
    return 0 if summary['digital_checks_passed'] else 1


if __name__ == '__main__':
    raise SystemExit(main())
