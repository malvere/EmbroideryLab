"""Aligned, fixed-scale Phase 2 contact sheet and concise comparison tables."""
from __future__ import annotations

import argparse
import csv
import json
from pathlib import Path

import quality_phase2_validation as q


def load(path):
    return json.loads(path.read_text(encoding="utf-8-sig"))


def supplement_baseline(output):
    """Recompute corrected independent reference rendering, without regenerating plans."""
    cases=load(output/"fixtures.json")["cases"]
    results=[]
    for case in cases:
        folder=output/"baseline"/case["name"]
        if not (folder/"evaluation.json").is_file():
            continue
        result=load(folder/"evaluation.json")
        if result["exit_code"] == 0:
            project=load(folder/"design.elab")
            source_project={**project,"objects":[{**o,"contours":q.source_rings(o)} for o in project["objects"]]}
            result["geometry_topology"]=q.av.topology(source_project)
            result["retained_source_rings"]=sum(len(q.source_rings(o)) for o in project["objects"])
            if "truth" in case:
                result["reference"]=q.reference_metrics(case,project,folder)
            q.write(folder/"evaluation.json",result)
        results.append(result)
    q.write(output/"baseline.json",results)
    q.report(output,cases)


def contact_sheet(output,selected):
    from PIL import Image,ImageDraw,ImageFont
    manifest=load(output/"fixtures.json")
    cases=[c for c in manifest["cases"] if c["name"] in selected]
    font_path=Path("C:/Windows/Fonts/segoeui.ttf")
    font=ImageFont.truetype(str(font_path),16)
    small=ImageFont.truetype(str(font_path),13)
    cell_w,cell_h=310,440
    sheet=Image.new("RGB",(cell_w*5,cell_h*len(cases)+100),"#f2f3f1")
    d=ImageDraw.Draw(sheet)
    d.text((18,12),"EmbroideryLab Phase 2 — aligned source coordinates",font=font,fill="#243343")
    d.text((18,40),"Same source, physical size, palette and stitch settings per pair. Red = reassigned raster pixels. Experimental; no fabric sew-out.",font=small,fill="#344454")
    titles=["Source","Baseline regions","Prepared regions","Changed pixels","Needle paths: before / after"]
    for row,case in enumerate(cases):
        name=case["name"]
        y0=100+row*cell_h
        d.text((14,y0),f'{name} — {case["width_mm"]} mm',font=font,fill="#243343")
        region_paths=[]
        for label in ("baseline","candidate"):
            folder=output/label/name
            region_paths.append(next((folder/file for file in ("actual-thread-colors.png","retained-source-geometry.png","prepared-labels-after.png")
                                      if (folder/file).is_file())))
        paths=[Path(case["source"]),*region_paths,output/"candidate"/name/"changed-pixels.png"]
        source=Image.open(paths[0]).convert("RGBA")
        if source.width > 800 or source.height > 800:
            source.thumbnail((800,800),Image.Resampling.LANCZOS)
        width,height=source.size
        factor=min((cell_w-22)/width,(cell_h-85)/height)
        target=(round(width*factor),round(height*factor))
        for column,path in enumerate(paths):
            if not path.is_file():
                continue
            image=Image.open(path).convert("RGBA")
            image=image.resize(target,Image.Resampling.NEAREST if column else Image.Resampling.LANCZOS)
            px=column*cell_w+(cell_w-target[0])//2
            py=y0+55
            d.text((column*cell_w+10,y0+28),titles[column],font=small,fill="#344454")
            # Neutral checker shows transparency; it is display only and never
            # used in colour learning, masks, reference metrics or generation.
            checker=Image.new("RGBA",target,"#e4e8e5")
            cd=ImageDraw.Draw(checker)
            for cy in range(0,target[1],10):
                for cx in range(0,target[0],10):
                    if (cx//10+cy//10)%2:
                        cd.rectangle((cx,cy,cx+9,cy+9),fill="#d8ddda")
            checker.alpha_composite(image)
            sheet.paste(checker.convert("RGB"),(px,py))
        # Needle paths are plotted into one full-source physical viewport with
        # the SAME transform. Both subpanels use 0.167mm technical line width.
        d.text((4*cell_w+10,y0+28),titles[4],font=small,fill="#344454")
        for index,label in enumerate(("baseline","candidate")):
            project=load(output/label/name/"design.elab")
            plan=load(output/label/name/"design.plan.json")
            mm_w=case["width_mm"]
            mm_h=project["source_image"]["height_mm"]
            high=Image.new("RGB",(round(mm_w*6)+1,round(mm_h*6)+1),"#e3e7e4")
            hd=ImageDraw.Draw(high)
            previous=(0.,0.)
            for s in plan["stitches"]:
                point=(s["x"],s["y"])
                if s["command"] == "Stitch":
                    hd.line((previous[0]*6,previous[1]*6,point[0]*6,point[1]*6),fill=tuple(s["color"]),width=1)
                if s["command"] in ("Stitch","Jump"):
                    previous=point
            high.thumbnail((cell_w-24,(cell_h-110)//2),Image.Resampling.LANCZOS)
            px=4*cell_w+(cell_w-high.width)//2
            py=y0+58+index*(cell_h-80)//2
            sheet.paste(high,(px,py))
            d.text((4*cell_w+10,py-15),label,font=small,fill="#344454")
    sheet.save(output/"contact-sheet.png")


def supplement_real_coverage(output):
    cases=[c for c in load(output/"fixtures.json")["cases"] if c["group"] == "real_unlabelled"]
    for label in ("baseline","candidate"):
        summary=load(output/(label+".json"))
        by_name={r["name"]:r for r in summary}
        for case in cases:
            folder=output/label/case["name"]
            result=by_name[case["name"]]
            stage=load(folder/"design.trace.json")
            w,h=stage["trace_dimensions"]
            sw,sh=stage["source_dimensions"]
            height=case["width_mm"]*sh/sw
            config=case["options"]
            required=q.threshold_witness(stage["labels_before_preparation"],w,h,config["min_region_pixels"],config["min_region_area_mm2"],case["width_mm"]*height/(w*h))
            plan=load(folder/"design.plan.json")
            generated=[{**s,**m} for s,m in zip(plan["stitches"],plan["metadata"],strict=True)]
            result["sampled_top_coverage"]=q.witness_top_coverage(required,w,h,case["width_mm"],height,generated)
            q.write(folder/"evaluation.json",result)
        q.write(output/(label+".json"),summary)
    q.report(output,load(output/"fixtures.json")["cases"])


def table(root):
    rows=[]
    for group in ("comparison","comparison-unpinned","comparison-paired"):
        output=root/group
        if not (output/"candidate.json").is_file():
            continue
        before={r["name"]:r for r in load(output/"baseline.json")}
        after={r["name"]:r for r in load(output/"candidate.json")}
        for name,b in after.items():
            a=before.get(name)
            if not a or a["exit_code"] != 0 or b["exit_code"] != 0:
                continue
            dims=b.get("stages",{}).get("source_dimensions")
            if not dims:
                from PIL import Image
                source=next(c["source"] for c in load(output/"fixtures.json")["cases"] if c["name"]==name)
                dims=list(Image.open(source).size)
            design=[b["width_mm"],b["width_mm"]*dims[1]/dims[0]]
            row=dict(case=name,group=b["group"],seed=q.SEED,width_mm=b["width_mm"],
                     source_dimensions_px=dims,design_mm=design,source_sha256=b["input_sha256"],
                     same_source_hash=a["input_sha256"]==b["input_sha256"])
            for label,r in (("before",a),("after",b)):
                dst=r["independent_dst"]
                option_path=(output/("baseline" if label=="before" else "candidate")/name/"options.json").resolve()
                row[f'options_path_{label}']=str(option_path)
                row[f'options_sha256_{label}']=q.av.sha(option_path)
                row[f'preparation_version_{label}']=r.get("stages",{}).get("preparation_version")
                bounds=dst.get("bounds_mm")
                row[f'sewn_bounds_mm_{label}']=bounds
                row[f'sewn_dimensions_mm_{label}']=[bounds[2]-bounds[0],bounds[3]-bounds[1]] if bounds else None
                row.update({f'{key}_{label}':r.get(key) for key in ("objects","needle_blocks","seconds","peak_working_set_bytes")})
                row.update({f'{key}_{label}':dst.get(key) for key in ("needle_count","planned_jump_count","jump_length_mm","trim_intentions","color_changes","short_stitches_below_03_mm")})
                row[f'components_{label}']=r.get("stages",{}).get("components_after_preparation")
                row[f'omitted_pixels_{label}']=r.get("stages",{}).get("omitted_pixel_count")
                ref=r.get("reference")
                row[f'semantic_reference_pass_{label}']=ref.get("accepted") if ref else None
                row[f'protected_core_retained_{label}']=ref.get("protected_core_retained") if ref else None
                row[f'actual_core_oklab_error_max_{label}']=ref.get("max_core_actual_thread_oklab_error") if ref else None
                full=r.get("whole_sewn_segment_containment")
                row[f'whole_sewn_segments_checked_{label}']=full.get("checked") if full else None
                row[f'whole_sewn_segments_complete_{label}']=full.get("checked")==dst["needle_count"] if full else None
                row[f'whole_sewn_segment_violations_{label}']=len(full["violations"]) if full else None
                coverage=r.get("sampled_top_coverage")
                row[f'top_only_sampled_coverage_{label}']=coverage.get("covered_fraction") if isinstance(coverage,dict) else None
            row["original_retained_raster_pixels_missing_after"]=b.get("original_retained_raster_witness",{}).get("missing_pixels")
            rows.append(row)
    with (root/"comparison-table.csv").open("w",encoding="utf-8-sig",newline="") as f:
        writer=csv.DictWriter(f,fieldnames=list(rows[0]))
        writer.writeheader()
        writer.writerows(rows)
    q.write(root/"comparison-table.json",rows)
    return rows


def consolidate_summary(root):
    groups,cases=[],[]
    for group in ("comparison","comparison-unpinned","comparison-paired"):
        source=load(root/group/"summary.json")
        versions=source["versions"]
        all_runs=[r for rows in versions.values() for r in rows]
        synthetic=[r for r in all_runs if r.get("reference")]
        candidate=versions["candidate"]
        groups.append(dict(group=group,cases=len(candidate),
                    independent_dst_all_pass=all(r["independent_dst"]["format_geometry_passed"] for r in all_runs),
                    whole_segment_runs=len(synthetic),
                    whole_segment_runs_complete=all(r["whole_sewn_segment_containment"]["checked"]==r["independent_dst"]["needle_count"] for r in synthetic),
                    whole_segment_violations=sum(len(r["whole_sewn_segment_containment"]["violations"]) for r in synthetic),
                    candidate_reference_pass=sum(bool(r.get("reference") and r["reference"]["accepted"]) for r in candidate),
                    candidate_reference_review=sum(bool(r.get("reference") and not r["reference"]["accepted"]) for r in candidate),
                    detail_regression_checks=len(source["detail_regression"]),
                    detail_regression_pass=all(r["passed"] for r in source["detail_regression"])))
        cases.extend({key:r.get(key) for key in ("name","group","input_sha256","objects","reference",
                    "original_retained_raster_witness","independent_dst","stages")} for r in candidate)
    q.write(root/"quality-summary.json",dict(groups=groups,case_count=len(cases),
                candidate_cli_sha256=versions["candidate"][0]["cli_sha256"],physical_sewout=False,cases=cases))


def main():
    parser=argparse.ArgumentParser()
    parser.add_argument("--root",type=Path,default=q.DEFAULT.parent)
    args=parser.parse_args()
    supplement_baseline(args.root/"comparison")
    supplement_real_coverage(args.root/"comparison")
    cases=load(args.root/"comparison"/"fixtures.json")["cases"]
    contact_sheet(args.root/"comparison",["user_crest","complex_emblem","flat_logo","thin_outline_parallel_bridge_rare"])
    rows=table(args.root)
    consolidate_summary(args.root)
    print("Wrote aligned contact sheet and",len(rows),"comparison rows")


if __name__ == "__main__":
    main()
