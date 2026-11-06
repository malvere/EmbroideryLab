"""Synthetic, independent baseline/candidate checks for EmbroideryLab.

Python 3.12+, Pillow 13.0.0, pyembroidery 1.5.1. All inputs are generated here;
no private artwork is read. A successful format check is not a fabric sew-out.
"""
from __future__ import annotations

import argparse
from collections import Counter
from datetime import datetime, timezone, timedelta
import hashlib
import html
import importlib
import importlib.metadata
import json
import math
from pathlib import Path
import platform
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[1]


def write_json(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, ensure_ascii=False, indent=2, allow_nan=False) + "\n", encoding="utf-8")


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def p(x, y):
    return {"x": float(x), "y": float(y)}


def rect(x, y, w, h):
    return [p(x, y), p(x+w, y), p(x+w, y+h), p(x, y+h)]


def circle(cx, cy, radius, n=64):
    return [p(cx+radius*math.cos(i*math.tau/n), cy+radius*math.sin(i*math.tau/n)) for i in range(n)]


def obj(name, rings, **changes):
    settings = dict(kind="Tatami", spacing_mm=.4, length_mm=3.5, angle_deg=0.,
                    auto_angle=False, underlay=False, pull_comp_mm=0., tie_in=False,
                    tie_off=False, underlay_inset_mm=.3, underlay_spacing_mm=2., underlay_length_mm=3.)
    settings.update(changes)
    return dict(id=1, name=name, contours=rings, color=[26, 87, 155], visible=True, settings=settings)


def prepare(output):
    from PIL import Image, ImageDraw
    folder = output / "fixtures"
    folder.mkdir(parents=True, exist_ok=True)
    cases = []

    def vector(name, objects, note, expected=None):
        value = {"version": 4, "name": name, "width_mm": 60., "objects": objects}
        for i, item in enumerate(objects):
            item["id"] = i+1
        path = folder / (name+".elab")
        write_json(path, value)
        cases.append(dict(name=name, source=path, mode="vector", note=note, expected=expected or {}))

    for comp in (0., .2):
        suffix = "0" if comp == 0 else "02"
        vector("rectangle_comp_"+suffix, [obj("Rectangle", [rect(5, 5, 30, 20)], pull_comp_mm=comp)],
               "30 x 20 mm rectangle; no underlay or locks", {"holes": [], "visible_objects": 1, "maximum_jumps": 1})
        vector("ring_comp_"+suffix, [obj("Rectangular ring", [rect(5, 5, 30, 20), rect(14, 10, 12, 10)], pull_comp_mm=comp)],
               "Protected 12 x 10 mm interior hole", {"holes": [rect(14, 10, 12, 10)], "visible_objects": 1, "hole_count": 1})
        vector("islands_comp_"+suffix, [obj("Independent components", [rect(5, 5, 10, 20), rect(25, 5, 10, 20)], pull_comp_mm=comp)],
               "Two independent islands in one editable object", {"holes": [rect(15.25, 5, 9.5, 20)], "visible_objects": 1, "component_count": 2})
    vector("multi_holes_island", [obj("B/8 holes", [rect(3, 3, 32, 40), rect(10, 9, 18, 11), rect(10, 26, 18, 11)]),
                                   obj("Island inside opening", [rect(16, 12, 5, 5)])],
           "Two holes and a separate island nested in the first opening", {"hole_count": 2, "component_count": 2})
    vector("c_shape", [obj("C shape", [[p(3, 3), p(35, 3), p(35, 10), p(12, 10), p(12, 30), p(35, 30), p(35, 37), p(3, 37)]])],
           "C shape; narrow neck, interval split/merge")
    vector("narrow_detail", [obj("0.12 mm thin detail", [rect(5, 5, 10, .12)])],
           "Previously fell between first scanline and boundary; must fill or diagnose explicitly", {"minimum_stitches": 1})
    for dense in (False, True):
        ring = rect(3, 3, 34, 12)
        if dense:
            ring = [p(3+i*12/100, 3) for i in range(101)]+[p(37, 3), p(37, 15), p(3, 15)]
        vector("auto_angle_"+("resampled" if dense else "corners"), [obj("Same polygon", [ring], auto_angle=True)],
               "Identical area; collinear vertices must not change angle/trajectory")
    for inset, spacing, length in ((.3, 2., 3.), (1.3, 3., 1.5)):
        tag = "default" if inset == .3 else "custom"
        vector("underlay_"+tag, [obj("Independent underlay settings", [rect(5, 5, 30, 20), rect(14, 10, 12, 10)],
                                    underlay=True, underlay_inset_mm=inset, underlay_spacing_mm=spacing,
                                    underlay_length_mm=length)], "Underlay settings must affect its own pass")
    layered = [obj("Red foundation", [rect(3, 3, 40, 30)]), obj("Blue insert", [rect(10, 9, 25, 18)]),
               obj("Red top detail", [rect(18, 13, 8, 10)])]
    layered[0]["color"] = layered[2]["color"] = [190, 30, 45]
    layered[1]["color"] = [20, 70, 170]
    vector("layer_order", layered, "Red / blue / red overlap; global color sorting would change artwork")
    vector("acute_variable_column", [obj("Variable narrow column", [[p(4, 6), p(38, 4), p(43, 18), p(38, 7), p(4, 9)]])],
           "Ambiguous acute corner: conservative classification or concrete warning")
    vector("y_branch", [obj("Y branch", [[p(18, 40), p(22, 40), p(22, 21), p(36, 7), p(33, 4),
                                          p(20, 17), p(7, 4), p(4, 7), p(18, 21)]])],
           "Branched geometry must not be falsely labelled a simple Satin column")

    options = dict(max_dimension=256, palette_size=6, background={"ConnectedColors": {"rgb": [[255, 255, 255]], "tolerance": 25.}},
                   min_region_pixels=1, simplify_tolerance_mm=0., smoothing_iterations=0)
    options_path = folder / "raster-options.json"
    write_json(options_path, options)
    # 6 x supersampling is solely for deterministic anti-aliased fixtures.
    def flag(size, aa=False):
        factor = 6 if aa else 1
        image = Image.new("RGBA", (size*factor, size*factor), "white")
        d = ImageDraw.Draw(image)
        xy = lambda box: tuple(round(v*size*factor/128) for v in box)
        d.rectangle(xy((17, 24, 111, 104)), fill="#111111")
        d.rectangle(xy((21, 28, 107, 100)), fill="#245ba4")
        d.rectangle(xy((21, 52, 107, 76)), fill="#ffffff")
        d.rectangle(xy((21, 76, 107, 100)), fill="#ce2439")
        d.rectangle(xy((51, 37, 77, 91)), fill="#111111")
        d.rectangle(xy((55, 41, 73, 87)), fill="#e5b52e")
        if factor != 1:
            image = image.resize((size, size), Image.Resampling.LANCZOS)
        return image

    def raster(name, image, note, width=64., extension="png", options_file=options_path, expected=None):
        path = folder / (name+"."+extension)
        if extension == "jpg":
            image.convert("RGB").save(path, quality=82, subsampling=0)
        else:
            image.save(path)
        # Independent illustration of the identical background decision used
        # by both CLIs. This is not a screenshot of the application's mask.
        from collections import deque
        exclusion = Image.new("RGBA", image.size, (0, 0, 0, 0))
        mask_source = Image.open(path).convert("RGBA")
        pixels = mask_source.load()
        selected = bytearray(image.width*image.height)
        queue = deque()
        connected_white = options_file == options_path
        def matches(x, y):
            rgba = pixels[x, y]
            return rgba[3] < 128 or (connected_white and sum((v-255)**2 for v in rgba[:3]) <= 25.**2)
        for y in range(image.height):
            for x in range(image.width):
                if (x in (0, image.width-1) or y in (0, image.height-1)) and matches(x, y):
                    selected[y*image.width+x] = 1
                    queue.append((x, y))
        while queue:
            x, y = queue.popleft()
            for nx, ny in ((x-1, y), (x+1, y), (x, y-1), (x, y+1)):
                if 0 <= nx < image.width and 0 <= ny < image.height and not selected[ny*image.width+nx] and matches(nx, ny):
                    selected[ny*image.width+nx] = 1
                    queue.append((nx, ny))
        # Transparency is excluded everywhere, including enclosed holes. It
        # must not become an extra flood seed for selecting enclosed white.
        for y in range(image.height):
            for x in range(image.width):
                if pixels[x, y][3] < 128:
                    selected[y*image.width+x] = 1
        for i, value in enumerate(selected):
            if value:
                exclusion.putpixel((i%image.width, i//image.width), (210, 30, 90, 145))
        binary = Image.new("L", image.size, 0)
        binary.putdata([255 if value else 0 for value in selected])
        binary.save(folder / (name+".excluded.png"))
        preview = mask_source.copy()
        preview.alpha_composite(exclusion)
        preview.save(folder / (name+".mask.png"))
        cases.append(dict(name=name, source=path, mode="raster", width_mm=width, options=options_file,
                          note=note, expected=expected or {}))

    raster("flag_128", flag(128), "Exterior white excluded, enclosed white stripe must remain", expected={"interior_white": (13., 29.)})
    raster("flag_256", flag(256), "Same 64 mm physical size; 2 x source resolution")
    raster("flag_small_mm", flag(128), "Same pixels at 32 mm physical width", width=32.)
    raster("flag_aa", flag(256, True), "Anti-aliased flag; no global erosion")
    raster("flag_jpeg", flag(256, True), "JPEG noise; palette is bounded, no dithering", extension="jpg")
    alpha = flag(128)
    for y in range(128):
        for x in range(128):
            if x < 17 or x > 111 or y < 24 or y > 104:
                alpha.putpixel((x, y), ((x*37)%256, (y*53)%256, ((x+y)*97)%256, 0))
    keep_options = dict(options, background="TransparentOnly")
    keep_path = folder / "keep-opaque-options.json"
    write_json(keep_path, keep_options)
    raster("flag_transparent_hidden_rgb", alpha, "Fully transparent random RGB must not enter palette", options_file=keep_path)
    topology = Image.new("RGBA", (128, 128), (0, 0, 0, 0))
    d = ImageDraw.Draw(topology)
    d.ellipse((7, 8, 61, 116), fill="#1c5d9a")
    d.ellipse((16, 18, 52, 106), fill=(255, 0, 255, 0))
    d.ellipse((29, 47, 38, 77), fill="#be2d3f")
    d.rectangle((72, 9, 120, 117), fill="#263142")
    d.rectangle((83, 21, 110, 51), fill=(3, 99, 5, 0))
    d.rectangle((83, 73, 110, 105), fill=(1, 2, 3, 0))
    raster("topology_O_B_8", topology, "Ring, two holes, nested island; arbitrary hidden RGB", options_file=keep_path, expected={"hole_count": 3, "component_count": 3})
    junction = Image.new("RGBA", (128, 128), (0, 0, 0, 0))
    d = ImageDraw.Draw(junction)
    d.rectangle((10, 10, 64, 117), fill="#c92e3f")
    d.rectangle((65, 10, 117, 64), fill="#2a699b")
    d.rectangle((65, 65, 117, 117), fill="#d7aa31")
    d.point((3, 3), fill="#263142")
    d.point((4, 4), fill="#263142")
    raster("t_junction_diagonal_contact", junction, "Three shared colors and 2 x 2 diagonal isolated pixels", options_file=keep_path, expected={"component_count": 5})
    write_json(output / "fixtures.json", [{**c, "source": str(c["source"]), "options": str(c.get("options", ""))} for c in cases])
    return cases


def run(command):
    begin = time.perf_counter()
    try:
        result = subprocess.run([str(v) for v in command], cwd=ROOT, capture_output=True,
                                encoding="utf-8", errors="replace", timeout=180)
        return dict(command=[str(v) for v in command], exit_code=result.returncode,
                    stdout=result.stdout, stderr=result.stderr, seconds=round(time.perf_counter()-begin, 5))
    except subprocess.TimeoutExpired as error:
        return dict(command=[str(v) for v in command], exit_code=None, stdout="", stderr=str(error),
                    seconds=round(time.perf_counter()-begin, 5))


def quantize(value):
    return (math.floor(value*10+.5) if value >= 0 else math.ceil(value*10-.5))/10


def inside(point, rings):
    x, y = point
    odd = False
    for ring in rings:
        for a, b in zip(ring, ring[1:]+ring[:1]):
            if (a["y"] > y) != (b["y"] > y) and x < a["x"]+(y-a["y"])*(b["x"]-a["x"])/(b["y"]-a["y"]):
                odd = not odd
    return odd


def dist_boundary(point, rings):
    x, y = point
    distance = math.inf
    for ring in rings:
        for a, b in zip(ring, ring[1:]+ring[:1]):
            dx, dy = b["x"]-a["x"], b["y"]-a["y"]
            t = min(1., max(0., ((x-a["x"])*dx+(y-a["y"])*dy)/max(dx*dx+dy*dy, 1e-30)))
            distance = min(distance, math.hypot(x-a["x"]-t*dx, y-a["y"]-t*dy))
    return distance


def top_coverage(saved, generated):
    """Independent sampled thread-buffer estimate, separate from punctures.

    Fixed effective width is 0.30 mm; sample pitch is 0.23 mm. These are
    reported diagnostic assumptions, not fabric or renderer parameters.
    Pitch deliberately differs from the 0.40 mm row spacing to avoid aliasing.
    Legacy plans cannot distinguish sewn travel/locks from top and are unscored.
    """
    radius, pitch, cell = .15, .23, .80
    objects = {o["id"]: o for o in saved["objects"] if o.get("visible", True)}
    buckets = {}
    previous = (0., 0.)
    unavailable = set()
    for stitch in generated:
        current = (stitch["x"], stitch["y"])
        if stitch["command"] == "Stitch":
            role = stitch.get("role")
            if role is None:
                unavailable.add(stitch["object_id"])
            elif role == "TopFill":
                grid = buckets.setdefault(stitch["object_id"], {})
                a = (quantize(previous[0]), quantize(previous[1]))
                b = (quantize(current[0]), quantize(current[1]))
                for iy in range(math.floor((min(a[1], b[1])-radius)/cell), math.floor((max(a[1], b[1])+radius)/cell)+1):
                    for ix in range(math.floor((min(a[0], b[0])-radius)/cell), math.floor((max(a[0], b[0])+radius)/cell)+1):
                        grid.setdefault((ix, iy), []).append((a, b))
        if stitch["command"] in ("Stitch", "Jump"):
            previous = current
    details = []
    for identity, item in objects.items():
        rings = item.get("contours", [])
        if identity in unavailable or not rings or item.get("settings", {}).get("kind") == "Running":
            continue
        points = [q for ring in rings for q in ring]
        if not points:
            continue
        x0, x1 = min(q["x"] for q in points), max(q["x"] for q in points)
        y0, y1 = min(q["y"] for q in points), max(q["y"] for q in points)
        total, covered = 0, 0
        grid = buckets.get(identity, {})
        for iy in range(max(1, math.ceil((y1-y0)/pitch))):
            y = y0+(iy+.5)*pitch
            for ix in range(max(1, math.ceil((x1-x0)/pitch))):
                x = x0+(ix+.5)*pitch
                if not inside((x, y), rings):
                    continue
                total += 1
                for a, b in grid.get((math.floor(x/cell), math.floor(y/cell)), []):
                    dx, dy = b[0]-a[0], b[1]-a[1]
                    t = min(1., max(0., ((x-a[0])*dx+(y-a[1])*dy)/max(dx*dx+dy*dy, 1e-30)))
                    if math.hypot(x-a[0]-t*dx, y-a[1]-t*dy) <= radius+1e-8:
                        covered += 1
                        break
        details.append(dict(object_id=identity, probes=total, covered=covered,
                            covered_fraction=round(covered/total, 5) if total else None))
    total = sum(v["probes"] for v in details)
    covered = sum(v["covered"] for v in details)
    return dict(effective_thread_width_mm=.3, sample_pitch_mm=.23, top_only=True,
                role_partition_verified=True,
                unavailable_legacy_object_ids=sorted(unavailable), probes=total, covered=covered,
                covered_fraction=round(covered/total, 5) if total else None, objects=details)


def topology(saved):
    shells = holes = 0
    area = 0.
    for item in saved["objects"]:
        if not item.get("visible", True):
            continue
        rings = item.get("contours", [])
        for index, ring in enumerate(rings):
            if not ring:
                continue
            point = (ring[0]["x"], ring[0]["y"])
            depth = sum(inside(point, [other]) for j, other in enumerate(rings) if index != j)
            ring_area = abs(sum(a["x"]*b["y"]-b["x"]*a["y"] for a, b in zip(ring, ring[1:]+ring[:1])))/2
            if depth % 2:
                holes += 1
                area -= ring_area
            else:
                shells += 1
                area += ring_area
    return dict(component_count=shells, hole_count=holes, visible_area_mm2=round(area, 6))


def raster_occupancy(case, records, pe):
    """Check exported thread against the independent full input exclusion map."""
    from PIL import Image
    image = Image.open(case["source"].with_name(case["name"]+".excluded.png"))
    excluded = image.load()
    scale = case["width_mm"]/image.width
    allowance = .075
    faces = {}
    probes = escaped = 0
    failures = []
    previous = (0., 0.)
    def allowed(q):
        x, y = q
        ix, iy = math.floor(x/scale), math.floor(y/scale)
        if not (0 <= ix < image.width and 0 <= iy < image.height):
            cx, cy = min(image.width-1, max(0, ix)), min(image.height-1, max(0, iy))
            dx = max(0., -x, x-image.width*scale)
            dy = max(0., -y, y-image.height*scale)
            return not excluded[cx, cy] and math.hypot(dx, dy) <= allowance
        if not excluded[ix, iy]:
            return True
        key = (ix, iy)
        if key not in faces:
            boundary = []
            for cy in range(max(0, iy-1), min(image.height, iy+2)):
                for cx in range(max(0, ix-1), min(image.width, ix+2)):
                    if not excluded[cx, cy]:
                        continue
                    for nx, ny, a, b in ((cx-1, cy, (cx, cy), (cx, cy+1)),
                                         (cx+1, cy, (cx+1, cy), (cx+1, cy+1)),
                                         (cx, cy-1, (cx, cy), (cx+1, cy)),
                                         (cx, cy+1, (cx, cy+1), (cx+1, cy+1))):
                        if 0 <= nx < image.width and 0 <= ny < image.height and not excluded[nx, ny]:
                            boundary.append(((a[0]*scale, a[1]*scale), (b[0]*scale, b[1]*scale)))
            faces[key] = boundary
        for a, b in faces[key]:
            dx, dy = b[0]-a[0], b[1]-a[1]
            t = min(1., max(0., ((x-a[0])*dx+(y-a[1])*dy)/max(dx*dx+dy*dy, 1e-30)))
            if math.hypot(x-a[0]-t*dx, y-a[1]-t*dy) <= allowance+1e-8:
                return True
        return False
    for x, y, flag in records:
        current = (x, y)
        if flag == pe.STITCH:
            steps = max(1, math.ceil(math.dist(previous, current)/.1))
            for i in range(steps+1):
                q = (previous[0]+(x-previous[0])*i/steps, previous[1]+(y-previous[1])*i/steps)
                probes += 1
                if not allowed(q):
                    escaped += 1
                    if len(failures) < 8:
                        failures.append(q)
        if flag in (pe.STITCH, pe.JUMP):
            previous = current
    return dict(probes=probes, beyond_source_mask=escaped, first_failures=failures,
                source_pixel_mm=scale, sample_pitch_mm=.1, quantization_allowance_mm=allowance)


def inspect(target, generated, saved, pe, expected):
    pattern = pe.read(str(target))
    records = [(x/10, y/10, pe.decode_embroidery_command(code)[0]) for x, y, code in pattern.stitches]
    needle = [(x, y) for x, y, flag in records if flag == pe.STITCH]
    source_stitches = [s for s in generated if s["command"] == "Stitch"]
    objects = {o["id"]: o for o in saved["objects"]}
    underlay_passes = {(s["object_id"], s.get("pass_id")) for s in generated if s.get("role") == "Underlay"}
    machine_limit = saved.get("machine_profile", {}).get("max_stitch_mm", math.inf)
    ambiguous_length_objects = set()
    def stitch_limit(stitch):
        settings = objects.get(stitch.get("object_id"), {}).get("settings", {})
        top, underlay = settings.get("length_mm", 3.5), settings.get("underlay_length_mm", 3.)
        role = stitch.get("role")
        if role is None and settings.get("underlay", True):
            ambiguous_length_objects.add(stitch.get("object_id"))
            limit = max(top, underlay)
        elif role == "Underlay" or (role in ("Travel", "Lock") and (stitch.get("object_id"), stitch.get("pass_id")) in underlay_passes):
            limit = underlay
        else:
            limit = top
        return min(limit, machine_limit)
    length_limits = [stitch_limit(s) for s in source_stitches]
    expected_xy = [(quantize(s["x"]), quantize(s["y"])) for s in source_stitches]
    same = len(needle) == len(expected_xy) and all(math.dist(a, b) < 1e-8 for a, b in zip(needle, expected_xy))
    counts = Counter(flag for _, _, flag in records)
    jumps, nonzero_jumps, stitches, jump_length, stitch_length, maximum = 0, 0, 0, 0., 0., 0.
    holes_crossed = probes = 0
    lengths = []
    length_violations = []
    sewn_points = []
    prev = (0., 0.)
    final = prev
    actual_controls = []
    actual_trim_anchors = []
    controls_stationary = True
    holes = expected.get("holes", [])
    # The 0.1 mm grid yields a maximum point displacement of sqrt(2)*0.05 mm.
    # Use 0.075 mm as a diagnostic boundary allowance, not a wider render stroke.
    allowance = .075
    for x, y, flag in records:
        pos = (x, y)
        if flag == pe.JUMP:
            jumps += 1
            distance = math.dist(prev, pos)
            nonzero_jumps += distance > 1e-9
            jump_length += distance
        if flag == pe.STITCH:
            distance = math.dist(prev, pos)
            limit = length_limits[stitches] if stitches < len(length_limits) else machine_limit
            if distance > limit+1e-7:
                source = source_stitches[stitches] if stitches < len(source_stitches) else {}
                if len(length_violations) < 12:
                    length_violations.append(dict(needle_index=stitches, object_id=source.get("object_id"), role=source.get("role"), length_mm=distance, limit_mm=limit))
            stitches += 1
            lengths.append(distance)
            stitch_length += distance
            maximum = max(maximum, distance)
            sewn_points.extend((prev, pos))
            if holes:
                steps = max(1, math.ceil(distance/.05))
                for i in range(steps+1):
                    q = (prev[0]+(x-prev[0])*i/steps, prev[1]+(y-prev[1])*i/steps)
                    probes += 1
                    if inside(q, holes) and dist_boundary(q, holes) > allowance:
                        holes_crossed += 1
        if flag in (pe.COLOR_CHANGE, pe.STOP):
            actual_controls.append((stitches, pos))
        if flag == pe.TRIM:
            actual_trim_anchors.append((stitches, pos))
        if flag in (pe.COLOR_CHANGE, pe.STOP, pe.TRIM, pe.END):
            controls_stationary &= math.dist(prev, pos) < 1e-8
        if flag in (pe.STITCH, pe.JUMP):
            prev = pos
            final = pos
    planned_colors = sum(s["command"] == "ColorChange" for s in generated)
    end_ok = counts[pe.END] == 1 and bool(records) and records[-1][2] == pe.END
    color_ok = counts[pe.COLOR_CHANGE]+counts[pe.STOP] == planned_colors
    source_final = next(((quantize(s["x"]), quantize(s["y"])) for s in reversed(generated)
                         if s["command"] in ("Stitch", "Jump")), (0., 0.))
    final_ok = math.dist(source_final, final) < 1e-8
    bounds = [min((q[axis] for q in sewn_points), default=0.) for axis in (0, 1)]+[max((q[axis] for q in sewn_points), default=0.) for axis in (0, 1)]
    source_points = []
    source_previous = (0., 0.)
    source_controls = []
    source_trim_anchors = []
    source_needle_count = 0
    source_controls_stationary = True
    for stitch in generated:
        pos = (quantize(stitch["x"]), quantize(stitch["y"]))
        if stitch["command"] == "Stitch":
            source_points.extend((source_previous, pos))
            source_needle_count += 1
        if stitch["command"] == "ColorChange":
            source_controls.append((source_needle_count, pos))
        if stitch["command"] == "Trim":
            source_trim_anchors.append((source_needle_count, pos))
        if stitch["command"] in ("ColorChange", "Trim", "End"):
            source_controls_stationary &= math.dist(source_previous, pos) < 1e-8
        if stitch["command"] in ("Stitch", "Jump"):
            source_previous = pos
    source_bounds = [min((q[axis] for q in source_points), default=0.) for axis in (0, 1)]+[max((q[axis] for q in source_points), default=0.) for axis in (0, 1)]
    dimensions_ok = all(abs(a-b) < 1e-8 for a, b in zip(bounds, source_bounds))
    color_anchors_ok = len(source_controls) == len(actual_controls) and all(a[0] == b[0] and math.dist(a[1], b[1]) < 1e-8 for a, b in zip(source_controls, actual_controls))
    anchor_key = lambda v: (v[0], round(v[1][0], 7), round(v[1][1], 7))
    expected_trims = Counter(anchor_key(v) for v in source_trim_anchors)
    raw_dst_jump_count = raw_dst_marker_count = None
    trim_encoding = saved.get("machine_profile", {}).get("dst_trim_encoding", "ThreeJump")
    if target.suffix.lower() == ".dst":
        # Disable PyEmbroidery's heuristic clipping so the profile's explicit
        # three-zero-Jump representation can be verified independently.
        raw = pe.read(str(target), settings={"trim_at": 999999999, "clipping": False})
        raw_dst_jump_count = 0
        raw_needles, zero_run, raw_previous = 0, 0, (0., 0.)
        markers = []
        for x, y, code in raw.stitches:
            flag = pe.decode_embroidery_command(code)[0]
            pos = (x/10, y/10)
            if flag == pe.STITCH:
                raw_needles += 1
            if flag == pe.JUMP:
                raw_dst_jump_count += 1
            if flag == pe.JUMP and math.dist(pos, raw_previous) < 1e-8:
                zero_run += 1
                if zero_run % 3 == 0:
                    markers.append((raw_needles, pos))
            else:
                zero_run = 0
            if flag in (pe.STITCH, pe.JUMP):
                raw_previous = pos
        raw_dst_marker_count = len(markers)
        actual_marker_counts = Counter(anchor_key(v) for v in markers)
        trim_mapping_ok = trim_encoding != "ThreeJump" or all(actual_marker_counts[key] >= count for key, count in expected_trims.items())
    else:
        trim_mapping_ok = Counter(anchor_key(v) for v in actual_trim_anchors) == expected_trims
    limits = max(length_limits, default=min(3.5, machine_limit))
    # Split quantized segments should stay within physical length (plus 1e-7).
    physical_length_ok = not length_violations
    roles = Counter(s.get("role", "LegacyUnspecified") for s in source_stitches)
    return dict(needle_coordinates_equal=same, end_ok=end_ok, color_changes_equal=color_ok,
                final_position_equal=final_ok, needle_count=stitches, jump_count=jumps,
                nonzero_jump_count=nonzero_jumps,
                planned_jump_count=sum(s["command"] == "Jump" for s in generated),
                jump_length_mm=round(jump_length, 5), stitch_length_mm=round(stitch_length, 5),
                maximum_stitch_mm=round(maximum, 8), configured_maximum_mm=limits,
                physical_length_ok=physical_length_ok, short_stitches_below_03_mm=sum(v < .3 for v in lengths),
                per_segment_length_violations=length_violations,
                legacy_length_role_ambiguity_object_ids=sorted(ambiguous_length_objects),
                color_control_anchors_equal=color_anchors_ok,
                controls_stationary=controls_stationary and source_controls_stationary,
                trim_profile_mapping_verified=trim_mapping_ok,
                dst_trim_encoding=trim_encoding if target.suffix.lower() == ".dst" else None,
                raw_dst_jump_records=raw_dst_jump_count,
                raw_dst_zero_jump_trim_triples=raw_dst_marker_count,
                bounds_mm=bounds, color_changes=planned_colors, trim_intentions=sum(s["command"] == "Trim" for s in generated),
                stitched_dimensions_equal=dimensions_ok,
                inferred_trims=counts[pe.TRIM], roles=dict(roles), hole_probes=probes,
                protected_hole_probes=holes_crossed, quantization_allowance_mm=allowance,
                routing_expectation_ok=jumps <= expected.get("maximum_jumps", math.inf),
                detail_retained=stitches >= expected.get("minimum_stitches", 0),
                format_geometry_passed=same and end_ok and color_ok and color_anchors_ok and controls_stationary and source_controls_stationary and trim_mapping_ok and final_ok and dimensions_ok and physical_length_ok and holes_crossed == 0,
                sha256=sha(target)), records


def render(path, project, generated, records, pe, note):
    from PIL import Image, ImageDraw, ImageChops, ImageFont
    coords = [(0., 0.)]+[(x, y) for x, y, flag in records if flag in (pe.STITCH, pe.JUMP)]
    if not coords:
        return
    x0, y0 = min(x for x, _ in coords), min(y for _, y in coords)
    x1, y1 = max(x for x, _ in coords), max(y for _, y in coords)
    scale = min(650/max(x1-x0, 1), 560/max(y1-y0, 1))
    tf = lambda q: (375+(q[0]-(x0+x1)/2)*scale, 365+(q[1]-(y0+y1)/2)*scale)
    image = Image.new("RGB", (1500, 710), "#f7f7f4")
    draw = ImageDraw.Draw(image)
    font_path = Path("C:/Windows/Fonts/segoeui.ttf")
    font = ImageFont.truetype(str(font_path), 17) if font_path.is_file() else ImageFont.load_default()
    draw.text((20, 14), note, fill="#263142", font=font)
    draw.text((20, 45), "Editable regions / holes", fill="#263142", font=font)
    draw.text((770, 45), "PyEmbroidery: actual exported stitches / jumps", fill="#263142", font=font)
    by_id = {o["id"]: o for o in project["objects"]}
    for o in project["objects"]:
        if not o.get("visible", True):
            continue
        rings = o.get("contours", [])
        if not rings and o.get("satin_column"):
            col = o["satin_column"]
            rings = [col["left"]+list(reversed(col["right"]))] if not col.get("closed") else [col["left"], col["right"]]
        mask = Image.new("1", image.size, 0)
        for ring in rings:
            piece = Image.new("1", image.size, 0)
            ImageDraw.Draw(piece).polygon([tf((q["x"], q["y"])) for q in ring], fill=1)
            mask = ImageChops.logical_xor(mask, piece)
        image.paste(tuple(o["color"]), mask=mask)
    draw = ImageDraw.Draw(image)
    index = 0
    stitch_source = [s for s in generated if s["command"] == "Stitch"]
    prev = (0., 0.)
    for x, y, flag in records:
        pos = (x, y)
        a, b = tf(prev), tf(pos)
        a = (a[0]+750, a[1]); b = (b[0]+750, b[1])
        if flag == pe.STITCH:
            source = stitch_source[index] if index < len(stitch_source) else {}
            color = tuple(source.get("color", [30, 88, 156]))
            if color == (255, 255, 255):
                color = (188, 188, 183)
            role = source.get("role", "")
            if role in ("Travel", "HiddenTravel"):
                color = (225, 143, 30)
            draw.line([a, b], fill=color, width=1)
            index += 1
        elif flag == pe.JUMP:
            draw.line([a, b], fill="#dfb2b1", width=1)
        if flag in (pe.STITCH, pe.JUMP):
            prev = pos
    image.save(path)


def evaluate(cli, label, cases, output, pe):
    results = []
    for case in cases:
        directory = output / label / case["name"]
        directory.mkdir(parents=True, exist_ok=True)
        target = directory / "design.dst"
        raster_options = case.get("options")
        pinned_palette = None
        if label == "candidate" and case["mode"] == "raster":
            baseline_project = output / "baseline" / case["name"] / "design.elab"
            if baseline_project.is_file():
                previous = json.loads(baseline_project.read_text(encoding="utf-8"))
                pinned_palette = [list(c) for c in dict.fromkeys(tuple(o["color"]) for o in previous["objects"])]
                fixed_options = json.loads(case["options"].read_text(encoding="utf-8"))
                fixed_options["pinned_colors"] = pinned_palette
                raster_options = directory / "options.json"
                write_json(raster_options, fixed_options)
        command = [cli, "generate", case["source"], target]
        if case["mode"] == "raster":
            command = [cli, "generate-with-options", case["source"], target, case["width_mm"], raster_options]
        execution = run(command)
        write_json(directory / "command.json", execution)
        result = dict(name=case["name"], label=label, note=case["note"], seconds=execution["seconds"], exit_code=execution["exit_code"])
        if execution["exit_code"] == 0:
            generated = json.loads(target.with_suffix(".stitches.json").read_text(encoding="utf-8"))
            saved = json.loads(target.with_suffix(".elab").read_text(encoding="utf-8"))
            plan_path = target.with_suffix(".plan.json")
            if plan_path.is_file():
                plan = json.loads(plan_path.read_text(encoding="utf-8"))
                if len(plan["metadata"]) != len(generated):
                    raise RuntimeError("Plan metadata does not align with saved command sequence")
                generated = [{**s, **meta} for s, meta in zip(generated, plan["metadata"])]
            checks, records = inspect(target, generated, saved, pe, case["expected"])
            result.update(checks)
            if case["mode"] == "raster":
                result["source_mask_occupancy"] = raster_occupancy(case, records, pe)
            result["topology"] = topology(saved)
            result["topology_expectations_ok"] = all(result["topology"][key] == value for key, value in case["expected"].items() if key in ("hole_count", "component_count"))
            result["sampled_top_coverage"] = top_coverage(saved, generated)
            result["objects"] = len([o for o in saved["objects"] if o.get("visible", True)])
            result["kinds"] = dict(Counter(o.get("settings", {}).get("kind", "Tatami") for o in saved["objects"] if o.get("visible", True)))
            result["palette_rgb"] = [list(c) for c in sorted(set(tuple(o["color"]) for o in saved["objects"]))]
            result["fixed_palette_matches_baseline"] = pinned_palette is None or result["palette_rgb"] == sorted(pinned_palette)
            quality = run([cli, "quality", target.with_suffix(".elab"), directory / "quality.json"])
            write_json(directory / "quality-command.json", quality)
            render(directory / "preview.png", saved, generated, records, pe, case["name"]+" - "+label)
            reopened = directory / "reopened.dst"
            rerun = run([cli, "generate", target.with_suffix(".elab"), reopened])
            result["reopen_identical"] = rerun["exit_code"] == 0 and sha(target) == sha(reopened)
            result["quality_ok"] = quality["exit_code"] == 0
            if case["expected"].get("interior_white"):
                q = case["expected"]["interior_white"]
                result["interior_white_retained"] = any(inside(q, o.get("contours", [])) and min(o["color"]) > 210 for o in saved["objects"])
            result["passed"] = result["format_geometry_passed"] and result["routing_expectation_ok"] and result["detail_retained"] and result["topology_expectations_ok"] and result["fixed_palette_matches_baseline"] and result["reopen_identical"] and result["quality_ok"] and result.get("interior_white_retained", True)
            result["passed"] &= result.get("source_mask_occupancy", {}).get("beyond_source_mask", 0) == 0
            if case["name"] in ("rectangle_comp_02", "ring_comp_02", "flag_128", "y_branch"):
                dsb_target = directory / "dsb" / "design.dsb"
                dsb_command = [cli, "generate", case["source"], dsb_target]
                if case["mode"] == "raster":
                    dsb_command = [cli, "generate-with-options", case["source"], dsb_target, case["width_mm"], raster_options]
                dsb_execution = run(dsb_command)
                write_json(directory / "dsb-command.json", dsb_execution)
                if dsb_execution["exit_code"] == 0:
                    dsb_generated = json.loads(dsb_target.with_suffix(".stitches.json").read_text(encoding="utf-8"))
                    dsb_saved = json.loads(dsb_target.with_suffix(".elab").read_text(encoding="utf-8"))
                    dsb_plan_path = dsb_target.with_suffix(".plan.json")
                    if dsb_plan_path.is_file():
                        dsb_plan = json.loads(dsb_plan_path.read_text(encoding="utf-8"))
                        if len(dsb_generated) != len(dsb_plan["metadata"]):
                            raise RuntimeError("DSB plan metadata does not align with commands")
                        dsb_generated = [{**s, **meta} for s, meta in zip(dsb_generated, dsb_plan["metadata"])]
                    result["dsb_independent"], _ = inspect(dsb_target, dsb_generated, dsb_saved, pe, case["expected"])
                    result["passed"] &= result["dsb_independent"]["format_geometry_passed"]
                else:
                    result["dsb_independent"] = dict(format_geometry_passed=False, error=dsb_execution["stderr"])
                    result["passed"] = False
        else:
            result["error"] = execution["stderr"]
            result["passed"] = False
        write_json(directory / "evaluation.json", result)
        results.append(result)
        print(label, case["name"], "checked" if execution["exit_code"] == 0 else "rejected", flush=True)
    by_name = {c["name"]: c for c in results}
    def signature(name):
        path = output / label / name / "design.stitches.json"
        if by_name[name]["exit_code"] != 0 or not path.is_file():
            return None
        values = json.loads(path.read_text(encoding="utf-8"))
        return [(s["command"], round(s["x"], 6), round(s["y"], 6)) for s in values]
    angle_signatures = [signature(name) for name in ("auto_angle_corners", "auto_angle_resampled")]
    stable_angle = all(value is not None for value in angle_signatures) and angle_signatures[0] == angle_signatures[1]
    for name in ("auto_angle_corners", "auto_angle_resampled"):
        by_name[name]["resampling_invariant"] = stable_angle
        by_name[name]["passed"] &= stable_angle
    underlay_signatures = [signature(name) for name in ("underlay_default", "underlay_custom")]
    underlay_changed = all(value is not None for value in underlay_signatures) and underlay_signatures[0] != underlay_signatures[1]
    for name in ("underlay_default", "underlay_custom"):
        by_name[name]["underlay_parameters_effective"] = underlay_changed
        by_name[name]["passed"] &= underlay_changed
    for case in results:
        write_json(output / label / case["name"] / "evaluation.json", case)
    write_json(output / label / "summary.json", dict(cli=str(cli), cli_sha256=sha(cli), cases=results))
    return results


def prepare_swatches(cli, output):
    """Small controlled variants for a future physical sew-out; not sewn here."""
    folder = output / "trial-swatches"
    result = []
    for name, spacing, underlay, compensation in (("A_reference", .40, True, .0),
                                                  ("B_compensation", .40, True, .20),
                                                  ("C_sparse", .45, True, .0),
                                                  ("D_no_underlay", .40, False, .0)):
        item = obj(name, [rect(5, 5, 30, 20), rect(16, 11, 8, 8)], spacing_mm=spacing,
                   underlay=underlay, pull_comp_mm=compensation, tie_in=True, tie_off=True)
        source = folder / (name+".elab")
        write_json(source, dict(version=4, name=name, width_mm=40., objects=[item]))
        execution = run([cli, "generate", source, folder / (name+".dst")])
        write_json(folder / (name+".command.json"), execution)
        result.append(dict(name=name, row_spacing_mm=spacing, underlay=underlay,
                           compensation_mm_per_side=compensation, generated=execution["exit_code"] == 0))
    write_json(folder / "profiles.json", result)
    (folder / "README.txt").write_text(
        "Prepared for a future physical sew-out. No fabric or machine has been tested.\n"
        "Each sample has the same 30 x 20 mm rectangle and protected 8 x 8 mm hole.\n"
        "A -> B isolates compensation (0 -> 0.2 mm per side).\n"
        "A -> C isolates row spacing (0.40 -> 0.45 mm); A -> D isolates underlay.\n"
        "Keep ELAB and thread sidecars. Record fabric, stabilizer, thread, needle, machine,\n"
        "hooping, measured dimensions, visible gaps, puckering and the reverse-side travel.\n"
        "Review physical fit/hoop and machine trim profile before sewing.\n", encoding="utf-8")
    return result


def report(output, results):
    by_label = {label: {c["name"]: c for c in cases} for label, cases in results.items()}
    names = list(next(iter(by_label.values())))
    rows, cards = [], []
    for name in names:
        views = []
        cells = []
        for label, cases in by_label.items():
            case = cases[name]
            status = "PASS" if case["passed"] else "REVIEW"
            coverage = case.get("sampled_top_coverage", {}).get("covered_fraction")
            coverage_text = f'{coverage*100:.1f}% top sample' if coverage is not None else 'top sample unavailable'
            cells.append(f'<td>{label}: {status}<br>{case.get("needle_count", "—")} stitches / {case.get("jump_count", "—")} reader jumps (plan {case.get("planned_jump_count", "—")})<br>{case.get("jump_length_mm", "—")} mm jump<br>{case.get("color_changes", "—")} color changes / {len(case.get("palette_rgb", []))} used RGBs<br>{coverage_text}<br>{case["seconds"]:.3f} s</td>')
            preview = output / label / name / "preview.png"
            views.append(f'<div><b>{html.escape(label)}</b><img src="{label}/{name}/preview.png" alt="{html.escape(name)} {label}"><a href="{label}/{name}/evaluation.json">Numbers/checks</a> · <a href="{label}/{name}/design.elab">ELAB</a> · <a href="{label}/{name}/design.dst">DST</a> · <a href="{label}/{name}/quality.json">Diagnostic JSON</a></div>' if case["exit_code"] == 0 and preview.is_file() else f'<div>{label}: rejected; <a href="{label}/{name}/command.json">reason</a></div>')
        rows.append('<tr><th>'+html.escape(name)+'</th>'+''.join(cells)+'</tr>')
        source = next((path for path in (output / "fixtures").glob(name+".*") if path.suffix in (".png", ".jpg") and path.stem == name), None)
        before = ''
        if source:
            before = f'<section class="inputs"><div>Source<img src="fixtures/{source.name}" alt="Synthetic source {name}"></div><div>Independent background-mask illustration<img src="fixtures/{name}.mask.png" alt="Mask illustration {name}"></div></section>'
        cards.append('<article><h2>'+html.escape(name)+'</h2><p>'+html.escape(next(iter(by_label.values()))[name]["note"])+'</p>'+before+'<section>'+''.join(views)+'</section></article>')
    document = '<!doctype html><html lang="en"><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>EmbroideryLab baseline / candidate</title><style>body{font:15px/1.5 system-ui;margin:28px;color:#243240;background:#f5f5f2}main{max-width:1550px;margin:auto}table{border-collapse:collapse;width:100%}td,th{border:1px solid #ccd2d6;padding:7px;text-align:left}article{margin-top:26px;padding:18px;background:white;border:1px solid #d5dadd}section{display:grid;grid-template-columns:repeat(auto-fit,minmax(550px,1fr));gap:12px}img{width:100%;display:block}h2{font-size:20px}a{color:#135d92}</style><main><h1>EmbroideryLab: reproducible baseline / candidate</h1><p>All inputs are synthetic and generated by tools/autodigitize_validation.py. Settings, millimetres, input and renderer stay identical between versions. Each panel shows editable regions and independent PyEmbroidery DST stitches. Pink = jump, orange = travel when pass metadata exists. Raster fixture files and options are in <a href="fixtures.json">fixtures.json</a>.</p><p>PASS means bounded machine-format and geometry checks, not fabric acceptance. Hole probes use 0.05 mm spacing and a 0.075 mm band justified by DST 0.1 mm coordinate quantization (sqrt(2) × 0.05 mm point error); sampling does not prove mathematical containment. Physical stitch length is checked after quantization. Baseline defects remain marked REVIEW. No 3D image is substituted for technical geometry evidence.</p><table>'+''.join(rows)+'</table>'+''.join(cards)+'</main></html>'
    native = output.parent / "gui"
    if native.is_dir():
        capture_manifest = native / "captures.json"
        accepted_captures = None
        if capture_manifest.is_file():
            accepted_captures = {r["name"] for r in json.loads(capture_manifest.read_text(encoding="utf-8"))["results"] if r["passed"]}
        images = [path for path in sorted(native.glob('*.png')) if not path.name.startswith('.') and (not path.name.startswith('candidate_') or accepted_captures is not None and path.stem in accepted_captures)]
        figures = ''.join(f'<div><a href="../gui/{html.escape(path.name)}"><img src="../gui/{html.escape(path.name)}" alt="{html.escape(path.stem)}"></a><p>{html.escape(path.stem)}</p></div>' for path in images)
        if figures:
            gallery = '<article id="native"><h2>Native GUI evidence before the redesign</h2><p>Synthetic artwork only. Historical native input verification and its executable hash: <a href="../gui/native-qa.json">native-qa.json</a>. The saved cleaned project retains source pixels and mask: <a href="../gui-state/native-generated.elab">ELAB</a>. These incumbent-layout captures predate the newly requested dark workstation. Rendering and sampled checks do not establish fabric quality.</p><section>'+figures+'</section></article>'
            document = document.replace('</main></html>', gallery+'</main></html>')
    workstation = output.parent / "redesign" / "workstation-first-archive" / "matrix.json"
    if workstation.is_file():
        gallery = '<article><h2>Redesigned workstation: first review archive</h2><p><a href="../redesign/workstation-first-archive/index.html">Archived native review</a> · <a href="../redesign/workstation-first-archive/matrix.json">Original executable and evidence provenance</a>. This first review retains the issues found before the correction pass.</p></article>'
        document = document.replace('</main></html>', gallery+'</main></html>')
    confirmation = output.parent / "redesign" / "workstation-confirmation" / "matrix.json"
    if confirmation.is_file():
        gallery = '<article><h2>Redesigned workstation: final confirmation</h2><p><a href="../redesign/workstation-confirmation/index.html">Final 24-frame and 20-operation matrix</a> · <a href="../redesign/workstation-confirmation/before-and-after.html">Before and after with provenance</a> · <a href="../redesign/workstation-confirmation/matrix.json">Per-file binary hashes and limitations</a>. Public artwork is synthetic. The genuine loading/cancel probe is separately identified and its image remains private. Native OS active Cancel, held-middle drag, other OS DPI and physical sew-out are not claimed.</p></article>'
        document = document.replace('</main></html>', gallery+'</main></html>')
    (output / "index.html").write_text(document, encoding="utf-8")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--baseline", type=Path)
    parser.add_argument("--candidate", type=Path)
    parser.add_argument("--output", type=Path, default=ROOT/"outputs/autodigitize/comparison")
    parser.add_argument("--prepare-only", action="store_true")
    args = parser.parse_args()
    output = args.output.resolve()
    cases = prepare(output)
    if args.prepare_only:
        print(len(cases), "synthetic cases prepared", output)
        return
    import pyembroidery as pe
    import PIL
    if importlib.metadata.version("pyembroidery") != "1.5.1" or PIL.__version__ != "13.0.0":
        raise RuntimeError("Use pinned pyembroidery 1.5.1 and Pillow 13.0.0")
    results = {}
    for label in ("baseline", "candidate"):
        cli = getattr(args, label)
        if cli:
            results[label] = evaluate(cli.resolve(), label, cases, output, pe)
        elif (output / label / "summary.json").is_file():
            results[label] = json.loads((output / label / "summary.json").read_text(encoding="utf-8"))["cases"]
    raster_names = {case["name"] for case in cases if case["mode"] == "raster"}
    for label, values in results.items():
        for result in values:
            if result["name"] not in raster_names or result["exit_code"] != 0:
                continue
            saved = json.loads((output / label / result["name"] / "design.elab").read_text(encoding="utf-8"))
            colors = [tuple(o["color"]) for o in saved["objects"] if o.get("visible", True)]
            runs = int(bool(colors)) + sum(a != b for a, b in zip(colors, colors[1:]))
            result["initial_import_color_runs"] = runs
            result["one_block_per_used_rgb"] = runs == len(set(colors))
            result["passed"] &= result["one_block_per_used_rgb"]
            write_json(output / label / result["name"] / "evaluation.json", result)
        label_summary = output / label / "summary.json"
        provenance = json.loads(label_summary.read_text(encoding="utf-8"))
        provenance["cases"] = values
        write_json(label_summary, provenance)
    swatches = prepare_swatches(args.candidate.resolve(), output) if args.candidate else None
    if swatches is None and (output / "summary.json").is_file():
        swatches = json.loads((output / "summary.json").read_text(encoding="utf-8")).get("trial_swatches")
    if not results:
        raise RuntimeError("Specify --baseline or --candidate")
    report(output, results)
    summary = dict(checked_at=datetime.now(timezone(timedelta(hours=3))).isoformat(), timezone="Europe/Moscow",
                   python=sys.version, platform=platform.platform(), reader_version=importlib.metadata.version("pyembroidery"),
                   python_executable=sys.executable,
                   reader_source_sha256={name: sha(Path(importlib.import_module("pyembroidery."+name).__file__)) for name in ("DstReader", "DsbReader")},
                   pillow_version=PIL.__version__, cases=len(cases), results=results, trial_swatches=swatches)
    write_json(output / "summary.json", summary)
    print(json.dumps({label: {"passed": sum(c["passed"] for c in cases), "total": len(cases)} for label, cases in results.items()}, indent=2))
    # Deliberately non-zero only for candidate failure: a baseline captures defects.
    if "candidate" in results and any(not c["passed"] for c in results["candidate"]):
        raise SystemExit(1)


if __name__ == "__main__":
    main()
