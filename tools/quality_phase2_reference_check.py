"""Check trial DST against the authored source ELAB, independently of saving.

This second pass prevents a changed output polygon from becoming its own
permitted-area reference. It reads the original authored geometry and uses
PyEmbroidery plus the analytic whole-segment checker from the trial harness.
"""
from __future__ import annotations

import argparse
import json
from pathlib import Path
import sys

import autodigitize_validation as av
import quality_phase2_sewout as sq


def original_fields_equal(before, after):
    if isinstance(before, dict):
        return isinstance(after, dict) and all(key in after and original_fields_equal(value, after[key])
                                               for key, value in before.items())
    if isinstance(before, list):
        return isinstance(after, list) and len(before) == len(after) and all(original_fields_equal(a, b)
                                                                            for a, b in zip(before, after))
    return before == after


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--packet', type=Path, required=True)
    args = parser.parse_args()
    import pyembroidery as pe
    packet = args.packet.resolve()
    summary = json.loads((packet/'summary.json').read_text(encoding='utf-8'))
    results = []
    for entry in summary['cases']:
        folder = packet/entry['name']
        reference = json.loads(Path(entry['source']).read_text(encoding='utf-8'))
        saved = json.loads((folder/'design.elab').read_text(encoding='utf-8'))
        plan = json.loads((folder/'design.plan.json').read_text(encoding='utf-8'))
        generated = [{**s, **m} for s, m in zip(plan['stitches'], plan['metadata'])]
        records = [(x/10, y/10, pe.decode_embroidery_command(code)[0])
                   for x, y, code in pe.read(str(folder/'design.dst')).stitches]
        segments = sq.whole_segment_checks(reference, generated, records, pe)
        source_preserved = original_fields_equal(reference['objects'], saved['objects'])
        result = dict(name=entry['name'], authored_source=str(Path(entry['source']).resolve()),
                      authored_source_sha256=av.sha(Path(entry['source'])),
                      original_object_decisions_preserved=source_preserved,
                      original_embedded_source_preserved=reference.get('source_image') == saved.get('source_image'),
                      whole_segments_against_original_reference=segments,
                      passed=source_preserved and segments['passed'] and reference.get('source_image') == saved.get('source_image'))
        av.write_json(folder/'authored-reference-verification.json', result)
        results.append(result)
    report = dict(command_line=[sys.executable]+sys.argv, script_sha256=av.sha(Path(__file__)),
                  passed=all(item['passed'] for item in results), cases=results,
                  allowance_mm=sq.ALLOWANCE_MM, physical_sewout_performed=False)
    av.write_json(packet/'authored-reference-summary.json', report)
    print(json.dumps(dict(passed=report['passed'], cases=len(results),
                          checked_segments=sum(item['whole_segments_against_original_reference']['checked_sewn_segments'] for item in results))))
    return 0 if report['passed'] else 1


if __name__ == '__main__':
    raise SystemExit(main())
