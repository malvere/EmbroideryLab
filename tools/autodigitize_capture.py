"""Capture the application's public framebuffer API; no OS input automation."""
from __future__ import annotations

import argparse
from datetime import datetime, timezone, timedelta
import itertools
import json
import math
import os
from pathlib import Path
import subprocess
import time

import autodigitize_validation as av


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--gui", required=True, type=Path, help="Build with --features capture")
    parser.add_argument("--project", type=Path)
    parser.add_argument("--empty", action="store_true", help="Capture the native initial empty workspace")
    parser.add_argument("--output", type=Path, default=av.ROOT/"outputs/autodigitize/gui")
    parser.add_argument("--sizes", nargs="+", default=["1360x900", "1024x680"])
    parser.add_argument("--languages", nargs="+", default=["ru", "en"])
    parser.add_argument("--views", nargs="+", default=["2d", "3d"])
    parser.add_argument("--themes", nargs="+", default=["dark"])
    parser.add_argument("--timeline", choices=["expanded", "collapsed"], default="expanded")
    parser.add_argument("--panels", choices=["both", "none"], default="both")
    parser.add_argument("--state-root", type=Path, default=av.ROOT/"outputs/autodigitize/gui-state/captures")
    args = parser.parse_args()
    from PIL import Image
    if args.empty == (args.project is not None):
        parser.error("Specify exactly one of --project or --empty")
    gui, project, output = args.gui.resolve(), args.project.resolve() if args.project else None, args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    results = []
    native_scale = None
    for theme, language, size, view in itertools.product(args.themes, args.languages, args.sizes, args.views):
        width, height = (int(v) for v in size.split("x"))
        if not (850 <= width <= 3840 and 480 <= height <= 2160):
            raise ValueError("Capture size is outside application's supported range")
        if theme not in ("dark", "light") or view not in ("2d", "3d") or language not in ("ru", "en"):
            raise ValueError("Unsupported theme, view or language")
        name = f"candidate_{theme}_{language}_{size}_{view}"
        target = output / (name+".png")
        state = args.state_root.resolve() / name
        roaming, local = state/"Roaming", state/"Local"
        roaming.mkdir(parents=True, exist_ok=True)
        local.mkdir(parents=True, exist_ok=True)
        workspace = dict(theme=theme.title(), layers_open=args.panels == "both",
                         inspector_open=args.panels == "both", timeline_open=args.timeline == "expanded",
                         layers_width=228, inspector_width=278, timeline_height=105,
                         grid=True, rulers=False, hints=False)
        av.write_json(roaming / "EmbroideryLab" / "preferences.json",
                      dict(language=language, workspace=workspace))
        captured = state / f"frame-{time.time_ns()}.png"
        env = dict(os.environ, APPDATA=str(roaming), LOCALAPPDATA=str(local),
                   EMBROIDERYLAB_LANGUAGE=language, EMBROIDERYLAB_CAPTURE_SIZE=size,
                   EMBROIDERYLAB_CAPTURE_TO=str(captured), EMBROIDERYLAB_THEME=theme)
        env.pop("EMBROIDERYLAB_CAPTURE_3D", None)
        env.pop("EMBROIDERYLAB_CAPTURE_EMPTY", None)
        if args.empty:
            env["EMBROIDERYLAB_CAPTURE_EMPTY"] = "1"
        if view == "3d":
            env["EMBROIDERYLAB_CAPTURE_3D"] = "1"
        start = time.perf_counter()
        try:
            execution = subprocess.run([str(gui)] + ([str(project)] if project else []), env=env, cwd=av.ROOT,
                                       capture_output=True, encoding="utf-8", errors="replace", timeout=45)
            result = dict(name=name, exit_code=execution.returncode, stdout=execution.stdout,
                          stderr=execution.stderr, seconds=round(time.perf_counter()-start, 4),
                          requested_viewport_points=[width, height], captured_file=str(captured))
            result.update(theme=theme, seeded_workspace=workspace)
            if execution.returncode == 0 and captured.is_file():
                with Image.open(captured) as image:
                    result["actual_pixels"] = list(image.size)
                    sx, sy = image.width/width, image.height/height
                if native_scale is None:
                    native_scale = (sx+sy)/2
                # Eframe screenshots contain physical pixels; viewport
                # sizes and egui gestures use logical points. Preserve
                # the original frame and report the native DPI scale.
                matches = .5 <= sx <= 4 and math.isclose(sx, sy, abs_tol=.002) and math.isclose(sx, native_scale, abs_tol=.002)
                result["pixels_per_point"] = sx
                result["logical_viewport_matches"] = matches
                result["raw_pixels_equal_viewport_points"] = result["actual_pixels"] == [width, height]
                result["passed"] = matches
                if result["passed"]:
                    captured.replace(target)
                    result["artifact_file"] = str(target)
                    result["sha256"] = av.sha(target)
            else:
                result["passed"] = False
        except subprocess.TimeoutExpired:
            result = dict(name=name, exit_code=None, passed=False,
                          error="Application capture timed out; incomplete output is not accepted")
        results.append(result)
        av.write_json(output / (name+".json"), result)
        print(name, "PASS" if result["passed"] else "FAIL", flush=True)
    manifest = dict(checked_at=datetime.now(timezone(timedelta(hours=3))).isoformat(),
                    gui=str(gui), gui_sha256=av.sha(gui), source_project=str(project) if project else None,
                    source_project_sha256=av.sha(project) if project else None,
                    empty_workspace=args.empty, scope="Application framebuffer capture; no physical sew-out",
                    isolated_appdata=True, results=results, passed=all(r["passed"] for r in results))
    manifest["native_pixels_per_point"] = native_scale
    av.write_json(output / "captures.json", manifest)
    print(json.dumps(dict(passed=manifest["passed"], captures=len(results)), indent=2))
    if not manifest["passed"]:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
