"""Calibrate and supplement Phase 2's unchanged sampled TopFill witness.

This is an offline diagnostic extension, not a thread/fabric simulator.  The
controlled coupon oracle is analytical and independent of the stitch importer.
No acceptance threshold is inferred from candidate results.
"""
from __future__ import annotations

import argparse
from collections import Counter, defaultdict
import hashlib
import json
import math
from pathlib import Path

import quality_phase2_validation as q

RADIUS = .15
PHASES = ((.125, .125), (.375, .625), (.625, .375), (.875, .875))


def segments(generated, object_types=None):
    """The segment starts at the previous machine position, including jumps."""
    previous = (0., 0.)
    result = []
    for stitch in generated:
        point = (q.av.quantize(stitch["x"]), q.av.quantize(stitch["y"]))
        if stitch["command"] == "Stitch":
            result.append(dict(a=previous, b=point, role=stitch.get("role", "Unknown"),
                               color=stitch.get("color"), object_id=stitch.get("object_id"),
                               kind=(object_types or {}).get(stitch.get("object_id"), "Unknown")))
        if stitch["command"] in ("Stitch", "Jump"):
            previous = point
    return result


def distance(point, a, b):
    dx, dy = b[0]-a[0], b[1]-a[1]
    t = min(1., max(0., ((point[0]-a[0])*dx+(point[1]-a[1])*dy)/max(dx*dx+dy*dy, 1e-30)))
    return math.hypot(point[0]-a[0]-t*dx, point[1]-a[1]-t*dy)


class Index:
    def __init__(self, rows, radius=RADIUS, cell=.8):
        self.rows, self.radius, self.cell = rows, radius, cell
        self.buckets = defaultdict(list)
        for i, row in enumerate(rows):
            a, b = row["a"], row["b"]
            for gy in range(math.floor((min(a[1], b[1])-radius)/cell), math.floor((max(a[1], b[1])+radius)/cell)+1):
                for gx in range(math.floor((min(a[0], b[0])-radius)/cell), math.floor((max(a[0], b[0])+radius)/cell)+1):
                    self.buckets[gx, gy].append(i)

    def hits(self, point):
        return [i for i in self.buckets.get((math.floor(point[0]/self.cell), math.floor(point[1]/self.cell)), ())
                if distance(point, self.rows[i]["a"], self.rows[i]["b"]) <= self.radius+1e-8]


def sample(required, w, h, width, height, rows, factor=1, phase=(.5, .5), expected_color=None):
    """Subdivide the SAME witness pixel support; never silently change its mask.

    For each retained raster cell all subcells have the original witness color.
    This is a piecewise constant raster-area diagnostic, not a continuous oracle.
    Multiplicity counts segment buffers and includes stitch-end overlaps, so it
    must not be interpreted directly as repeated needle penetration.
    """
    index = Index(rows)
    covered = required_count = correct = repeated = 0
    by_kind = Counter()
    cells = defaultdict(Counter)
    for i, wanted in enumerate(required):
        if not wanted:
            continue
        x, y = i % w, i // w
        for sy in range(factor):
            for sx in range(factor):
                p = ((x+(sx+phase[0])/factor)*width/w, (y+(sy+phase[1])/factor)*height/h)
                hits = index.hits(p)
                required_count += 1
                covered += bool(hits)
                repeated += len(hits) > 1
                by_kind.update({rows[j]["kind"] for j in hits})
                cell = cells[math.floor(p[0]), math.floor(p[1])]
                cell["probes"] += 1
                cell["covered"] += bool(hits)
                cell["multiple_segment_buffers"] += len(hits) > 1
                if expected_color is not None:
                    correct += any(rows[j]["color"] == expected_color[i] for j in hits)
    total = max(1, required_count)
    return dict(probes=required_count, covered_fraction=covered/total,
                color_correct_covered_fraction=correct/total if expected_color is not None else None,
                multiple_segment_buffer_fraction=repeated/total,
                coverage_by_object_type={k: v/total for k, v in sorted(by_kind.items())},
                local_cells=[dict(xy_mm=list(k), probes=v["probes"], uncovered_fraction=1-v["covered"]/v["probes"],
                                 multiple_segment_buffer_fraction=v["multiple_segment_buffers"]/v["probes"])
                             for k, v in sorted(cells.items())])


def stability(required, w, h, width, height, rows, factors=(1, 2, 4, 8), expected_color=None):
    levels = []
    for factor in factors:
        values = [sample(required, w, h, width, height, rows, factor, phase, expected_color) for phase in PHASES]
        fractions = [r["covered_fraction"] for r in values]
        levels.append(dict(factor=factor, pitch_xy_mm=[width/w/factor, height/h/factor],
                           phase_fractions=fractions, mean=sum(fractions)/len(fractions),
                           minimum=min(fractions), maximum=max(fractions), phase_span=max(fractions)-min(fractions),
                           color_correct_mean=(sum(r["color_correct_covered_fraction"] for r in values)/len(values)
                                               if expected_color is not None else None)))
    return dict(phases=PHASES, levels=levels,
                last_mean_change=abs(levels[-1]["mean"]-levels[-2]["mean"]) if len(levels)>1 else None,
                precision_note="Phase range and refinement change are measured sensitivity, not a statistical confidence interval or proven bound.")


def horizontal_row_diagnostic(rows, expected_spacing=.4, angle_tolerance=.02):
    """Only report horizontal row evidence; do not infer generic Satin rows."""
    horizontal = [r for r in rows if abs(r["a"][1]-r["b"][1]) <= angle_tolerance and abs(r["a"][0]-r["b"][0]) > .1]
    ys = sorted({round((r["a"][1]+r["b"][1])/2, 6) for r in horizontal})
    gaps = [dict(from_y_mm=a, to_y_mm=b, gap_mm=b-a,
                 missing_nominal_rows=max(0, round((b-a)/expected_spacing)-1))
            for a, b in zip(ys, ys[1:]) if b-a > 1.5*expected_spacing]
    duplicates = Counter((r["a"], r["b"]) if r["a"] <= r["b"] else (r["b"], r["a"]) for r in horizontal)
    events = defaultdict(list)
    for row in horizontal:
        y = round((row["a"][1]+row["b"][1])/2, 6)
        lo, hi = sorted((row["a"][0], row["b"][0]))
        events[y].extend(((lo, 1), (hi, -1)))
    repeated_length = 0.
    for values in events.values():
        count, previous = 0, None
        for x, delta in sorted(values):
            if previous is not None and count > 1:
                repeated_length += x-previous
            count += delta
            previous = x
    return dict(horizontal_segments=len(horizontal), distinct_row_coordinates=len(ys), spacing_mm=expected_spacing,
                row_gaps=gaps, exact_duplicate_segments=sum(n-1 for n in duplicates.values()),
                repeated_horizontal_centerline_length_mm=repeated_length,
                note="Directional diagnostic only. A legitimate hole or multiple disconnected objects may also create row gaps; interpret against independent geometry.")


def empty_disk_probes(required, w, h, width, height, rows, search_radius=.8):
    """Coarse deterministic local gap evidence; no rows/holes acceptance oracle.

    Probe at 1 mm cell centers on required support. A reported disk is empty of
    TOP buffers, but its entire disk need not belong to required geometry. The
    latter must be checked independently before calling it a missing fill block.
    """
    index = Index(rows, radius=search_radius)
    probes = []
    for iy in range(math.ceil(height)):
        for ix in range(math.ceil(width)):
            p = ix+.5, iy+.5
            if p[0] >= width or p[1] >= height:
                continue
            i = min(h-1, int(p[1]*h/height))*w+min(w-1, int(p[0]*w/width))
            if not required[i]:
                continue
            candidates = index.buckets.get((math.floor(p[0]/index.cell), math.floor(p[1]/index.cell)), ())
            nearest = min((distance(p, rows[j]["a"], rows[j]["b"]) for j in candidates), default=search_radius)
            radius = max(0., min(search_radius, nearest)-RADIUS)
            if radius > .2:
                probes.append(dict(xy_mm=list(p), empty_top_buffer_radius_lower_bound_mm=radius))
    return dict(search_radius_mm=search_radius, reporting_radius_over_mm=.2, probes=probes,
                note="Coarse 1mm grid evidence. A 0.4mm regular row field with 0.3mm buffers has ideal interior empty radius <=0.05mm. Geometry boundaries and intended holes also produce empty disks; inspect source before declaring a defect.")


def coupon_generated(variant):
    generated = []
    for row in range(30):
        y = .2+.4*row
        if variant == "missing_row" and row == 14:
            continue
        intervals = [(0., 20.)]
        if variant == "hole" and 4. <= y <= 8.:
            intervals = [(0., 8.), (12., 20.)]
        if variant == "missing_block" and 4. <= y <= 8.:
            intervals = [(0., 8.), (12., 20.)]
        for x0, x1 in intervals:
            generated.extend([dict(x=x0, y=y, command="Jump"),
                              dict(x=x1, y=y, command="Stitch", role="TopFill", color="blue" if variant == "wrong_color" else "red", object_id=1)])
        if variant == "duplicate_block" and 4. <= y <= 8.:
            generated.extend([dict(x=8., y=y, command="Jump"), dict(x=12., y=y, command="Stitch", role="TopFill", color="red", object_id=1)])
    # An underlay across a missing block MUST NOT improve the top-only result.
    if variant == "missing_block":
        for row in range(10):
            y = 4.2+.4*row
            generated.extend([dict(x=8., y=y, command="Jump"), dict(x=12., y=y, command="Stitch", role="Underlay", color="red", object_id=1)])
    return generated


def exact_horizontal_union(rows, bounds, holes=()):
    """Independent exact area for horizontal full-width clipped stroke strips.

    The caller uses full-width rows, optionally with duplicate interior pieces.
    End caps then lie outside the target or inside an already counted strip.
    Hole/partial-row cap integration is intentionally not claimed here.
    """
    if holes:
        return None
    x0, y0, x1, y1 = bounds
    cuts = sorted({y0, y1, *[max(y0, min(y1, r["a"][1]+d)) for r in rows for d in (-RADIUS, RADIUS)]})
    area = 0.
    for lo, hi in zip(cuts, cuts[1:]):
        y = (lo+hi)/2
        intervals = sorted((max(x0, min(r["a"][0], r["b"][0])), min(x1, max(r["a"][0], r["b"][0])))
                           for r in rows if abs(y-r["a"][1]) < RADIUS)
        end = x0
        length = 0.
        for a, b in intervals:
            length += max(0., b-max(end, a))
            end = max(end, b)
        area += length*(hi-lo)
    return area


def calibration(output):
    output.mkdir(parents=True, exist_ok=True)
    manifest = dict(version=1, geometry_mm=[20., 12.], rows=30, spacing_mm=.4, effective_thread_width_mm=.3,
                    raster_size=[77, 46], phases=PHASES, supersample_factors=[1, 2, 4, 8],
                    reference="analytical rectangle [0,20]x[0,12], red; hole [8,12]x[4,8]",
                    expectations=dict(correct_rectangle_union_fraction=.75, missing_row_union_fraction=.725,
                                      duplicate_block_union_fraction=.75, wrong_color_union_fraction=.75,
                                      wrong_color_correct_fraction=0., missing_block_less_than_correct=True,
                                      duplicates_detected_without_union_gain=True, underlay_excluded=True),
                    frozen_before_measurement=True)
    manifest_path = output/"calibration-manifest.json"
    encoded = json.dumps(manifest, ensure_ascii=False, sort_keys=True, indent=2)+"\n"
    if manifest_path.is_file() and manifest_path.read_text(encoding="utf-8") != encoded:
        raise RuntimeError("Existing frozen calibration manifest differs; choose a fresh evidence directory")
    manifest_path.write_text(encoded, encoding="utf-8")
    (output/"calibration-manifest.sha256").write_text(hashlib.sha256(encoded.encode()).hexdigest()+"\n", encoding="utf-8")
    w, h = manifest["raster_size"]
    results = []
    for variant in ("correct_rectangle", "missing_row", "missing_block", "duplicate_block", "hole", "wrong_color"):
        generated = coupon_generated(variant)
        required = [not (variant == "hole" and 8. < (i%w+.5)*20/w < 12. and 4. < (i//w+.5)*12/h < 8.) for i in range(w*h)]
        rows = [r for r in segments(generated, {1:"Tatami"}) if r["role"] == "TopFill"]
        result = dict(variant=variant, old_metric=q.witness_top_coverage(required, w, h, 20., 12., generated),
                      stability=stability(required, w, h, 20., 12., rows, expected_color=["red"]*(w*h)),
                      center_grid=sample(required, w, h, 20., 12., rows, expected_color=["red"]*(w*h)),
                      rows=horizontal_row_diagnostic(rows),
                      local_large_gaps=empty_disk_probes(required,w,h,20.,12.,rows),
                      exact_union_fraction=(exact_horizontal_union(rows, (0.,0.,20.,12.))/240. if variant in ("correct_rectangle","missing_row","duplicate_block","wrong_color") else None),
                      segment_roles=dict(Counter(r["role"] for r in segments(generated))))
        q.write(output/(variant+".json"), result)
        results.append(result)
    by_name = {r["variant"]: r for r in results}
    assert math.isclose(by_name["correct_rectangle"]["exact_union_fraction"], .75, abs_tol=1e-12)
    assert math.isclose(by_name["missing_row"]["exact_union_fraction"], .725, abs_tol=1e-12)
    assert math.isclose(by_name["duplicate_block"]["exact_union_fraction"], .75, abs_tol=1e-12)
    assert by_name["wrong_color"]["center_grid"]["color_correct_covered_fraction"] == 0.
    assert by_name["wrong_color"]["old_metric"]["covered_fraction"] == by_name["correct_rectangle"]["old_metric"]["covered_fraction"]
    assert by_name["missing_block"]["old_metric"]["covered_fraction"] < by_name["correct_rectangle"]["old_metric"]["covered_fraction"]
    assert by_name["duplicate_block"]["center_grid"]["multiple_segment_buffer_fraction"] > by_name["correct_rectangle"]["center_grid"]["multiple_segment_buffer_fraction"]
    assert math.isclose(by_name["duplicate_block"]["rows"]["repeated_horizontal_centerline_length_mm"], 40.)
    assert not by_name["correct_rectangle"]["local_large_gaps"]["probes"]
    assert by_name["missing_block"]["local_large_gaps"]["probes"]
    assert by_name["missing_row"]["rows"]["row_gaps"][0]["missing_nominal_rows"] == 1
    missing_without_underlay = [s for s in coupon_generated("missing_block") if s.get("role") != "Underlay"]
    assert q.witness_top_coverage([True]*(w*h),w,h,20.,12.,missing_without_underlay)["covered_fraction"] == by_name["missing_block"]["old_metric"]["covered_fraction"]
    q.write(output/"summary.json", dict(manifest_sha256=hashlib.sha256(encoded.encode()).hexdigest(), tests_passed=True, cases=results,
                                       exclusions="Underlay, Travel, Lock excluded; no fabric coverage, visual occlusion or machine trim claim."))
    render_calibration(output, results)
    return results


def render_calibration(output, results):
    from PIL import Image, ImageDraw
    scale = 20
    image = Image.new("RGB", (1200, 840), "#eeeeea")
    draw = ImageDraw.Draw(image)
    for i, result in enumerate(results):
        ox, oy = 40+(i%2)*600, 35+(i//2)*280
        draw.text((ox, oy), result["variant"], fill="black")
        draw.rectangle((ox, oy+20, ox+400, oy+260), fill="white", outline="#333333")
        if result["variant"] == "hole":
            draw.rectangle((ox+160, oy+100, ox+240, oy+180), fill="#eeeeea")
        for row in segments(coupon_generated(result["variant"])):
            if row["role"] != "TopFill":
                continue
            a, b = row["a"], row["b"]
            draw.line((ox+a[0]*scale, oy+20+a[1]*scale, ox+b[0]*scale, oy+20+b[1]*scale),
                      fill="#2455aa" if result["variant"] == "wrong_color" else "#aa3040", width=6)
    image.save(output/"calibration-coupons.png")


def aa_color_diagnostic(output):
    """Known area rasterization; test the old OKLab-segment hypothesis only."""
    from PIL import Image, ImageDraw
    output = output/"colored-aa"
    output.mkdir(parents=True, exist_ok=True)
    manifest = dict(version=1, endpoints={"red_blue":[[255,0,0],[0,0,255]], "red_green":[[255,0,0],[0,255,0]]},
                    coverage_alphas_u8=[0,32,64,128,191,223,255], old_oklab_residual_bound=.035,
                    models=["Pillow alpha_composite: encoded-sRGB 8-bit alpha", "linear-light sRGB area mixture then encoding"],
                    edge_geometry="64x32 pixels; half-plane x > 24 + y*0.375; exact 16x16 box area sampling per pixel",
                    threshold_tuning=False)
    encoded=json.dumps(manifest,sort_keys=True,indent=2)+"\n"
    path=output/"colored-edge-manifest.json"
    if path.is_file() and path.read_text(encoding="utf-8") != encoded:
        raise RuntimeError("Frozen colored-edge diagnostic changed")
    path.write_text(encoded,encoding="utf-8")

    def decode(v):
        v/=255
        return v/12.92 if v <= .04045 else ((v+.055)/1.055)**2.4

    def encode(v):
        return round(255*(12.92*v if v <= .0031308 else 1.055*v**(1/2.4)-.055))

    def blend(a,b,alpha,model):
        if model == "encoded_srgb":
            rgb=Image.alpha_composite(Image.new("RGBA",(1,1),(*a,255)),Image.new("RGBA",(1,1),(*b,alpha))).getpixel((0,0))[:3]
            expected=tuple((a[c]*(255-alpha)+b[c]*alpha+127)//255 for c in range(3))
            assert rgb == expected, (rgb,expected)
            return rgb
        t=alpha/255
        return tuple(encode((1-t)*decode(a[c])+t*decode(b[c])) for c in range(3))

    def projection(rgb,a,b):
        aa,bb,s=q.oklab(a),q.oklab(b),q.oklab(rgb)
        d=[bb[c]-aa[c] for c in range(3)]
        norm=sum(v*v for v in d)
        t=sum((s[c]-aa[c])*d[c] for c in range(3))/norm
        projected=[aa[c]+min(1.,max(0.,t))*d[c] for c in range(3)]
        residual=math.dist(s,projected)
        return dict(projected_t=t,oklab_segment_residual=residual,passes_old_residual_bound=residual<=.035,
                    nearest_endpoint_error=min(math.dist(s,aa),math.dist(s,bb)))

    rows=[]
    panel=Image.new("RGB",(1110,650),"#eeeeea")
    draw=ImageDraw.Draw(panel)
    for pair_index,(name,(a,b)) in enumerate(manifest["endpoints"].items()):
        for model_index,model in enumerate(("encoded_srgb","linear_light")):
            probes=[]
            for alpha in manifest["coverage_alphas_u8"]:
                rgb=blend(a,b,alpha,model)
                probes.append(dict(alpha_u8=alpha,coverage=alpha/255,rgb=rgb,**projection(rgb,a,b)))
            edge=Image.new("RGB",(64,32))
            mixed=[]
            for y in range(32):
                for x in range(64):
                    hits=sum((x+(sx+.5)/16)>24+(y+(sy+.5)/16)*.375 for sy in range(16) for sx in range(16))
                    alpha=round(hits/256*255)
                    rgb=blend(a,b,alpha,model)
                    edge.putpixel((x,y),rgb)
                    if 0<alpha<255:
                        mixed.append(projection(rgb,a,b)["oklab_segment_residual"])
            edge.save(output/(name+"-"+model+"-edge.png"))
            ox,oy=30+model_index*540,30+pair_index*320
            draw.text((ox,oy),name+" / "+model,fill="black")
            panel.paste(edge.resize((512,256),Image.Resampling.NEAREST),(ox,oy+20))
            rows.append(dict(pair=name,model=model,probes=probes,edge_mixed_pixels=len(mixed),
                             edge_maximum_oklab_segment_residual=max(mixed),edge_mixed_pixels_above_old_bound=sum(v>.035 for v in mixed)))
    panel.save(output/"known-colored-edges.png")
    result=dict(manifest_sha256=hashlib.sha256(encoded.encode()).hexdigest(),cases=rows,
                note="Independent known rasterization diagnostics, not thresholds fitted to a corpus. A straight OKLab segment is not a physically universal mixing model. This does not authorize removing detail or changing production AA/color space.")
    q.write(output/"summary.json",result)
    return result


def analyze_folder(folder, factors=(1, 2)):
    """Existing saved outputs only; no generation, parameter or oracle changes."""
    folder = Path(folder)
    plan = json.loads((folder/"design.plan.json").read_text(encoding="utf-8-sig"))
    project = json.loads((folder/"design.elab").read_text(encoding="utf-8-sig"))
    trace = json.loads((folder/"design.trace.json").read_text(encoding="utf-8-sig"))
    generated = [{**s, **m} for s, m in zip(plan["stitches"], plan["metadata"], strict=True)]
    kinds = {o["id"]:o.get("settings", {}).get("kind", "Unknown") for o in project["objects"]}
    rows = [r for r in segments(generated, kinds) if r["role"] == "TopFill"]
    w, h = trace["trace_dimensions"]
    sw, sh = trace["source_dimensions"]
    width = project["width_mm"]
    height = width*sh/sw
    options = json.loads((folder/"options.json").read_text(encoding="utf-8-sig"))
    required = q.threshold_witness(trace["labels_before_preparation"], w, h, options.get("min_region_pixels",1),
                                   options.get("min_region_area_mm2",0.), width*height/w/h)
    expected_colors = [trace["palette"][v] if v != 255 else None for v in trace["labels_before_preparation"]]
    by_type = {}
    for kind in ("Running", "Satin", "Tatami"):
        selected = [o for o in project["objects"] if kinds[o["id"]] == kind]
        if not selected:
            by_type[kind] = dict(objects=0, probes=0, covered_fraction=None)
            continue
        _, rgb = q.project_labels({**project, "objects":selected}, (w,h), True)
        support = [p[3] >= 128 for p in rgb.get_flattened_data()]
        by_type[kind] = dict(objects=len(selected), **sample(support,w,h,width,height,[r for r in rows if r["kind"] == kind]))
        by_type[kind]["reference_note"] = "Saved source geometry of objects with this stitch type; self-geometry coverage only. Running narrow centerlines are not expected to fill their source area like Tatami."
    return dict(old_metric=q.witness_top_coverage(required,w,h,width,height,generated),
                trace_dimensions=[w,h],physical_dimensions_mm=[width,height],
                required_witness_sha256=hashlib.sha256(bytes(required)).hexdigest(),required_pixels=sum(required),
                input_sha256={name:q.av.sha(folder/name) for name in ("design.plan.json","design.elab","design.trace.json","options.json")},
                stability=stability(required,w,h,width,height,rows,factors=factors,expected_color=expected_colors),
                center_grid=sample(required,w,h,width,height,rows,expected_color=expected_colors),
                source_geometry_by_stitch_type=by_type,
                top_fill_segments_by_object_type=dict(Counter(r["kind"] for r in rows)),
                local_large_gaps=empty_disk_probes(required,w,h,width,height,rows),
                horizontal_row_evidence=horizontal_row_diagnostic(rows),
                color_reference_note="Color-correct buffers use immutable pre-preparation quantized palette labels, not semantic artwork truth. A legitimate segmentation reassignment may lower this diagnostic. Independent vector reference checks decide color/detail preservation on synthetic fixtures.",
                note="Per-type global fractions share the immutable total witness denominator and may overlap; source_geometry_by_stitch_type uses each type's own saved geometry. Neither is an acceptance score. For color truth and required-geometry loss use the separate independent reference checker. Local gaps require geometry/context review.")


def truth_diagnostic(folder, truth_path, factors=(1,2,4)):
    """Optional independent authored RGBA witness, with the same buffer model."""
    from PIL import Image
    folder,truth_path=Path(folder),Path(truth_path)
    project=json.loads((folder/"design.elab").read_text(encoding="utf-8-sig"))
    plan=json.loads((folder/"design.plan.json").read_text(encoding="utf-8-sig"))
    generated=[{**s,**m} for s,m in zip(plan["stitches"],plan["metadata"],strict=True)]
    truth=Image.open(truth_path).convert("RGBA")
    w,h=truth.size
    width=project["width_mm"]
    height=(project.get("source_image") or {}).get("height_mm",width*h/w)
    pixels=list(truth.get_flattened_data())
    required=[p[3]>=128 for p in pixels]
    colors=[list(p[:3]) for p in pixels]
    rows=[r for r in segments(generated,{o["id"]:o.get("settings",{}).get("kind","Unknown") for o in project["objects"]}) if r["role"]=="TopFill"]
    _,restored=q.project_labels(project,(w,h),True)
    actual=list(restored.get_flattened_data())
    return dict(truth_sha256=q.av.sha(truth_path),truth_dimensions=[w,h],required_pixels=sum(required),
                old_metric=q.witness_top_coverage(required,w,h,width,height,generated),
                stability=stability(required,w,h,width,height,rows,factors,colors),
                center_grid=sample(required,w,h,width,height,rows,expected_color=colors),
                local_large_gaps=empty_disk_probes(required,w,h,width,height,rows),
                restored_geometry_rgba_errors=sum(a!=b for a,b in zip(pixels,actual,strict=True)),
                missing_required_geometry_pixels=sum(p[3]>=128 and a[3]<128 for p,a in zip(pixels,actual,strict=True)),
                note="Independent hand-authored RGBA truth, fixed support and color for every version. TopFill geometric buffer only; no fabric coverage/hidden-path claim. Exact saved-geometry raster and sewn-buffer metrics are separate.")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=q.ROOT/"outputs/quality-phase21-20261106/coverage")
    parser.add_argument("--folder", type=Path, action="append", help="Saved design folder(s) to diagnose without regenerating")
    parser.add_argument("--factors", default="1,2", help="Supersampling factors for saved designs; coupons always use 1,2,4,8")
    parser.add_argument("--aa-only", action="store_true", help="Known encoded-sRGB/linear-light colored edge diagnostic only")
    parser.add_argument("--truth",type=Path,help="Optional independent authored RGBA truth for all --folder cases")
    args = parser.parse_args()
    if args.aa_only:
        result=aa_color_diagnostic(args.output)
        for row in result["cases"]:
            print(row["pair"],row["model"],row["edge_maximum_oklab_segment_residual"],row["edge_mixed_pixels_above_old_bound"],flush=True)
    elif args.folder:
        for folder in args.folder:
            result=analyze_folder(folder, tuple(map(int,args.factors.split(","))))
            if args.truth:
                result["independent_truth_topfill"]=truth_diagnostic(folder,args.truth,tuple(map(int,args.factors.split(","))))
            relative = folder.resolve().relative_to(q.ROOT)
            destination = args.output/"saved-designs"/relative/"coverage-calibrated.json"
            q.write(destination, result)
            print(folder, result["old_metric"]["covered_fraction"], result["stability"]["last_mean_change"], destination, flush=True)
    else:
        for row in calibration(args.output):
            print(row["variant"], row["old_metric"]["covered_fraction"], row["stability"]["levels"][-1]["mean"], flush=True)


if __name__ == "__main__":
    main()
