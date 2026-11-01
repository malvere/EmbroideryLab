"""Reproducible independent CLI/format/geometry checks for EmbroideryLab 0.5.

Run with Python 3.13 + Pillow 13.0.0 + pyembroidery 1.5.1, after building CLI.
This script creates its own fixtures/outputs; it never operates a machine,
reads private artwork, or invokes installed embroidery software. All fixtures are synthetic.
"""
from __future__ import annotations

import argparse
from collections import Counter
from copy import deepcopy
from datetime import datetime, timedelta, timezone
import hashlib
import html
import importlib
import importlib.metadata
import json
import math
from pathlib import Path
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[1]


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def write_json(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2, ensure_ascii=False, allow_nan=False) + "\n", encoding="utf-8")


def point(x, y):
    return {"x": x, "y": y}


def rect(x, y, w, h):
    return [point(x, y), point(x+w, y), point(x+w, y+h), point(x, y+h)]


def settings(kind="Satin", **changes):
    value = {"kind": kind, "spacing_mm": .4, "length_mm": 8., "underlay": False,
             "tie_in": False, "tie_off": False, "pull_comp_mm": 0.}
    value.update(changes)
    return value


def project(name, objects):
    return {"version": 4, "name": name, "width_mm": 100., "objects": objects}


def satin_object(name, left, right, **column):
    return {"id": 1, "name": name, "satin_column": {"left": left, "right": right, **column},
            "color": [30, 88, 156], "visible": True, "settings": settings()}


def length(path):
    return sum(math.hypot(b["x"]-a["x"], b["y"]-a["y"]) for a, b in zip(path, path[1:]))


def circle(radius, n=64, cx=30., cy=30.):
    return [point(cx+radius*math.cos(i*math.tau/n), cy+radius*math.sin(i*math.tau/n)) for i in range(n)]


def cleaned_axis(name, with_old_spur=False):
    # Manually prepared fixtures exercise persistence and conversion, not an
    # inaccessible CLI pruning operation. Rust API tests cover the actual edit.
    boundary = [rect(10., 10., 30., 4.)]
    path = [point(12., 12.), point(25., 12.), point(38., 12.)]
    graph = {"source_object_id": 1, "source_contours": boundary, "pixel_size_mm": .2,
             "vertices": [{"id": 1, "point": path[0]}, {"id": 2, "point": path[-1]}],
             "branches": [{"id": 1, "start_vertex": 1, "end_vertex": 2, "points": path}]}
    source = {"id": 1, "name": "Synthetic source region", "contours": boundary, "visible": False,
              "color": [30, 88, 156], "settings": settings("Tatami")}
    axes = {"id": 2, "name": name, "running_paths": [path], "centerline_graph": graph,
            "visible": True, "color": [30, 88, 156], "settings": settings("Running", length_mm=2.5)}
    result = project(name, [source, axes])
    before = None
    if with_old_spur:
        before = deepcopy(result)
        junction, tip = point(25., 12.), point(25., 11.)
        old = before["objects"][1]
        old["running_paths"] = [[path[0], junction], [junction, path[-1]], [junction, tip]]
        old["centerline_graph"]["vertices"] += [{"id": 3, "point": junction}, {"id": 4, "point": tip}]
        old["centerline_graph"]["branches"] = [
            {"id": 1, "start_vertex": 1, "end_vertex": 3, "points": old["running_paths"][0]},
            {"id": 2, "start_vertex": 3, "end_vertex": 2, "points": old["running_paths"][1]},
            {"id": 3, "start_vertex": 3, "end_vertex": 4, "points": old["running_paths"][2]}]
    return result, before


def prepare(output):
    cases = []

    def add(name, value, operation="generate", arguments=(), label="Synthetic fixture", before=None):
        folder = output / name
        source = folder / "source.elab"
        write_json(source, value)
        if before is not None:
            write_json(folder / "before_manual_cleanup.elab", before)
        cases.append({"name": name, "label": label, "source": source,
                      "operation": operation, "arguments": list(arguments)})

    straight = satin_object("Straight Satin", [point(5., 8.), point(45., 8.)], [point(5., 12.), point(45., 12.)],
                            rungs=[{"left": .25, "right": .25}, {"left": .75, "right": .75}])
    add("straight_satin", project("Synthetic straight Satin", [straight]))
    rails = [[point(30.+r*math.cos(-.8+i*2.2/64), 25.+r*math.sin(-.8+i*2.2/64)) for i in range(65)] for r in (10., 14.)]
    add("c_curve_satin", project("Synthetic C-curve Satin", [satin_object(
        "C curve", *rails, rungs=[{"left": t, "right": t} for t in (.25, .5, .75)])]))
    # S-bend fixture has equal overall side lengths but alternating inner bends.
    left, right = [], []
    for i in range(97):
        x = i*30./96
        angle = x*math.tau/30.
        y = 20.+3.*math.sin(angle)
        slope = 3.*math.tau/30.*math.cos(angle)
        nx, ny = -slope/math.sqrt(1.+slope*slope), 1./math.sqrt(1.+slope*slope)
        left.append(point(8.+x+2.*nx, y+2.*ny))
        right.append(point(8.+x-2.*nx, y-2.*ny))
    rungs = [{"left": length(left[:i+1])/length(left), "right": length(right[:i+1])/length(right)} for i in range(8, 96, 8)]
    add("s_curve_satin", project("Synthetic S-curve Satin", [satin_object("S curve", left, right, rungs=rungs)]))
    for name, path in [("closed_circle_outline", circle(12.)), ("closed_rectangle_outline", rect(10., 10., 30., 20.))]:
        source = {"id": 1, "name": name, "contours": [path], "visible": False,
                  "color": [30, 88, 156], "settings": settings("Tatami")}
        add(name, project("Synthetic " + name, [source]), "satin-outline", (1, 2., "closed"))
    axis, _ = cleaned_axis("Simple editable axis")
    add("axis_to_satin", axis, "satin-from-branch", (2, 1))
    cleaned, before = cleaned_axis("Manually cleaned spur graph", True)
    add("manually_cleaned_spur", cleaned, "satin-from-branch", (2, 1),
        "Synthetic graph; cleanup prepared manually; CLI conversion tested", before)
    crossed = deepcopy(straight)
    crossed["satin_column"]["rungs"] = [{"left": .25, "right": .75}, {"left": .75, "right": .25}]
    invalid = [("crossing_rungs", project("Invalid crossing rungs", [crossed]), "generate", ())]
    small = {"id": 1, "name": "Small circle", "contours": [circle(3., 32)], "visible": False,
             "color": [30, 88, 156], "settings": settings("Tatami")}
    invalid.append(("collapsed_wide_outline", project("Invalid wide outline", [small]), "satin-outline", (1, 20., "closed")))
    return cases, invalid


def run(command):
    start = time.perf_counter()
    try:
        result = subprocess.run([str(v) for v in command], cwd=ROOT, capture_output=True,
                                text=True, encoding="utf-8", errors="replace", timeout=180)
        return {"command": [str(v) for v in command], "exit_code": result.returncode,
                "stdout": result.stdout, "stderr": result.stderr, "seconds": round(time.perf_counter()-start, 4)}
    except subprocess.TimeoutExpired as error:
        return {"command": [str(v) for v in command], "exit_code": None,
                "stdout": "", "stderr": str(error), "seconds": round(time.perf_counter()-start, 4)}


def away(value):
    return (math.floor(value*10.+.5) if value >= 0. else math.ceil(value*10.-.5))/10.


def xy(value):
    return value["x"], value["y"]


def rings_for(obj):
    if obj.get("satin_column"):
        column = obj["satin_column"]
        if any(h.get("incoming") or h.get("outgoing") for h in column.get("left_handles", [])+column.get("right_handles", [])):
            raise ValueError("Independent fixture evaluator does not flatten arbitrary cubic handles")
        return [column["left"], column["right"]] if column.get("closed") else [column["left"]+list(reversed(column["right"]))]
    if obj.get("centerline_graph"):
        return obj["centerline_graph"]["source_contours"]
    return obj.get("contours", [])


def inside(q, rings):
    x, y = q
    odd = False
    for ring in rings:
        for a, b in zip(ring, ring[1:]+ring[:1]):
            ax, ay = xy(a)
            bx, by = xy(b)
            if (ay > y) != (by > y) and x < ax+(y-ay)*(bx-ax)/(by-ay):
                odd = not odd
    return odd


def boundary_distance(q, rings):
    x, y = q
    best = math.inf
    for ring in rings:
        for a, b in zip(ring, ring[1:]+ring[:1]):
            ax, ay = xy(a)
            bx, by = xy(b)
            dx, dy = bx-ax, by-ay
            t = max(0., min(1., ((x-ax)*dx+(y-ay)*dy)/max(dx*dx+dy*dy, 1e-30)))
            best = min(best, math.hypot(x-ax-t*dx, y-ay-t*dy))
    return best


def external_check(target, generated, saved, pe):
    pattern = pe.read(str(target))
    records = [(x/10., y/10., pe.decode_embroidery_command(code)[0]) for x, y, code in pattern.stitches]
    needles = [(x, y) for x, y, flag in records if flag == pe.STITCH]
    expected = [s for s in generated if s["command"] == "Stitch"]
    expected_xy = [(away(s["x"]), away(s["y"])) for s in expected]
    equal = len(needles) == len(expected_xy) and all(math.dist(a, b) < 1e-7 for a, b in zip(needles, expected_xy))
    counts = Counter(flag for _, _, flag in records)
    objects = {obj["id"]: obj for obj in saved["objects"]}
    rings = {key: rings_for(obj) for key, obj in objects.items()}
    probes = escaped = 0
    maximum_outside = maximum_stitch = 0.
    failures = []
    previous = (0., 0.)
    index = 0
    for x, y, flag in records:
        current = (x, y)
        if flag == pe.STITCH:
            maximum_stitch = max(maximum_stitch, math.dist(previous, current))
            if index < len(expected):
                region = rings.get(expected[index]["object_id"], [])
                steps = max(1, math.ceil(math.dist(previous, current)/.1))
                for i in range(steps+1):
                    blend = i/steps
                    q = (previous[0]+(x-previous[0])*blend, previous[1]+(y-previous[1])*blend)
                    probes += 1
                    if region and not inside(q, region):
                        distance = boundary_distance(q, region)
                        maximum_outside = max(maximum_outside, distance)
                        if distance > .13+1e-8:
                            escaped += 1
                            if len(failures) < 8:
                                failures.append({"point": q, "distance_mm": distance, "needle_index": index})
            index += 1
        if flag in (pe.STITCH, pe.JUMP):
            previous = current
    controls = counts[pe.END] == 1 and counts[pe.COLOR_CHANGE] == 0 and counts[pe.STOP] == 0 and counts[pe.NEEDLE_SET] == 0
    maximum_allowed = max((obj.get("settings", {}).get("length_mm", 3.5) for obj in saved["objects"] if obj.get("visible", True)), default=0.)
    result = {"needle_coordinates_equal_quantized_json": equal, "needle_count": len(needles),
              "expected_needle_count": len(expected), "controls_ok": controls, "end_count": counts[pe.END],
              "jump_records": counts[pe.JUMP], "inferred_trims": counts[pe.TRIM],
              "sampled_probes": probes, "sampling_step_mm": .1, "boundary_allowance_mm": .13,
              "probes_beyond_allowance": escaped, "maximum_outside_distance_mm": maximum_outside,
              "maximum_stitch_mm": maximum_stitch, "length_within_setting": maximum_stitch <= maximum_allowed+1e-7,
              "first_escaped_probes": failures, "sha256": sha(target)}
    result["passed"] = equal and controls and escaped == 0 and result["length_within_setting"]
    return result, records


def render_png(path, saved, records, label, pe):
    from PIL import Image, ImageChops, ImageDraw, ImageFont
    points = [(x, y) for x, y, flag in records if flag in (pe.STITCH, pe.JUMP)]
    if not points:
        return
    x0, y0 = min(x for x, _ in points), min(y for _, y in points)
    w, h = max(x for x, _ in points)-x0, max(y for _, y in points)-y0
    scale = min(1050/max(w, 1.), 650/max(h, 1.))
    transform = lambda p: (600+(p[0]-x0-w/2)*scale, 430+(p[1]-y0-h/2)*scale)
    image = Image.new("RGB", (1200, 850), "#fafbfc")
    for obj in saved["objects"]:
        if not obj.get("visible", True):
            continue
        mask = Image.new("1", image.size, 0)
        for ring in rings_for(obj):
            piece = Image.new("1", image.size, 0)
            ImageDraw.Draw(piece).polygon([transform(xy(p)) for p in ring], fill=1)
            mask = ImageChops.logical_xor(mask, piece)
        image.paste("#e1e5e9", mask=mask)
    draw = ImageDraw.Draw(image)
    font_path = Path("C:/Windows/Fonts/segoeui.ttf")
    font = ImageFont.truetype(str(font_path), 23) if font_path.exists() else ImageFont.load_default(size=23)
    draw.text((28, 22), label, fill="#243447", font=font)
    draw.text((28, 57), "Independent machine-file preview; gray area / blue sewn stitches / coral jumps", fill="#596a7d", font=font)
    previous = None
    for x, y, flag in records:
        current = transform((x, y))
        if previous is not None and flag in (pe.STITCH, pe.JUMP):
            draw.line([previous, current], fill="#205e9b" if flag == pe.STITCH else "#d8a393", width=2 if flag == pe.STITCH else 1)
        if flag in (pe.STITCH, pe.JUMP):
            previous = current
    image.save(path)


def evaluate(case, output, cli, pe):
    directory = case["source"].parent
    before = sha(case["source"])
    result = {"name": case["name"], "label": case["label"], "source_sha256": before, "formats": {}}
    for extension in ("dst", "dsb"):
        target = directory / extension / ("design."+extension)
        execution = run([cli, case["operation"], case["source"], target, *case["arguments"]])
        write_json(directory / (extension+"_command.json"), execution)
        if execution["exit_code"] != 0:
            result["formats"][extension] = {"passed": False, "execution": execution}
            continue
        saved = json.loads(target.with_suffix(".elab").read_text(encoding="utf-8"))
        generated = json.loads(target.with_suffix(".stitches.json").read_text(encoding="utf-8"))
        checks, records = external_check(target, generated, saved, pe)
        graph_ok = all(not obj.get("centerline_graph") or obj["running_paths"] == [branch["points"] for branch in obj["centerline_graph"]["branches"]] for obj in saved["objects"])
        # A second CLI generation opens the persisted ELAB rather than using
        # transient state; compare the generated machine bytes exactly.
        reopened = target.with_name("reopened."+extension)
        reopen_execution = run([cli, "generate", target.with_suffix(".elab"), reopened])
        reopened_equal = reopen_execution["exit_code"] == 0 and sha(reopened) == sha(target)
        quality_path = target.with_name("quality.json")
        quality_execution = run([cli, "quality", target, quality_path])
        quality_ok = quality_execution["exit_code"] == 0 and quality_path.is_file()
        checks.update(schema_v4=saved["version"] == 4, graph_paths_canonical=graph_ok,
                      reopen_machine_bytes_equal=reopened_equal, quality_command_ok=quality_ok)
        checks["passed"] &= checks["schema_v4"] and graph_ok and reopened_equal and quality_ok
        checks["output_sha256"] = {str(path.relative_to(output)): sha(path) for path in target.parent.iterdir() if path.is_file()}
        result["formats"][extension] = checks
        write_json(target.with_name("reopen_command.json"), reopen_execution)
        write_json(target.with_name("quality_command.json"), quality_execution)
        if extension == "dst":
            render_png(directory / "preview.png", saved, records, case["label"], pe)
            quality_elab = directory / "quality_project.json"
            execution_elab = run([cli, "quality", target.with_suffix(".elab"), quality_elab])
            result["project_quality_command_ok"] = execution_elab["exit_code"] == 0 and quality_elab.is_file()
    result["source_unchanged"] = sha(case["source"]) == before
    result["passed"] = result["source_unchanged"] and result.get("project_quality_command_ok", False) and all(v["passed"] for v in result["formats"].values())
    write_json(directory / "evaluation.json", result)
    return result


def invalid_checks(fixtures, output, cli):
    results = []
    for name, value, operation, arguments in fixtures:
        directory = output / "invalid" / name
        source = directory / "source.elab"
        write_json(source, value)
        for extension in ("dst", "dsb"):
            target = directory / extension / ("protected."+extension)
            protected = [target, target.with_suffix(".elab"), target.with_suffix(".stitches.json"), target.with_suffix(".svg")]
            for path in protected:
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(b"VALID EXISTING OUTPUT MUST SURVIVE\n")
            before = {str(path): sha(path) for path in protected}
            execution = run([cli, operation, source, target, *arguments])
            retained = before == {str(path): sha(path) for path in protected}
            result = {"name": name, "format": extension, "rejected": execution["exit_code"] not in (None, 0),
                      "existing_files_unchanged": retained, "execution": execution}
            result["passed"] = result["rejected"] and retained
            write_json(directory / (extension+"_rejection.json"), result)
            results.append(result)
    return results


def artifacts(summary, output):
    cards = []
    for case in summary["cases"]:
        name = case["name"]
        status = "PASS" if case["passed"] else "CHECK REQUIRED"
        preview = f'<img src="{name}/preview.png" alt="Independent machine stitches for {html.escape(name)}">' if (output/name/"preview.png").exists() else ""
        cards.append(f'<article><h2>{html.escape(name)}</h2><p><b>{status}</b> · {html.escape(case["label"])}</p>{preview}<p><a href="{name}/evaluation.json">Checks</a> · <a href="{name}/source.elab">Source</a></p></article>')
    document = '<!doctype html><html lang="en"><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>EmbroideryLab 0.5 independent checks</title><style>body{font:16px/1.5 system-ui;background:#f3f5f7;color:#203247;margin:0}main{max-width:1250px;margin:auto;padding:32px}header{max-width:950px}.grid{display:grid;grid-template-columns:repeat(auto-fit,minmax(350px,1fr));gap:20px}article{background:white;padding:20px;border:1px solid #d9e0e7}h2{font-size:20px}img{width:100%;height:290px;object-fit:contain}a{color:#155cab}</style><main><header><h1>EmbroideryLab 0.5 · independent checks</h1><p>Synthetic Satin curves, closed outlines, and manually prepared cleaned centerline graphs. DST and DSB are decoded by pyembroidery 1.5.1; PNGs are drawn independently with Pillow 13.0.0.</p><p>Geometry checks sample sewn segments every 0.1 mm with a 0.13 mm diagnostic boundary band for quantization. PASS is a bounded format/geometry check, not proof of stitch quality on fabric. Cleanup fixtures are manually prepared: the CLI does not expose branch deletion. No arbitrary lettering recognition or physical stitch quality is implied. No machine is operated.</p><p><a href="summary.json">Summary, command evidence and hashes</a></p></header><div class="grid">'+"\n".join(cards)+'</div></main></html>'
    (output / "index.html").write_text(document, encoding="utf-8")
    from PIL import Image, ImageDraw
    sheet = Image.new("RGB", (1200, 470*math.ceil(max(1, len(summary["cases"]))/2)), "#f3f5f7")
    for i, case in enumerate(summary["cases"]):
        path = output / case["name"] / "preview.png"
        if path.exists():
            tile = Image.open(path).convert("RGB")
            tile.thumbnail((600, 425))
            x, y = i % 2*600, i // 2*470
            sheet.paste(tile, (x, y))
            ImageDraw.Draw(sheet).text((x+16, y+430), case["name"]+": "+("PASS" if case["passed"] else "CHECK"), fill="#203247")
    sheet.save(output / "contact-sheet.png")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--cli", type=Path, default=ROOT/"embroidery-engine/target/debug/embroidery-cli.exe")
    parser.add_argument("--output", type=Path, default=ROOT/"outputs/quality_v05")
    parser.add_argument("--prepare-only", action="store_true")
    args = parser.parse_args()
    output = args.output.resolve()
    originals = [Path(__file__).resolve()]+[p for p in (ROOT/"tests"/"fixtures").glob("*") if p.is_file()]
    before = {str(p): sha(p) for p in originals}
    cases, invalid = prepare(output)
    if args.prepare_only:
        print(json.dumps({"prepared": len(cases), "invalid": len(invalid), "output": str(output)}, indent=2))
        return
    import pyembroidery as pe
    import PIL
    if importlib.metadata.version("pyembroidery") != "1.5.1" or PIL.__version__ != "13.0.0":
        raise RuntimeError("Use pinned pyembroidery 1.5.1 and Pillow 13.0.0")
    summary = {"checked_at": datetime.now(timezone(timedelta(hours=3))).isoformat(), "timezone": "Europe/Moscow",
               "python": sys.executable, "python_version": sys.version, "cli": str(args.cli.resolve()), "cli_sha256": sha(args.cli),
               "reader": {"package": "pyembroidery", "version": importlib.metadata.version("pyembroidery"),
                          "module_sha256": {name: sha(importlib.import_module("pyembroidery."+name).__file__) for name in ("DstReader", "DsbReader")}},
               "renderer": {"package": "Pillow", "version": PIL.__version__},
               "source_sha256_before": before,
               "cases": [evaluate(case, output, args.cli.resolve(), pe) for case in cases],
               "invalid_cases": invalid_checks(invalid, output, args.cli.resolve())}
    summary["source_sha256_after"] = {str(p): sha(p) for p in originals}
    summary["originals_unchanged"] = before == summary["source_sha256_after"]
    summary["passed"] = summary["originals_unchanged"] and all(case["passed"] for case in summary["cases"]+summary["invalid_cases"])
    artifacts(summary, output)
    summary["artifact_sha256"] = {str(path.relative_to(output)): sha(path) for path in output.rglob("*") if path.is_file() and path.name != "summary.json"}
    write_json(output / "summary.json", summary)
    print(json.dumps({"passed": summary["passed"], "cases": len(cases), "formats": len(cases)*2, "invalid_checks": len(summary["invalid_cases"]), "output": str(output)}, indent=2))
    raise SystemExit(0 if summary["passed"] else 1)


if __name__ == "__main__":
    main()
