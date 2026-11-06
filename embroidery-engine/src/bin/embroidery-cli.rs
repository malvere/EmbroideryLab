use embroidery_lab::{
    Command, Project, axis_edit, centerline_workflow, dsb, dst, engine, import, quality, satin_edit,
};
use std::path::{Path, PathBuf};

fn read(path: &Path) -> Result<Vec<u8>, String> {
    std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))
}
fn write(path: &Path, bytes: impl AsRef<[u8]>) -> Result<(), String> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(path, bytes).map_err(|e| e.to_string())
}
fn render(stitches: &[embroidery_lab::Stitch], path: &Path) -> Result<(), String> {
    let mut bounds = (0_f64, 0_f64, 1_f64, 1_f64);
    for s in stitches {
        bounds.0 = bounds.0.min(s.x);
        bounds.1 = bounds.1.min(s.y);
        bounds.2 = bounds.2.max(s.x);
        bounds.3 = bounds.3.max(s.y);
    }
    let mut svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"1000\" height=\"800\" viewBox=\"{} {} {} {}\"><rect x=\"{}\" y=\"{}\" width=\"100%\" height=\"100%\" fill=\"#f7f5ee\"/>",
        bounds.0 - 3.,
        bounds.1 - 3.,
        bounds.2 - bounds.0 + 6.,
        bounds.3 - bounds.1 + 6.,
        bounds.0 - 3.,
        bounds.1 - 3.
    );
    let mut current = (0., 0.);
    for s in stitches {
        if matches!(s.command, Command::Stitch | Command::Jump) {
            let color = if s.command == Command::Jump {
                "#bb8799".into()
            } else {
                format!("#{:02x}{:02x}{:02x}", s.color[0], s.color[1], s.color[2])
            };
            let dash = if s.command == Command::Jump {
                " stroke-dasharray=\"0.7 0.6\" opacity=\"0.5\""
            } else {
                ""
            };
            svg.push_str(&format!("<path d=\"M {:.3} {:.3} L {:.3} {:.3}\" fill=\"none\" stroke=\"{color}\" stroke-width=\"0.16\"{dash}/>",current.0,current.1,s.x,s.y));
        }
        current = (s.x, s.y);
    }
    svg.push_str("</svg>");
    write(path, svg)
}
fn output(project: &Project, path: &Path) -> Result<(), String> {
    let plan = engine::generate_plan(project)?;
    let stitches = &plan.stitches;
    let is_dsb = path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("dsb"));
    let (bytes, decoded) = if is_dsb {
        let records = dsb::from_stitches(stitches)?;
        let bytes = dsb::encode(&records, &project.name)?;
        let decoded = dsb::preview(&dsb::decode(&bytes)?)?;
        (bytes, decoded)
    } else {
        let bytes = dst::encode_with_trim(
            stitches,
            &project.name,
            project.machine_profile.dst_trim_encoding,
        )?;
        let decoded = dst::decode(&bytes)?;
        (bytes, decoded)
    };
    write(path, bytes)?;
    write(
        &path.with_extension("plan.json"),
        serde_json::to_vec_pretty(&plan).map_err(|e| e.to_string())?,
    )?;
    write(
        &path.with_extension("threads.json"),
        serde_json::to_vec_pretty(&embroidery_lab::commands::thread_sidecar(project, stitches))
            .map_err(|e| e.to_string())?,
    )?;
    project.save(&path.with_extension("elab"))?;
    write(
        &path.with_extension("stitches.json"),
        serde_json::to_vec_pretty(stitches).map_err(|e| e.to_string())?,
    )?;
    render(stitches, &path.with_extension("svg"))?;
    println!(
        "{}",
        serde_json::to_string_pretty(&dst::stats(&decoded)).map_err(|e| e.to_string())?
    );
    Ok(())
}
fn run() -> Result<(), String> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let usage = "Usage: embroidery-cli trace-report INPUT OUTPUT.json WIDTH_MM OPTIONS.json | autodigitize INPUT.png OUTPUT.dst [WIDTH_MM] [OPTIONS.json] | demo OUTPUT.dst | satin-demo OUTPUT.dst | satin-outline INPUT.elab OUTPUT.(dst|dsb) OBJECT_NUMBER WIDTH_MM [closed] | satin-from-branch INPUT.elab OUTPUT.(dst|dsb) OBJECT_NUMBER BRANCH_NUMBER | quality INPUT.(elab|dst|dsb) OUTPUT.json | generate INPUT.(jpg|jpeg|png|pdf|svg|elab) OUTPUT.(dst|dsb) [width_mm] | generate-with-options INPUT.(jpg|png) OUTPUT.(dst|dsb) WIDTH_MM OPTIONS.json | centerlines INPUT.elab OUTPUT.(dst|dsb) OBJECT_NUMBER [PIXEL_MM] | project INPUT OUTPUT.elab [width_mm] | inspect INPUT.dst | roundtrip INPUT.dst OUTPUT.dst | render INPUT.dst OUTPUT.svg | dsb-roundtrip INPUT.dsb OUTPUT.dsb | dsb-records INPUT.dsb OUTPUT.json | dsb-from-dst INPUT.dst OUTPUT.dsb";
    let command = args.first().ok_or(usage)?;
    let input = || {
        args.get(1)
            .map(PathBuf::from)
            .ok_or_else(|| usage.to_string())
    };
    let target = || {
        args.get(2)
            .map(PathBuf::from)
            .ok_or_else(|| usage.to_string())
    };
    let width = || {
        args.get(3)
            .map_or(Ok(100.), |s| s.parse::<f64>().map_err(|e| e.to_string()))
    };
    match command.as_str() {
        "trace-report" => {
            let options = args
                .get(4)
                .map(|p| {
                    read(Path::new(p)).and_then(|bytes| {
                        serde_json::from_slice::<import::RasterOptions>(&bytes)
                            .map_err(|e| e.to_string())
                    })
                })
                .transpose()?
                .unwrap_or_else(import::RasterOptions::autodigitize);
            let decoded = import::decode_raster_file(&input()?)?;
            let (_, report) =
                import::load_pixels_with_report(&decoded, &input()?, width()?, &options)?;
            write(
                &target()?,
                serde_json::to_vec_pretty(&report).map_err(|e| e.to_string())?,
            )
        }
        "satin-outline" => {
            let mut project = Project::load(&input()?)?;
            let index: usize = args
                .get(3)
                .ok_or(usage)?
                .parse::<usize>()
                .map_err(|_| "OBJECT_NUMBER must be a positive integer")?
                .checked_sub(1)
                .ok_or("First object is number 1")?;
            let width: f64 = args
                .get(4)
                .ok_or(usage)?
                .parse()
                .map_err(|_| "WIDTH_MM must be a number")?;
            let closed = match args.get(5).map(String::as_str) {
                None | Some("open") => false,
                Some("closed") => true,
                _ => return Err("Outline type must be open or closed".into()),
            };
            let source = project
                .objects
                .get(index)
                .ok_or("Object number is out of range")?;
            let paths = if source.running_paths.is_empty() {
                source.flattened_contours()
            } else {
                source.running_paths.clone()
            };
            if paths.len() != 1 {
                return Err("Select an object with exactly one path or contour".into());
            }
            let column = satin_edit::outline(&paths[0], closed, width)?;
            let id = project
                .objects
                .iter()
                .map(|o| o.id)
                .max()
                .unwrap_or(0)
                .checked_add(1)
                .ok_or("Object ID limit reached")?;
            project.objects.push(embroidery_lab::Object {
                id,
                name: format!("Satin outline {id}"),
                color: source.color,
                satin_column: Some(column),
                settings: embroidery_lab::Settings {
                    kind: embroidery_lab::StitchKind::Satin,
                    length_mm: 8.,
                    ..Default::default()
                },
                ..Default::default()
            });
            project.version = 4;
            output(&project, &target()?)
        }
        "satin-from-branch" => {
            let mut project = Project::load(&input()?)?;
            let index = args
                .get(3)
                .ok_or(usage)?
                .parse::<usize>()
                .map_err(|_| "OBJECT_NUMBER must be a positive integer")?
                .checked_sub(1)
                .ok_or("First object is number 1")?;
            let branch = args
                .get(4)
                .ok_or(usage)?
                .parse::<usize>()
                .map_err(|_| "BRANCH_NUMBER must be a positive integer")?
                .checked_sub(1)
                .ok_or("First branch is number 1")?;
            let source = project
                .objects
                .get(index)
                .ok_or("Object number is out of range")?
                .clone();
            let columns = axis_edit::semi_auto_satin(&source, branch)?;
            let mut id = project
                .objects
                .iter()
                .map(|o| o.id)
                .max()
                .unwrap_or(0)
                .checked_add(1)
                .ok_or("Object ID limit reached")?;
            project.objects[index].visible = false;
            if source.running_paths.len() > 1 {
                let mut remaining = source.clone();
                axis_edit::delete_branch(&mut remaining, branch)?;
                remaining.id = id;
                remaining.name.push_str(" · remaining");
                project.objects.push(remaining);
                id = id.checked_add(1).ok_or("Object ID limit reached")?;
            }
            for column in columns {
                project.objects.push(embroidery_lab::Object {
                    id,
                    name: format!("Satin column {id}"),
                    color: source.color,
                    satin_column: Some(column),
                    settings: embroidery_lab::Settings {
                        kind: embroidery_lab::StitchKind::Satin,
                        length_mm: 8.,
                        ..source.settings.clone()
                    },
                    ..Default::default()
                });
                id = id.checked_add(1).ok_or("Object ID limit reached")?;
            }
            project.version = 4;
            output(&project, &target()?)
        }
        "quality" => {
            let path = input()?;
            let extension = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            let project = if ["dst", "dsb"].contains(&extension.as_str()) {
                None
            } else {
                Some(Project::load(&path)?)
            };
            let plan = project.as_ref().map(engine::generate_plan).transpose()?;
            let stitches = match extension.as_str() {
                "dst" => dst::decode(&read(&path)?)?,
                "dsb" => dsb::preview(&dsb::decode(&read(&path)?)?)?,
                _ => plan.as_ref().unwrap().stitches.clone(),
            };
            let issues = if let (Some(plan), Some(project)) = (&plan, &project) {
                quality::analyze_plan(plan, project, Default::default())
            } else {
                quality::analyze(&stitches, Default::default())
            };
            let coverage = if let (Some(plan), Some(project)) = (&plan, &project) {
                quality::coverage_estimates(plan, project, 0.45)
            } else {
                vec![]
            };
            let report:Vec<_>=issues.iter().map(|i|serde_json::json!({"kind":format!("{:?}",i.kind),"object_id":i.object_id,"point":i.point,"command_index":i.index,"severity":"warning"})).collect();
            write(&target()?,serde_json::to_vec_pretty(&serde_json::json!({"short_stitch_mm":0.3,"long_jump_mm":10.,"density_grid_mm":1.,"max_punctures_per_cell":20,"issues":report,"coverage":coverage,"segment_density":plan.as_ref().map(|p|quality::segment_density(p,1.)),"plan_summary":plan.as_ref().map(quality::plan_summary),"warnings":plan.as_ref().map(|p|&p.warnings)})).map_err(|e|e.to_string())?)
        }
        "autodigitize" => {
            let options = args
                .get(4)
                .map(|p| {
                    read(Path::new(p)).and_then(|bytes| {
                        serde_json::from_slice::<import::RasterOptions>(&bytes)
                            .map_err(|e| e.to_string())
                    })
                })
                .transpose()?
                .unwrap_or_else(import::RasterOptions::autodigitize);
            let mut project = import::load_image_with_options(&input()?, width()?, &options)?;
            let _warnings = embroidery_lab::autodigitize::classify_project(
                &mut project,
                &std::sync::atomic::AtomicBool::new(false),
            );
            output(&project, &target()?)
        }

        "demo" => output(&Project::demo(), &input()?),
        "centerlines" => {
            let project = Project::load(&input()?)?;
            let number: usize = args
                .get(3)
                .ok_or(usage)?
                .parse()
                .map_err(|_| "OBJECT_NUMBER must be a positive integer (first object is 1)")?;
            let selected = number.checked_sub(1).ok_or("First object is number 1")?;
            let pixel_mm = args.get(4).map_or(Ok(0.2), |s| {
                s.parse::<f64>().map_err(|_| "PIXEL_MM must be a number")
            })?;
            let (candidate, report) =
                centerline_workflow::running_copy(&project, selected, pixel_mm)?;
            output(&candidate, &target()?)?;
            write(
                &target()?.with_extension("centerlines.json"),
                serde_json::to_vec_pretty(&report).map_err(|e| e.to_string())?,
            )?;
            eprintln!(
                "Created {} paths; {} isolated points skipped. Source object {} retained hidden; other objects retain their visibility.",
                report.paths.len(),
                report.isolated_points.len(),
                number
            );
            Ok(())
        }
        "generate" => {
            let p = import::load(&input()?, width()?)?;
            output(&p, &target()?)
        }
        "generate-with-options" => {
            let option_path = args.get(4).ok_or(usage)?;
            let bytes = read(Path::new(option_path))?;
            if bytes.len() > 1_000_000 {
                return Err("Import options exceed 1MB".into());
            }
            let options: import::RasterOptions =
                serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
            let p = import::load_image_with_options(&input()?, width()?, &options)?;
            output(&p, &target()?)?;
            write(
                &target()?.with_extension("import.json"),
                serde_json::to_vec_pretty(&serde_json::json!({
                    "input": input()?, "width_mm": width()?, "options": options,
                }))
                .map_err(|e| e.to_string())?,
            )
        }
        "satin-demo" => {
            use embroidery_lab::{Object, Point, SatinColumn, Settings, StitchKind};
            let p = Project {
                name: "Synthetic paired-rail satin".into(),
                objects: vec![Object {
                    name: "Curved column".into(),
                    satin_column: Some(SatinColumn {
                        left: vec![
                            Point::new(15., 10.),
                            Point::new(20., 25.),
                            Point::new(40., 40.),
                        ],
                        right: vec![
                            Point::new(22., 10.),
                            Point::new(27., 22.),
                            Point::new(44., 34.),
                        ],
                        ..SatinColumn::default()
                    }),
                    settings: Settings {
                        kind: StitchKind::Satin,
                        length_mm: 8.,
                        ..Settings::default()
                    },
                    ..Object::default()
                }],
                ..Project::default()
            };
            output(&p, &input()?)
        }
        "project" => {
            let p = import::load(&input()?, width()?)?;
            p.save(&target()?)
        }
        "inspect" => {
            if input()?
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("dsb"))
            {
                let records = dsb::decode(&read(&input()?)?)?;
                let mut counts = std::collections::BTreeMap::new();
                for record in &records {
                    *counts
                        .entry(format!("{:?}", record.command))
                        .or_insert(0_usize) += 1;
                }
                println!("{}",serde_json::to_string_pretty(&serde_json::json!({
                    "format":"DSB", "records":records.len(), "raw_commands":counts,
                    "preview_stats":dst::stats(&dsb::preview(&records)?),
                    "note":"Stop/needle/trim remain distinct in raw records; preview is for visualization only.",
                })).map_err(|e|e.to_string())?);
                return Ok(());
            }
            let s = dst::decode(&read(&input()?)?)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&dst::stats(&s)).map_err(|e| e.to_string())?
            );
            Ok(())
        }
        "roundtrip" => {
            let s = dst::decode(&read(&input()?)?)?;
            write(&target()?, dst::encode(&s, "Roundtrip")?)
        }
        "dsb-roundtrip" => {
            let records = dsb::decode(&read(&input()?)?)?;
            write(&target()?, dsb::encode(&records, "DSB roundtrip")?)
        }
        "dsb-records" => {
            let records = dsb::decode(&read(&input()?)?)?;
            write(
                &target()?,
                serde_json::to_vec_pretty(&records).map_err(|e| e.to_string())?,
            )
        }
        "dsb-from-dst" => {
            let records = dsb::from_stitches(&dst::decode(&read(&input()?)?)?)?;
            write(&target()?, dsb::encode(&records, "Manual color stops")?)
        }
        "render" => {
            let path = input()?;
            let bytes = read(&path)?;
            let s = if path
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("dsb"))
            {
                dsb::preview(&dsb::decode(&bytes)?)?
            } else {
                dst::decode(&bytes)?
            };
            render(&s, &target()?)
        }
        _ => Err(usage.into()),
    }
}
fn main() {
    if let Err(error) = run() {
        eprintln!("Error: {error}");
        std::process::exit(1);
    }
}
