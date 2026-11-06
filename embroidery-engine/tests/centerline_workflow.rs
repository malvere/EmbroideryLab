use embroidery_lab::{
    Command, Object, Point, Project, Settings, StitchKind, centerline_workflow, dsb, dst, engine,
};
use std::{path::PathBuf, process::Command as ProcessCommand};

fn rectangle(x: f64, y: f64, width: f64, height: f64) -> Vec<Point> {
    vec![
        Point::new(x, y),
        Point::new(x + width, y),
        Point::new(x + width, y + height),
        Point::new(x, y + height),
    ]
}
fn area() -> Object {
    Object {
        id: 17,
        name: "Distinct original bar".into(),
        contours: vec![rectangle(10., 20., 30., 4.)],
        color: [173, 52, 106],
        settings: Settings {
            length_mm: 2.,
            angle_deg: 37.,
            pull_comp_mm: 0.25,
            tie_in: false,
            tie_off: false,
            ..Settings::default()
        },
        ..Object::default()
    }
}
fn project() -> Project {
    Project {
        name: "Transaction fixture".into(),
        width_mm: 75.,
        objects: vec![area()],
        ..Project::default()
    }
}
fn bytes<T: serde::Serialize>(value: &T) -> Vec<u8> {
    serde_json::to_vec(value).unwrap()
}
fn error_preserves(project: &Project, selected: usize, pixel: f64, message: &str) {
    let original = bytes(project);
    let error = centerline_workflow::running_copy(project, selected, pixel).unwrap_err();
    assert!(
        error.contains(message),
        "expected {message:?}, got {error:?}"
    );
    assert_eq!(
        bytes(project),
        original,
        "failed extraction changed original"
    );
}

#[test]
fn copy_retains_original_geometry_order_settings_and_other_objects() {
    let mut p = project();
    let mut first = area();
    first.id = 5;
    first.name = "Before selected".into();
    first.visible = false;
    let mut last = area();
    last.id = 41;
    last.name = "After selected".into();
    last.contours = vec![rectangle(40., 30., 8., 2.)];
    p.objects.insert(0, first);
    p.objects.push(last);
    let original = bytes(&p);
    let (candidate, report) = centerline_workflow::running_copy(&p, 1, 0.25).unwrap();
    assert_eq!(bytes(&p), original);
    assert_eq!(candidate.version, 4);
    assert_eq!(candidate.name, p.name);
    assert_eq!(candidate.width_mm, p.width_mm);
    assert_eq!(candidate.objects.len(), 4);
    assert_eq!(bytes(&candidate.objects[0]), bytes(&p.objects[0]));
    assert_eq!(bytes(&candidate.objects[3]), bytes(&p.objects[2]));
    let mut expected_source = p.objects[1].clone();
    expected_source.visible = false;
    assert_eq!(bytes(&candidate.objects[1]), bytes(&expected_source));
    let copy = &candidate.objects[2];
    assert_eq!(copy.id, 42);
    assert!(copy.visible);
    assert_eq!(copy.color, p.objects[1].color);
    assert_eq!(copy.running_paths, report.paths);
    assert!(!copy.running_paths.is_empty());
    assert!(copy.contours.is_empty());
    assert!(copy.bezier_segments.is_empty());
    assert!(copy.satin_column.is_none());
    assert_eq!(copy.settings.kind, StitchKind::Running);
    assert_eq!(copy.settings.length_mm, p.objects[1].settings.length_mm);
    assert!(!copy.settings.underlay);
    assert_eq!(copy.settings.pull_comp_mm, 0.);
    assert!(!copy.settings.tie_in && !copy.settings.tie_off);
}

#[test]
fn extracted_open_centerline_exports_without_outline_or_endpoint_closure() {
    let p = project();
    let original = bytes(&p);
    let (candidate, report) = centerline_workflow::running_copy(&p, 0, 0.25).unwrap();
    assert_eq!(report.paths.len(), 1);
    let path = &report.paths[0];
    assert_eq!(candidate.objects[1].running_paths, report.paths);
    let editable = bytes(&candidate);
    assert!(path.first().unwrap().distance(*path.last().unwrap()) > 20.);
    let generated = engine::generate(&candidate).unwrap();
    assert!(
        generated
            .iter()
            .all(|s| s.object_id == candidate.objects[1].id)
    );
    assert_eq!(
        generated
            .iter()
            .filter(|s| s.command == Command::Jump)
            .count(),
        1
    );
    assert_eq!(bytes(&p), original);
    assert_eq!(
        bytes(&candidate),
        editable,
        "Generation must retain the precise editable centerline"
    );
    let quantize =
        |point: Point| Point::new((point.x * 10.).round() / 10., (point.y * 10.).round() / 10.);
    let grid_end = quantize(*path.last().unwrap());
    let grid_start = quantize(*path.first().unwrap());
    let grid_tolerance = std::f64::consts::SQRT_2 * 0.05 + 1e-10;
    assert_eq!(generated.first().unwrap().point(), grid_start);
    assert_eq!(generated.last().unwrap().point(), grid_end);
    assert!(grid_end.distance(*path.last().unwrap()) <= grid_tolerance);
    let encoded_dst = dst::encode(&generated, "Centerline").unwrap();
    let encoded_dsb = dsb::encode(&dsb::from_stitches(&generated).unwrap(), "Centerline").unwrap();
    for decoded in [
        generated.clone(),
        dst::decode(&encoded_dst).unwrap(),
        dsb::preview(&dsb::decode(&encoded_dsb).unwrap()).unwrap(),
    ] {
        assert!(
            decoded
                .last()
                .unwrap()
                .point()
                .distance(*path.last().unwrap())
                <= grid_tolerance
        );
        assert_eq!(decoded.last().unwrap().point(), grid_end);
        let sewn: Vec<_> = decoded
            .iter()
            .filter(|s| s.command == Command::Stitch)
            .collect();
        assert!(!sewn.is_empty());
        assert!(sewn.iter().all(|s| (s.y - 22.).abs() <= 0.31));
        assert!(
            sewn.last()
                .unwrap()
                .point()
                .distance(*path.first().unwrap())
                > 20.
        );
    }
}

#[test]
fn invalid_selection_resolution_and_already_running_leave_document_unchanged() {
    let p = project();
    error_preserves(&p, 1, 0.25, "Select an area");
    error_preserves(&p, usize::MAX, 0.25, "Select an area");
    for pixel in [0., -0.1, 0.001, 11., f64::NAN, f64::INFINITY] {
        error_preserves(&p, 0, pixel, "pixel size");
    }
    error_preserves(&p, 0, 0.005, "2048");
    let (candidate, _) = centerline_workflow::running_copy(&p, 0, 0.25).unwrap();
    error_preserves(&candidate, 1, 0.25, "already have centerlines");
}

#[test]
fn isolated_point_and_id_overflow_are_transactional_errors() {
    let mut p = project();
    p.objects[0].contours = vec![rectangle(0., 0., 0.25, 0.25)];
    error_preserves(&p, 0, 0.25, "only isolated points");
    p.objects[0].contours = vec![rectangle(0., 0., 0.04, 0.02)];
    error_preserves(&p, 0, 0.01, "0.1mm export grid");
    p = project();
    p.objects[0].id = u64::MAX;
    error_preserves(&p, 0, 0.25, "Object ID limit");
}

#[test]
fn candidate_over_object_or_node_budget_preserves_valid_original() {
    let mut p = project();
    p.objects.extend((1..4096).map(|id| Object {
        id,
        visible: false,
        ..Object::default()
    }));
    assert!(p.validate_limits().is_ok());
    error_preserves(&p, 0, 0.25, "4096 objects");
    p = project();
    p.objects.push(Object {
        id: 30,
        visible: false,
        running_paths: vec![
            (0..99_996)
                .map(|i| Point::new((i % 2) as f64, 0.))
                .collect(),
        ],
        settings: Settings {
            kind: StitchKind::Running,
            ..Settings::default()
        },
        ..Object::default()
    });
    assert!(p.validate_limits().is_ok());
    error_preserves(&p, 0, 0.25, "100,000");
}

struct TempDir(PathBuf);
impl TempDir {
    fn new() -> Self {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "embroidery-centerline-cli-{}-{nonce}",
            std::process::id()
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        // This directory is created by this test and contains only its artifacts.
        if let Ok(entries) = std::fs::read_dir(&self.0) {
            for entry in entries.flatten() {
                let _ = std::fs::remove_file(entry.path());
            }
        }
        let _ = std::fs::remove_dir(&self.0);
    }
}

#[test]
fn cli_writes_machine_files_editable_schema_four_and_geometry_report() {
    let temp = TempDir::new();
    let source = temp.0.join("source.elab");
    project().save(&source).unwrap();
    let original = std::fs::read(&source).unwrap();
    for extension in ["dst", "dsb"] {
        let target = temp.0.join(format!("centerline-{extension}.{extension}"));
        let output = ProcessCommand::new(env!("CARGO_BIN_EXE_embroidery-cli"))
            .arg("centerlines")
            .arg(&source)
            .arg(&target)
            .arg("1")
            .arg("0.25")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(std::fs::read(&source).unwrap(), original);
        let editable = Project::load(&target.with_extension("elab")).unwrap();
        assert_eq!(editable.version, 4);
        assert!(!editable.objects[0].visible);
        assert_eq!(editable.objects.len(), 2);
        let report: serde_json::Value = serde_json::from_slice(
            &std::fs::read(target.with_extension("centerlines.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(report["paths"].as_array().unwrap().len(), 1);
        assert!(report["isolated_points"].as_array().unwrap().is_empty());
        assert_eq!(report["pixel_size_mm"], 0.25);
        assert!(
            report["skeleton_pixels"].as_u64().unwrap()
                < report["foreground_pixels"].as_u64().unwrap()
        );
        assert!(target.with_extension("svg").is_file());
        assert!(target.with_extension("stitches.json").is_file());
        let machine_bytes = std::fs::read(&target).unwrap();
        let decoded = if extension == "dst" {
            dst::decode(&machine_bytes).unwrap()
        } else {
            dsb::preview(&dsb::decode(&machine_bytes).unwrap()).unwrap()
        };
        let path = &editable.objects[1].running_paths[0];
        assert_ne!(path.first(), path.last());
        assert!(
            decoded
                .last()
                .unwrap()
                .point()
                .distance(*path.last().unwrap())
                <= 0.071
        );
    }
    let target = temp.0.join("invalid.dst");
    let invalid = ProcessCommand::new(env!("CARGO_BIN_EXE_embroidery-cli"))
        .arg("centerlines")
        .arg(&source)
        .arg(&target)
        .arg("0")
        .output()
        .unwrap();
    assert!(!invalid.status.success());
    assert!(!target.exists());
    assert_eq!(std::fs::read(source).unwrap(), original);
}
