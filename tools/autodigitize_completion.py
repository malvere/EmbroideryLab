"""Independent before/after checks for the final algorithm-completion cases."""
from __future__ import annotations
import argparse
from collections import Counter
import html
import json
import math
from pathlib import Path
import autodigitize_validation as av


def rect_length(a, b, bounds):
    lo, hi = 0., 1.
    for x, delta, low, high in ((a[0], b[0]-a[0], bounds[0], bounds[2]),
                                (a[1], b[1]-a[1], bounds[1], bounds[3])):
        if abs(delta) < 1e-12:
            if not low < x < high:
                return 0.
        else:
            p, q = (low-x)/delta, (high-x)/delta
            lo, hi = max(lo, min(p, q)), min(hi, max(p, q))
            if hi <= lo:
                return 0.
    return math.dist(a, b)*max(0., hi-lo)


def main():
    from PIL import Image
    import pyembroidery as pe
    parser = argparse.ArgumentParser()
    parser.add_argument('--baseline', required=True, type=Path)
    parser.add_argument('--candidate', required=True, type=Path)
    parser.add_argument('--output', type=Path, default=av.ROOT/'outputs/autodigitize/completion/probes')
    args = parser.parse_args()
    out = args.output.resolve()
    fixtures = out/'fixtures'
    fixtures.mkdir(parents=True, exist_ok=True)
    lower = av.obj('Lower fill', [av.rect(0, 0, 30, 20)], underlay=True)
    upper = av.obj('Upper detail', [av.rect(10, 5, 10, 10)])
    upper.update(id=2, color=[215, 155, 25])
    av.write_json(fixtures/'layers.elab', dict(version=5, name='Layer cutout', objects=[lower, upper], width_mm=30))
    av.write_json(fixtures/'sparse.elab', dict(version=5, name='Exposed travel', objects=[av.obj('Sparse rows', [av.rect(0, 0, 30, 20)], spacing_mm=2.)], width_mm=30))
    Image.new('RGBA', (128, 128), (190, 30, 40, 255)).save(fixtures/'square.png')
    av.write_json(fixtures/'square-options.json', dict(max_dimension=128, palette_size=2,
                  background='TransparentOnly', min_region_pixels=1,
                  simplify_tolerance_mm=.1, smoothing_iterations=1))
    results = {}
    for label in ('baseline', 'candidate'):
        cli = getattr(args, label).resolve()
        values = {}
        for name in ('layers', 'sparse', 'square'):
            folder = out/label/name
            folder.mkdir(parents=True, exist_ok=True)
            target = folder/'design.dst'
            command = [cli, 'generate', fixtures/(name+'.elab'), target]
            if name == 'square':
                command = [cli, 'generate-with-options', fixtures/'square.png', target, '100', fixtures/'square-options.json']
            execution = av.run(command)
            av.write_json(folder/'command.json', execution)
            assert execution['exit_code'] == 0, execution
            saved = json.loads(target.with_suffix('.elab').read_text(encoding='utf-8-sig'))
            plan = json.loads(target.with_suffix('.plan.json').read_text(encoding='utf-8-sig'))
            generated = [{**s, **m} for s, m in zip(plan['stitches'], plan['metadata'])]
            checks, records = av.inspect(target, generated, saved, pe, {})
            assert checks['format_geometry_passed'], checks
            needles = [s for s in generated if s['command'] == 'Stitch']
            previous, index = (0., 0.), 0
            role_lengths = Counter()
            inside_hidden = 0.
            top_segments = []
            for x, y, flag in records:
                position = (x, y)
                if flag == pe.STITCH:
                    item = needles[index]
                    index += 1
                    role_lengths[item['role']] += math.dist(previous, position)
                    if item['object_id'] == 1:
                        inside_hidden += rect_length(previous, position, (10.25, 5.25, 19.75, 14.75))
                    if item['role'] == 'TopFill':
                        top_segments.append(tuple(sorted((previous, position))))
                if flag in (pe.STITCH, pe.JUMP):
                    previous = position
            points = [ring for obj in saved['objects'] for ring in obj.get('contours', [])]
            area = sum(abs(sum(a['x']*b['y']-b['x']*a['y'] for a, b in zip(ring, ring[1:]+ring[:1]))/2) for ring in points)
            result = dict(exe_sha256=av.sha(cli), seconds=execution['seconds'],
                          independent_dst=checks, role_lengths_mm=dict(role_lengths),
                          lower_sewn_length_inside_hidden_core_mm=inside_hidden,
                          source_contour_area_mm2=area, top_segment_multiset=sorted(top_segments))
            av.render(folder/'comparison.png', saved, generated, records, pe, name+' — '+label)
            av.write_json(folder/'evaluation.json', result)
            values[name] = result
        results[label] = values
    before, after = results['baseline'], results['candidate']
    assert before['layers']['lower_sewn_length_inside_hidden_core_mm'] > 100.
    assert after['layers']['lower_sewn_length_inside_hidden_core_mm'] < 1e-8
    assert before['sparse']['role_lengths_mm'].get('Travel', 0.) > 0.
    assert after['sparse']['role_lengths_mm'].get('Travel', 0.) == 0.
    assert before['sparse']['top_segment_multiset'] == after['sparse']['top_segment_multiset']
    assert abs(before['square']['source_contour_area_mm2']-8750.) < 1e-6
    assert abs(after['square']['source_contour_area_mm2']-10000.) < 1e-6
    for values in results.values():
        for value in values.values():
            value.pop('top_segment_multiset')
    av.write_json(out/'summary.json', dict(passed=True, scope='Synthetic algorithm cases; independent PyEmbroidery; no sew-out', results=results))
    rows = []
    for name in ('layers', 'sparse', 'square'):
        images = ''.join(f'<figure><img src="{label}/{name}/comparison.png"><figcaption>{html.escape(label)}</figcaption></figure>' for label in ('baseline', 'candidate'))
        rows.append(f'<section><h2>{name}</h2><div>{images}</div></section>')
    (out/'index.html').write_text('<!doctype html><meta charset="utf-8"><title>Algorithm completion comparison</title><style>body{font:16px system-ui;margin:24px;background:#f4f5f6;color:#202a34}section>div{display:flex;gap:20px}figure{margin:0;width:48%}img{max-width:100%}</style><h1>Actual decoded stitches: previous / completed algorithms</h1><p>Identical inputs and settings; original contours remain editable. Images use the same technical renderer. <a href="summary.json">Measured results and binary hashes</a>. No fabric claim.</p>'+''.join(rows), encoding='utf-8')
    print(json.dumps(dict(passed=True, cases=3, output=str(out)), indent=2))


if __name__ == '__main__':
    main()
