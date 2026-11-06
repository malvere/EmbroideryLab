"""Focused Phase 2.1 plan/independent-DST operational audit.

Read-only inputs. Existing Phase 2 validation and density implementations are
reused; transfer causes that are absent from MachinePlan are never inferred.
"""
from __future__ import annotations

import argparse
from collections import Counter, defaultdict
import csv
import importlib.metadata
import json
import math
from pathlib import Path
import shutil
import sys

import autodigitize_validation as av
import quality_phase2_density as density
import quality_phase2_validation as q


def read(path):
    return json.loads(path.read_text(encoding="utf-8-sig"))


def bucket(length):
    if length <= 1.: return "0-1mm"
    if length <= 3.: return "1-3mm"
    if length <= 10.: return "3-10mm"
    return ">10mm"


def plan_metrics(project, plan):
    generated = [{**s, **m} for s, m in zip(plan["stitches"], plan["metadata"], strict=True)]
    previous = (0., 0.)
    previous_sewn_object = None
    previous_sewn_color = None
    counts, lengths = Counter(), Counter()
    transfers = []
    travel_run = None
    trimmed = False
    objects = {o["id"]: o for o in project["objects"]}
    blocks = set()
    top_by_kind = Counter()
    def flush():
        nonlocal travel_run
        if travel_run is not None:
            transfers.append(travel_run)
            travel_run = None
    for index, s in enumerate(generated):
        point = s["x"], s["y"]
        command, role = s["command"], s["role"]
        counts[command] += 1
        if command == "Trim":
            trimmed = True
        if command in ("Stitch", "Jump"):
            length = math.dist(previous, point)
            lengths[command] += length
            if command == "Stitch":
                counts["Stitch:"+role] += 1
                lengths["Stitch:"+role] += length
                blocks.add((s["object_id"], s["pass_id"]))
                if role == "TopFill":
                    top_by_kind[objects[s["object_id"]].get("settings", {}).get("kind", "Tatami")] += 1
            is_travel = command == "Stitch" and role == "Travel"
            if command == "Jump" or is_travel:
                relation = ("initial_positioning" if previous_sewn_object is None else
                            "between_objects" if previous_sewn_object != s["object_id"] else "within_object")
                color_relation = ("initial" if previous_sewn_color is None else
                                  "same_color" if previous_sewn_color == s["color"] else "different_color")
                transport = "sewn_Travel" if is_travel else "planned_Jump"
                row = dict(first_plan_index=index, last_plan_index=index, transport=transport,
                           object_relation=relation, color_relation=color_relation,
                           target_object_id=s["object_id"], color_rgb=s["color"],
                           length_mm=length, segments=1, trim_requested_before=bool(trimmed),
                           reason=("object_entry" if relation == "between_objects" else
                                   "initial_positioning" if relation == "initial_positioning" else "span_or_pass_transfer"))
                if is_travel:
                    key = (s["object_id"], s["pass_id"], tuple(s["color"]))
                    if travel_run is not None and travel_run["_key"] == key:
                        travel_run["last_plan_index"] = index
                        travel_run["length_mm"] += length
                        travel_run["segments"] += 1
                    else:
                        flush()
                        travel_run = {**row, "_key": key}
                else:
                    flush()
                    transfers.append(row)
                trimmed = False
            else:
                flush()
            if command == "Stitch":
                previous_sewn_object = s["object_id"]
                previous_sewn_color = s["color"]
            previous = point
        elif command not in ("Trim",):
            flush()
    flush()
    groups = defaultdict(lambda: dict(count=0, segments=0, length_mm=0., trim_requests=0))
    color_groups = defaultdict(lambda: dict(count=0, segments=0, length_mm=0., trim_requests=0))
    for transfer in transfers:
        transfer.pop("_key", None)
        transfer["length_bin"] = bucket(transfer["length_mm"])
        key = " / ".join(transfer[k] for k in ("transport", "object_relation", "color_relation", "length_bin", "reason"))
        row = groups[key]
        row["count"] += 1
        row["segments"] += transfer["segments"]
        row["length_mm"] += transfer["length_mm"]
        row["trim_requests"] += transfer["trim_requested_before"]
        color_key = transfer["transport"]+" / RGB "+",".join(str(c) for c in transfer["color_rgb"])
        color_row = color_groups[color_key]
        color_row["count"] += 1
        color_row["segments"] += transfer["segments"]
        color_row["length_mm"] += transfer["length_mm"]
        color_row["trim_requests"] += transfer["trim_requested_before"]
    sewn = [(s["x"], s["y"]) for s in generated if s["command"] == "Stitch"]
    bounds = [min(v[i] for v in sewn) for i in (0, 1)] + [max(v[i] for v in sewn) for i in (0, 1)] if sewn else None
    return dict(objects=len(project["objects"]), needle_blocks=len(blocks), command_counts=dict(counts),
                planned_lengths_mm=dict(lengths), top_needles_by_kind=dict(top_by_kind),
                reconstructed_transfer_runs=len(transfers), transfer_groups=dict(groups), transfers_by_rgb=dict(color_groups),
                warning_counts=dict(Counter(w["message"] for w in plan.get("warnings", []))),
                planned_sewn_bounds_mm=bounds,
                planned_sewn_dimensions_mm=[bounds[2]-bounds[0], bounds[3]-bounds[1]] if bounds else [0., 0.],
                nominal_width_mm=project["width_mm"],
                nominal_height_mm=project.get("source_image", {}).get("height_mm"),
                object_colors_rgb=sorted({tuple(o["color"]) for o in project["objects"]}),
                machine_profile=project.get("machine_profile"),
                transition_reason_scope="Observable object-entry/span-or-pass categorisation only. MachinePlan does not serialize routing rejection causes; no geometric-concealment reason inferred.",
                logical_transfer_scope="Each planned Jump command and each contiguous sewn Travel run in one object/pass. This reconstruction is separate from split DST movements and true generator-level route intentions."), transfers, generated


def inspect_folder(folder, output):
    import pyembroidery as pe
    project, plan = read(folder/"design.elab"), read(folder/"design.plan.json")
    result, transfers, generated = plan_metrics(project, plan)
    independent, records = av.inspect(folder/"design.dst", generated, project, pe, {})
    result["independent_dst"] = independent
    result["density"] = density.measure(folder)
    result["inputs_sha256"] = {name: av.sha(folder/name) for name in ("design.elab", "design.plan.json", "design.dst")}
    result["input_folder"] = str(folder.resolve())
    result["decoded_dst_command_counts"] = dict(Counter(str(pe.decode_embroidery_command(code)[0]) for _, _, code in records))
    result["raw_dst_record_count"] = max(0, (len((folder/"design.dst").read_bytes())-512)//3)
    output.mkdir(parents=True, exist_ok=True)
    av.write_json(output/"metrics.json", result)
    if transfers:
        with (output/"transfers.csv").open("w", encoding="utf-8", newline="") as stream:
            writer = csv.DictWriter(stream, fieldnames=list(transfers[0]))
            writer.writeheader()
            writer.writerows(transfers)
    return result


def self_checks():
    project = dict(width_mm=10., objects=[dict(id=1, settings=dict(kind="Tatami"), color=[1, 2, 3]), dict(id=2, settings=dict(kind="Running"), color=[4, 5, 6])])
    stitches = [dict(x=x, y=0., command=c, object_id=obj, color=color)
                for x, c, obj, color in [(1., "Jump", 1, [1,2,3]), (2., "Stitch", 1, [1,2,3]),
                                        (3., "Stitch", 1, [1,2,3]), (4., "Stitch", 1, [1,2,3]),
                                        (4., "Trim", 1, [1,2,3]), (20., "Jump", 2, [4,5,6]),
                                        (21., "Stitch", 2, [4,5,6])]]
    metadata = [dict(role=r, pass_id=1) for r in ("Travel", "TopFill", "Travel", "Travel", "Control", "Travel", "TopFill")]
    result, runs, _ = plan_metrics(project, dict(stitches=stitches, metadata=metadata))
    assert result["command_counts"]["Stitch"] == 4
    assert result["command_counts"]["Jump"] == 2
    assert len(runs) == 3 and runs[1]["segments"] == 2
    assert runs[2]["object_relation"] == "between_objects" and runs[2]["color_relation"] == "different_color"
    assert runs[2]["trim_requested_before"] and runs[2]["length_bin"] == ">10mm"
    return dict(transport_role_not_command=True, contiguous_travel_run=True, object_color_boundary=True, trim_request_distinct=True)


def pair_truth_checks(output, source, truth_path):
    """Independent authored RGBA truth against saved source geometry, not RAG labels."""
    from PIL import Image, ImageDraw
    truth = Image.open(truth_path).convert("RGBA")
    expected = list(truth.get_flattened_data())
    w, h = truth.size
    dominant = Counter(expected).most_common(1)[0][0]
    isolated_detail_centers=[]
    for i,color in enumerate(expected):
        if color==dominant or color[3]<128:
            continue
        x,y=i%w,i//w
        incident=[j for j in (i-1 if x else -1,i+1 if x+1<w else -1,i-w if y else -1,i+w if y+1<h else -1) if j>=0]
        if all(expected[j]!=color for j in incident):
            isolated_detail_centers.append((x,y,color))
    results, images, actual = {}, {}, {}
    for label in ("standard_phase2", "candidate"):
        project = read(output/label/"design.elab")
        _, restored = q.project_labels(project, truth.size, return_rgb=True)
        restored.save(output/label/"restored-source-geometry.png")
        got = list(restored.get_flattened_data())
        plan=read(output/label/"design.plan.json")
        previous=(0.,0.);top=[]
        for stitch,meta in zip(plan["stitches"],plan["metadata"],strict=True):
            point=(av.quantize(stitch["x"]),av.quantize(stitch["y"]))
            if stitch["command"]=="Stitch" and meta["role"]=="TopFill":
                top.append((previous,point,tuple(stitch["color"])))
            if stitch["command"] in ("Stitch","Jump"):
                previous=point
        detail_witness=[]
        height_mm=project["source_image"]["height_mm"]
        for x,y,color in isolated_detail_centers:
            point=((x+.5)*project["width_mm"]/w,(y+.5)*height_mm/h)
            nearest=None
            for a,b,rgb in top:
                if rgb!=color[:3]:
                    continue
                d=(b[0]-a[0],b[1]-a[1]);norm=d[0]**2+d[1]**2
                t=max(0.,min(1.,((point[0]-a[0])*d[0]+(point[1]-a[1])*d[1])/norm)) if norm else 0.
                distance=math.dist(point,(a[0]+t*d[0],a[1]+t*d[1]))
                if nearest is None or distance<nearest[0]:
                    nearest=(distance,a,b)
            detail_witness.append(dict(source_xy=[x,y],required_rgb=color[:3],
                center_mm=point,color_correct_top_distance_mm=nearest[0] if nearest else None,
                nearest_color_correct_top_segment_mm=[nearest[1],nearest[2]] if nearest else None,
                inside_fixed_030mm_top_buffer=bool(nearest and nearest[0]<=.15)))
        actual[label], images[label] = got, restored
        wrong = [i for i, (a, b) in enumerate(zip(expected, got, strict=True)) if a != b]
        colors = {}
        for color in sorted(set(expected)):
            wanted = [p == color for p in expected]
            observed = [p == color for p in got]
            colors[str(color)] = dict(required_pixels=sum(wanted), correct_pixels=sum(a and b for a,b in zip(wanted, observed, strict=True)),
                intended_components=q.component_info(wanted,w,h)["count"], actual_components=q.component_info(observed,w,h)["count"],
                intended_holes=q.component_info([not p for p in wanted],w,h,holes=True)["internal"],
                actual_holes=q.component_info([not p for p in observed],w,h,holes=True)["internal"])
        results[label] = dict(required_foreground_pixels=sum(p[3] >= 128 for p in expected),
            source_geometry_elab_sha256=av.sha(output/label/"design.elab"), machine_plan_sha256=av.sha(output/label/"design.plan.json"),
            missing_foreground_pixels=sum(a[3] >= 128 and b[3] < 128 for a,b in zip(expected,got,strict=True)),
            exact_rgba_wrong_pixels=len(wrong), wrong_examples_xy=[(i%w,i//w) for i in wrong[:100]],
            non_dominant_detail_pixels=sum(p != dominant and p[3] >= 128 for p in expected),
            non_dominant_detail_pixels_wrong=sum(a != dominant and a[3] >= 128 and a != b for a,b in zip(expected,got,strict=True)),
            per_authored_color=colors, isolated_detail_top_witness=detail_witness,
            isolated_detail_top_witness_scope="Color-correct TopFill only, DST-quantized segments, fixed0.30mm buffer; single-point witness, no area/fabric claim. Underlay/Travel/Lock excluded.",
            absolute_verdict="PASS" if not wrong else "REVIEW")
    newly_wrong = [i for i,(a,b,c) in enumerate(zip(expected,actual["standard_phase2"],actual["candidate"],strict=True)) if a==b and a!=c]
    corrected = [i for i,(a,b,c) in enumerate(zip(expected,actual["standard_phase2"],actual["candidate"],strict=True)) if a!=b and a==c]
    evidence = dict(command_line=sys.orig_argv, script_sha256=av.sha(Path(__file__)),
        truth_sha256=av.sha(truth_path), source_sha256=av.sha(source), truth_path=str(truth_path.resolve()),
        reference="Authored fixture RGBA, fixed before candidate. Saved source contours/Running source snapshot/Satin rails rendered by existing q.project_labels; source-pixel centers. No label-map or stitch-coverage substitution.",
        truth_dimensions_px=[w,h], variants=results, previously_correct_pixels_lost=len(newly_wrong),
        newly_wrong_examples_xy=[(i%w,i//w) for i in newly_wrong], corrected_pixels=len(corrected),
        corrected_examples_xy=[(i%w,i//w) for i in corrected])
    q.write(output/"independent-authored-truth.json", evidence)
    panels = [("Source",Image.open(source).convert("RGBA")),("Authored truth",truth),
              ("Stage2 Standard",images["standard_phase2"]),("Candidate",images["candidate"])]
    zoom=6
    montage=Image.new("RGB",(w*zoom*4,h*zoom+25),"#eeeeee")
    draw=ImageDraw.Draw(montage)
    for n,(title,im) in enumerate(panels):
        draw.text((n*w*zoom+5,5),title,fill="black")
        montage.paste(im.resize((w*zoom,h*zoom),Image.Resampling.NEAREST),(n*w*zoom,25),im.resize((w*zoom,h*zoom),Image.Resampling.NEAREST).getchannel("A"))
    montage.save(output/"source-truth-standard-candidate-6x.png")
    remaining = [i for i,(a,b) in enumerate(zip(expected,actual["candidate"],strict=True)) if a!=b]
    for tag, locations in (("corrected",corrected),("remaining",remaining)):
        if not locations:
            continue
        x,y=locations[0]%w,locations[0]//w
        box=(max(0,x-4),max(0,y-4),min(w,x+5),min(h,y+5))
        cw,ch=box[2]-box[0],box[3]-box[1]
        focused=Image.new("RGB",(cw*24*4,ch*24+25),"#eeeeee")
        focused_draw=ImageDraw.Draw(focused)
        for n,(title,im) in enumerate(panels):
            focused_draw.text((n*cw*24+4,5),f"{title} ({x},{y})",fill="black")
            crop=im.crop(box).resize((cw*24,ch*24),Image.Resampling.NEAREST)
            focused.paste(crop,(n*cw*24,25),crop.getchannel("A"))
        focused.save(output/(tag+"-detail-24x.png"))
    return evidence


def sewout_pair(source, options, width_mm, standard_cli, candidate_cli, output, candidate_profile="Contextual", truth_path=None):
    """Prepare one frozen-input pair; never overwrites the original eight."""
    output.mkdir(parents=True, exist_ok=True)
    source_dir = output/"sources"
    source_dir.mkdir(exist_ok=True)
    frozen_source = source_dir/source.name
    if frozen_source.is_file() and av.sha(source) != av.sha(frozen_source):
        raise RuntimeError("Frozen coupon source differs; use a separate output folder")
    if source.resolve() != frozen_source.resolve() and not frozen_source.is_file():
        shutil.copy2(source, frozen_source)
    config = read(options)
    config["preparation"] = "Standard"
    frozen_options = source_dir/"options.json"
    if frozen_options.is_file() and read(frozen_options) != config:
        raise RuntimeError("Frozen coupon options differ; use a separate output folder")
    q.write(frozen_options, config)
    source_sha256 = av.sha(source)
    case = dict(name="phase21_problem_motif", source=str(frozen_source.resolve()),
                width_mm=width_mm, options=config, group="real_unlabelled", limits={})
    q.write(output/"acceptance-before-run.json", dict(source=str(source.resolve()), source_sha256=source_sha256,
            frozen_source_sha256=av.sha(frozen_source), options_sha256=q.value_hash(config),
            width_mm=width_mm, preparation_by_variant=dict(standard_phase2="Standard", candidate=candidate_profile), generators_identical=True,
            machine_model_owner_supplied="Fortever FT-1201R", actual_machine_profile_verified=False,
            expectation="Pair must produce different DST to evidence a changed motif. Independent format checks and repeat generation required; no physical outcome inferred.",
            physical_observations=None))
    results = {}
    for label, cli in (("standard_phase2", standard_cli), ("candidate", candidate_cli)):
        folder = output/label
        folder.mkdir(exist_ok=True)
        variant_config = dict(config, preparation="Standard" if label == "standard_phase2" else candidate_profile)
        q.write(folder/"options.json", variant_config)
        execution = q.measured_run([cli, "autodigitize", frozen_source, folder/"design.dst", width_mm, folder/"options.json"], folder)
        q.write(folder/"command.json", execution)
        if execution["exit_code"] != 0:
            raise RuntimeError(execution)
        (folder/"trace-command").mkdir(exist_ok=True)
        q.trace_report(cli, case, variant_config, folder)
        evaluated = q.evaluate(dict(case, options=variant_config), folder)
        q.write(folder/"evaluation.json", evaluated)
        result = inspect_folder(folder, output/"operations"/label)
        repeat = folder/"repeat"
        repeat.mkdir(exist_ok=True)
        regeneration = av.run([cli, "generate", folder/"design.elab", repeat/"design.dst"])
        q.write(repeat/"command.json", regeneration)
        result["repeat_generation_equal"] = regeneration["exit_code"] == 0 and av.sha(repeat/"design.dst") == av.sha(folder/"design.dst")
        if regeneration["exit_code"] == 0:
            original_project, reopened = read(folder/"design.elab"), read(repeat/"design.elab")
            result["saved_project_preservation"] = dict(
                source_image=original_project.get("source_image") == reopened.get("source_image"),
                recipe_masks_palette=original_project.get("raster_recipe") == reopened.get("raster_recipe"),
                manual_geometry=original_project["objects"] == reopened["objects"],
                machine_profile=original_project.get("machine_profile") == reopened.get("machine_profile"))
        result["cli_sha256"] = av.sha(cli)
        result["options_sha256"] = q.value_hash(variant_config)
        results[label] = result
    different = results["standard_phase2"]["inputs_sha256"]["design.dst"] != results["candidate"]["inputs_sha256"]["design.dst"]
    truth_evidence=pair_truth_checks(output,source,truth_path) if truth_path else None
    q.write(output/"summary.json", dict(variants=results, source_unchanged=av.sha(source)==source_sha256,
            dst_different=different, independent_authored_truth=truth_evidence, physical_sewout=False, machine_profile_verified=False))
    fields = ["variant", "machine_model", "hoop", "controller", "trim_profile", "thread", "fabric", "stabilizer", "speed", "actual_width_mm", "actual_height_mm", "visible_gaps", "puckering", "breaks", "notes"]
    observations = output/"observations.csv"
    # Human observations are never rewritten by regeneration, even if the
    # owner added columns, multiple rows, or a different annotation format.
    if not observations.exists():
        with observations.open("w", encoding="utf-8", newline="") as stream:
            writer = csv.DictWriter(stream, fieldnames=fields)
            writer.writeheader()
            for label in results:
                writer.writerow(dict(variant=label, machine_model="Fortever FT-1201R"))
    rows = []
    for label, result in results.items():
        dimensions = result["independent_dst"]["bounds_mm"]
        stitched = [dimensions[2]-dimensions[0], dimensions[3]-dimensions[1]]
        rows.append(f"| {label} | {result['nominal_width_mm']:.3f} × {result['nominal_height_mm']:.3f} | {stitched[0]:.3f} × {stitched[1]:.3f} | {len(result['object_colors_rgb'])} | {result['command_counts'].get('Stitch',0)} | {result['command_counts'].get('Trim',0)} |")
    text = f"# Пара для последующей отшивки\n\nОдин побайтово одинаковый исходник и параметры генераторов; подготовка Standard этапа 2 сравнивается с {candidate_profile}. Исходные восемь проектов этапа 2 сохранены.\n\n| Вариант | Номинальный размер источника, мм | Фактический стежковый размер DST, мм | Цветов | Проколов | Запросов Trim |\n| --- | ---: | ---: | ---: | ---: | ---: |\n"+"\n".join(rows)+f"\n\nDST различаются: {different}. Независимое чтение и повторная генерация сохранены в summary.json; панели одинакового масштаба — preview.png каждого варианта.\n\nМодель Fortever FT-1201R сообщена владельцем. Размер пялец, контроллер, поддержка обрезки, пределы перемещений, скорость, нить и ткань не подтверждены. До отшивки проверьте размещение и фактические габариты в пяльцах, предельные стежок/переход, кодирование Trim, схему цветов, материал и стабилизатор. Профиль в ELAB — сохранённая настройка проекта, а не подтверждённая спецификация машины. Файлы на машину не отправлялись; физические результаты заполняет владелец в observations.csv после отшивки.\n"
    if truth_evidence:
        old=truth_evidence["variants"]["standard_phase2"];new=truth_evidence["variants"]["candidate"]
        text += f"\nНезависимый авторский эталон проверен по сохранённой исходной геометрии ELAB через q.project_labels: неверных RGBA-центров {old['exact_rgba_wrong_pixels']} → {new['exact_rgba_wrong_pixels']}; новых потерь ранее правильных пикселей {truth_evidence['previously_correct_pixels_lost']}. Контрольные детали вне основного цвета: {new['non_dominant_detail_pixels']}, неверных {new['non_dominant_detail_pixels_wrong']}. Абсолютный вердикт кандидата: {new['absolute_verdict']}. Полное сравнение цвета и топологии — independent-authored-truth.json; одинаковый шестикратный масштаб — source-truth-standard-candidate-6x.png.\n"
    (output/"README.md").write_text(text, encoding="utf-8")
    return results


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--variant", action="append", default=[], help="LABEL=DIRECT_DESIGN_FOLDER")
    parser.add_argument("--group", action="append", default=[], help="LABEL=FOLDER_CONTAINING_CASE_FOLDERS")
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--pair-source", type=Path)
    parser.add_argument("--pair-options", type=Path)
    parser.add_argument("--pair-truth", type=Path)
    parser.add_argument("--pair-width", type=float)
    parser.add_argument("--standard-cli", type=Path)
    parser.add_argument("--candidate-cli", type=Path)
    parser.add_argument("--candidate-profile", default="Contextual")
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    result = dict(command_line=[sys.executable]+sys.argv, script_sha256=av.sha(Path(__file__)),
                  python_version=sys.version, pillow_version=importlib.metadata.version("Pillow"),
                  independent_reader="pyembroidery "+importlib.metadata.version("pyembroidery"),
                  self_checks=self_checks(), cases={})
    if args.pair_source:
        if not all((args.pair_options, args.pair_width, args.standard_cli, args.candidate_cli)):
            parser.error("--pair-source requires --pair-options, --pair-width, --standard-cli and --candidate-cli")
        result["cases"].update(sewout_pair(args.pair_source, args.pair_options, args.pair_width,
                args.standard_cli.resolve(), args.candidate_cli.resolve(), args.output/"sewout-pair", args.candidate_profile, args.pair_truth))
    for spec in args.variant:
        name, path = spec.split("=", 1)
        result["cases"][name] = inspect_folder(Path(path), args.output/name)
        print(name, result["cases"][name]["command_counts"], flush=True)
    for spec in args.group:
        label, path = spec.split("=", 1)
        for folder in sorted(Path(path).iterdir()):
            if (folder/"design.plan.json").is_file():
                name = label+"/"+folder.name
                result["cases"][name] = inspect_folder(folder, args.output/name)
                print(name, result["cases"][name]["command_counts"], flush=True)
    result["all_independent_format_checks_passed"] = all(c["independent_dst"]["format_geometry_passed"] for c in result["cases"].values())
    result["all_requested_repeat_checks_passed"] = all(c.get("repeat_generation_equal", True) for c in result["cases"].values())
    result["all_saved_project_preservation_checks_passed"] = all(all(c.get("saved_project_preservation", {}).values()) for c in result["cases"].values())
    if args.pair_source:
        result["pair_dst_different"] = (result["cases"]["standard_phase2"]["inputs_sha256"]["design.dst"] !=
                                        result["cases"]["candidate"]["inputs_sha256"]["design.dst"])
    av.write_json(args.output/"summary.json", result)
    return 0 if (result["all_independent_format_checks_passed"] and result["all_requested_repeat_checks_passed"] and result["all_saved_project_preservation_checks_passed"]
                 and result.get("pair_dst_different", True)) else 1


if __name__ == "__main__":
    raise SystemExit(main())
