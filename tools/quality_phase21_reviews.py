"""Local, read-only audit of the frozen Phase 2 independent reference failures.

No acceptance limits or references are changed. Outputs include per-property
violations and same-coordinate nearest-neighbour crops, never private sources.
"""
from __future__ import annotations
import argparse
from collections import Counter, deque
import html
import json
from pathlib import Path
import subprocess

import quality_phase2_validation as q

ROOT = Path(__file__).resolve().parents[1]
PHASE2 = ROOT / "outputs/quality-phase2-20261106"
GROUPS = ("comparison", "comparison-unpinned", "comparison-paired")


def read(path):
    return json.loads(path.read_text(encoding="utf-8-sig"))


def components(labels, w, h, color):
    seen = bytearray(w*h)
    out = []
    for seed, label in enumerate(labels):
        if label != color or seen[seed]:
            continue
        pending = [seed]
        seen[seed] = 1
        for i in pending:
            x, y = i%w, i//w
            for n in ([i-1] if x else []) + ([i+1] if x+1<w else []) + ([i-w] if y else []) + ([i+w] if y+1<h else []):
                if not seen[n] and labels[n] == color:
                    seen[n] = 1
                    pending.append(n)
        out.append(pending)
    return out


def labels(image):
    return [255 if p[3] < 128 else q.COLORS.index(p[:3]) for p in image.convert("RGBA").get_flattened_data()]


def topology_map(label_map, w, h):
    result=[]
    for color in sorted(set(label_map)-{255}):
        mask=[v==color for v in label_map]
        info=q.component_info(mask,w,h)
        holes=q.component_info([not v for v in mask],w,h)["internal"]
        result.append(dict(rgb=q.COLORS[color],components=info["count"],holes=holes))
    return result


def semantic_stage(stage, key):
    palette = stage["palette"]
    nearest = {i:min(range(len(q.COLORS)), key=lambda k:sum((a-b)**2 for a,b in zip(q.COLORS[k],rgb))) for i,rgb in enumerate(palette)}
    return [255 if i == 255 else nearest[i] for i in stage[key]]


def violation_properties(reference, limits):
    properties = []
    for key, comparison, limit in (("core_color_accuracy", "minimum", limits["color_core_accuracy_min"]),
                                  ("forbidden_background_core_leaks", "maximum", 0),
                                  ("symmetric_boundary_error_bound_mm", "maximum", limits["boundary_error_max_mm"])):
        value=reference[key]
        if (comparison == "minimum" and value < limit) or (comparison == "maximum" and value > limit):
            properties.append(dict(property=key,value=value,requirement=comparison,limit=limit))
    for c in reference["color_regions"]:
        for prop in ("components", "holes"):
            expected,actual=c["truth_"+prop],c["predicted_"+prop]
            if expected != actual:
                properties.append(dict(property=prop,rgb=c["rgb"],expected=expected,actual=actual))
        if c["protected_core_recall"] < limits["marked_core_recall_min"]:
            properties.append(dict(property="protected_core_recall",rgb=c["rgb"],value=c["protected_core_recall"],minimum=limits["marked_core_recall_min"]))
    return properties


def render_crop(images, point, output, radius=9, zoom=12):
    from PIL import Image, ImageDraw
    w,h=images[0][1].size
    x,y=point
    box=(max(0,x-radius),max(0,y-radius),min(w,x+radius+1),min(h,y+radius+1))
    tile_w,tile_h=(box[2]-box[0])*zoom,(box[3]-box[1])*zoom
    panel=Image.new("RGB",(len(images)*tile_w,tile_h+30),(236,236,232))
    draw=ImageDraw.Draw(panel)
    for k,(name,img) in enumerate(images):
        tile=Image.new("RGBA",img.size,(207,207,203,255))
        tile.alpha_composite(img.convert("RGBA"))
        tile=tile.crop(box).convert("RGB").resize((tile_w,tile_h),Image.Resampling.NEAREST)
        panel.paste(tile,(k*tile_w,30))
        draw.text((k*tile_w+3,8),name,fill="black")
        px,py=k*tile_w+(x-box[0])*zoom,30+(y-box[1])*zoom
        draw.rectangle((px,py,px+zoom-1,py+zoom-1),outline=(255,0,255),width=1)
    panel.save(output)
    return list(box)


def run(output, candidate_root=None):
    from PIL import Image
    output.mkdir(parents=True,exist_ok=True)
    if candidate_root:
        pending=[]
        for group in GROUPS:
            for case in q.load_frozen_manifest(PHASE2/group/"fixtures.json"):
                if not (candidate_root/group/"candidate"/case["name"]/"evaluation.json").is_file():
                    pending.append(group+"/"+case["name"])
        if pending:
            raise RuntimeError("Candidate corpus is incomplete; refusing a partial comparison: "+", ".join(pending))
    rows=[]
    for group in GROUPS:
        manifest=PHASE2/group/"fixtures.json"
        cases=q.load_frozen_manifest(manifest)
        for case in cases:
            if not case.get("truth"):
                continue
            name=case["name"]
            standard_folder=PHASE2/group/"candidate"/name
            evaluation=read(standard_folder/"evaluation.json")
            ref=evaluation["reference"]
            if ref["accepted"]:
                continue
            truth=Image.open(case["truth"]).convert("RGBA")
            source=Image.open(case["source"]).convert("RGBA")
            w,h=truth.size
            expected=labels(truth)
            standard=Image.open(standard_folder/"regions.png").convert("RGBA")
            legacy=Image.open(PHASE2/group/"baseline"/name/"regions.png").convert("RGBA")
            actual_rgb=Image.open(standard_folder/"actual-thread-colors.png").convert("RGBA")
            predicted=labels(standard)
            stage=read(standard_folder/"design.trace.json")
            before=semantic_stage(stage,"labels_before_preparation")
            after=semantic_stage(stage,"labels_after_preparation")
            # Far-away false components explain the strict symmetric set bound.
            points=[]
            explanations=[]
            for c in ref["color_regions"]:
                color=q.COLORS.index(tuple(c["rgb"]))
                if c["truth_components"]==c["predicted_components"] and c["truth_holes"]==c["predicted_holes"]:
                    continue
                dist=q.distance_to([v==color for v in expected],w,h)
                false_components=[component for component in components(predicted,w,h,color) if not any(expected[i]==color for i in component)]
                false_components.sort(key=lambda cp:max(dist[i] for i in cp),reverse=True)
                if false_components:
                    cp=false_components[0]
                    i=max(cp,key=lambda i:dist[i])
                else:
                    mismatches=[i for i in range(w*h) if (expected[i]==color)!=(predicted[i]==color)]
                    i=max(mismatches,key=lambda i:dist[i])
                points.append(i)
                source_rgba=source.getpixel((i%w,i//w))
                palette_index=stage["labels_before_preparation"][i]
                explanations.append(dict(rgb=c["rgb"],point=[i%w,i//w],point_mm=[i%w*case["width_mm"]/w,i//w*case["width_mm"]/w],
                    expected_label=expected[i],source_rgba=source_rgba,actual_thread_rgba=actual_rgb.getpixel((i%w,i//w)),
                    source_to_actual_thread_oklab_error=sum((a-b)**2 for a,b in zip(q.oklab(source_rgba[:3]),q.oklab(actual_rgb.getpixel((i%w,i//w))[:3])))**.5,
                    palette_index=palette_index,palette_rgb=stage["palette"][palette_index] if palette_index!=255 else None,
                    stage_before_label=before[i],stage_after_label=after[i],geometry_label=predicted[i],
                    distance_from_expected_color_pixels=dist[i],false_components=len(false_components),
                    source_alpha_partial=0<source_rgba[3]<255,
                    earliest_observed_stage="palette/label assignment" if before[i] == predicted[i] else "components/contours/classification or reference rasterization",
                    cause="extra semantic-color island" if false_components else "boundary/hole discrepancy"))
            if ref["forbidden_background_core_leaks"]:
                boundary=[]
                for i in range(w*h):
                    x,y=i%w,i//w
                    ns=([i-1] if x else [])+([i+1] if x+1<w else [])+([i-w] if y else [])+([i+w] if y+1<h else [])
                    boundary.append(any(expected[i]!=expected[n] for n in ns))
                bd=q.distance_to(boundary,w,h)
                leaks=[i for i in range(w*h) if expected[i]==255 and bd[i]>2 and predicted[i]!=255]
                assert len(leaks)==ref["forbidden_background_core_leaks"]
                i=max(leaks,key=lambda i:bd[i])
                points.append(i)
                pi=stage["labels_before_preparation"][i]
                p=source.getpixel((i%w,i//w))
                explanations.append(dict(rgb=None,point=[i%w,i//w],point_mm=[i%w*case["width_mm"]/w,i//w*case["width_mm"]/w],
                    expected_label=255,source_rgba=p,actual_thread_rgba=actual_rgb.getpixel((i%w,i//w)),palette_index=pi,
                    palette_rgb=stage["palette"][pi] if pi!=255 else None,stage_before_label=before[i],stage_after_label=after[i],geometry_label=predicted[i],
                    boundary_distance_pixels=bd[i],total_forbidden_background_core_leaks=len(leaks),source_alpha_partial=0<p[3]<255,
                    earliest_observed_stage="background mask / label assignment",cause="JPEG foreground outside independent artwork support"))
            # One crop per distinct explanatory color, same pixel coordinates.
            candidate_folder=candidate_root/group/"candidate"/name if candidate_root else None
            images=[("source",source),("truth",truth),("Legacy",legacy),("Standard2",standard)]
            candidate_evaluation=None
            candidate_change_diagnostics=None
            if candidate_folder and (candidate_folder/"evaluation.json").is_file():
                candidate_evaluation=read(candidate_folder/"evaluation.json")
                candidate_image=Image.open(candidate_folder/"regions.png").convert("RGBA")
                images.append(("candidate",candidate_image))
                candidate_labels=labels(candidate_image)
                changed=[i for i,(a,b) in enumerate(zip(predicted,candidate_labels)) if a!=b]
                corrected=[i for i in changed if candidate_labels[i]==expected[i] and predicted[i]!=expected[i]]
                regressed=[i for i in changed if predicted[i]==expected[i] and candidate_labels[i]!=expected[i]]
                remaining=[i for i in changed if i not in corrected and i not in regressed]
                # Add the actual change locations as well as unchanged worst-case
                # defects. The frozen boundary band remains semantically ambiguous.
                for group_points in (corrected,regressed,remaining):
                    if group_points:
                        points.append(group_points[0])
                candidate_change_diagnostics=dict(changed_semantic_geometry_pixels=len(changed),
                    corrected_against_discrete_truth=len(corrected),regressed_against_discrete_truth=len(regressed),
                    changed_but_still_wrong=len(remaining),
                    first_corrected_point=[corrected[0]%w,corrected[0]//w] if corrected else None,
                    first_regressed_point=[regressed[0]%w,regressed[0]//w] if regressed else None,
                    note="Pixel comparison includes the ambiguous boundary band; not an extra acceptance gate or a fabric claim")
            crops=[]
            for k,i in enumerate(dict.fromkeys(points)):
                file=f"{group}-{name}-{k}.png"
                box=render_crop(images,(i%w,i//w),output/file)
                crops.append(dict(file=file,point=[i%w,i//w],box=box))
            row=dict(group=group,name=name,role=case["group"],source_sha256=q.av.sha(Path(case["source"])),truth_sha256=q.av.sha(Path(case["truth"])),
                     limits=case["limits"],standard_properties=violation_properties(ref,case["limits"]),
                     legacy_reference=read(PHASE2/group/"baseline"/name/"evaluation.json")["reference"],
                     standard_reference=ref,candidate_reference=candidate_evaluation["reference"] if candidate_evaluation else None,
                     candidate_change_diagnostics=candidate_change_diagnostics,
                     source_dimensions=[w,h],explanations=explanations,crops=crops,
                     topology_by_stage=dict(truth=topology_map(expected,w,h),labels_before=topology_map(before,w,h),labels_after=topology_map(after,w,h),source_geometry=topology_map(predicted,w,h)),
                     changed_from_trace_after_to_geometry_pixels=sum(a!=b for a,b in zip(after,predicted)),
                     minimal_reproduction=dict(manifest=str(manifest),case=name,source=case["source"],truth=case["truth"],options=case["options"],width_mm=case["width_mm"],
                         note="Use the frozen source and exact options with trace-report; the selected source crop is diagnostic context, not a replacement fixture: cropping changes palette and region adjacency."))
            rows.append(row)
            print(name,[(e["point"],e["source_rgba"],e["palette_rgb"],e["stage_before_label"],e["geometry_label"]) for e in explanations],flush=True)
    q.write(output/"review-audit.json",dict(cases=rows,count=len(rows),oracle_changed=False,private_inputs_included=False))
    if not candidate_root and not (output/"standard-review-audit.json").exists():
        q.write(output/"standard-review-audit.json",dict(cases=rows,count=len(rows),oracle_changed=False,private_inputs_included=False))
    cards=[]
    for row in rows:
        cards.append(f'<article><h2>{html.escape(row["name"])}</h2><pre>{html.escape(json.dumps(row["standard_properties"],ensure_ascii=False,indent=2))}</pre>'+''.join(f'<p>Pixel {crop["point"]}, crop {crop["box"]}</p><img src="{crop["file"]}">' for crop in row["crops"])+"</article>")
    (output/"index.html").write_text('<!doctype html><meta charset="utf-8"><title>Phase 2.1 REVIEW audit</title><style>body{font:15px system-ui;background:#eee;margin:24px}article{background:white;padding:18px;margin:20px 0}img{max-width:100%;image-rendering:pixelated}pre{white-space:pre-wrap}</style><h1>Frozen independent reference REVIEW audit</h1><p>Same-coordinate crops; magenta square marks the diagnostic pixel. Acceptance limits and oracle unchanged. No private inputs.</p>'+''.join(cards),encoding="utf-8")
    return rows


def self_test():
    """Small independent tests of the interpretation used in this audit."""
    from PIL import Image
    for i,rgb in enumerate(q.COLORS):
        assert labels(Image.new("RGBA",(1,1),(*rgb,255))) == [i]
    assert labels(Image.new("RGBA",(1,1),(29,47,199,0))) == [255]
    # A far-away false island must fail symmetric set distance/topology even
    # when every marked core of the main region is correct.
    expected=[255]*49
    expected[8]=expected[9]=expected[15]=expected[16]=0
    predicted=expected.copy()
    predicted[40]=0
    assert q.component_info([v==0 for v in expected],7,7)["count"] == 1
    assert q.component_info([v==0 for v in predicted],7,7)["count"] == 2
    distance=q.distance_to([v==0 for v in expected],7,7)
    assert distance[40] == 6
    # Reproduces unrelated palette attraction of dark gold at a known
    # rasterized black/gold boundary. This does not authorize relabelling a
    # pinned green source component; it isolates the quantization ambiguity.
    source=q.oklab((183,151,23))
    distances=[sum((a-b)**2 for a,b in zip(source,q.oklab(rgb))) for rgb in q.COLORS]
    assert min(range(6),key=distances.__getitem__) == 5
    print("review audit self-test: PASS (semantic labels, false island, palette attraction)")


def reproduce(audit_folder, case_name, cli):
    """Isolate palette attraction to a single immutable source sample.

    Palette is frozen to the observed palette; this intentionally tests label
    assignment only, not global palette selection or contextual preparation.
    """
    from PIL import Image
    audit=read(audit_folder/"review-audit.json")
    case=next(row for row in audit["cases"] if row["name"] == case_name)
    stage=read(PHASE2/case["group"]/"candidate"/case_name/"design.trace.json")
    folder=audit_folder/"minimal"/case_name
    folder.mkdir(parents=True,exist_ok=True)
    results=[]
    for number,sample in enumerate(case["explanations"]):
        source=folder/f"sample-{number}.png"
        Image.new("RGBA",(1,1),tuple(sample["source_rgba"])).save(source)
        options=q.options(mode="Standard",palette=len(stage["palette"]),size=32)
        options["pinned_colors"]=stage["palette"]
        config=folder/f"sample-{number}.options.json"
        q.write(config,options)
        target=folder/f"sample-{number}.trace.json"
        command=[str(cli),"trace-report",str(source),str(target),"1",str(config)]
        process=subprocess.run(command,cwd=ROOT,capture_output=True,text=True,encoding="utf-8")
        actual=read(target)["labels_before_preparation"][0] if process.returncode == 0 else None
        result=dict(command=command,exit_code=process.returncode,expected_palette_index=sample["palette_index"],actual_palette_index=actual,
                    passed=process.returncode==0 and actual==sample["palette_index"],stderr=process.stderr,
                    scope="1-pixel label-assignment reproducer with full original palette frozen; not a contextual merge test")
        results.append(result)
    q.write(folder/"results.json",results)
    if not all(r["passed"] for r in results):
        raise RuntimeError(f"Minimal palette reproduction differs: {case_name}")
    print(f"{case_name}: {len(results)} minimal label samples reproduced")


def resolution_audit(output, cli, profile):
    """Same bytes at native and reduced tracing; keeps physical size/floors."""
    from PIL import Image
    manifest=PHASE2/"comparison/fixtures.json"
    cases=q.load_frozen_manifest(manifest)
    selected=[c for c in cases if c["name"] in ("same_emblem_384px","user_crest")]
    rows=[]
    for case in selected:
        source=Path(case["source"])
        with Image.open(source) as image:
            source_size=image.size
            detected_format=image.format
        reduced=192 if case["name"]=="same_emblem_384px" else 512
        for maximum in (reduced,max(source_size)):
            folder=output/"resolution"/profile/case["name"]/str(maximum)
            folder.mkdir(parents=True,exist_ok=True)
            config={**case["options"],"max_dimension":maximum,"preparation":profile}
            options=folder/"options.json"
            q.write(options,config)
            target=folder/"design.trace.json"
            command=[str(cli),"trace-report",str(source),str(target),str(case["width_mm"]),str(options)]
            record=q.measured_run(command,folder)
            record.update(name=case["name"],source_sha256=q.av.sha(source),cli_sha256=q.av.sha(cli),
                          detected_format=detected_format,source_dimensions=source_size,width_mm=case["width_mm"],
                          height_mm=case["width_mm"]*source_size[1]/source_size[0],options_sha256=q.av.sha(options),
                          purpose="Same-byte native vs reduced trace; object reduction alone is not a quality score")
            if record["exit_code"] == 0:
                stage=read(target)
                record["stages"]={k:v for k,v in stage.items() if not k.startswith("labels_")}
                w,h=stage["trace_dimensions"]
                for key in ("labels_before_preparation","labels_after_preparation"):
                    rgba=Image.new("RGBA",(w,h))
                    rgba.putdata([(*stage["palette"][v],255) if v!=255 else (0,0,0,0) for v in stage[key]])
                    rgba.save(folder/(key+".png"))
                semantic=semantic_stage(stage,"labels_after_preparation")
                record["semantic_topology_after"]=topology_map(semantic,w,h)
                if case.get("truth"):
                    truth=Image.open(case["truth"]).convert("RGBA")
                    expected=labels(truth)
                    # A native-grid comparison for the same-byte source; it
                    # measures source label recovery, not contour generation.
                    semantic_image=Image.new("RGBA",(w,h))
                    semantic_image.putdata([(*q.COLORS[v],255) if v!=255 else (0,0,0,0) for v in semantic])
                    semantic_image=semantic_image.resize(truth.size,Image.Resampling.NEAREST)
                    semantic_image.save(folder/"semantic-labels-on-original-grid.png")
                    actual=labels(semantic_image)
                    record["native_grid_semantic_mismatch_pixels"]=sum(a!=b for a,b in zip(expected,actual))
                    record["native_grid_false_foreground_pixels"]=sum(a==255 and b!=255 for a,b in zip(expected,actual))
                    record["native_grid_missing_foreground_pixels"]=sum(a!=255 and b==255 for a,b in zip(expected,actual))
            q.write(folder/"measurement.json",record)
            rows.append(record)
            print(profile,case["name"],maximum,record["exit_code"],record.get("stages",{}).get("object_count"),flush=True)
    q.write(output/"resolution"/(profile+".json"),dict(profile=profile,cases=rows,generators_run=False))
    return rows


def accepted_map(stage, options, width_mm, height_mm):
    """Independent component acceptance, including immutable provenance.

    This is a raster acceptance witness, not a contour/fabric reconstruction.
    The audited crest has no protected pixels or pinned colours.
    """
    from PIL import Image
    w,h=stage["trace_dimensions"]
    area=width_mm*height_mm/(w*h)
    original=q.threshold_witness(stage["labels_before_preparation"],w,h,
        options["min_region_pixels"],options["min_region_area_mm2"],area)
    after=stage["labels_after_preparation"]
    keep=bytearray(w*h)
    for color in set(after)-{255}:
        for component in components(after,w,h,color):
            retained=any(original[i] for i in component)
            passes=len(component)>=options["min_region_pixels"] and len(component)*area+1e-12>=options["min_region_area_mm2"]
            if retained or passes:
                for i in component:
                    keep[i]=1
    image=Image.new("RGBA",(w,h))
    image.putdata([(*stage["palette"][v],255) if keep[i] else (0,0,0,0) for i,v in enumerate(after)])
    return image,keep,original


def real_crest_crops(output, candidate_root):
    """Private local evidence for every difference from Standard, no oracle."""
    from PIL import Image
    case=next(c for c in q.load_frozen_manifest(PHASE2/"comparison/fixtures.json") if c["name"]=="user_crest")
    legacy_folder=PHASE2/"comparison/baseline/user_crest"
    standard_folder=PHASE2/"comparison/candidate/user_crest"
    candidate_folder=candidate_root/"comparison/candidate/user_crest"
    candidate_trace=candidate_folder/"design.trace.json"
    # Resolution audit is the same final executable/input/options and may finish
    # before the full stitch corpus; use its report only as a documented fallback.
    if not candidate_trace.is_file():
        candidate_trace=output/"resolution/Contextual/user_crest/512/design.trace.json"
    paths=[legacy_folder/"design.trace.json",standard_folder/"design.trace.json",candidate_trace]
    stages=[read(path) for path in paths]
    assert all(s["trace_dimensions"]==stages[0]["trace_dimensions"] for s in stages)
    assert all(s["palette"]==stages[0]["palette"] for s in stages)
    assert all(s["labels_before_preparation"]==stages[0]["labels_before_preparation"] for s in stages)
    w,h=stages[0]["trace_dimensions"]
    source=Image.open(case["source"]).convert("RGBA")
    width=case["width_mm"]
    height=width*source.height/source.width
    # Display-only native samples aligned to the trace-grid physical extent;
    # analytical values continue to come from immutable stage reports.
    source_display=source.resize((w,h),Image.Resampling.NEAREST)
    label_images=[]
    accepted_images=[]
    accepted=[]
    for name,stage in zip(("Legacy","Standard2","candidate"),stages):
        image=Image.new("RGBA",(w,h))
        image.putdata([(*stage["palette"][v],255) if v!=255 else (0,0,0,0) for v in stage["labels_after_preparation"]])
        label_images.append((name+" labels",image))
        acceptance,keep,original=accepted_map(stage,case["options"],width,height)
        accepted_images.append((name+" accepted",acceptance))
        accepted.append(keep)
    standard,candidate=stages[1]["labels_after_preparation"],stages[2]["labels_after_preparation"]
    differences=[i for i,(a,b) in enumerate(zip(standard,candidate)) if a!=b]
    original_labels=stages[0]["labels_before_preparation"]
    original_sizes={}
    for color in set(original_labels)-{255}:
        for component in components(original_labels,w,h,color):
            for i in component:
                original_sizes[i]=len(component)
    folder=output/"real-crest"
    folder.mkdir(parents=True,exist_ok=True)
    rows=[]
    for number,i in enumerate(differences):
        point=(i%w,i//w)
        label_path=folder/f"difference-{number:02d}-labels.png"
        accepted_path=folder/f"difference-{number:02d}-accepted.png"
        box=render_crop([("native source display",source_display)]+label_images,point,label_path)
        render_crop([("no semantic truth",source_display)]+accepted_images,point,accepted_path)
        native_box=(box[0]*source.width/w,box[1]*source.height/h,box[2]*source.width/w,box[3]*source.height/h)
        native_crop=source.crop(tuple(round(v) for v in native_box))
        native_crop.save(folder/f"difference-{number:02d}-native-source.png")
        rows.append(dict(point=list(point),physical_center_mm=[(point[0]+.5)*width/w,(point[1]+.5)*height/h],
            trace_box=box,native_source_box=native_box,original_component_pixels=original_sizes[i],
            labels=[stage["labels_after_preparation"][i] for stage in stages],
            actual_palette_rgb=[stage["palette"][stage["labels_after_preparation"][i]] for stage in stages],
            accepted=[bool(keep[i]) for keep in accepted],
            original_retained=bool(original[i]),labels_crop=label_path.name,accepted_crop=accepted_path.name,
            semantic_verdict="UNCERTAIN: source has no independent semantic labels; geometric reassignment is measured, not automatic artwork PASS"))
    result=dict(source_sha256=q.av.sha(Path(case["source"])),source_format=Image.open(case["source"]).format,
        source_dimensions=list(source.size),trace_dimensions=[w,h],trace_reports=[str(p) for p in paths],
        pixel_area_mm2=width*height/(w*h),difference_count=len(rows),
        accepted_pixel_counts=[sum(keep) for keep in accepted],
        original_retained_pixels_lost=[sum(wanted and not keep[i] for i,wanted in enumerate(original)) for keep in accepted],
        previous_omissions_now_accepted=sum(not accepted[1][i] and accepted[2][i] for i in differences),
        crops=rows,private_local_artifacts=True,
        note="Source displayed using native nearest samples for alignment only; accepted maps independently reconstruct raster threshold/provenance, not final contour inclusion or fabric coverage. No semantic oracle for real crest.")
    q.write(folder/"comparison.json",result)
    cards=[]
    for row in rows:
        cards.append('<article><h2>'+str(row["point"])+'</h2><p>'+html.escape(str(row["actual_palette_rgb"]))+'; accepted '+str(row["accepted"])+', original component '+str(row["original_component_pixels"])+' px</p><img src="'+row["labels_crop"]+'"><img src="'+row["accepted_crop"]+'"></article>')
    (folder/"index.html").write_text('<!doctype html><meta charset="utf-8"><title>Private crest changes</title><style>body{font:15px system-ui;background:#eee;margin:24px}article{background:white;padding:18px;margin:20px 0}img{max-width:100%;image-rendering:pixelated}</style><h1>Private local crest comparison</h1><p>No independent semantic truth. Actual palette RGB; upper row labels, lower row independent raster acceptance/provenance. No automatic PASS.</p>'+''.join(cards),encoding="utf-8")
    print("crest differences",len(rows),"previous omissions now accepted",result["previous_omissions_now_accepted"],flush=True)
    return result


if __name__ == "__main__":
    parser=argparse.ArgumentParser()
    parser.add_argument("--output",type=Path,default=ROOT/"outputs/quality-phase21-20261106/reviews")
    parser.add_argument("--candidate-root",type=Path)
    parser.add_argument("--self-test",action="store_true")
    parser.add_argument("--reproduce",help="Name of an already-audited case; tests one source sample per failed color")
    parser.add_argument("--cli",type=Path,default=PHASE2/"final-bin/embroidery-cli.exe")
    parser.add_argument("--resolution",action="store_true",help="Same-byte native vs downsample trace-only experiment")
    parser.add_argument("--profile",default="Standard")
    parser.add_argument("--crest",action="store_true",help="Local private real-crest label/acceptance crops")
    args=parser.parse_args()
    if args.self_test:
        self_test()
    elif args.reproduce:
        reproduce(args.output,args.reproduce,args.cli)
    elif args.resolution:
        resolution_audit(args.output,args.cli,args.profile)
    elif args.crest:
        if not args.candidate_root:
            parser.error("--crest requires --candidate-root")
        real_crest_crops(args.output,args.candidate_root)
    else:
        run(args.output,args.candidate_root)
