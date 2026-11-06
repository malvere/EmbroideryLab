"""Offline RAG reason histograms; first reasons and overlapping conditions stay separate."""
from __future__ import annotations

import argparse
from collections import Counter
import csv
import hashlib
import html
import json
from pathlib import Path

import quality_phase2_validation as q

BINS = ("1px", "2px", "3-4px", "5+px")
REQUIRED_REASONS = ("flat_source","protected","pinned","width","no_neighbor","insufficient_neighbor",
                    "weak_boundary","color_error","topology","invalidation","computation_limit",
                    "reassigned_close","reassigned_aa")
REQUIRED_CONDITIONS = ("flat_source","protected","pinned","width","invalidation","insufficient_neighbor",
                       "weak_boundary","flat_enclosed_close_target","flat_micro_aa_pair","flat_micro_aa_one_ring")
ROOT = q.ROOT
DEFAULT = ROOT/"outputs/quality-phase21-20261106/rag"


def flood(labels, w, h):
    seen = bytearray(len(labels))
    result = []
    for seed, label in enumerate(labels):
        if label == 255 or seen[seed]:
            continue
        pixels=[seed]
        seen[seed]=1
        for i in pixels:
            x,y=i%w,i//w
            for n in ([i-1] if x else [])+([i+1] if x+1<w else [])+([i-w] if y else [])+([i+w] if y+1<h else []):
                if not seen[n] and labels[n] == label:
                    seen[n]=1
                    pixels.append(n)
        result.append(pixels)
    return result


def size_bin(n):
    return 0 if n==1 else 1 if n==2 else 2 if n<=4 else 3


def independent_fate(trace, options, area):
    w,h=trace["trace_dimensions"]
    before,after=trace["labels_before_preparation"],trace["labels_after_preparation"]
    original,final=flood(before,w,h),flood(after,w,h)
    floors=(options.get("min_region_pixels",1),options.get("min_region_area_mm2",0.))
    keep=bytearray(len(before))
    omitted=[]
    for pixels in original:
        if len(pixels)>=floors[0] and len(pixels)*area+1e-12>=floors[1]:
            for i in pixels:keep[i]=1
        else:
            omitted.append(pixels)
    accepted=bytearray(len(after))
    for pixels in final:
        retained=any(keep[i] for i in pixels)
        if retained or (len(pixels)>=floors[0] and len(pixels)*area+1e-12>=floors[1]):
            for i in pixels:accepted[i]=1
    fates=Counter()
    examples=[]
    by_bin=[Counter() for _ in BINS]
    for pixels in omitted:
        changed=sum(before[i]!=after[i] for i in pixels)
        accepted_pixels=sum(accepted[i] for i in pixels)
        fate=("reassigned_into_accepted" if changed else "accepted_by_neighbor_growth") if accepted_pixels==len(pixels) else ("reassigned_still_omitted" if changed else "unchanged_still_omitted")
        fates[fate]+=1
        by_bin[size_bin(len(pixels))][fate]+=1
        if changed or accepted_pixels:
            examples.append(dict(original_pixels=len(pixels),original_label=before[pixels[0]],
                                 changed_pixels=changed,accepted_pixels=accepted_pixels,fate=fate,
                                 xy=[[i%w,i//w] for i in pixels[:16]]))
    return dict(before_components=len(original),after_components=len(final),foreground_pixels=sum(v!=255 for v in before),
                before_component_count_by_bin=[sum(size_bin(len(p))==b for p in original) for b in range(4)],
                changed_pixels=sum(a!=b for a,b in zip(before,after,strict=True)),
                foreground_support_changed_pixels=sum((a==255)!=(b==255) for a,b in zip(before,after,strict=True)),
                originally_accepted_pixels=sum(keep),previously_accepted_pixels_lost=sum(a and not b for a,b in zip(keep,accepted)),
                originally_omitted_components=len(omitted),originally_omitted_pixels=sum(map(len,omitted)),
                originally_omitted_size_histogram=dict(Counter(map(len,omitted))),omitted_fates=dict(fates),
                omitted_fates_by_bin=[dict(bin=name,fates=dict(values)) for name,values in zip(BINS,by_bin)],
                changed_or_newly_accepted_omitted_examples=examples,
                final_omitted_pixels=sum(v!=255 and not accepted[i] for i,v in enumerate(after)),
                note="Independent four-neighbor traversal and original acceptance provenance. This crest has no pinned/protected pixels; do not use this simplified threshold oracle for a protected fixture.")


def check_bins(counts,pixels):
    assert len(counts)==len(pixels)==4
    assert pixels[0]==counts[0]
    assert pixels[1]==counts[1]*2
    assert counts[2]*3<=pixels[2]<=counts[2]*4
    assert pixels[3]>=counts[3]*5


def read_version(folder):
    summary=json.loads((folder/"summary.json").read_text(encoding="utf-8-sig"))
    trace=json.loads((folder/"design.trace.json").read_text(encoding="utf-8-sig"))
    options=json.loads((folder/"options.json").read_text(encoding="utf-8-sig"))
    area=summary["pixel_area_mm2"]
    fate=independent_fate(trace,options,area)
    assert fate["before_components"]==trace["components_before_preparation"]
    assert fate["after_components"]==trace["components_after_preparation"]
    assert fate["foreground_pixels"]==trace["foreground_pixels"]
    assert fate["foreground_support_changed_pixels"]==0
    assert fate["previously_accepted_pixels_lost"]==summary["previously_accepted_pixels_lost"]
    assert fate["omitted_fates"]==summary["omitted_fates"]
    assert fate["originally_omitted_components"]==summary["original_omitted_components"]
    assert fate["final_omitted_pixels"]==trace["omitted_pixel_count"]
    passes=trace["preparation_diagnostics"]["passes"]
    first=[]
    conditions=[]
    pass_checks=[]
    for index,p in enumerate(passes,1):
        counts=p["first_reasons"]
        pixels=p["first_reason_pixels"]
        total_components=sum(map(sum,counts.values()))
        total_pixels=sum(map(sum,pixels.values()))
        assert total_pixels==fate["foreground_pixels"]
        if index==1:assert total_components==fate["before_components"]
        for reason,c in counts.items():
            check_bins(c,pixels[reason])
            first.append(dict(version=summary["label"],pass_index=index,reason=reason,
                              components=c,pixels=pixels[reason],area_mm2=[v*area for v in pixels[reason]]))
        for reason,c in p["conditions"].items():
            cp=p.get("condition_pixels",p.get("conditions_pixels",{})).get(reason)
            if cp is not None:check_bins(c,cp)
            conditions.append(dict(version=summary["label"],pass_index=index,reason=reason,
                                   components=c,pixels=cp,area_mm2=[v*area for v in cp] if cp is not None else None))
        for reason in REQUIRED_REASONS:
            if reason not in counts:
                first.append(dict(version=summary["label"],pass_index=index,reason=reason,components=[0]*4,pixels=[0]*4,area_mm2=[0.]*4))
        for reason in REQUIRED_CONDITIONS:
            if reason not in p["conditions"]:
                cp=[0]*4 if "condition_pixels" in p or "conditions_pixels" in p else None
                conditions.append(dict(version=summary["label"],pass_index=index,reason=reason,components=[0]*4,pixels=cp,area_mm2=[0.]*4 if cp is not None else None))
        pass_checks.append(dict(pass_index=index,exclusive_component_population=total_components,
                                exclusive_pixel_population=total_pixels,exclusive_area_mm2=total_pixels*area,
                                first_reasons_partition_foreground=True,conditions_overlap=True))
    for row in summary["first_reasons"]:
        raw=next(r for r in first if r["pass_index"]==row["pass_index"] and r["reason"]==row["reason"])
        assert all(raw[k]==row[k] for k in ("components","pixels","area_mm2"))
    stage_rows={}
    stage_checks={}
    for key,index in (("context_pass",0),("limit_probe",-1)):
        stage=trace["preparation_diagnostics"].get(key)
        if stage is None:
            stage_rows[key]=[]
            continue
        counts=stage["first_reasons"]
        pixels=stage["first_reason_pixels"]
        assert sum(map(sum,pixels.values()))==fate["foreground_pixels"]
        stage_rows[key]=[]
        for reason,c in counts.items():
            check_bins(c,pixels[reason])
            stage_rows[key].append(dict(version=summary["label"],pass_index=index,reason=reason,
                                        components=c,pixels=pixels[reason],area_mm2=[v*area for v in pixels[reason]]))
        stage_checks[key]=dict(components=sum(map(sum,counts.values())),pixels=sum(map(sum,pixels.values())),
                               comparison_work=(trace["preparation_diagnostics"].get("limit_probe_comparison_work") if key=="limit_probe" else None))
    return dict(label=summary["label"],summary=summary,trace={k:v for k,v in trace.items() if not k.startswith("labels_")},
                first=first,conditions=conditions,independent_fate=fate,pass_checks=pass_checks,
                dedicated_stages=stage_rows,dedicated_stage_checks=stage_checks,
                input_sha256={name:hashlib.sha256((folder/name).read_bytes()).hexdigest() for name in ("summary.json","design.trace.json","options.json","command.json") if (folder/name).is_file()})


def numeric(v,metric):
    return f"{v:.3f}" if metric=="area_mm2" else str(v)


def chart(path,versions,rows,pass_index,metric,overlapping):
    """Four explicit bin panels; paired variant bars share each panel's axis."""
    from PIL import Image,ImageDraw,ImageFont
    names=sorted({r["reason"] for r in rows if r["pass_index"]==pass_index})
    names=[name for name in names if any(r[metric] is not None for r in rows if r["reason"]==name and r["pass_index"]==pass_index)]
    if not names:return False
    font=ImageFont.truetype("C:/Windows/Fonts/segoeui.ttf",17)
    small=ImageFont.truetype("C:/Windows/Fonts/segoeui.ttf",14)
    titlefont=ImageFont.truetype("C:/Windows/Fonts/segoeui.ttf",23)
    width,height=2200,160+max(1,len(names))*max(70,28*len(versions)+20)
    image=Image.new("RGB",(width,height),"#fafaf7")
    draw=ImageDraw.Draw(image)
    stage="bounded context sweep" if pass_index==0 else "dry limit probe (NOT applied)" if pass_index==-1 else f"pass {pass_index}"
    title=("Overlapping conditions (DO NOT SUM)" if overlapping else "Exclusive FIRST reasons")+f" | {stage} | {metric}"
    draw.text((28,16),title,font=titlefont,fill="#152c39")
    draw.text((28,51),"Population belongs to this pass only; components may be revisited in later passes.",font=small,fill="#334d5b")
    colors={name:color for name,color in zip(versions,("#526b86","#d47331","#699a78"))}
    for i,name in enumerate(versions):
        x=1500+i*300
        draw.rectangle((x,24,x+20,44),fill=colors[name])
        draw.text((x+28,20),name,font=font,fill="#233643")
    row_height=max(70,28*len(versions)+20)
    labelwidth,panelwidth=385,445
    indexed={(r["version"],r["pass_index"],r["reason"]):r for r in rows}
    for b,bin_name in enumerate(BINS):
        px=labelwidth+b*panelwidth
        maximum=max((r[metric][b] for r in rows if r["pass_index"]==pass_index and r[metric] is not None),default=0)
        draw.text((px+10,82),bin_name+f" | max {numeric(maximum,metric)}",font=font,fill="#152c39")
        for fraction in (0,.5,1):
            x=px+20+fraction*325
            draw.line((x,115,x,height-20),fill="#deded8",width=1)
        for n,name in enumerate(names):
            y=125+n*row_height
            if b==0:draw.text((24,y+5),name,font=font,fill="#233643")
            for vi,version in enumerate(versions):
                row=indexed.get((version,pass_index,name))
                values=row[metric] if row else [0]*4
                if values is None:continue
                value=values[b]
                length=value/max(1e-30,maximum)*325
                yy=y+vi*28
                if value:draw.rectangle((px+20,yy,px+20+length,yy+18),fill=colors[version])
                draw.text((px+25+length,yy-2),numeric(value,metric),font=small,fill="#233643")
    image.save(path)
    return True


def write_csv(path,rows,kind):
    with path.open("w",encoding="utf-8-sig",newline="") as handle:
        writer=csv.DictWriter(handle,fieldnames=("kind","version","pass_index","reason","bin","components","pixels","area_mm2"))
        writer.writeheader()
        for row in rows:
            for i,bin_name in enumerate(BINS):
                writer.writerow(dict(kind=kind,version=row["version"],pass_index=row["pass_index"],reason=row["reason"],bin=bin_name,
                                     **{metric:row[metric][i] if row[metric] is not None else "" for metric in ("components","pixels","area_mm2")}))


def doc_report(path,versions,charts):
    lines=["# Диагностика RAG этапа 2.1", "", "Отчёт строится `tools/quality_phase21_rag_report.py` по сырым картам меток и отдельным summary. Никакие параметры сегментации или критерии не изменяются.", "",
           "## Как читать причины", "", "Первая причина взаимоисключающая: каждая компонента одного прохода попадает ровно в одну строку. Дополнительные условия пересекаются: их нельзя складывать между собой или с первыми причинами. Популяция перестраивается между проходами; сумма трёх проходов не является числом уникальных компонент.", "",
           "Гистограммы раздельно показывают 1 px, 2 px, 3–4 px и 5+ px, количество компонент, пиксели и площадь. Общие оси используются для одинакового прохода/показателя всех доступных вариантов. Полные точные числа — CSV/JSON в локальном `outputs/quality-phase21-20261106/rag/report/`.", ""]
    if len(versions)<2:lines.extend(["**Кандидат ещё не измерен: текущий отчёт частичный, Standard этапа 2.**",""])
    for version in versions:
        s=version["summary"];f=version["independent_fate"]
        lines.extend(["## "+version["label"],"",f"Карта {s['trace_dimensions'][0]}×{s['trace_dimensions'][1]}, площадь пикселя {s['pixel_area_mm2']:.11f} мм². Один пиксель меньше 0,1 мм², два — {2*s['pixel_area_mm2']:.11f} мм² и превышают порог.","",
                      "| Проход | Компоненты | Пиксели | Площадь мм² |", "|---|---:|---:|---:|"])
        for p in version["pass_checks"]:
            lines.append(f"| {p['pass_index']} | {p['exclusive_component_population']} | {p['exclusive_pixel_population']} | {p['exclusive_area_mm2']:.5f} |")
        lines.extend(["", "Первые причины и дополнительные условия сохранены отдельно. Проверено независимым flood-fill: первый проход соответствует числу исходных компонент; пиксели всех первых причин каждого прохода точно разбивают foreground.","",
                      "| Проход | Первая причина flat_source: компоненты по 1/2/3–4/5+ px | Пиксели по тем же группам |", "|---|---|---|"])
        for p in version["pass_checks"]:
            row=next((r for r in version["first"] if r["pass_index"]==p["pass_index"] and r["reason"]=="flat_source"),None)
            if row:lines.append(f"| {p['pass_index']} | {row['components']} | {row['pixels']} |")
        initial_flat=next((r for r in version["first"] if r["pass_index"]==1 and r["reason"]=="flat_source"),None)
        if initial_flat:
            lines.extend(["",f"В первом проходе flat_source — первая причина для {sum(initial_flat['components'])} компонент / {sum(initial_flat['pixels'])} пикселей / {sum(initial_flat['area_mm2']):.5f} мм². В группе 1 px: {initial_flat['components'][0]} из {f['before_component_count_by_bin'][0]} исходных однопиксельных компонент остановлены именно этой первой причиной."])
        lines.extend(["", "Это число фактически остановленных ранним guard компонент, а не число доказанных шумовых артефактов. Однородность не доказывает ни намеренность, ни допустимость переназначения.","",
                      "| Дополнительное условие | Проход | Компоненты 1/2/3–4/5+ px |", "|---|---:|---|"])
        for p in version["pass_checks"]:
            for name in ("flat_enclosed_close_target","flat_micro_aa_pair","flat_micro_aa_one_ring"):
                row=next((r for r in version["conditions"] if r["pass_index"]==p["pass_index"] and r["reason"]==name),None)
                lines.append(f"| {name} | {p['pass_index']} | {row['components'] if row else [0,0,0,0]} |")
        diag=version["trace"]["preparation_diagnostics"]
        lines.extend(["",f"Диагностический просмотр ещё одного прохода: {s['next_pass_changed_pixels']} потенциально изменённых пикселей. Это dry-run на копии финальных меток; он не включён в результат и не означает разрешения увеличить число проходов. Счётчик бюджета маркирующих стадий: {s['comparison_work']}; для контекстной стадии он включает консервативную верхнюю оценку работы, а не замер времени.","",
                      f"Отдельная диагностическая оценка контекстной работы: {diag.get('diagnostic_context_work_upper_bound','отсутствует')}; достигнут её собственный предел: `{diag.get('diagnostic_context_limit_reached','не записан')}`. Она не расходует бюджет исторических маркирующих проходов. Если предел достигнут, таблица потенциальных контекстных условий неполна; отсутствие наблюдений не доказывает невозможность изменения.","",
                      f"Исходно исключено {f['originally_omitted_components']} компонент / {f['originally_omitted_pixels']} пикселей. Гистограмма размеров: `{f['originally_omitted_size_histogram']}`.","",
                      "| Судьба исходно исключённой компоненты | Количество |", "|---|---:|"])
        for name,count in sorted(f["omitted_fates"].items()):lines.append(f"| {name} | {count} |")
        lines.extend(["",f"Независимый обход: ранее принятых пикселей {f['originally_accepted_pixels']}; потеряно {f['previously_accepted_pixels_lost']}; поддержка foreground изменилась в {f['foreground_support_changed_pixels']} пикселях; переназначено меток {f['changed_pixels']}. Финально исключённых пикселей {f['final_omitted_pixels']}.","",
                      "`reassigned_into_accepted` означает реальное изменение метки с последующим включением в принятую компоненту. `accepted_by_neighbor_growth` — метка самого пикселя не менялась, но выросла связная область того же цвета. `unchanged_still_omitted` — старое исключение остаётся, а не исправляется. Прозрачностью переназначение не заменялось.",""])
        if any(r["pixels"] is None for r in version["conditions"]):
            lines.extend(["Для этого снимка дополнительные условия содержат только количества компонент. Пиксели и площадь пересекающихся условий не выведены из групп 3–4/5+ px догадкой; соответствующие значения отсутствуют. Первые причины имеют точные пиксели и площадь.",""])
        if version["dedicated_stages"].get("context_pass"):
            lines.extend(["### Отдельный ограниченный контекстный просмотр", "", "Три обычных прохода Standard этапа 2 сохранены. После них кандидат выполняет один ограниченный проход только по плоским микрокомпонентам до 4 px с исходными пределами 0,55 мм / 0,035 OKLab и свежими зависимостями. Это отдельная стадия `context_pass`, а не четвёртый обычный RAG-проход.","",
                          "| Первая причина контекстной стадии | Компоненты 1/2/3–4/5+ px | Пиксели 1/2/3–4/5+ px |", "|---|---|---|"])
            for r in version["dedicated_stages"]["context_pass"]:lines.append(f"| {r['reason']} | {r['components']} | {r['pixels']} |")
            lines.extend(["", "`invalidation` означает отказ использовать уже изменённый узел или его зависимость до нового графа; число таких отказов не равно числу устранимых дефектов. `uncertain_context` сохраняет деталь. Реально переназначенные пиксели отделены от возможных кандидатов.",""])
        if version["dedicated_stages"].get("limit_probe"):
            lines.extend(["### Диагностический просмотр лимита", "", "Гистограмма `limit_probe` относится к копии финальной карты. Эти изменения не применены к продукту. Ни count этого просмотра, ни его бюджет не добавляются к обычным трём проходам.","",
                          "| Первая причина dry-run | Компоненты 1/2/3–4/5+ px | Пиксели 1/2/3–4/5+ px |", "|---|---|---|"])
            for r in version["dedicated_stages"]["limit_probe"]:lines.append(f"| {r['reason']} | {r['components']} | {r['pixels']} |")
            lines.extend(["",f"Отдельно учтённая работа dry-run: {diag.get('limit_probe_comparison_work','не записана')}. Проба наследует исходный бюджет, не начинает безлимитный новый поиск.",""])
    if len(versions)==2:
        a,b=versions
        same_passes=a["trace"]["preparation_diagnostics"]["passes"]==b["trace"]["preparation_diagnostics"]["passes"]
        comparison_note="Кандидат добавляет отдельную контекстную стадию." if same_passes else "Смысл диагностических причин между этими снимками различается; для прямого сравнения нужно повторить Standard и кандидата одной инструментированной сборкой."
        lines.extend(["## Ablation относительно Standard этапа 2", "", f"Диагностика всех трёх обычных проходов по структуре JSON одинакова: `{same_passes}`. {comparison_note} Для выбора AA применяется одно полное 8-соседнее кольцо; более строгое двухкольцевое условие сохранено как отдельная диагностическая ablation. Их потенциальные кандидаты не суммируются и не выдаются за фактические изменения.",""])
    lines.extend(["## Ограничения", "", "Прохождение сохранности не является абсолютным качеством рисунка. Проверка судьбы малых областей здесь рассчитана для герба без pinned/protected; такие защиты требуют штатного контракта импортера. Классы цветовой ошибки, топологического запрета и invalidation не объединяются в одну предполагаемую причину. Вычислительный лимит и наличие четвёртого допустимого шага различаются. Скрытая визуальная укрывистость не выводится из этой карты.","",
                  "Гистограммы находятся только локально; пользовательский источник и производные не публикуются."])
    path.write_text("\n".join(lines)+"\n",encoding="utf-8")


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root",type=Path,default=DEFAULT)
    parser.add_argument("--allow-partial",action="store_true")
    parser.add_argument("--doc",type=Path,default=ROOT/"docs/quality-phase21-rag.md")
    args=parser.parse_args()
    labels=[name for name in ("standard","candidate") if (args.root/name/"summary.json").is_file()]
    if not args.allow_partial and len(labels)!=2:raise RuntimeError("Need both Standard and candidate summaries; use --allow-partial for explicitly partial diagnosis")
    versions=[read_version(args.root/name) for name in labels]
    if not versions:raise RuntimeError("No RAG diagnostics found")
    output=args.root/"report";output.mkdir(parents=True,exist_ok=True)
    first=[r for v in versions for r in v["first"]]
    conditions=[r for v in versions for r in v["conditions"]]
    context=[r for v in versions for r in v["dedicated_stages"].get("context_pass",[])]
    dry=[r for v in versions for r in v["dedicated_stages"].get("limit_probe",[])]
    write_csv(output/"first-reasons-exclusive.csv",first,"exclusive_first_reason")
    write_csv(output/"conditions-overlapping.csv",conditions,"overlapping_condition_DO_NOT_SUM")
    write_csv(output/"context-first-reasons-exclusive.csv",context,"dedicated_bounded_context_sweep")
    write_csv(output/"dry-limit-first-reasons-exclusive.csv",dry,"diagnostic_probe_NOT_applied")
    charts=[]
    for rows,prefix,overlap in ((first,"first",False),(conditions,"conditions",True),(context,"context",False),(dry,"dry-limit",False)):
        for index in sorted({r["pass_index"] for r in rows}):
            for metric in ("components","pixels","area_mm2"):
                path=output/f"{prefix}-pass{index}-{metric}.png"
                if chart(path,labels,rows,index,metric,overlap):charts.append(path.name)
    q.write(output/"report.json",dict(versions=versions,charts=charts,first_reasons_exclusive=True,additional_conditions_overlap=True))
    doc_report(args.doc,versions,charts)
    cards="".join(f'<section><h2>{html.escape(name)}</h2><img src="{html.escape(name)}"></section>' for name in charts)
    (output/"index.html").write_text('<!doctype html><meta charset="utf-8"><title>RAG refusal reasons</title><style>body{font:16px system-ui;margin:24px;background:#fafaf7;color:#18313e}img{max-width:100%}section{margin:36px 0}</style><h1>RAG: exclusive first reasons / overlapping conditions</h1><p>Separate populations per pass. Do not sum conditions or passes. Exact values: CSV and report.json. Local private derivatives only.</p>'+cards,encoding="utf-8")
    for v in versions:
        print(v["label"],v["independent_fate"]["omitted_fates"],"next-pass",v["summary"]["next_pass_changed_pixels"],flush=True)


if __name__=="__main__":
    main()
