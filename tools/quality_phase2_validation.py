"""Independent Phase 2 raster/needle comparison. No runtime dependency in Rust.

Reference labels come from Pillow vector primitives, never from the importer.
Acceptance thresholds are written by --prepare-only before candidate execution.
Quality is a conjunction of detail/topology/boundary constraints, not object count.
"""
from __future__ import annotations

import argparse
from collections import Counter, deque
import hashlib
import html
import json
import math
import os
from pathlib import Path
import random
import shutil
import subprocess
import sys
import time

import autodigitize_validation as av

ROOT = Path(__file__).resolve().parents[1]
DEFAULT = ROOT / "outputs/quality-phase2-20261106/comparison"
COLORS = [(0, 0, 0), (255, 255, 255), (200, 28, 45), (20, 93, 180), (245, 201, 30), (12, 155, 90)]
SEED = 2601106


def measured_run(command, folder, timeout=240):
    """Windows process peak working set, not Python allocator or system RAM."""
    import ctypes
    from ctypes import wintypes
    class Counters(ctypes.Structure):
        _fields_ = [("cb",wintypes.DWORD),("PageFaultCount",wintypes.DWORD)]+[(n,ctypes.c_size_t) for n in
                    ("PeakWorkingSetSize","WorkingSetSize","QuotaPeakPagedPoolUsage","QuotaPagedPoolUsage",
                     "QuotaPeakNonPagedPoolUsage","QuotaNonPagedPoolUsage","PagefileUsage","PeakPagefileUsage")]
    command = [str(c) for c in command]
    start = time.perf_counter()
    peak, exit_code = 0, None
    out, err = folder/"stdout.txt",folder/"stderr.txt"
    with out.open("wb") as stdout, err.open("wb") as stderr:
        process = subprocess.Popen(command,cwd=ROOT,stdout=stdout,stderr=stderr)
        while process.poll() is None:
            if os.name == "nt":
                counters = Counters()
                counters.cb = ctypes.sizeof(counters)
                if ctypes.windll.psapi.GetProcessMemoryInfo(wintypes.HANDLE(int(process._handle)),
                                                           ctypes.byref(counters),counters.cb):
                    peak = max(peak,counters.PeakWorkingSetSize)
            if time.perf_counter()-start > timeout:
                process.kill()
                process.wait()
                break
            time.sleep(.03)
        else:
            exit_code = process.returncode
    return dict(command=command,exit_code=exit_code,seconds=round(time.perf_counter()-start,5),
                peak_working_set_bytes=peak or None,
                peak_memory_method="Windows GetProcessMemoryInfo PeakWorkingSetSize, polled every 30ms",
                stdout=out.read_text(encoding="utf-8",errors="replace"),
                stderr=err.read_text(encoding="utf-8",errors="replace"))


def write(path, value):
    av.write_json(path, value)


def value_hash(value):
    encoded=json.dumps(value,ensure_ascii=False,sort_keys=True,separators=(",",":")).encode("utf-8")
    return hashlib.sha256(encoded).hexdigest()


def fixture_lock(manifest_path):
    """Hashes cover source bytes, oracle bytes, settings and frozen limits."""
    manifest=json.loads(manifest_path.read_text(encoding="utf-8"))
    return dict(manifest_sha256=av.sha(manifest_path),seed=manifest["seed"],cases=[
        dict(name=c["name"],source_sha256=av.sha(Path(c["source"])),
             truth_sha256=av.sha(Path(c["truth"])) if c.get("truth") else None,
             options_sha256=value_hash(c["options"]),limits_sha256=value_hash(c["limits"]),
             width_mm=c["width_mm"])
        for c in manifest["cases"]])


def load_frozen_manifest(manifest_path):
    manifest_path=manifest_path.resolve()
    lock_path=manifest_path.with_name("fixture-filehashes.json")
    expected=json.loads(lock_path.read_text(encoding="utf-8"))
    actual=fixture_lock(manifest_path)
    if actual != expected:
        raise RuntimeError("Frozen manifest/source/reference/settings/threshold hashes changed; refusing to regenerate or overwrite fixtures")
    return json.loads(manifest_path.read_text(encoding="utf-8"))["cases"]


def options(mode="Legacy", palette=8, size=256):
    return dict(max_dimension=size, palette_size=palette, background="TransparentOnly",
                min_region_pixels=1, min_region_area_mm2=0., simplify_tolerance_mm=0.,
                smoothing_iterations=0, preserve_shared_boundaries=True, denoise_passes=0,
                pinned_colors=[], preparation=mode)


def vector_image(kind, size, scale=1):
    """Known vector drawings in a 64-unit square, supersampled independently."""
    from PIL import Image, ImageDraw
    image = Image.new("RGBA", (size, size), (29, 47, 199, 0))
    draw = ImageDraw.Draw(image)
    factor = size / 64

    def box(bounds):
        return tuple(round(v*factor) for v in bounds)

    def rectangle(bounds, color):
        draw.rectangle(box(bounds), fill=(*COLORS[color], 255))

    def ellipse(bounds, color):
        draw.ellipse(box(bounds), fill=(*COLORS[color], 255) if color >= 0 else (1, 2, 3, 0))

    def polygon(points, color):
        draw.polygon([(round(x*factor), round(y*factor)) for x, y in points], fill=(*COLORS[color], 255))

    if kind == "flag":
        rectangle((0, 0, 64, 64), 1)
        rectangle((6, 12, 58, 52), 2)
        rectangle((21, 21, 43, 43), 1)
        rectangle((30, 16, 34, 48), 1)
    elif kind == "logo":
        polygon([(7, 53), (23, 9), (33, 9), (49, 53), (39, 53), (34, 39), (21, 39), (16, 53)], 3)
        polygon([(24, 31), (28, 19), (32, 31)], 1)
        rectangle((52, 13, 56, 53), 2)
        rectangle((52, 7, 56, 10), 5)
    elif kind == "emblem":
        polygon([(8, 7), (56, 7), (54, 41), (45, 53), (32, 60), (19, 53), (10, 41)], 0)
        polygon([(11, 10), (53, 10), (51, 40), (43, 50), (32, 56), (21, 50), (13, 40)], 4)
        ellipse((20, 16, 44, 42), 3)
        ellipse((25, 21, 39, 37), 1)
        rectangle((30, 13, 34, 48), 2)
        rectangle((17, 27, 47, 31), 2)
        for x in (16, 46):
            ellipse((x, 12, x+3, 15), 5)
        polygon([(26, 49), (32, 43), (38, 49), (32, 53)], 1)
    elif kind == "letters":
        ellipse((3, 7, 20, 57), 0)
        ellipse((8, 13, 15, 51), -1)
        rectangle((24, 7, 28, 57), 0)
        ellipse((24, 7, 42, 32), 0)
        ellipse((24, 31, 42, 57), 0)
        ellipse((29, 12, 37, 27), -1)
        ellipse((29, 36, 37, 52), -1)
        ellipse((45, 7, 62, 33), 3)
        ellipse((45, 31, 62, 57), 3)
        ellipse((50, 13, 57, 27), -1)
        ellipse((50, 37, 57, 51), -1)
    elif kind == "lines":
        rectangle((7, 7, 57, 9), 0)
        rectangle((7, 12, 57, 14), 0)
        rectangle((9, 22, 11, 55), 3)
        rectangle((14, 22, 16, 55), 3)
        rectangle((10, 37, 15, 39), 3)
        # Small rare colour and isolated same-colour symbol are deliberate.
        rectangle((48, 46, 51, 49), 5)
        rectangle((53, 49, 56, 52), 0)
        rectangle((24, 23, 44, 51), 2)
        rectangle((26, 25, 42, 49), 1)
    return image


def prepare(folder,unpinned=False,paired=False):
    from PIL import Image
    manifest_path=folder/"fixtures.json"
    if manifest_path.is_file():
        # Existing evidence is immutable. In particular, selecting an output
        # folder without its historical generation flags must NEVER redraw it.
        return load_frozen_manifest(manifest_path)
    folder.mkdir(parents=True, exist_ok=True)
    fixtures = folder / "fixtures"
    fixtures.mkdir(exist_ok=True)
    cases = []
    rng = random.Random(SEED)
    designs = [
        ("flag_exterior_interior_white", "flag", "tuning", 192, 60., "aa"),
        ("flat_logo", "logo", "tuning", 192, 60., "noise"),
        ("complex_emblem", "emblem", "tuning", 192, 60., "noise"),
        ("letters_O_B_8", "letters", "control", 192, 60., "aa"),
        ("thin_outline_parallel_bridge_rare", "lines", "control", 192, 60., "aa"),
        ("transparent_edges", "logo", "control", 192, 60., "aa"),
        ("jpeg_artifacts", "emblem", "control", 192, 60., "jpeg"),
        ("same_emblem_384px", "emblem", "control", 384, 60., "noise"),
        ("same_emblem_30mm", "emblem", "control", 192, 30., "noise"),
        ("same_emblem_90mm", "emblem", "control", 192, 90., "noise"),
    ]
    if unpinned:
        designs=[(name+"_unpinned",kind,group,size,width,artifact) for name,kind,group,size,width,artifact in designs
                 if name in ("flat_logo","complex_emblem","thin_outline_parallel_bridge_rare","jpeg_artifacts")]
    if paired:
        designs=[d for d in designs if d[0] == "complex_emblem"]
    for name, kind, group, size, width, artifact in designs:
        reference = vector_image(kind, size)
        image = vector_image(kind, size*4).resize((size, size), Image.Resampling.LANCZOS)
        if artifact == "noise":
            # Fixed weak near-black variations inside otherwise flat cores:
            # zero-area deletions and disconnected pinning cannot solve these.
            pix = image.load()
            for y in range(2, size-2):
                for x in range(2, size-2):
                    p = pix[x, y]
                    if p[3] == 255 and rng.random() < .045:
                        shift = rng.choice((-11, -7, 7, 11))
                        pix[x, y] = (*[max(0, min(255, c+shift)) for c in p[:3]], 255)
        background = "TransparentOnly"
        path = fixtures / (name + (".jpg" if artifact == "jpeg" else ".png"))
        if kind == "flag":
            background = {"ConnectedColors": {"rgb": [[255, 255, 255]], "tolerance": 12.}}
            # Reference excludes only exterior white, preserving internal white.
            rp = reference.load()
            for y in range(size):
                for x in range(size):
                    if rp[x, y][:3] == (255, 255, 255) and not (6*size/64 <= x <= 58*size/64 and 12*size/64 <= y <= 52*size/64):
                        rp[x, y] = (29, 47, 199, 0)
        if artifact == "jpeg":
            rgb = Image.new("RGB", image.size, "white")
            rgb.paste(image, mask=image.getchannel("A"))
            rgb.save(path, quality=72, subsampling=2)
            background = {"ConnectedColors": {"rgb": [[255, 255, 255]], "tolerance": 18.}}
        else:
            image.save(path)
        reference.save(fixtures / (name + ".truth.png"))
        base = options(size=max(256, size))
        base["background"] = background
        # Pinned colours are same between versions, strictly test no colour merge.
        base["pinned_colors"] = [] if unpinned else [list(c) for c in COLORS]
        cases.append(dict(name=name, kind=kind, group=group, source=str(path.resolve()),
                          truth=str((fixtures / (name + ".truth.png")).resolve()),
                          width_mm=width, options=base, artifact=artifact,
                          limits=dict(color_core_accuracy_min=.99 if artifact != "jpeg" else .97,
                                      boundary_error_max_mm=2.5*width/size,
                                      topology_exact=True, marked_core_recall_min=1.,
                                      notes="Core excludes a fixed two-pixel ambiguous raster boundary band; max boundary discrepancy is symmetric and conservative Manhattan distance.")))
    for name, source, width in [
        ("user_crest", Path("C:/Users/Admin/Downloads/Ездец.svg (1).jpg"), 100.),
        ("real_flat_logo", Path("C:/Users/Admin/Downloads/anduril_industries_logo.png"), 60.)]:
        if source.is_file() and not unpinned and not paired:
            base = options(size=512)
            base.update(min_region_area_mm2=.1, simplify_tolerance_mm=.1)
            cases.append(dict(name=name, group="real_unlabelled", source=str(source),
                              width_mm=width, options=base, limits=None,
                              note="No labelled reference; metrics and visual review are evidence, no detail-preservation proof."))
    if paired:
        base=cases[0]
        cases=[]
        for width in (30.,60.,90.):
            cases.append({**base,"name":f"identical_source_{int(width)}mm","group":"paired_control",
                          "width_mm":width,"limits":{**base["limits"],"boundary_error_max_mm":2.5*width/192}})
    manifest = dict(seed=SEED, preparation="experimental opt-in", frozen_before_candidate=True,
                    reference="Independent Pillow primitives before resampling/noise; source RGBA untouched.",
                    cases=cases)
    path = folder / "fixtures.json"
    if path.is_file():
        old = json.loads(path.read_text(encoding="utf-8"))
        if old != manifest:
            raise RuntimeError("Frozen fixture manifest differs: use a new output directory, never weaken thresholds after candidate results")
    else:
        write(path, manifest)
    write(folder/"fixture-filehashes.json",fixture_lock(path))
    return cases


def component_info(mask, w, h, holes=False):
    """Independent four-neighbour binary components; outside counted separately."""
    visited = bytearray(w*h)
    sizes, internal = [], 0
    for seed, occupied in enumerate(mask):
        if not occupied or visited[seed]:
            continue
        pending = deque([seed])
        visited[seed] = 1
        count, touches = 0, False
        while pending:
            i = pending.popleft()
            x, y = i % w, i // w
            count += 1
            touches |= x == 0 or y == 0 or x == w-1 or y == h-1
            for n in ((i-1 if x else -1), (i+1 if x+1<w else -1),
                      (i-w if y else -1), (i+w if y+1<h else -1)):
                if n >= 0 and mask[n] and not visited[n]:
                    visited[n] = 1
                    pending.append(n)
        sizes.append(count)
        internal += not touches
    return dict(count=len(sizes), internal=internal, sizes=sorted(sizes))


def distance_to(mask, w, h):
    distance = [w+h+1] * (w*h)
    pending = deque()
    for i, v in enumerate(mask):
        if v:
            distance[i] = 0
            pending.append(i)
    while pending:
        i = pending.popleft()
        x, y = i % w, i // w
        for n in ((i-1 if x else -1), (i+1 if x+1<w else -1),
                  (i-w if y else -1), (i+w if y+1<h else -1)):
            if n >= 0 and distance[n] > distance[i]+1:
                distance[n] = distance[i]+1
                pending.append(n)
    return distance


def threshold_witness(labels,w,h,min_pixels,min_area_mm2,pixel_area_mm2):
    """Independent pre-preparation retained-component provenance, no tracing."""
    visited=bytearray(w*h)
    witness=bytearray(w*h)
    for seed,label in enumerate(labels):
        if label==255 or visited[seed]:
            continue
        pending=[seed]
        visited[seed]=1
        for i in pending:
            x,y=i%w,i//w
            for n in ([i-1] if x else [])+([i+1] if x+1<w else [])+([i-w] if y else [])+([i+w] if y+1<h else []):
                if not visited[n] and labels[n]==label:
                    visited[n]=1
                    pending.append(n)
        if len(pending)>=min_pixels and len(pending)*pixel_area_mm2+1e-12>=min_area_mm2:
            for i in pending:
                witness[i]=1
    return witness


def witness_top_coverage(required,w,h,width_mm,height_mm,generated):
    """Top-only .30mm thread buffer against ORIGINAL retained raster samples."""
    cell,radius=.8,.15
    buckets={}
    previous=(0.,0.)
    segment_count=0
    for stitch in generated:
        point=(av.quantize(stitch["x"]),av.quantize(stitch["y"]))
        if stitch["command"] == "Stitch" and stitch.get("role") == "TopFill":
            segment_count+=1
            for gy in range(math.floor((min(previous[1],point[1])-radius)/cell),math.floor((max(previous[1],point[1])+radius)/cell)+1):
                for gx in range(math.floor((min(previous[0],point[0])-radius)/cell),math.floor((max(previous[0],point[0])+radius)/cell)+1):
                    buckets.setdefault((gx,gy),[]).append((previous,point))
        if stitch["command"] in ("Stitch","Jump"):
            previous=point
    covered=0
    for i,required_pixel in enumerate(required):
        if not required_pixel:
            continue
        x,y=(i%w+.5)*width_mm/w,(i//w+.5)*height_mm/h
        for a,b in buckets.get((math.floor(x/cell),math.floor(y/cell)),[]):
            dx,dy=b[0]-a[0],b[1]-a[1]
            t=min(1.,max(0.,((x-a[0])*dx+(y-a[1])*dy)/max(dx*dx+dy*dy,1e-30)))
            if math.hypot(x-a[0]-t*dx,y-a[1]-t*dy)<=radius+1e-8:
                covered+=1
                break
    total=sum(required)
    return dict(top_only=True,effective_thread_width_mm=.30,reference="Original pre-preparation retained raster witness",
                sample_pitch_x_mm=width_mm/w,sample_pitch_y_mm=height_mm/h,probes=total,covered=covered,
                covered_fraction=covered/max(1,total),top_segments=segment_count,
                note="Independent sampled geometric buffer diagnostic, not colour-aware artwork truth, mathematical containment or fabric coverage. Underlay/travel/locks excluded.")


def project_labels(project, size, return_rgb=False):
    from PIL import Image, ImageDraw, ImageChops
    w, h = size
    out = Image.new("I", size, 255)
    rgb_out = Image.new("RGBA",size)
    width = project["width_mm"]
    height = (project.get("source_image") or {}).get("height_mm",width*h/w)
    for obj in project["objects"]:
        mask = Image.new("1", size)
        for ring in source_rings(obj):
            part = Image.new("1", size)
            # Raster samples are at pixel centres. Pillow polygon spans are
            # inclusive, so render at 4x and sample centres to avoid +1px growth.
            fine = Image.new("1", (w*4, h*4))
            points = [(p["x"]*w/width*4-0.5, p["y"]*h/height*4-0.5) for p in ring]
            ImageDraw.Draw(fine).polygon(points, fill=1)
            fp = fine.load()
            pp = part.load()
            xs=[q[0] for q in points]
            ys=[q[1] for q in points]
            for y in range(max(0,math.floor(min(ys)/4)-1),min(h,math.ceil(max(ys)/4)+2)):
                for x in range(max(0,math.floor(min(xs)/4)-1),min(w,math.ceil(max(xs)/4)+2)):
                    pp[x, y] = fp[4*x+2, 4*y+2]
            mask = ImageChops.logical_xor(mask, part)
        color = min(range(len(COLORS)), key=lambda k: sum((a-b)**2 for a,b in zip(COLORS[k], obj["color"])))
        out.paste(color, mask=mask)
        rgb_out.paste((*obj["color"],255),mask=mask)
    return (out,rgb_out) if return_rgb else out


def oklab(rgb):
    r,g,b=[v/255/12.92 if v/255<=.04045 else ((v/255+.055)/1.055)**2.4 for v in rgb]
    l=(.4122214708*r+.5363325363*g+.0514459929*b)**(1/3)
    m=(.2119034982*r+.6806995451*g+.1073969566*b)**(1/3)
    s=(.0883024619*r+.2817188376*g+.6299787005*b)**(1/3)
    return (.2104542553*l+.793617785*m-.0040720468*s,
            1.9779984951*l-2.428592205*m+.4505937099*s,
            .0259040371*l+.7827717662*m-.808675766*s)


def source_rings(obj):
    """Automatic Running retains its filled raster source in the editable graph."""
    if obj.get("contours"):
        return obj["contours"]
    if (obj.get("centerline_graph") or {}).get("source_contours"):
        return obj["centerline_graph"]["source_contours"]
    column=obj.get("satin_column")
    if column:
        return [column["left"],column["right"]] if column.get("closed") else [column["left"]+list(reversed(column["right"]))]
    return []


def reference_metrics(case, project, folder):
    from PIL import Image
    truth = Image.open(case["truth"]).convert("RGBA")
    size = truth.size
    w, h = size
    expected = [255 if p[3] < 128 else COLORS.index(p[:3]) for p in truth.getdata()]
    predicted_image,actual_rgb = project_labels(project, size,True)
    predicted = list(predicted_image.getdata())
    preview = Image.new("RGBA", size)
    preview.putdata([(*COLORS[v], 255) if v != 255 else (0,0,0,0) for v in predicted])
    preview.save(folder / "regions.png")
    actual_rgb.save(folder/"actual-thread-colors.png")
    pixels = w*h
    boundary = [False]*pixels
    for i in range(pixels):
        x, y = i%w, i//w
        boundary[i] = any(expected[i] != expected[n] for n in
                          ([i-1] if x else [])+([i+1] if x+1<w else [])+
                          ([i-w] if y else [])+([i+w] if y+1<h else []))
    boundary_dist = distance_to(boundary, w, h)
    core = [i for i in range(pixels) if expected[i] != 255 and boundary_dist[i] > 2]
    accuracy = sum(expected[i] == predicted[i] for i in core)/max(1,len(core))
    details, topology_ok, all_cores = [], True, True
    maximum_boundary = 0
    for color in sorted((set(expected)|set(predicted))-{255}):
        em = [v == color for v in expected]
        pm = [v == color for v in predicted]
        ec, pc = component_info(em,w,h), component_info(pm,w,h)
        eh = component_info([not v for v in em],w,h)["internal"]
        ph = component_info([not v for v in pm],w,h)["internal"]
        exact = ec["count"] == pc["count"] and eh == ph
        topology_ok &= exact
        color_core = [i for i in core if expected[i] == color]
        recall = sum(predicted[i] == color for i in color_core)/len(color_core) if color_core else 1.
        all_cores &= recall >= case["limits"]["marked_core_recall_min"]
        inter = sum(a and b for a,b in zip(em,pm))
        union = sum(a or b for a,b in zip(em,pm))
        # Symmetric label-set distance bounds the local boundary displacement.
        ed, pd = distance_to(em,w,h), distance_to(pm,w,h)
        error = max([ed[i] for i,v in enumerate(pm) if v] + [pd[i] for i,v in enumerate(em) if v], default=0)
        maximum_boundary = max(maximum_boundary,error)
        details.append(dict(rgb=COLORS[color], truth_components=ec["count"], predicted_components=pc["count"],
                            truth_holes=eh, predicted_holes=ph, iou=round(inter/max(1,union),6),
                            protected_core_pixels=len(color_core), protected_core_recall=round(recall,6),
                            boundary_error_bound_mm=error*case["width_mm"]/w))
    maximum_mm = maximum_boundary*case["width_mm"]/w
    background_core = [i for i in range(pixels) if expected[i] == 255 and boundary_dist[i] > 2]
    background_leaks = sum(predicted[i] != 255 for i in background_core)
    actual_pixels=list(actual_rgb.get_flattened_data())
    expected_lab={k:oklab(c) for k,c in enumerate(COLORS)}
    actual_lab={p[:3]:oklab(p[:3]) for p in set(actual_pixels)}
    color_errors=[math.dist(expected_lab[expected[i]],actual_lab[actual_pixels[i][:3]])
                  for i in core if actual_pixels[i][3] >= 128]
    return dict(core_color_accuracy=round(accuracy,6), color_regions=details,
                exact_topology=bool(topology_ok), protected_core_retained=bool(all_cores),
                forbidden_background_core_pixels=len(background_core),
                forbidden_background_core_leaks=background_leaks,
                mean_core_actual_thread_oklab_error=sum(color_errors)/max(1,len(color_errors)),
                max_core_actual_thread_oklab_error=max(color_errors,default=0),
                color_metric_note="Semantic labels use nearest known reference colour; actual thread RGB OKLab error is independently reported and not hidden by class grouping.",
                symmetric_boundary_error_bound_mm=maximum_mm,
                accepted=accuracy >= case["limits"]["color_core_accuracy_min"] and topology_ok and all_cores
                and background_leaks == 0 and maximum_mm <= case["limits"]["boundary_error_max_mm"],
                boundary_note="Conservative Manhattan set distance on independent pixel-centre raster; no fabric claim.")


def evaluate(case, folder):
    import pyembroidery as pe
    target = folder / "design.dst"
    project = json.loads(target.with_suffix(".elab").read_text(encoding="utf-8-sig"))
    plan = json.loads(target.with_suffix(".plan.json").read_text(encoding="utf-8-sig"))
    generated = [{**s, **m} for s,m in zip(plan["stitches"],plan["metadata"],strict=True)]
    checks, records = av.inspect(target,generated,project,pe,{})
    role_counts = Counter(s.get("role", "Unknown") for s in generated if s["command"] == "Stitch")
    blocks = len({(s["object_id"],s.get("pass_id")) for s in generated if s["command"] == "Stitch"})
    source_project={**project,"objects":[{**o,"contours":source_rings(o)} for o in project["objects"]]}
    result = dict(objects=len(project["objects"]), contours=sum(len(o.get("contours",[])) for o in project["objects"]),
                  retained_source_rings=sum(len(source_rings(o)) for o in project["objects"]),
                  needle_blocks=blocks, role_stitches=dict(role_counts), independent_dst=checks,
                  geometry_topology=av.topology(source_project),
                  omission_warnings=[i for i in project.get("raster_recipe",{}).get("issues",[]) if i["object_id"] == 0],
                  limits=case["limits"], width_mm=case["width_mm"])
    if "truth" in case:
        result["reference"] = reference_metrics(case,project,folder)
        result["sampled_top_coverage"] = av.top_coverage(project,generated)
        from quality_phase2_sewout import segment_containment
        by_id = {o["id"]:o for o in project["objects"]}
        previous = (0.,0.)
        violations, checked, escaped = [], 0, 0.
        for index,stitch in enumerate(generated):
            point = (av.quantize(stitch["x"]),av.quantize(stitch["y"]))
            if stitch["command"] == "Stitch":
                obj = by_id[stitch["object_id"]]
                rings = source_rings(obj)
                if rings:
                    # Default physical compensation is exactly zero in these
                    # fixtures. DST endpoint rounding permits sqrt(2)*.05 mm.
                    compensation = obj.get("settings",{}).get("pull_comp_mm",0.)
                    if compensation:
                        raise RuntimeError("Independent coupon checker requires explicit compensated geometry")
                    passed,length = segment_containment(previous,point,rings,.075)
                    checked += 1
                    escaped += length
                    if not passed:
                        violations.append(dict(index=index,object_id=obj["id"],role=stitch.get("role"),escaped_mm=length))
            if stitch["command"] in ("Stitch","Jump"):
                previous = point
        expected_checked=sum(s["command"] == "Stitch" for s in generated)
        result["whole_sewn_segment_containment"] = dict(checked=checked,expected_sewn_segments=expected_checked,
                    complete=checked==expected_checked,violations=violations,
                    escaped_length_mm=escaped,allowance_mm=.075,passed=not violations and checked==expected_checked,
                    method="Analytical parameter intervals in even-odd polygon plus explicit 0.075mm edge capsules; all sewn roles, not endpoints only")
    else:
        result["reference"] = None
        result["sampled_top_coverage"] = "Unscored real source to avoid treating self-geometry as an independent artwork reference."
    report = folder / "design.trace.json"
    if report.is_file():
        stage = json.loads(report.read_text(encoding="utf-8"))
        result["stages"] = {k:v for k,v in stage.items() if not k.startswith("labels_")}
        if case["group"] == "real_unlabelled" and stage.get("labels_before_preparation"):
            from PIL import Image
            w,h=stage["trace_dimensions"]
            sw,sh=stage["source_dimensions"]
            area=case["width_mm"]**2*sh/sw/(w*h)
            config=case["options"]
            required=threshold_witness(stage["labels_before_preparation"],w,h,
                                      config["min_region_pixels"],config["min_region_area_mm2"],area)
            _,rgb=project_labels(project,(w,h),True)
            rgb.save(folder/"retained-source-geometry.png")
            actual=[p[3]>=128 for p in rgb.get_flattened_data()]
            missing=[i for i,(wanted,got) in enumerate(zip(required,actual,strict=True)) if wanted and not got]
            result["original_retained_raster_witness"]=dict(required_pixels=sum(required),missing_pixels=len(missing),
                    missing_area_mm2=len(missing)*area,missing_examples_xy=[(i%w,i//w) for i in missing[:100]],
                    passed=not missing,note="Independent flood of pre-preparation labels with original floors, checked against saved filled source geometry (Running graph snapshot/Satin rails). No semantic colour/detail annotation or fabric proof.")
            result["sampled_top_coverage"]=witness_top_coverage(required,w,h,case["width_mm"],case["width_mm"]*sh/sw,generated)
    av.render(folder / "preview.png",project,generated,records,pe,case["name"])
    return result


def trace_report(cli,case,config,folder):
    from PIL import Image
    execution = measured_run([cli,"trace-report",case["source"],folder/"design.trace.json",
                              case["width_mm"],folder/"options.json"],folder/"trace-command")
    write(folder/"trace-command.json",execution)
    if execution["exit_code"] != 0:
        return
    stage = json.loads((folder/"design.trace.json").read_text(encoding="utf-8"))
    size = tuple(stage["trace_dimensions"])
    palette = stage["palette"]
    independent={}
    for field,name in (("labels_before_preparation","before"),("labels_after_preparation","after")):
        if field in stage:
            labels=stage[field]
            independent[name]=sum(component_info([v==color for v in labels],*size)["count"]
                                  for color in set(labels)-{255})
    stage["independent_four_neighbor_components"]=independent
    stage["independent_component_counts_match"]=all(independent.get(name)==stage.get("components_"+name+"_preparation")
                                                    for name in ("before","after"))
    write(folder/"design.trace.json",stage)
    for field,name in (("labels_before_preparation","palette-labels-before.png"),
                       ("labels_after_preparation","prepared-labels-after.png")):
        if field in stage:
            image = Image.new("RGBA",size)
            image.putdata([(*palette[v],255) if v != 255 else (0,0,0,0) for v in stage[field]])
            image.save(folder/name)
    if "labels_before_preparation" in stage:
        changed = Image.new("RGBA",size)
        changed.putdata([(240,20,80,255) if a != b else (0,0,0,0) for a,b in
                         zip(stage["labels_before_preparation"],stage["labels_after_preparation"],strict=True)])
        changed.save(folder/"changed-pixels.png")


def run_version(cli, label, cases, output, profile,diagnostic_cli=None,reevaluate=False):
    results = []
    for case in cases:
        folder = output / label / case["name"]
        folder.mkdir(parents=True, exist_ok=True)
        config = dict(case["options"])
        if label == "baseline":
            config.pop("preparation", None)
        else:
            config["preparation"] = profile
        write(folder / "options.json",config)
        target = folder / "design.dst"
        # Preserve the exact fresh reproduction of the historical crest.
        preserved = ROOT / "outputs/quality-phase2-20261106/baseline"
        if reevaluate:
            execution=json.loads((folder/"command.json").read_text(encoding="utf-8"))
        elif label == "baseline" and case["name"] == "user_crest" and (preserved / "design.dst").is_file():
            for source in preserved.glob("design.*"):
                shutil.copy2(source,folder/source.name)
            execution = dict(command=[str(cli),"autodigitize",case["source"],str(target),str(case["width_mm"])],
                             exit_code=0, seconds=None, note="Copied fresh exact prechange reproduction, not rerun.")
        else:
            execution = measured_run([cli,"autodigitize",case["source"],target,case["width_mm"],folder/"options.json"],folder)
        write(folder / "command.json",execution)
        if diagnostic_cli and execution["exit_code"] == 0:
            (folder/"trace-command").mkdir(exist_ok=True)
            trace_report(diagnostic_cli,case,config,folder)
        executed_cli=Path(execution["command"][0])
        result = dict(name=case["name"], group=case["group"], cli_sha256=av.sha(executed_cli),
                      input_sha256=av.sha(Path(case["source"])), seconds=execution["seconds"],
                      peak_working_set_bytes=execution.get("peak_working_set_bytes"),
                      exit_code=execution["exit_code"], options=config)
        if execution["exit_code"] == 0:
            result.update(evaluate(case,folder))
        else:
            result["error"] = execution["stderr"]
        write(folder / "evaluation.json",result)
        results.append(result)
        print(label,case["name"],"exit",result["exit_code"],"objects",result.get("objects"),
              "reference",result.get("reference",{}).get("accepted") if result.get("reference") else "unlabelled",flush=True)
    prior_path=output/(label+".json")
    if prior_path.is_file():
        prior=json.loads(prior_path.read_text(encoding="utf-8"))
        replaced={r["name"] for r in results}
        results=[r for r in prior if r["name"] not in replaced]+results
    write(prior_path,results)
    return results


def report(output, cases):
    versions = {label:json.loads((output/(label+".json")).read_text(encoding="utf-8"))
                for label in ("baseline","candidate") if (output/(label+".json")).is_file()}
    rows, cards = [], []
    preservation=[]
    for case in cases:
        cells, pictures = [], []
        for label, results in versions.items():
            result = next(r for r in results if r["name"] == case["name"])
            dst = result.get("independent_dst",{})
            ref = result.get("reference")
            status = "unlabelled" if ref is None else ("PASS" if ref["accepted"] else "REVIEW")
            cells.append(f'<td>{label}: {status}<br>{result.get("objects","error")} objects / {result.get("needle_blocks","—")} blocks<br>{dst.get("needle_count","—")} needles / {dst.get("jump_count","—")} reader jumps<br>{dst.get("jump_length_mm","—")} mm jump; {dst.get("trim_intentions","—")} trims</td>')
            if result["exit_code"] == 0:
                pictures.append(f'<div><h3>{label}: {status}</h3><img src="{label}/{case["name"]}/preview.png"><a href="{label}/{case["name"]}/evaluation.json">Measurements</a></div>')
        before_path=output/"baseline"/case["name"]/"regions.png"
        after_path=output/"candidate"/case["name"]/"regions.png"
        if "truth" in case and before_path.is_file() and after_path.is_file():
            from PIL import Image
            truth=Image.open(case["truth"]).convert("RGBA")
            w,h=truth.size
            expected=list(truth.get_flattened_data())
            before=list(Image.open(before_path).convert("RGBA").get_flattened_data())
            after=list(Image.open(after_path).convert("RGBA").get_flattened_data())
            labels=[255 if p[3]<128 else COLORS.index(p[:3]) for p in expected]
            boundary=[False]*(w*h)
            for i in range(w*h):
                x,y=i%w,i//w
                boundary[i]=any(labels[n]!=labels[i] for n in
                               ([i-1] if x else [])+([i+1] if x+1<w else [])+
                               ([i-w] if y else [])+([i+w] if y+1<h else []))
            d=distance_to(boundary,w,h)
            core=[i for i in range(w*h) if d[i]>2]
            def same(a,b):
                return (a[3]<128 and b[3]<128) or a==b
            lost=sum(same(before[i],expected[i]) and not same(after[i],expected[i]) for i in core)
            fixed=sum(not same(before[i],expected[i]) and same(after[i],expected[i]) for i in core)
            preservation.append(dict(name=case["name"],known_correct_core_pixels_lost=lost,
                                     known_core_errors_corrected=fixed,passed=lost==0,
                                     note="Includes foreground details and transparent holes/exterior. Absolute reference gate remains separate."))
        rows.append('<tr><th>'+html.escape(case["name"])+f'<br>{case["width_mm"]} mm; '+case["group"]+'</th>'+''.join(cells)+'</tr>')
        cards.append('<article><h2>'+html.escape(case["name"])+f' — {case["width_mm"]} mm</h2><section>'+''.join(pictures)+'</section></article>')
    (output/"index.html").write_text('<!doctype html><meta charset="utf-8"><title>EmbroideryLab Phase 2</title><style>body{font:15px/1.4 system-ui;margin:24px;background:#f5f5f2;color:#253343}table{border-collapse:collapse}th,td{border:1px solid #aaa;padding:8px}article{background:white;margin-top:28px;padding:16px}section{display:flex;flex-wrap:wrap;gap:12px}section>div{flex:1;min-width:450px}img{width:100%}a{color:#1765a0}</style><h1>Phase 2: same sources, dimensions and stitch settings</h1><p>Experimental opt-in preparation. Object counts are complexity measurements. Acceptance requires independent reference detail, topology, colour and physical boundary constraints. Unlabelled real sources have no automatic artwork-quality verdict. Rendered thread width never changes between versions. No physical sew-out.</p><table>'+''.join(rows)+'</table>'+''.join(cards),encoding="utf-8")
    write(output/"summary.json",dict(seed=SEED,versions=versions,detail_regression=preservation,physical_sewout=False,
                                    note="Geometric checks are not proof of fabric quality."))


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--output",type=Path)
    parser.add_argument("--manifest",type=Path,help="Read and hash-verify existing frozen cases; never regenerate their inputs or thresholds")
    parser.add_argument("--baseline",type=Path)
    parser.add_argument("--candidate",type=Path)
    parser.add_argument("--profile",default="Standard")
    parser.add_argument("--prepare-only",action="store_true")
    parser.add_argument("--only",nargs="*")
    parser.add_argument("--diagnostic-cli",type=Path)
    parser.add_argument("--reevaluate",action="store_true")
    parser.add_argument("--unpinned",action="store_true")
    parser.add_argument("--paired",action="store_true")
    args = parser.parse_args()
    output=args.output or (args.manifest.parent if args.manifest else DEFAULT)
    cases=load_frozen_manifest(args.manifest) if args.manifest else prepare(output,args.unpinned,args.paired)
    if args.prepare_only:
        print("Frozen",len(cases),"cases",output)
        return
    if args.only:
        cases=[c for c in cases if c["name"] in args.only]
    if args.baseline:
        run_version(args.baseline.resolve(),"baseline",cases,output,args.profile,args.diagnostic_cli,args.reevaluate)
    if args.candidate:
        run_version(args.candidate.resolve(),"candidate",cases,output,args.profile,args.diagnostic_cli or args.candidate,args.reevaluate)
    report(output,cases)


if __name__ == "__main__":
    main()
