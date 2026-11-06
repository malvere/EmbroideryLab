"""Local phase 2.1 provenance and three-way runner; never publishes artifacts."""
from pathlib import Path
import argparse, hashlib, json, os, shutil, subprocess, sys, time

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / 'outputs/quality-phase21-20261106'
OLD = ROOT / 'outputs/quality-phase2-20261106'

def write(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, ensure_ascii=False, indent=2), encoding='utf-8')

def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()

def freeze():
    folder = OUT/'initial'
    if folder.exists():
        raise RuntimeError('Initial snapshot already exists; refusing overwrite')
    folder.mkdir(parents=True)
    state = {}
    for args in (['git','status','--porcelain=v1','-uno'],['git','rev-parse','HEAD'],
                 ['git','diff','--binary'],['git','show','--stat','59b0dd641a387aa87cba0b4335eda4d58a2c9300']):
        r = subprocess.run(args, cwd=ROOT, capture_output=True)
        state[' '.join(args)] = dict(code=r.returncode, stdout=r.stdout.decode('utf-8','replace'), stderr=r.stderr.decode('utf-8','replace'))
    hashes = {}
    for source in [*sorted((ROOT/'embroidery-engine/src').rglob('*')), *sorted((ROOT/'embroidery-engine/tests').rglob('*')),
                   ROOT/'embroidery-engine/Cargo.toml',ROOT/'embroidery-engine/Cargo.lock',
                   ROOT/'tools/quality_phase2_validation.py', ROOT/'docs/quality-phase2-report.md',ROOT/'docs/quality-phase2-reproduction.md']:
        if not source.is_file(): continue
        rel = source.relative_to(ROOT)
        dest = folder/rel
        dest.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(source, dest)
        hashes[str(rel)] = sha(source)
    for group in ('comparison','comparison-unpinned','comparison-paired'):
        for name in ('fixtures.json','fixture-filehashes.json','summary.json'):
            source = OLD/group/name
            dest = folder/group/name
            dest.parent.mkdir(parents=True,exist_ok=True)
            shutil.copy2(source,dest)
            hashes[str(source.relative_to(ROOT))] = sha(source)
    for name, source in [('legacy.exe',OLD/'baseline-bin/embroidery-cli-source.exe'),
                          ('standard.exe',OLD/'final-bin/embroidery-cli.exe')]:
        shutil.copy2(source,folder/name)
        hashes[name]=sha(source)
    source = Path('C:/Users/Admin/Downloads/Ездец.svg (1).jpg')
    from PIL import Image
    with Image.open(source) as image:
        state['private_source'] = dict(path=str(source),format=image.format,dimensions=image.size,sha256=sha(source))
    state['hashes']=hashes
    state['python']=sys.version
    write(folder/'state.json',state)
    print(json.dumps({'snapshot':str(folder),'source':state['private_source'],'head':state['git rev-parse HEAD']['stdout'].strip()},ensure_ascii=False))

def run(label,cli,profile):
    import quality_phase2_validation as q
    for group in ('comparison','comparison-unpinned','comparison-paired'):
        cases=q.load_frozen_manifest(OLD/group/'fixtures.json')
        target=OUT/label/group
        target.mkdir(parents=True,exist_ok=True)
        q.run_version(Path(cli),label,cases,target,profile,diagnostic_cli=Path(cli))
        print(label,group,'complete',flush=True)

def equivalence():
    rows=[]
    for group in ('comparison','comparison-unpinned','comparison-paired'):
        manifest=json.loads((OLD/group/'fixtures.json').read_text(encoding='utf-8'))
        for c in manifest['cases']:
            a=OLD/group/'candidate'/c['name']
            b=OUT/'standard'/group/'standard'/c['name']
            matches={}
            for suffix in ('dst','elab','plan.json','stitches.json'):
                pa=a/('design.'+suffix); pb=b/('design.'+suffix)
                if pa.exists() and pb.exists():
                    matches[suffix]=sha(pa)==sha(pb)
            rows.append(dict(group=group,name=c['name'],files=matches,passed=bool(matches) and all(matches.values())))
    write(OUT/'standard-equivalence.json',rows)
    print('Stage2 Standard exact output matches',sum(r['passed'] for r in rows),'/',len(rows))
    if not all(r['passed'] for r in rows):raise SystemExit(1)

def microfixtures():
    from PIL import Image,ImageDraw
    import quality_phase2_validation as q
    folder=OUT/'micro'
    if (folder/'fixtures.json').exists():raise RuntimeError('Frozen fixtures already exist')
    folder.mkdir(parents=True,exist_ok=True)
    cases=[]
    # Held-out colours on the same geometry are fixed before implementation.
    for name,base,noise,point,group in [
        ('enclosed_grey',(160,160,160),(170,170,170),(0,0,0),'tuning'),
        ('enclosed_blue',(64,120,180),(69,125,185),(245,180,30),'holdout')]:
        im=Image.new('RGBA',(64,64),(*base,255));d=ImageDraw.Draw(im)
        d.rectangle((36,11,36,38),fill=(255,255,255,255))
        d.ellipse((8,39,25,56),fill=(*point,255));d.ellipse((12,43,21,52),fill=(*base,255))
        d.rectangle((44,47,54,49),fill=(*point,255));d.rectangle((48,43,50,54),fill=(*point,255))
        d.point((46,16),fill=(*point,255))
        truth=folder/(name+'-truth.png'); im.save(truth)
        d.point((19,19),fill=(*noise,255));d.point((25,29),fill=(*noise,255))
        source=folder/(name+'.png');im.save(source)
        cases.append(dict(name=name,group=group,source=str(source),truth=str(truth),width_mm=12.8,
                          options=q.options(palette=5,size=64),
                          limits=dict(note='Exact hand-built truth; noise points excluded, intentional point/white line/hole/bridge required. Compare all colors and source support; no frozen phase2 limits changed.')))
    write(folder/'fixtures.json',dict(seed=None,cases=cases))
    write(folder/'fixture-filehashes.json',dict(manifest_sha256=sha(folder/'fixtures.json'),
        cases=[dict(name=c['name'],source=sha(c['source']),truth=sha(c['truth']),options=q.value_hash(c['options'])) for c in cases]))
    print('Frozen independent micro fixtures',len(cases))

def diagnostic(cli,label):
    import quality_phase2_validation as q
    folder=OUT/'rag'/label;folder.mkdir(parents=True,exist_ok=True)
    source=Path('C:/Users/Admin/Downloads/Ездец.svg (1).jpg')
    opts=json.loads((OLD/'comparison/candidate/user_crest/options.json').read_text())
    opts['preparation']='Contextual' if label=='candidate' else 'Standard'
    write(folder/'options.json',opts)
    command=[str(cli),'trace-report',str(source),str(folder/'design.trace.json'),'100',str(folder/'options.json')]
    start=time.perf_counter();result=subprocess.run(command,cwd=ROOT,capture_output=True)
    write(folder/'command.json',dict(command=command,exit_code=result.returncode,seconds=time.perf_counter()-start,
          stdout=result.stdout.decode('utf-8','replace'),stderr=result.stderr.decode('utf-8','replace'),cli_sha256=sha(cli)))
    if result.returncode:raise RuntimeError(result.stderr.decode('utf-8','replace'))
    trace=json.loads((folder/'design.trace.json').read_text())
    old=json.loads((OLD/'comparison/candidate/user_crest/design.trace.json').read_text())
    w,h=trace['trace_dimensions'];area=100/w*(133.8/h)
    diagnostics=trace['preparation_diagnostics']
    table=[]
    for number,p in enumerate(diagnostics['passes']):
        for reason,counts in p['first_reasons'].items():
            pixels=p['first_reason_pixels'][reason]
            table.append(dict(pass_index=number+1,reason=reason,bins=['1px','2px','3-4px','5+px'],components=counts,pixels=pixels,area_mm2=[v*area for v in pixels]))
    # Independent component traversal; final acceptance uses original provenance.
    def components(labels):
        seen=set();out=[]
        for i,v in enumerate(labels):
            if v==255 or i in seen:continue
            pending=[i];seen.add(i)
            for j in pending:
                x,y=j%w,j//w
                for n in ([j-1] if x else [])+([j+1] if x+1<w else [])+([j-w] if y else [])+([j+w] if y+1<h else []):
                    if n not in seen and labels[n]==v:seen.add(n);pending.append(n)
            out.append(pending)
        return out
    before=trace['labels_before_preparation'];after=trace['labels_after_preparation']
    original=components(before); final=components(after)
    keep=[False]*len(before);omitted=[]
    for c in original:
        if len(c)*area+1e-12>=.1:
            for i in c:keep[i]=True
        else:omitted.append(c)
    accepted=[False]*len(after)
    for c in final:
        if any(keep[i] for i in c) or len(c)*area+1e-12>=.1:
            for i in c:accepted[i]=True
    from collections import Counter
    fates=Counter()
    for c in omitted:
        if all(accepted[i] for i in c):
            fates['reassigned_into_accepted' if any(before[i]!=after[i] for i in c) else 'accepted_by_neighbor_growth']+=1
        elif any(before[i]!=after[i] for i in c):fates['reassigned_still_omitted']+=1
        else:fates['unchanged_still_omitted']+=1
    result=dict(label=label,trace_dimensions=[w,h],pixel_area_mm2=area,first_reasons=table,
                extra_conditions=diagnostics['passes'],next_pass_changed_pixels=diagnostics['next_pass_changed_pixels'],
                comparison_work=diagnostics['comparison_work'],original_omitted_components=len(omitted),
                original_omitted_pixels=sum(map(len,omitted)),omitted_size_histogram=dict(Counter(len(c) for c in omitted)),
                omitted_fates=dict(fates),previously_accepted_pixels_lost=sum(a and not b for a,b in zip(keep,accepted)),
                stage2_labels_exact=after==old['labels_after_preparation'])
    write(folder/'summary.json',result)
    print(json.dumps({k:v for k,v in result.items() if k not in ('first_reasons','extra_conditions')},ensure_ascii=False))

def summary():
    """Join frozen evidence; do not rerun generation or alter quality gates."""
    import csv, html
    from collections import Counter, defaultdict
    from urllib.parse import quote
    from PIL import Image, __version__ as pillow_version
    import quality_phase2_validation as q
    groups=('comparison','comparison-unpinned','comparison-paired')
    def read(path):
        return json.loads(Path(path).read_text(encoding='utf-8-sig'))
    def link(path):
        return quote(os.path.relpath(Path(path).resolve(),OUT).replace('\\','/'),safe='/')
    def evidence(path):
        return dict(path=str(Path(path).resolve()),sha256=sha(path))
    baseline_ops=read(OUT/'operations/corpus-baseline/summary.json')
    candidate_ops=read(OUT/'operations/corpus-candidate/summary.json')
    operation_labels={
        'comparison':('legacy','standard_phase2','candidate'),
        'comparison-unpinned':('legacy_unpinned','standard_unpinned','candidate_unpinned'),
        'comparison-paired':('legacy_paired','standard_paired','candidate_paired')}
    variants=('Legacy','Standard_phase2','Contextual_candidate')
    rows=[];commands=[];preservation=[];manifest_evidence=[]
    for group in groups:
        manifest=OLD/group/'fixtures.json'
        cases=q.load_frozen_manifest(manifest)
        manifest_evidence.append(evidence(manifest))
        for case in cases:
            name=case['name']
            source=Path(case['source'])
            with Image.open(source) as image:
                source_size=list(image.size);detected_format=image.format
            source_hash=sha(source)
            nominal_height=case['width_mm']*source_size[1]/source_size[0]
            folders=(OLD/group/'baseline'/name,OUT/'standard'/group/'standard'/name,
                     OUT/'candidate'/group/'candidate'/name)
            row=dict(name=name,manifest_group=group,corpus_role=case['group'],source=str(source),
                     source_sha256=source_hash,source_format=detected_format,source_dimensions=source_size,
                     truth=case.get('truth'),truth_sha256=sha(case['truth']) if case.get('truth') else None,
                     nominal_dimensions_mm=[case['width_mm'],nominal_height],frozen_limits=case['limits'],
                     frozen_options=case['options'],variants={})
            normalized_expected={k:v for k,v in case['options'].items() if k!='preparation'}
            for number,(variant,folder) in enumerate(zip(variants,folders)):
                evaluation=read(folder/'evaluation.json')
                if evaluation['exit_code']!=0 or evaluation['input_sha256']!=source_hash:
                    raise RuntimeError(f'Failed run or changed input: {variant}/{name}')
                normalized={k:v for k,v in evaluation['options'].items() if k!='preparation'}
                if normalized!=normalized_expected:
                    raise RuntimeError(f'Comparison settings differ beyond preparation: {variant}/{name}')
                op_label=operation_labels[group][number]
                op_root=OUT/'operations'/('corpus-candidate' if number==2 else 'corpus-baseline')
                op_path=op_root/op_label/name/'metrics.json'
                operations=read(op_path)
                # Standard operations were measured from the historical result;
                # require byte identity to the newly reproduced Standard files.
                for filename,expected_hash in operations['inputs_sha256'].items():
                    if sha(folder/filename)!=expected_hash:
                        raise RuntimeError(f'Stale operation measurement: {variant}/{name}/{filename}')
                independent=evaluation['independent_dst'];counts=operations['command_counts']
                density=operations['density'];bounds=independent['bounds_mm']
                sewn=[bounds[2]-bounds[0],bounds[3]-bounds[1]]
                reference=evaluation.get('reference')
                status='UNLABELLED' if reference is None else ('PASS' if reference['accepted'] else 'REVIEW')
                flat=dict(reference_status=status,objects=evaluation['objects'],needle_blocks=evaluation['needle_blocks'],
                    punctures=independent['needle_count'],planned_jump_commands=counts.get('Jump',0),
                    sewn_travel_stitches=counts.get('Stitch:Travel',0),
                    reconstructed_transfer_runs=operations['reconstructed_transfer_runs'],
                    trim_requests=counts.get('Trim',0),dst_raw_jump_records=independent['raw_dst_jump_records'],
                    dst_decoded_jump_records=independent['jump_count'],
                    dst_decoded_nonzero_jump_records=independent['nonzero_jump_count'],
                    dst_inferred_trim_sequences=independent['inferred_trims'],dst_trim_encoding=independent['dst_trim_encoding'],
                    color_changes=independent['color_changes'],used_object_colors=len(operations['object_colors_rgb']),
                    planned_jump_length_mm=operations['planned_lengths_mm'].get('Jump',0.),
                    decoded_jump_length_mm=independent['jump_length_mm'],
                    sewn_travel_length_mm=operations['planned_lengths_mm'].get('Stitch:Travel',0.),
                    sewn_length_mm=independent['stitch_length_mm'],
                    max_all_role_length_density_mm_per_mm2=density['maximum_all_role_sewn_length_mm_per_mm2'],
                    max_top_length_density_mm_per_mm2=density['maximum_top_fill_length_mm_per_mm2'],
                    max_all_role_punctures_per_mm2=density['maximum_all_role_punctures_per_mm2'],
                    max_top_punctures_per_mm2=density['maximum_top_fill_punctures_per_mm2'],
                    sewn_width_mm=sewn[0],sewn_height_mm=sewn[1],seconds=evaluation['seconds'],
                    peak_working_set_bytes=evaluation['peak_working_set_bytes'])
                stage=evaluation.get('stages',{})
                flat.update(trace_width_px=(stage.get('trace_dimensions') or [None,None])[0],
                            trace_height_px=(stage.get('trace_dimensions') or [None,None])[1],
                            omitted_regions=stage.get('omitted_region_count'),omitted_pixels=stage.get('omitted_pixel_count'))
                coverage=evaluation.get('sampled_top_coverage')
                flat['old_sampled_top_coverage_fraction']=coverage.get('covered_fraction') if isinstance(coverage,dict) else None
                files={filename:evidence(folder/filename) for filename in
                       ('evaluation.json','options.json','command.json','trace-command.json','design.dst','design.elab','design.plan.json','design.stitches.json','design.trace.json')
                       if (folder/filename).is_file()}
                for filename in ('command.json','trace-command.json'):
                    if filename in files:
                        commands.append(dict(variant=variant,case=name,manifest_group=group,**files[filename],record=read(folder/filename)))
                row['variants'][variant]=dict(folder=str(folder),metrics=flat,evaluation=evaluation,
                    operation_metrics=operations,operation_evidence=evidence(op_path),files=files,
                    cli_sha256=evaluation['cli_sha256'],nominal_dimensions_mm=row['nominal_dimensions_mm'],
                    actual_sewn_bounds_mm=bounds,actual_sewn_dimensions_mm=sewn)
            if case.get('truth'):
                truth=Image.open(case['truth']).convert('RGBA');w,h=truth.size
                expected=[255 if p[3]<128 else q.COLORS.index(p[:3]) for p in truth.get_flattened_data()]
                label_maps=[];rgb_maps=[]
                for folder in folders[1:]:
                    region=Image.open(folder/'regions.png').convert('RGBA')
                    label_maps.append([255 if p[3]<128 else q.COLORS.index(p[:3]) for p in region.get_flattened_data()])
                    rgb_maps.append(list(Image.open(folder/'actual-thread-colors.png').convert('RGBA').get_flattened_data()))
                standard,candidate=label_maps
                boundary=[]
                for i in range(w*h):
                    x,y=i%w,i//w
                    ns=([i-1] if x else [])+([i+1] if x+1<w else [])+([i-w] if y else [])+([i+w] if y+1<h else [])
                    boundary.append(any(expected[i]!=expected[n] for n in ns))
                distance=q.distance_to(boundary,w,h)
                correct=[i for i in range(w*h) if distance[i]>2 and standard[i]==expected[i]]
                foreground=[i for i in correct if expected[i]!=255]
                background=[i for i in correct if expected[i]==255]
                failures=[i for i in correct if candidate[i]!=expected[i]]
                physical_rgb_changes=[i for i in foreground if rgb_maps[0][i]!=rgb_maps[1][i]]
                actual_color_changes=[]
                source_pixels=Image.open(source).convert('RGBA') if physical_rgb_changes else None
                for i in physical_rgb_changes:
                    truth_rgb=q.COLORS[expected[i]];before=rgb_maps[0][i];after=rgb_maps[1][i]
                    truth_lab=q.oklab(truth_rgb)
                    old_error=sum((a-b)**2 for a,b in zip(truth_lab,q.oklab(before[:3])))**.5
                    new_error=sum((a-b)**2 for a,b in zip(truth_lab,q.oklab(after[:3])))**.5
                    actual_color_changes.append(dict(point=[i%w,i//w],truth_rgb=truth_rgb,
                        source_rgba=source_pixels.getpixel((i%w,i//w)) if source_pixels.size==(w,h) else None,
                        standard_rgba=before,candidate_rgba=after,standard_oklab_error=old_error,
                        candidate_oklab_error=new_error,old_exact_truth_rgb=before[:3]==truth_rgb,
                        candidate_exact_truth_rgb=after[:3]==truth_rgb,
                        error_decreased=new_error<old_error,error_increased=new_error>old_error))
                check=dict(name=name,manifest_group=group,standard_correct_foreground_core_pixels=len(foreground),
                    standard_correct_background_core_pixels=len(background),lost_standard_correct_core_pixels=len(failures),
                    actual_thread_rgba_changes_on_correct_foreground_cores=len(physical_rgb_changes),
                    actual_rgba_identity=not physical_rgb_changes,actual_color_changes=actual_color_changes,
                    actual_color_error_increased_pixels=sum(c['error_increased'] for c in actual_color_changes),
                    actual_color_error_decreased_pixels=sum(c['error_decreased'] for c in actual_color_changes),
                    first_lost_points=[[i%w,i//w] for i in failures[:20]],
                    passed=not failures and not any(c['error_increased'] for c in actual_color_changes),
                    note='Independent truth/core reconstruction; two-pixel ambiguous boundary band excluded. Absolute reference acceptance remains separate.')
                preservation.append(check);row['standard_correct_core_preservation']=check
            for metric in ('objects','needle_blocks','punctures','planned_jump_commands','trim_requests','sewn_length_mm','decoded_jump_length_mm'):
                a=row['variants']['Standard_phase2']['metrics'][metric]
                b=row['variants']['Contextual_candidate']['metrics'][metric]
                row.setdefault('candidate_minus_standard',{})[metric]=b-a
            rows.append(row)
    absolute={variant:dict(Counter(row['variants'][variant]['metrics']['reference_status'] for row in rows)) for variant in variants}
    physical_groups=defaultdict(list)
    for row in rows:
        opts={k:v for k,v in row['frozen_options'].items() if k!='preparation'}
        physical_groups[(row['source_sha256'],q.value_hash(opts))].append(dict(name=row['name'],manifest_group=row['manifest_group'],nominal_dimensions_mm=row['nominal_dimensions_mm']))
    same_bytes=[dict(source_sha256=key[0],same_options_except_preparation_sha256=key[1],cases=value)
                for key,value in physical_groups.items() if len({tuple(v['nominal_dimensions_mm']) for v in value})>1]
    version_commands=[]
    for binary in ('rustc.exe','cargo.exe'):
        executable=Path('C:/Users/Admin/.cargo/bin')/binary
        result=subprocess.run([str(executable),'--version'],capture_output=True,text=True,encoding='utf-8',errors='replace',cwd=ROOT)
        version_commands.append(dict(command=[str(executable),'--version'],exit_code=result.returncode,stdout=result.stdout,stderr=result.stderr))
    overview=dict(cases=len(rows),synthetic_cases=len(preservation),absolute_reference=absolute,
        standard_correct_core_preservation=dict(cases=preservation,passed_cases=sum(p['passed'] for p in preservation),
            lost_correct_core_pixels=sum(p['lost_standard_correct_core_pixels'] for p in preservation),
            actual_thread_rgba_identity_cases=sum(p['actual_rgba_identity'] for p in preservation),
            actual_thread_rgba_changes_on_correct_foreground_cores=sum(p['actual_thread_rgba_changes_on_correct_foreground_cores'] for p in preservation),
            actual_color_error_increased_pixels=sum(p['actual_color_error_increased_pixels'] for p in preservation),
            actual_color_error_decreased_pixels=sum(p['actual_color_error_decreased_pixels'] for p in preservation)),
        same_byte_physical_size_groups=same_bytes,manifest_evidence=manifest_evidence,
        source_snapshot=evidence(OUT/'initial/state.json'),standard_reproduction=evidence(OUT/'standard-equivalence.json'),
        versions=dict(python=sys.version,pillow=pillow_version,independent_reader=baseline_ops.get('independent_reader'),toolchain_commands=version_commands,
            cli_hashes={v:sorted({r['variants'][v]['cli_sha256'] for r in rows}) for v in variants},
            cargo_manifest_sha256=sha(ROOT/'embroidery-engine/Cargo.toml'),cargo_lock_sha256=sha(ROOT/'embroidery-engine/Cargo.lock')),
        command_index=str(OUT/'executed-commands.json'),summary_command=[sys.executable,*sys.argv],
        timing_note='Recorded generation wall time and Windows PeakWorkingSetSize. VM had concurrent builds/validation; these runs are not a controlled speed comparison.',
        measurement_contract='Object counts are complexity, not quality. Old sampled top coverage is geometric, top-only, 0.30mm thread width, not fabric coverage. Logical transfer runs, sewn Travel, planned Jump, DST records and Trim requests are distinct.',
        artifacts_private_local=True)
    write(OUT/'comparison-table.json',dict(summary=overview,cases=rows))
    write(OUT/'quality-summary.json',overview)
    write(OUT/'executed-commands.json',dict(summary_command=overview['summary_command'],runs=commands,
        operation_summaries=[evidence(OUT/'operations/corpus-baseline/summary.json'),evidence(OUT/'operations/corpus-candidate/summary.json')]))
    csv_rows=[]
    for row in rows:
        flat={k:row[k] for k in ('name','manifest_group','corpus_role','source_sha256','source_format')}
        flat.update(source_width_px=row['source_dimensions'][0],source_height_px=row['source_dimensions'][1],
                    nominal_width_mm=row['nominal_dimensions_mm'][0],nominal_height_mm=row['nominal_dimensions_mm'][1])
        for variant in variants:
            flat.update({variant+'_'+key:value for key,value in row['variants'][variant]['metrics'].items()})
        csv_rows.append(flat)
    with (OUT/'comparison-table.csv').open('w',encoding='utf-8-sig',newline='') as stream:
        writer=csv.DictWriter(stream,fieldnames=list(csv_rows[0]));writer.writeheader();writer.writerows(csv_rows)
    h=html.escape
    def anchor(path,title):return '<a href="'+link(path)+'">'+h(title)+'</a>'
    def measured(value,suffix):return 'not recorded' if value is None else f'{value:.3g} {suffix}'
    headers='<tr><th>Case / group</th><th>Nominal mm / source px</th>'+''.join('<th>'+h(v)+'<br>objects / blocks / needles<br>planned Jump / Trim<br>absolute reference</th>' for v in variants)+'</tr>'
    table=[];cards=[]
    for number,row in enumerate(rows):
        name=row['name'];size=row['nominal_dimensions_mm'];pixels=row['source_dimensions']
        cells=['<td><a href="#case'+str(number)+'">'+h(name)+'</a><br>'+h(row['corpus_role'])+'</td>',
               '<td>'+f'{size[0]:g} × {size[1]:g} mm<br>{pixels[0]} × {pixels[1]} px'+'</td>']
        panels=[]
        for variant in variants:
            data=row['variants'][variant];m=data['metrics'];folder=Path(data['folder'])
            cells.append('<td>'+f'{m["objects"]} / {m["needle_blocks"]} / {m["punctures"]}<br>{m["planned_jump_commands"]} / {m["trim_requests"]}<br>'+h(m['reference_status'])+'</td>')
            region=next((folder/n for n in ('actual-thread-colors.png','retained-source-geometry.png','prepared-labels-after.png','regions.png') if (folder/n).is_file()),None)
            preview=next((folder/n for n in ('preview.png','design.svg') if (folder/n).is_file()),None)
            images=''.join('<a href="'+link(p)+'"><img loading="lazy" src="'+link(p)+'"></a>' for p in (region,preview) if p)
            panels.append('<section><h3>'+h(variant)+'</h3><p>'+f'Sewn {m["sewn_width_mm"]:.3g} × {m["sewn_height_mm"]:.3g} mm; colors {m["used_object_colors"]}; '+measured(m['seconds'],'s')+'; peak '+measured(m['peak_working_set_bytes'],'B')+'</p>'+images+'<p>'+anchor(folder/'evaluation.json','Full evaluation')+' · '+anchor(data['operation_evidence']['path'],'Operations / density')+' · '+anchor(folder/'command.json','Executed command')+'</p></section>')
        table.append('<tr>'+''.join(cells)+'</tr>')
        source_links=anchor(row['source'],'Source: '+row['source_format'])
        if row['truth']:source_links+=' · '+anchor(row['truth'],'Independent truth')
        cards.append('<article id="case'+str(number)+'"><h2>'+h(name)+'</h2><p>'+source_links+'</p><div class="panels">'+''.join(panels)+'</div></article>')
    counts='; '.join(v+': '+str(absolute[v]) for v in variants)
    navigation=' · '.join(anchor(OUT/path,title) for path,title in (
        ('comparison-table.csv','CSV'),('comparison-table.json','Full three-way JSON'),('quality-summary.json','Summary / versions'),
        ('executed-commands.json','Executed commands'),('reviews/index.html','11 REVIEW crops'),('reviews/real-crest/index.html','Private crest crops'),
        ('coverage/summary.json','Coverage calibration'),('coverage/calibration-coupons.png','Calibration coupons')))
    page='<!doctype html><meta charset="utf-8"><title>EmbroideryLab Phase 2.1 — local three-way comparison</title><style>body{font:15px/1.45 system-ui;margin:24px;background:#f3f3ef;color:#22303b}table{border-collapse:collapse;width:100%}th,td{border:1px solid #aaa;padding:8px;text-align:left}article{background:white;padding:18px;margin:26px 0}.panels{display:flex;gap:18px;flex-wrap:wrap}section{flex:1;min-width:260px}img{max-width:100%;max-height:560px;object-fit:contain}a{color:#135d8d}h3{overflow-wrap:anywhere}</style><h1>Phase 2.1: Legacy / reproduced Standard / Contextual candidate</h1><p>Private local report. Identical inputs, physical sizes and generator settings; preparation is the only comparison setting changed. No physical sew-out.</p><p>'+navigation+'</p><p>'+h(counts)+'</p><p>Independent preservation of Standard-correct foreground/background cores: '+str(overview['standard_correct_core_preservation']['passed_cases'])+'/'+str(len(preservation))+'; lost correct core pixels '+str(overview['standard_correct_core_preservation']['lost_correct_core_pixels'])+'. Absolute reference REVIEW remains separate.</p><p>'+h(overview['timing_note'])+'</p><p>'+h(overview['measurement_contract'])+'</p><table>'+headers+''.join(table)+'</table>'+''.join(cards)
    color_check=overview['standard_correct_core_preservation']
    color_note='<p>Actual RGBA identity is a separate diagnostic: '+str(color_check['actual_thread_rgba_identity_cases'])+'/'+str(len(preservation))+' cases. Changed Standard-correct core pixels: '+str(color_check['actual_thread_rgba_changes_on_correct_foreground_cores'])+'; actual OKLab error to independent truth decreased at '+str(color_check['actual_color_error_decreased_pixels'])+' and increased at '+str(color_check['actual_color_error_increased_pixels'])+' pixels. Full coordinates, immutable source samples and RGB errors are in the summary JSON; identity is not substituted for independent truth.</p>'
    page=page.replace('<table>',color_note+'<table>',1)
    (OUT/'index.html').write_text(page,encoding='utf-8')
    print(json.dumps(dict(cases=len(rows),absolute_reference=absolute,preserved_core_cases=overview['standard_correct_core_preservation']['passed_cases'],
        lost_correct_core_pixels=overview['standard_correct_core_preservation']['lost_correct_core_pixels'],
        actual_color_error_increased_pixels=color_check['actual_color_error_increased_pixels'],
        actual_color_error_decreased_pixels=color_check['actual_color_error_decreased_pixels'],index=str(OUT/'index.html')),ensure_ascii=False))

if __name__=='__main__':
    p=argparse.ArgumentParser();p.add_argument('action',choices=['freeze','run','equivalence','microfixtures','diagnostic','summary']);p.add_argument('--label');p.add_argument('--cli');p.add_argument('--profile',default='Standard')
    a=p.parse_args()
    if a.action=='freeze':freeze()
    elif a.action=='equivalence':equivalence()
    elif a.action=='microfixtures':microfixtures()
    elif a.action=='diagnostic':diagnostic(a.cli,a.label)
    elif a.action=='summary':summary()
    else:run(a.label,a.cli,a.profile)
