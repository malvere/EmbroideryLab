"""Generation-only source packaging equivalence against frozen Phase 2.1.

All outputs live separately from the frozen candidate. No trace/evaluation
rerender, input redraw, tuning or oracle editing is performed. Differences in
diagnostic text remain explicit and never count as byte equality.
"""
from __future__ import annotations

import argparse
from collections import Counter
import json
from pathlib import Path
import re
import sys

import autodigitize_validation as av
import quality_phase2_validation as q

ROOT=Path(__file__).resolve().parents[1]
PHASE=ROOT/"outputs/quality-phase21-20261106"
OLD=ROOT/"outputs/quality-phase2-20261106"
SUFFIXES=("dst","stitches.json","elab","plan.json")
MISSING={"__missing_field__":True}


def read(path):
    return json.loads(path.read_text(encoding="utf-8-sig"))


def differences(expected,actual,path="$"):
    """Complete structural field differences; array order is significant."""
    if expected==actual:
        return []
    if isinstance(expected,dict) and isinstance(actual,dict):
        rows=[]
        for key in sorted(expected.keys()|actual.keys()):
            if key not in expected or key not in actual:
                rows.append(dict(path=path+"."+key,expected=expected.get(key,MISSING),actual=actual.get(key,MISSING)))
            else:
                rows.extend(differences(expected[key],actual[key],path+"."+key))
        return rows
    if isinstance(expected,list) and isinstance(actual,list):
        rows=[]
        for i,(a,b) in enumerate(zip(expected,actual)):
            rows.extend(differences(a,b,f"{path}[{i}]"))
        if len(expected)!=len(actual):
            rows.append(dict(path=path+".length",expected=len(expected),actual=len(actual)))
            for i in range(min(len(expected),len(actual)),max(len(expected),len(actual))):
                rows.append(dict(path=f"{path}[{i}]",expected=expected[i] if i<len(expected) else MISSING,
                                 actual=actual[i] if i<len(actual) else MISSING))
        return rows
    return [dict(path=path,expected=expected,actual=actual)]


def uncertainty_text_only(row):
    path=row["path"]
    text_path=(re.fullmatch(r"\$\.warnings\[\d+\]\.message",path)
               or re.fullmatch(r"\$\.raster_recipe\.issues\[\d+\]\.message",path)
               or re.fullmatch(r"\$\.raster_recipe\.warnings\[\d+\]",path))
    a,b=row["expected"],row["actual"]
    pattern=r"\d+ ambiguous narrow regions retained"
    return bool(text_path and isinstance(a,str) and isinstance(b,str)
                and re.search(pattern,a) and re.search(pattern,b)
                and re.sub(pattern,"<count> ambiguous narrow regions retained",a)
                    ==re.sub(pattern,"<count> ambiguous narrow regions retained",b))


def compare(expected,actual,folder):
    result={}
    for suffix in SUFFIXES:
        a,b=expected/("design."+suffix),actual/("design."+suffix)
        row=dict(expected_path=str(a.resolve()),actual_path=str(b.resolve()),
                 expected_sha256=av.sha(a),actual_sha256=av.sha(b),bytes_equal=a.read_bytes()==b.read_bytes())
        if suffix!="dst":
            delta=differences(read(a),read(b))
            row.update(json_equal=not delta,field_diff_count=len(delta),
                       uncertainty_warning_only=bool(delta) and all(uncertainty_text_only(d) for d in delta))
            if delta:
                diff_path=folder/("diff-"+suffix)
                q.write(diff_path,dict(field_diff_count=len(delta),differences=delta))
                row["complete_field_differences"]=str(diff_path.resolve())
                row["diff_paths"]=[d["path"] for d in delta]
        result[suffix]=row
    plan_a,plan_b=read(expected/"design.plan.json"),read(actual/"design.plan.json")
    project_a,project_b=read(expected/"design.elab"),read(actual/"design.elab")
    exact=all(r["bytes_equal"] for r in result.values())
    strict_machine=(result["dst"]["bytes_equal"] and result["stitches.json"]["bytes_equal"]
                    and plan_a["stitches"]==plan_b["stitches"] and plan_a["metadata"]==plan_b["metadata"])
    nondiagnostic_equal=all(r["bytes_equal"] or r.get("json_equal",False) or r.get("uncertainty_warning_only",False)
                            for r in result.values())
    preservation={k:project_a.get(k)==project_b.get(k) for k in ("objects","source_image","machine_profile","width_mm","name","version")}
    return dict(files=result,all_outputs_byte_equal=exact,strict_machine_output_equal=strict_machine,
        all_nondiagnostic_fields_equal=nondiagnostic_equal,preservation_fields_equal=preservation,
        verdict="EXACT" if exact else "EXPLICIT_UNCERTAINTY_WARNING_DIFF" if strict_machine and nondiagnostic_equal
                else "DIFFERENCE_REQUIRES_REVIEW")


def execute(cli,source,options,width,folder,timeout):
    folder.mkdir(parents=True,exist_ok=True)
    options_path=folder/"options.json"
    q.write(options_path,options)
    original_sha=av.sha(source)
    command=[cli,"autodigitize",source,folder/"design.dst",width,options_path]
    run=q.measured_run(command,folder,timeout=timeout)
    q.write(folder/"command.json",run)
    if run["exit_code"]!=0:
        raise RuntimeError(f"Generation failed ({source}); see {folder/'command.json'}")
    unchanged=original_sha==av.sha(source)
    if not unchanged:
        raise RuntimeError(f"Source changed during generation: {source}")
    return dict(source=str(source.resolve()),source_sha256=original_sha,source_unchanged=unchanged,
                options_sha256=q.value_hash(options),cli_sha256=av.sha(cli),execution=run)


def existing_fields_preserved(original,current):
    if isinstance(original,dict):
        return isinstance(current,dict) and all(k in current and existing_fields_preserved(v,current[k]) for k,v in original.items())
    if isinstance(original,list):
        return isinstance(current,list) and len(original)==len(current) and all(existing_fields_preserved(a,b) for a,b in zip(original,current))
    return original==current


def legacy_smoke(cli,legacy_cli,out,timeout):
    rows=[]
    crest=next(c for c in q.load_frozen_manifest(OLD/"comparison/fixtures.json") if c["name"]=="user_crest")
    folder=out/"legacy/crest"
    config=dict(crest["options"]);config.pop("preparation",None)
    row=execute(cli,Path(crest["source"]),config,crest["width_mm"],folder,timeout)
    row.update(case="legacy_crest",comparison=compare(OLD/"comparison/baseline/user_crest",folder,folder))
    rows.append(row)
    micro=read(PHASE/"micro/fixtures.json")
    # Two fixed examples are replayed, never selected or adjusted from results.
    for case in micro["cases"]:
        source=Path(case["source"])
        config=dict(case["options"]);config.pop("preparation",None)
        base=out/"legacy"/case["name"]
        old,new=base/"old_cli",base/"current_cli"
        old_execution=execute(legacy_cli,source,config,case["width_mm"],old,timeout)
        row=execute(cli,source,config,case["width_mm"],new,timeout)
        row.update(case="legacy_"+case["name"],old_cli_execution=old_execution,comparison=compare(old,new,base))
        rows.append(row)
    source=OLD/"sewout/sources/02_ring_hole.elab"
    initial_sha=av.sha(source)
    base=out/"legacy/existing_manual_elab"
    for label,executable in (("old_cli",legacy_cli),("current_cli",cli)):
        folder=base/label;folder.mkdir(parents=True,exist_ok=True)
        run=q.measured_run([executable,"generate",source,folder/"design.dst"],folder,timeout=timeout)
        q.write(folder/"command.json",run)
        if run["exit_code"]!=0:
            raise RuntimeError(f"Old manual ELAB generation failed: {folder}")
    saved=read(base/"current_cli/design.elab");original=read(source)
    row=dict(case="existing_manual_elab",source=str(source.resolve()),source_sha256=initial_sha,
        source_unchanged=initial_sha==av.sha(source),comparison=compare(base/"old_cli",base/"current_cli",base),
        original_objects_preserved=existing_fields_preserved(original["objects"],saved["objects"]),
        original_source_image_preserved=original.get("source_image")==saved.get("source_image"),
        original_recipe_preserved=original.get("raster_recipe")==saved.get("raster_recipe"))
    rows.append(row)
    return rows


def main():
    parser=argparse.ArgumentParser()
    parser.add_argument("--cli",type=Path,required=True)
    parser.add_argument("--frozen",type=Path,default=PHASE/"candidate")
    parser.add_argument("--legacy-cli",type=Path,default=PHASE/"initial/legacy.exe")
    parser.add_argument("--output",type=Path)
    parser.add_argument("--timeout",type=float,default=360.)
    parser.add_argument("--skip-legacy-smoke",action="store_true",help="Report explicitly that legacy smoke was not executed in this run")
    args=parser.parse_args()
    cli=args.cli.resolve();initial_cli_sha=av.sha(cli)
    output=(args.output or PHASE/"source-equivalence"/cli.stem).resolve()
    if output.exists():
        raise RuntimeError(f"Evidence folder already exists; choose a fresh --output: {output}")
    output.mkdir(parents=True)
    cases=[]
    for group in ("comparison","comparison-unpinned","comparison-paired"):
        manifest=OLD/group/"fixtures.json"
        for case in q.load_frozen_manifest(manifest):
            frozen=args.frozen/group/"candidate"/case["name"]
            config=read(frozen/"options.json")
            folder=output/group/case["name"]
            source=Path(case["source"])
            q.write(output/"execution-contract.json",dict(cli=str(cli),cli_sha256=initial_cli_sha,
                script_sha256=av.sha(Path(__file__)),operation="generation-only; no trace/evaluation/tuning/oracle edits",
                expected_frozen_root=str(args.frozen.resolve()),legacy_smoke_requested=not args.skip_legacy_smoke))
            run=execute(cli,source,config,case["width_mm"],folder,args.timeout)
            comparison=compare(frozen,folder,folder)
            row=dict(name=case["name"],group=group,manifest_sha256=av.sha(manifest),**run,comparison=comparison)
            cases.append(row)
            q.write(output/"candidate-progress.json",cases)
            print(group,case["name"],comparison["verdict"],flush=True)
    smoke=[] if args.skip_legacy_smoke else legacy_smoke(cli,args.legacy_cli.resolve(),output,args.timeout)
    preserved=all(r["comparison"]["strict_machine_output_equal"] and r["comparison"]["all_nondiagnostic_fields_equal"] for r in cases+smoke)
    preserved &= all(r.get("original_objects_preserved",True) and r.get("original_source_image_preserved",True)
                     and r.get("original_recipe_preserved",True) and r["source_unchanged"] for r in smoke)
    summary=dict(command_line=sys.orig_argv,script_sha256=av.sha(Path(__file__)),cli=str(cli),cli_sha256=initial_cli_sha,
        cli_unchanged=initial_cli_sha==av.sha(cli),python=sys.version,candidate_cases=cases,legacy_smoke=smoke,
        candidate_case_count=len(cases),candidate_exact_output_cases=sum(r["comparison"]["all_outputs_byte_equal"] for r in cases),
        verdict_counts=dict(Counter(r["comparison"]["verdict"] for r in cases)),
        legacy_smoke_executed=not args.skip_legacy_smoke,
        all_candidate_machine_outputs_equal=all(r["comparison"]["strict_machine_output_equal"] for r in cases),
        all_candidate_nondiagnostic_fields_equal=all(r["comparison"]["all_nondiagnostic_fields_equal"] for r in cases),
        passed=bool(preserved and initial_cli_sha==av.sha(cli)))
    q.write(output/"summary.json",summary)
    print(json.dumps({k:v for k,v in summary.items() if k not in ("candidate_cases","legacy_smoke")}),flush=True)
    return 0 if summary["passed"] else 1


if __name__=="__main__":
    raise SystemExit(main())
