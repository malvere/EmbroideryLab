"""Verify immutable real RGBA sources and exact saved-ELAB regeneration."""
import argparse
import io
import json
from pathlib import Path

from PIL import Image
import quality_phase2_validation as q


def main():
    parser=argparse.ArgumentParser()
    parser.add_argument("--cli",type=Path,required=True)
    args=parser.parse_args()
    root=q.DEFAULT.parent
    cases=json.loads((q.DEFAULT/"fixtures.json").read_text(encoding="utf-8"))["cases"]
    rows=[]
    for case in cases:
        if case["group"] != "real_unlabelled":
            continue
        source=Path(case["source"])
        original=Image.open(source).convert("RGBA")
        folder=q.DEFAULT/"candidate"/case["name"]
        project=json.loads((folder/"design.elab").read_text(encoding="utf-8"))
        embedded=Image.open(io.BytesIO(bytes.fromhex(project["source_image"]["encoded"]))).convert("RGBA")
        output=root/"real-roundtrip"/case["name"]
        output.mkdir(parents=True,exist_ok=True)
        execution=q.measured_run([args.cli.resolve(),"generate",folder/"design.elab",output/"regenerated.dst"],output)
        q.write(output/"command.json",execution)
        result=dict(name=case["name"],source_dimensions_px=list(original.size),
                    original_source_sha256=q.av.sha(source),embedded_rgba_exact=original.size==embedded.size and original.tobytes()==embedded.tobytes(),
                    original_unchanged=q.av.sha(source)==json.loads((folder/"evaluation.json").read_text(encoding="utf-8"))["input_sha256"],
                    regenerate_exit_code=execution["exit_code"],
                    saved_elab_regenerates_same_dst=execution["exit_code"]==0 and q.av.sha(output/"regenerated.dst")==q.av.sha(folder/"design.dst"),
                    candidate_cli_sha256=q.av.sha(args.cli))
        result["passed"]=result["embedded_rgba_exact"] and result["original_unchanged"] and result["saved_elab_regenerates_same_dst"]
        rows.append(result)
        print(case["name"],result["passed"],flush=True)
    q.write(root/"real-roundtrip"/"summary.json",rows)
    if not all(r["passed"] for r in rows):
        raise SystemExit(1)


if __name__=="__main__":
    main()
