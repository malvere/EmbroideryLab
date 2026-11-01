//! Exercise the public CLI through editable v4 geometry and real machine files.
use embroidery_lab::{
    Command, Object, Point, Project, Settings, StitchKind, axis_edit, dsb, dst, engine,
};
use std::{
    path::{Path, PathBuf},
    process::Command as ProcessCommand,
};

fn p(x: f64, y: f64) -> Point {
    Point::new(x, y)
}
fn rect(y: f64) -> Vec<Point> {
    vec![p(0., y), p(30., y), p(30., y + 4.), p(0., y + 4.)]
}
struct TempDir(PathBuf);
impl TempDir {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "embroidery-cli-editor-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        if std::thread::panicking() {
            eprintln!(
                "CLI test artifacts retained for diagnosis: {}",
                self.0.display()
            );
            return;
        }
        // Only remove the flat test outputs in the exact directory this test created.
        if let Ok(entries) = std::fs::read_dir(&self.0) {
            for entry in entries.flatten() {
                let _ = std::fs::remove_file(entry.path());
            }
        }
        let _ = std::fs::remove_dir(&self.0);
    }
}
fn invoke(args: &[&std::ffi::OsStr]) -> std::process::Output {
    ProcessCommand::new(env!("CARGO_BIN_EXE_embroidery-cli"))
        .args(args)
        .output()
        .unwrap()
}
fn successful(output: std::process::Output) {
    assert!(
        output.status.success(),
        "CLI failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
fn quantized(point: Point) -> Point {
    p((point.x * 10.).round() / 10., (point.y * 10.).round() / 10.)
}

fn verify_outputs(path: &Path) -> Project {
    let project = Project::load(&path.with_extension("elab")).unwrap();
    assert_eq!(project.version, 4);
    let generated: Vec<embroidery_lab::Stitch> =
        serde_json::from_slice(&std::fs::read(path.with_extension("stitches.json")).unwrap())
            .unwrap();
    let regenerated = engine::generate(&project).unwrap();
    if generated.len() != regenerated.len() {
        for object in &project.objects {
            let saved_count = generated
                .iter()
                .filter(|s| s.object_id == object.id)
                .count();
            let reloaded_count = regenerated
                .iter()
                .filter(|s| s.object_id == object.id)
                .count();
            eprintln!(
                "object {} {:?}: saved {saved_count}, reloaded {reloaded_count}; satin {:?}",
                object.id,
                object.settings.kind,
                object.satin_column.as_ref().map(|column| (
                    column.left.len(),
                    column.right.len(),
                    column.rungs.len()
                ))
            );
        }
    }
    assert_eq!(generated.len(), regenerated.len());
    for (saved, regenerated) in generated.iter().zip(&regenerated) {
        assert_eq!(saved.command, regenerated.command);
        assert_eq!(saved.object_id, regenerated.object_id);
        assert_eq!(saved.color, regenerated.color);
        assert!(saved.point().distance(regenerated.point()) < 1e-9);
    }
    assert!(path.with_extension("svg").is_file());
    let bytes = std::fs::read(path).unwrap();
    let decoded = if path.extension().unwrap() == "dst" {
        dst::decode(&bytes).unwrap()
    } else {
        dsb::preview(&dsb::decode(&bytes).unwrap()).unwrap()
    };
    let actual: Vec<_> = decoded
        .iter()
        .filter(|s| s.command == Command::Stitch)
        .map(|s| s.point())
        .collect();
    let expected: Vec<_> = generated
        .iter()
        .filter(|s| s.command == Command::Stitch)
        .map(|s| quantized(s.point()))
        .collect();
    assert!(!expected.is_empty());
    assert_eq!(actual.len(), expected.len());
    for (actual, expected) in actual.iter().zip(&expected) {
        assert!(actual.distance(*expected) < 1e-7);
    }
    assert_eq!(
        decoded.iter().filter(|s| s.command == Command::End).count(),
        1
    );
    assert!(!decoded.iter().any(|s| s.command == Command::ColorChange));
    external_reader_if_available(path);
    project
}

/// No Python dependency is required for cargo test. CI/local QA can opt in using
/// EMBROIDERYLAB_TEST_PYTHON; the installed full uv 3.13 runtime is auto-detected.
fn external_reader_if_available(path: &Path) {
    let explicit = std::env::var_os("EMBROIDERYLAB_TEST_PYTHON");
    let python = explicit.clone().map(PathBuf::from).or_else(|| {
        let candidate = PathBuf::from(std::env::var_os("APPDATA")?)
            .join("uv/python/cpython-3.13.18-windows-x86_64-none/python.exe");
        candidate.is_file().then_some(candidate)
    });
    let Some(python) = python else {
        eprintln!(
            "Optional independent reader skipped: set EMBROIDERYLAB_TEST_PYTHON to a Python with pyembroidery"
        );
        return;
    };
    let script = r#"
import sys, json, math
try:
    import pyembroidery as pe
except ImportError:
    sys.exit(77)
pattern = pe.read(sys.argv[1])
source = json.load(open(sys.argv[2], encoding='utf-8'))
away = lambda x: math.floor(x*10+.5) if x >= 0 else math.ceil(x*10-.5)
expected = [(away(s['x']),away(s['y'])) for s in source if s['command']=='Stitch']
actual = [(x,y) for x,y,c in pattern.stitches if pe.decode_embroidery_command(c)[0]==pe.STITCH]
assert len(actual)==len(expected),(len(actual),len(expected))
assert all(math.dist(a,b)<1e-7 for a,b in zip(actual,expected))
flags = [pe.decode_embroidery_command(c)[0] for x,y,c in pattern.stitches]
assert flags.count(pe.END)==1,flags.count(pe.END)
assert not any(c in (pe.STOP,pe.COLOR_CHANGE,pe.NEEDLE_SET) for c in flags)
print('pyembroidery needle coordinates and controls match')
"#;
    let output = ProcessCommand::new(python)
        .arg("-c")
        .arg(script)
        .arg(path)
        .arg(path.with_extension("stitches.json"))
        .output()
        .unwrap();
    if output.status.code() == Some(77) && explicit.is_none() {
        eprintln!(
            "Optional independent reader skipped: pyembroidery is absent from detected Python"
        );
        return;
    }
    assert!(
        output.status.success(),
        "Independent reader failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn closed_outline_cli_preserves_hidden_source_hole_and_both_machine_formats() {
    let temp = TempDir::new();
    let source = temp.0.join("outline-source.elab");
    let project = Project {
        name: "Closed outline CLI".into(),
        objects: vec![Object {
            id: 7,
            visible: false,
            running_paths: vec![vec![
                p(10., 10.),
                p(30., 10.),
                p(30., 22.),
                p(10., 22.),
                p(10., 10.),
            ]],
            settings: Settings {
                kind: StitchKind::Running,
                ..Settings::default()
            },
            ..Object::default()
        }],
        ..Project::default()
    };
    project.save(&source).unwrap();
    let original = std::fs::read(&source).unwrap();
    for extension in ["dst", "dsb"] {
        let target = temp.0.join(format!("outline-{extension}.{extension}"));
        successful(invoke(&[
            "satin-outline".as_ref(),
            source.as_os_str(),
            target.as_os_str(),
            "1".as_ref(),
            "2".as_ref(),
            "closed".as_ref(),
        ]));
        assert_eq!(std::fs::read(&source).unwrap(), original);
        let result = verify_outputs(&target);
        assert_eq!(result.objects.len(), 2);
        assert!(!result.objects[0].visible);
        assert_eq!(
            result.objects[0].running_paths,
            project.objects[0].running_paths
        );
        let column = result.objects[1].satin_column.as_ref().unwrap();
        assert!(column.closed);
        assert_eq!(column.boundary_rings().unwrap().len(), 2);
        let points = engine::satin_stations(column, result.objects[1].settings.spacing_mm).unwrap();
        assert!(points.iter().all(|(a, b)| a.distance(*b) >= 0.15));
    }
}

#[test]
fn branch_cli_retains_hidden_original_and_other_island_without_duplicate_axis_stitches() {
    let temp = TempDir::new();
    let source = temp.0.join("branch-source.elab");
    let area = Object {
        id: 81,
        contours: vec![rect(0.), rect(8.)],
        ..Object::default()
    };
    let paths = vec![vec![p(2., 2.), p(28., 2.)], vec![p(2., 10.), p(28., 10.)]];
    let axes = Object {
        id: 82,
        name: "Two disconnected strokes".into(),
        centerline_graph: Some(axis_edit::attach(&paths, &area, 0.2).unwrap()),
        running_paths: paths.clone(),
        settings: Settings {
            kind: StitchKind::Running,
            underlay: false,
            tie_in: false,
            tie_off: false,
            ..Settings::default()
        },
        ..Object::default()
    };
    Project {
        version: 4,
        objects: vec![axes],
        ..Project::default()
    }
    .save(&source)
    .unwrap();
    let original = std::fs::read(&source).unwrap();
    for extension in ["dst", "dsb"] {
        let target = temp.0.join(format!("branch-{extension}.{extension}"));
        successful(invoke(&[
            "satin-from-branch".as_ref(),
            source.as_os_str(),
            target.as_os_str(),
            "1".as_ref(),
            "1".as_ref(),
        ]));
        assert_eq!(std::fs::read(&source).unwrap(), original);
        let result = verify_outputs(&target);
        assert_eq!(result.objects.len(), 3);
        assert!(!result.objects[0].visible);
        assert_eq!(result.objects[0].running_paths, paths);
        assert!(result.objects[1].visible);
        assert_eq!(result.objects[1].running_paths, vec![paths[1].clone()]);
        assert_eq!(result.objects[2].settings.kind, StitchKind::Satin);
        let satin = result.objects[2].satin_column.as_ref().unwrap();
        assert!(
            !satin.rungs.is_empty(),
            "real normal correspondences survive v4 serialization"
        );
        assert!(satin.left.iter().all(|p| (p.y - 4.).abs() < 1e-7));
        assert!(satin.right.iter().all(|p| p.y.abs() < 1e-7));
        assert!(
            engine::generate(&result)
                .unwrap()
                .iter()
                .all(|s| s.object_id != 82)
        );
    }
}

#[test]
fn ambiguous_branch_cli_and_invalid_outline_leave_existing_outputs_untouched() {
    let temp = TempDir::new();
    let source = temp.0.join("t-source.elab");
    let area = Object {
        contours: vec![vec![p(0., 0.), p(12., 0.), p(12., 10.), p(0., 10.)]],
        ..Object::default()
    };
    let paths = vec![
        vec![p(2., 5.), p(6., 5.)],
        vec![p(6., 5.), p(10., 5.)],
        vec![p(6., 5.), p(6., 8.)],
    ];
    let object = Object {
        centerline_graph: Some(axis_edit::attach(&paths, &area, 0.2).unwrap()),
        running_paths: paths,
        settings: Settings {
            kind: StitchKind::Running,
            ..Default::default()
        },
        ..Default::default()
    };
    Project {
        version: 4,
        objects: vec![object],
        ..Default::default()
    }
    .save(&source)
    .unwrap();
    let target = temp.0.join("existing.dst");
    std::fs::write(&target, b"existing machine file").unwrap();
    std::fs::write(target.with_extension("elab"), b"existing editable file").unwrap();
    let output = invoke(&[
        "satin-from-branch".as_ref(),
        source.as_os_str(),
        target.as_os_str(),
        "1".as_ref(),
        "1".as_ref(),
    ]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("junction"));
    let output = invoke(&[
        "satin-outline".as_ref(),
        source.as_os_str(),
        target.as_os_str(),
        "1".as_ref(),
        "-2".as_ref(),
    ]);
    assert!(!output.status.success());
    assert_eq!(std::fs::read(&target).unwrap(), b"existing machine file");
    assert_eq!(
        std::fs::read(target.with_extension("elab")).unwrap(),
        b"existing editable file"
    );
    assert!(!target.with_extension("stitches.json").exists());
}
