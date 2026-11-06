"""Run the opt-in native egui workflow, with isolated preferences and no OS input."""
from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import subprocess
import time

import autodigitize_validation as av


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--gui", type=Path, required=True)
    parser.add_argument("--input", type=Path, required=True)
    parser.add_argument("--options", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--width-mm", type=float, default=60.)
    parser.add_argument("--size", default="1366x768")
    parser.add_argument("--language", default="ru", choices=["ru", "en"])
    args = parser.parse_args()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    gui, source, options = args.gui.resolve(), args.input.resolve(), args.options.resolve()
    target = output / "workflow.png"
    roaming, local = output / "state/Roaming", output / "state/Local"
    roaming.mkdir(parents=True, exist_ok=True)
    local.mkdir(parents=True, exist_ok=True)
    env = dict(os.environ, APPDATA=str(roaming), LOCALAPPDATA=str(local),
               EMBROIDERYLAB_LANGUAGE=args.language, EMBROIDERYLAB_THEME="dark",
               EMBROIDERYLAB_CAPTURE_SIZE=args.size,
               EMBROIDERYLAB_CAPTURE_TO=str(target),
               EMBROIDERYLAB_CAPTURE_QUALITY_WORKFLOW="1",
               EMBROIDERYLAB_CAPTURE_OPTIONS=str(options),
               EMBROIDERYLAB_CAPTURE_WIDTH_MM=str(args.width_mm))
    for name in ("EMBROIDERYLAB_CAPTURE_EMPTY", "EMBROIDERYLAB_CAPTURE_3D", "EMBROIDERYLAB_CAPTURE_WORKER_PROBE"):
        env.pop(name, None)
    command = [str(gui), str(source)]
    started = time.perf_counter()
    result = dict(command=command, gui_sha256=av.sha(gui), source_sha256=av.sha(source),
                  options_sha256=av.sha(options), width_mm=args.width_mm, requested_points=args.size,
                  isolated_appdata=True, native_os_input=False, physical_sewout=False)
    try:
        execution = subprocess.run(command, cwd=av.ROOT, env=env, capture_output=True,
                                   encoding="utf-8", errors="replace", timeout=60)
        result.update(exit_code=execution.returncode, stdout=execution.stdout, stderr=execution.stderr)
        workflow_path = target.with_suffix(".workflow.json")
        workflow = json.loads(workflow_path.read_text(encoding="utf-8")) if workflow_path.exists() else {}
        result["workflow"] = workflow
        result["passed"] = execution.returncode == 0 and workflow.get("passed") is True and target.exists()
        if result["passed"]:
            import pyembroidery as pe
            from PIL import Image
            project = json.loads(target.with_suffix(".elab").read_text(encoding="utf-8"))
            plan = json.loads(target.with_suffix(".plan.json").read_text(encoding="utf-8"))
            generated = [{**s, **m} for s, m in zip(plan["stitches"], plan["metadata"], strict=True)]
            checks, _ = av.inspect(target.with_suffix(".dst"), generated, project, pe, {})
            result["independent_dst"] = checks
            result["passed"] &= checks["format_geometry_passed"]
            with Image.open(target) as frame:
                result["actual_pixels"] = list(frame.size)
    except subprocess.TimeoutExpired as error:
        result.update(passed=False, error="Native application workflow timed out", stdout=str(error.stdout), stderr=str(error.stderr))
    except Exception as error:
        result.update(passed=False, error=str(error))
    result["seconds"] = round(time.perf_counter() - started, 4)
    av.write_json(output / "run.json", result)
    print(json.dumps(result, ensure_ascii=False, indent=2))
    if not result["passed"]:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
