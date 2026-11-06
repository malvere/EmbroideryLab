"""Local-only comparison of a user-selected raster. Never publishes artwork."""
from __future__ import annotations

import argparse
from collections import Counter
import importlib.metadata
import html
import json
from pathlib import Path
import sys

import autodigitize_validation as av


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--input", required=True, type=Path)
    parser.add_argument("--baseline", type=Path)
    parser.add_argument("--candidate", type=Path)
    parser.add_argument("--output", type=Path, default=av.ROOT/"outputs/autodigitize/private-real")
    args = parser.parse_args()
    import pyembroidery as pe
    from PIL import Image
    if importlib.metadata.version("pyembroidery") != "1.5.1":
        raise RuntimeError("Use pyembroidery==1.5.1")
    source, output = args.input.resolve(), args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    original_hash = av.sha(source)
    options = output / "options.json"
    av.write_json(options, dict(max_dimension=512, palette_size=8, background="NearWhite",
                               min_region_pixels=6, simplify_tolerance_mm=.1, smoothing_iterations=0))
    opened = Image.open(source)
    input_mode = opened.mode
    image = opened.convert("RGBA")
    original_size = image.size
    image.thumbnail((800, 800), Image.Resampling.LANCZOS)
    # A CMYK JPEG can carry a large CMYK ICC profile. After conversion its
    # profile no longer describes RGBA bytes; do not attach it to this technical
    # preview or let viewers reinterpret alpha as a fourth CMYK component.
    Image.frombytes("RGBA", image.size, image.tobytes()).save(output / "source-preview.png")
    results = {}
    prior = {}
    if (output / "summary.json").is_file():
        prior = json.loads((output / "summary.json").read_text(encoding="utf-8"))
        if prior["input_sha256"] != original_hash:
            raise RuntimeError("Existing private comparison refers to a different input")
    for label in ("baseline", "candidate"):
        selected_cli = getattr(args, label)
        if selected_cli is None:
            if label in prior.get("results", {}):
                results[label] = prior["results"][label]
                stored = output / label / "design.elab"
                if stored.is_file():
                    saved = json.loads(stored.read_text(encoding="utf-8"))
                    results[label]["palette_rgb"] = [list(c) for c in sorted(set(tuple(o["color"]) for o in saved["objects"]))]
                    results[label].setdefault("options", json.loads(options.read_text(encoding="utf-8")))
                    av.write_json(output / label / "evaluation.json", results[label])
            continue
        cli = selected_cli.resolve()
        folder = output / label
        folder.mkdir(exist_ok=True)
        target = folder / "design.dst"
        selected_options = json.loads(options.read_text(encoding="utf-8"))
        if label == "candidate":
            baseline_project = output / "baseline" / "design.elab"
            if not baseline_project.is_file():
                raise RuntimeError("Generate baseline first to freeze its actual thread palette")
            baseline = json.loads(baseline_project.read_text(encoding="utf-8"))
            selected_options["pinned_colors"] = [list(c) for c in dict.fromkeys(tuple(o["color"]) for o in baseline["objects"])]
        options_for_label = folder / "options.json"
        av.write_json(options_for_label, selected_options)
        execution = av.run([cli, "generate-with-options", source, target, 100, options_for_label])
        av.write_json(folder / "command.json", execution)
        result = dict(cli=str(cli), cli_sha256=av.sha(cli), generation_seconds=execution["seconds"],
                      exit_code=execution["exit_code"])
        if execution["exit_code"] != 0:
            result["error"] = execution["stderr"]
            av.write_json(folder / "evaluation.json", result)
            results[label] = result
            continue
        project = json.loads(target.with_suffix(".elab").read_text(encoding="utf-8"))
        generated = json.loads(target.with_suffix(".stitches.json").read_text(encoding="utf-8"))
        plan_path = target.with_suffix(".plan.json")
        if plan_path.is_file():
            plan = json.loads(plan_path.read_text(encoding="utf-8"))
            if len(generated) != len(plan["metadata"]):
                raise RuntimeError("Misaligned plan metadata")
            generated = [{**s, **m} for s, m in zip(generated, plan["metadata"])]
            result["plan_warnings"] = plan.get("warnings", [])
        checks, records = av.inspect(target, generated, project, pe, {})
        result["independent_dst"] = checks
        result["topology"] = av.topology(project)
        result["objects"] = len(project["objects"])
        result["options"] = selected_options
        result["palette_rgb"] = [list(c) for c in sorted(set(tuple(o["color"]) for o in project["objects"]))]
        result["kinds"] = dict(Counter(o["settings"]["kind"] for o in project["objects"]))
        result["sampled_top_coverage"] = av.top_coverage(project, generated)
        quality = av.run([cli, "quality", target.with_suffix(".elab"), folder / "quality.json"])
        av.write_json(folder / "quality-command.json", quality)
        result["quality_exit_code"] = quality["exit_code"]
        if quality["exit_code"] == 0:
            value = json.loads((folder / "quality.json").read_text(encoding="utf-8"))
            result["quality_report"] = value
        av.render(folder / "preview.png", project, generated, records, pe, "Local-only real artwork - "+label)
        reopened = folder / "reopened.dst"
        reopen = av.run([cli, "generate", target.with_suffix(".elab"), reopened])
        av.write_json(folder / "reopen-command.json", reopen)
        result["reopen_identical"] = reopen["exit_code"] == 0 and av.sha(target) == av.sha(reopened)
        av.write_json(folder / "evaluation.json", result)
        results[label] = result
    if not results:
        raise RuntimeError("Specify --baseline or --candidate")
    for label, result in results.items():
        if result["exit_code"] == 0:
            saved = json.loads((output / label / "design.elab").read_text(encoding="utf-8"))
            colors = [tuple(o["color"]) for o in saved["objects"] if o.get("visible", True)]
            runs = int(bool(colors)) + sum(a != b for a, b in zip(colors, colors[1:]))
            result["initial_import_color_runs"] = runs
            result["one_block_per_used_rgb"] = runs == len(set(colors))
            av.write_json(output / label / "evaluation.json", result)
    summary = dict(scope="Local-only user artwork; do not publish derivatives", input=str(source),
                   input_sha256=original_hash, input_dimensions_px=original_size,
                   input_mode=input_mode,
                   source_preview_conversion="Pillow RGBA conversion; source ICC profile neither applied nor embedded",
                   options=json.loads(options.read_text(encoding="utf-8")), width_mm=100,
                   python_executable=sys.executable, reader_version=importlib.metadata.version("pyembroidery"),
                   source_unchanged=av.sha(source) == original_hash, results=results,
                   fixed_palette_matches=(results.get("baseline", {}).get("palette_rgb") == results.get("candidate", {}).get("palette_rgb")) if "candidate" in results else None,
                   initial_import_color_blocks_ok=all(r.get("one_block_per_used_rgb", False) for r in results.values()),
                   limits=["NearWhite is an explicitly fixed legacy baseline choice, not the new GUI default",
                           "Eight-color flat region quantization is not photostitch",
                           "Independent reader checks coordinates, dimensions, controls and physical lengths",
                           "Initial raster import should group disjoint regions into one block per used RGB; this check does not reorder manual vector layers",
                           "Spatial and coverage diagnostics use saved editable geometry and are advisory",
                           "No physical sew-out or arbitrary photograph quality claim"])
    av.write_json(output / "summary.json", summary)
    cards = []
    for label, result in results.items():
        if result["exit_code"] != 0:
            cards.append(f'<article><h2>{label}: rejected</h2><p>{html.escape(result.get("error", "Unknown generation failure"))}</p><a href="{label}/command.json">Command evidence</a></article>')
            continue
        data = result.get("independent_dst", {})
        order_status = "" if result["one_block_per_used_rgb"] else " · REVIEW: fragmented color order"
        cards.append(f'<article><h2>{label}{order_status}</h2><p>{result.get("objects", "—")} objects · {data.get("needle_count", "—")} stitches · {data.get("jump_count", "—")} reader-normalized jumps · {data.get("jump_length_mm", "—")} mm jump · {data.get("color_changes", "—")} color changes · {result["generation_seconds"]:.3f}s</p><img src="{label}/preview.png" alt="Local-only {label} vectors and independent machine stitches"><p><a href="{label}/evaluation.json">Full metrics</a> · <a href="{label}/design.elab">ELAB</a> · <a href="{label}/design.dst">DST</a> · <a href="{label}/quality.json">Quality</a></p></article>')
    (output / "index.html").write_text('<!doctype html><meta charset="utf-8"><title>Local-only artwork comparison</title><style>body{font:16px/1.5 system-ui;max-width:1500px;margin:30px auto;padding:0 20px;background:#f5f5f2;color:#263142}article{background:white;margin:20px 0;padding:20px;border:1px solid #d2d8dc}img{max-width:100%}.source{max-height:440px}a{color:#155d94}</style><h1>Local-only real artwork comparison</h1><p>Original and derivatives stay in this ignored local output directory. Both versions use the same 512 px / 8 colors / 100 mm / explicit legacy NearWhite settings, input and technical renderer. The candidate pins the exact eight RGB colors used by the baseline. This is bounded flat-color simplification, not photostitch or fabric validation.</p><img class="source" src="source-preview.png" alt="Private local source preview"><p><a href="summary.json">Summary and source hashes</a> · <a href="options.json">Exact fixed options</a></p>'+''.join(cards), encoding="utf-8")
    print(json.dumps({label: {"exit_code": r["exit_code"], "objects": r.get("objects"),
                             "seconds": r["generation_seconds"],
                             "one_block_per_used_rgb": r.get("one_block_per_used_rgb"),
                             "reopen_identical": r.get("reopen_identical"),
                             "independent_dst": {key: r.get("independent_dst", {}).get(key) for key in
                                 ("format_geometry_passed", "needle_count", "jump_count", "jump_length_mm", "color_changes", "maximum_stitch_mm")}}
                      for label, r in results.items()}, ensure_ascii=False, indent=2))
    if (not summary["source_unchanged"]
            or not summary["initial_import_color_blocks_ok"]
            or ("candidate" in results and not summary["fixed_palette_matches"])
            or any(r["exit_code"] != 0 or not r["independent_dst"]["format_geometry_passed"]
                                             or not r["reopen_identical"] for r in results.values())):
        raise SystemExit(1)


if __name__ == "__main__":
    main()
