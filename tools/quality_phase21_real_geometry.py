"""Read saved final source geometry with the existing Phase 2 independent renderer."""
from collections import Counter
import json
import math
from pathlib import Path
import time

import quality_phase2_validation as q
import quality_phase21_coverage as coverage

ROOT=q.ROOT
OUT=ROOT/"outputs/quality-phase21-20261106"
FOLDERS={
    "Legacy":ROOT/"outputs/quality-phase2-20261106/comparison/baseline/user_crest",
    "Standard_phase2":ROOT/"outputs/quality-phase2-20261106/comparison/candidate/user_crest",
    "Contextual":OUT/"candidate/comparison/candidate/user_crest",
}


def read(path):return json.loads(path.read_text(encoding="utf-8-sig"))


def source_storage(obj):
    if obj.get("contours"):return "contours"
    if (obj.get("centerline_graph") or {}).get("source_contours"):return "running_graph_source_contours"
    if obj.get("satin_column"):return "satin_rails"
    return "no_source_rings"


def local_probe(project,generated,point=(64.5,80.5)):
    objects={o["id"]:o for o in project["objects"]}
    containing=[]
    nearest_objects=[]
    for obj in project["objects"]:
        rings=q.source_rings(obj)
        if not rings:continue
        inside=q.av.inside(point,rings)
        distance=q.av.dist_boundary(point,rings)
        row=dict(object_id=obj["id"],kind=obj["settings"]["kind"],color=obj["color"],source_storage=source_storage(obj),
                 exact_probe_inside_source_rings=inside,source_boundary_distance_mm=distance,
                 settings=obj["settings"],source_rings=len(rings))
        if inside:containing.append(row)
        nearest_objects.append(row)
    all_segments=coverage.segments(generated,{k:o["settings"]["kind"] for k,o in objects.items()})
    top=[r for r in all_segments if r["role"]=="TopFill"]
    nearest=sorted((dict(**r,distance_mm=coverage.distance(point,r["a"],r["b"])) for r in top),key=lambda r:r["distance_mm"])[:15]
    radius=max(0.,nearest[0]["distance_mm"]-.15) if nearest else None
    if not containing:
        assessment="Probe point is outside all saved filled source rings. Its raster-cell witness inclusion does not establish a missing interior block at this physical point."
    elif all(o["kind"]=="Running" for o in containing):
        assessment="Probe belongs to Running source geometry. Its centerline buffers are not expected to fill the entire original raster region as Tatami; this is a purpose-specific fringe diagnostic, not proof of a missing Tatami block."
    elif radius is not None and any(o["source_boundary_distance_mm"]<radius for o in containing):
        assessment="The empty-buffer disk reaches a saved source boundary. It is not a fully internal missing-block witness; inspect the listed type, contour and top segments locally."
    else:
        assessment="Internal geometric gap evidence remains for the listed stitch type; no blanket absence-of-gaps verdict. Local row/shape inspection is required."
    return dict(point_mm=point,containing_objects=containing,nearest_source_objects=sorted(nearest_objects,key=lambda r:r["source_boundary_distance_mm"])[:5],
                nearest_top_segments=nearest,empty_top_buffer_radius_mm=radius,assessment=assessment,
                note="Exact point-in-source-rings and boundary distance use existing independent Phase 2 helpers. Segment endpoints quantized to DST0.1mm. Underlay,Travel,Lock excluded from gap distances.")


def render_local(output,versions,images,probe=(64.5,80.5)):
    from PIL import Image,ImageDraw,ImageFont
    size,span=420,4.
    ox,oy=probe[0]-span/2,probe[1]-span/2
    panel=Image.new("RGB",(size*len(versions),size+125),"#efefeb")
    font=ImageFont.truetype("C:/Windows/Fonts/segoeui.ttf",16)
    draw=ImageDraw.Draw(panel)
    for index,(name,item) in enumerate(versions.items()):
        left=index*size
        width,height=item["physical_dimensions_mm"]
        image=images[name]
        # Display only: common physical crop of the independently rendered map.
        bounds=(ox/width*image.width,oy/height*image.height,(ox+span)/width*image.width,(oy+span)/height*image.height)
        crop=image.transform((size,size),Image.Transform.EXTENT,bounds,Image.Resampling.NEAREST)
        background=Image.new("RGBA",(size,size),"white")
        muted=crop.copy();muted.putalpha(crop.getchannel("A").point(lambda a:round(a*.3)))
        background.alpha_composite(muted)
        cd=ImageDraw.Draw(background)
        plan=read(FOLDERS[name]/"design.plan.json")
        generated=[{**s,**m} for s,m in zip(plan["stitches"],plan["metadata"],strict=True)]
        for row in coverage.segments(generated):
            if row["role"]!="TopFill":continue
            a,b=row["a"],row["b"]
            if max(a[0],b[0])<ox-.15 or min(a[0],b[0])>ox+span+.15 or max(a[1],b[1])<oy-.15 or min(a[1],b[1])>oy+span+.15:continue
            aa=((a[0]-ox)/span*size,(a[1]-oy)/span*size)
            bb=((b[0]-ox)/span*size,(b[1]-oy)/span*size)
            color=tuple(row["color"])+(255,)
            cd.line((*aa,*bb),fill=color,width=round(.3/span*size))
            radius=.15/span*size
            for point in (aa,bb):cd.ellipse((point[0]-radius,point[1]-radius,point[0]+radius,point[1]+radius),fill=color)
        cx,cy=(probe[0]-ox)/span*size,(probe[1]-oy)/span*size
        cd.line((cx-9,cy,cx+9,cy),fill="#ff00a0",width=2);cd.line((cx,cy-9,cx,cy+9),fill="#ff00a0",width=2)
        panel.paste(background.convert("RGB"),(left,70))
        draw.text((left+12,10),name,font=font,fill="#203744")
        kinds=",".join(str(o["object_id"])+":"+o["kind"] for o in item["local_probe"]["containing_objects"]) or "outside saved source rings"
        draw.text((left+12,34),kinds,font=font,fill="#203744")
    draw.text((12,size+82),"All nearby TopFill buffers (0.30mm); muted saved source geometry; each panel 4x4mm; probe shown in pink.",font=font,fill="#203744")
    panel.save(output/"local-gap-source-and-topfill.png")


def main():
    output=OUT/"real-geometry";output.mkdir(parents=True,exist_ok=True)
    traces={name:read(folder/"design.trace.json") for name,folder in FOLDERS.items()}
    w,h=traces["Legacy"]["trace_dimensions"]
    required=q.threshold_witness(traces["Legacy"]["labels_before_preparation"],w,h,1,.1,100.*133.8/w/h)
    changes=[i for i,(a,b) in enumerate(zip(traces["Standard_phase2"]["labels_after_preparation"],traces["Contextual"]["labels_after_preparation"],strict=True)) if a!=b]
    versions={};images={}
    for name,folder in FOLDERS.items():
        start=time.perf_counter()
        project=read(folder/"design.elab");plan=read(folder/"design.plan.json")
        generated=[{**s,**m} for s,m in zip(plan["stitches"],plan["metadata"],strict=True)]
        _,rgb=q.project_labels(project,(w,h),return_rgb=True)
        images[name]=rgb
        rgb.save(output/(name+"-saved-source-geometry.png"))
        actual=list(rgb.get_flattened_data())
        missing=[i for i,v in enumerate(required) if v and actual[i][3]<128]
        changed_points=[]
        for i in changes:
            target=traces["Contextual"]["labels_after_preparation"][i]
            target_rgb=traces["Contextual"]["palette"][target]
            changed_points.append(dict(trace_xy=[i%w,i//w],center_mm=[(i%w+.5)*100/w,(i//w+.5)*133.8/h],
                                       intended_target_label=target,intended_target_rgb=target_rgb,
                                       actual_saved_geometry_rgba=actual[i],included=actual[i][3]>=128,
                                       matches_contextual_target=actual[i]==tuple(target_rgb)+(255,)))
        versions[name]=dict(folder=str(folder),elab_sha256=q.av.sha(folder/"design.elab"),trace_sha256=q.av.sha(folder/"design.trace.json"),
                            physical_dimensions_mm=[project["width_mm"],project["source_image"]["height_mm"]],
                            required_original_accepted_pixels=sum(required),missing_original_accepted_centers=len(missing),
                            missing_examples_trace_xy=[[i%w,i//w] for i in missing[:50]],
                            saved_source_geometry_support_pixels=sum(p[3]>=128 for p in actual),
                            source_storage_counts=dict(Counter(source_storage(o) for o in project["objects"])),
                            source_rings_by_type=dict(Counter(o["settings"]["kind"] for o in project["objects"] if q.source_rings(o))),
                            objects_without_source_rings=[o["id"] for o in project["objects"] if not q.source_rings(o)],
                            hidden_objects=[o["id"] for o in project["objects"] if not o.get("visible",True)],
                            changed_points=changed_points,new_points_included=sum(p["included"] for p in changed_points),
                            new_points_matching_contextual_target=sum(p["matches_contextual_target"] for p in changed_points),
                            local_probe=local_probe(project,generated),seconds=time.perf_counter()-start)
        print(name,"required_missing",len(missing),"changed_points_included",versions[name]["new_points_included"],flush=True)
    before=list(images["Standard_phase2"].get_flattened_data());after=list(images["Contextual"].get_flattened_data())
    rgba_diff=[i for i,(a,b) in enumerate(zip(before,after,strict=True)) if a!=b]
    result=dict(method="Existing quality_phase2_validation.project_labels at383x512, return_rgb=True; source_rings includes contours, Running graph source snapshots and Satin rails. No reconstructed acceptance mask is substituted for saved geometry.",
                trace_dimensions=[w,h],required_witness_pixels=sum(required),contextual_label_changes=len(changes),versions=versions,
                standard_to_contextual_saved_rgba_changed_pixels=len(rgba_diff),
                saved_rgba_changes_exactly_contextual_label_changes=set(rgba_diff)==set(changes),
                saved_alpha_lost_pixels=sum(a[3]>=128 and b[3]<128 for a,b in zip(before,after,strict=True)),
                saved_alpha_added_pixels=sum(a[3]<128 and b[3]>=128 for a,b in zip(before,after,strict=True)),
                original_centers_retained_all_versions=all(v["missing_original_accepted_centers"]==0 for v in versions.values()),
                contextual_changed_points_all_included_and_target_rgb=versions["Contextual"]["new_points_matching_contextual_target"]==len(changes),
                semantic_oracle=False,note="Actual saved geometry/support/color consistency only. Palette-target agreement does not prove semantic correctness or fabric coverage. Local gap assessment is bounded to one previously reported coordinate.")
    q.write(OUT/"real-geometry.json",result)
    render_local(output,versions,images)
    assert result["original_centers_retained_all_versions"]
    assert result["contextual_changed_points_all_included_and_target_rgb"]
    assert result["saved_alpha_lost_pixels"]==0
    assert result["saved_rgba_changes_exactly_contextual_label_changes"]


if __name__=="__main__":main()
