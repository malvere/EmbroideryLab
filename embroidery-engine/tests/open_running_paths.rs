use embroidery_lab::{Command, Object, Point, Project, Settings, StitchKind, dsb, dst, engine};

fn project(paths: Vec<Vec<Point>>) -> Project {
    Project {
        objects: vec![Object {
            name: "Open centerlines".into(),
            running_paths: paths,
            settings: Settings {
                kind: StitchKind::Running,
                length_mm: 2.,
                underlay: false,
                tie_in: false,
                tie_off: false,
                ..Settings::default()
            },
            ..Object::default()
        }],
        ..Project::default()
    }
}
#[test]
fn sub_filter_bends_are_preserved_before_machine_quantization() {
    let path = vec![
        Point::new(0., 0.),
        Point::new(0.01, 0.),
        Point::new(0.01, 0.01),
        Point::new(0.2, 0.01),
    ];
    let generated = engine::generate(&project(vec![path.clone()])).unwrap();
    let points: Vec<_> = generated
        .iter()
        .filter(|s| s.command == Command::Stitch)
        .map(|s| s.point())
        .collect();
    assert_eq!(points, path[1..]);
}

#[test]
fn path_collapsed_on_export_grid_is_an_error_instead_of_an_empty_success() {
    let error = engine::generate(&project(vec![vec![
        Point::new(0., 0.),
        Point::new(0.01, 0.),
    ]]))
    .unwrap_err();
    assert!(error.contains("0.1mm export grid"));
}

fn l_shape() -> Vec<Point> {
    vec![
        Point::new(10., 10.),
        Point::new(20., 10.),
        Point::new(20., 20.),
    ]
}

#[test]
fn open_polyline_keeps_ends_open_and_stitches_only_its_segments() {
    let p = project(vec![l_shape()]);
    let stitches = engine::generate(&p).unwrap();
    let mut previous = Point::default();
    for s in &stitches {
        if s.command == Command::Stitch {
            assert!(
                ((previous.y - 10.).abs() < 1e-8 && (s.y - 10.).abs() < 1e-8)
                    || ((previous.x - 20.).abs() < 1e-8 && (s.x - 20.).abs() < 1e-8),
                "unexpected closing or shortcut segment: {previous:?} -> {s:?}"
            );
            assert!(previous.distance(s.point()) <= 1.850001);
        }
        previous = s.point();
    }
    assert_eq!(stitches.first().unwrap().point(), Point::new(10., 10.));
    assert_eq!(stitches.last().unwrap().point(), Point::new(20., 20.));
    assert_eq!(
        stitches
            .iter()
            .filter(|s| s.command == Command::Jump)
            .count(),
        1
    );
    assert!(p.objects[0].flattened_contours().is_empty());
}

#[test]
fn distinct_paths_always_jump_instead_of_joining_nearby_ends() {
    let p = project(vec![
        vec![Point::new(1., 1.), Point::new(3., 1.)],
        vec![Point::new(3., 1.2), Point::new(1., 1.2)],
    ]);
    let stitches = engine::generate(&p).unwrap();
    let jumps: Vec<_> = stitches
        .iter()
        .filter(|s| s.command == Command::Jump)
        .collect();
    assert_eq!(jumps.len(), 2);
    assert_eq!(jumps[1].point(), Point::new(3., 1.2));
    let mut previous = Point::default();
    for s in stitches {
        if s.command == Command::Stitch {
            assert!((previous.y - s.y).abs() < 1e-8);
        }
        previous = s.point();
    }
}

#[test]
fn explicit_repeat_closes_cycle_without_changing_open_path_semantics() {
    let mut path = l_shape();
    path.push(path[0]);
    let stitches = engine::generate(&project(vec![path])).unwrap();
    assert_eq!(stitches.last().unwrap().point(), Point::new(10., 10.));
    assert!(
        stitches
            .iter()
            .any(|s| s.command == Command::Stitch && s.x < 20. && s.y > 10.)
    );
}

#[test]
fn open_paths_survive_dst_and_dsb_export_without_added_closing_stitches() {
    let generated = engine::generate(&project(vec![l_shape()])).unwrap();
    let dst = dst::decode(&dst::encode(&generated, "Open paths").unwrap()).unwrap();
    let dsb = dsb::preview(
        &dsb::decode(&dsb::encode(&dsb::from_stitches(&generated).unwrap(), "Open paths").unwrap())
            .unwrap(),
    )
    .unwrap();
    for decoded in [&dst, &dsb] {
        let end = decoded
            .iter()
            .rfind(|s| s.command == Command::Stitch)
            .unwrap();
        assert!(end.point().distance(Point::new(20., 20.)) < 1e-7);
    }
}

#[test]
fn open_path_schema_three_roundtrip_and_legacy_default() {
    let path = std::env::temp_dir().join(format!("embroidery-open-v3-{}.elab", std::process::id()));
    let p = project(vec![l_shape()]);
    p.save(&path).unwrap();
    let saved: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(saved["version"], 3);
    let loaded = Project::load(&path).unwrap();
    assert_eq!(loaded.version, 3);
    assert_eq!(loaded.objects[0].running_paths, p.objects[0].running_paths);
    assert_eq!(
        engine::generate(&loaded).unwrap().len(),
        engine::generate(&p).unwrap().len()
    );
    let mut old = serde_json::to_value(Project::demo()).unwrap();
    for object in old["objects"].as_array_mut().unwrap() {
        object.as_object_mut().unwrap().remove("running_paths");
    }
    std::fs::write(&path, serde_json::to_vec(&old).unwrap()).unwrap();
    let legacy = Project::load(&path).unwrap();
    assert_eq!(legacy.version, 1);
    assert!(legacy.objects[0].running_paths.is_empty());
    assert!(engine::generate(&legacy).is_ok());
    std::fs::remove_file(path).unwrap();
}

#[test]
fn invalid_path_geometry_and_mixed_stitch_types_are_rejected() {
    for bad in [f64::NAN, f64::INFINITY, 9991.] {
        let p = project(vec![vec![Point::new(0., 0.), Point::new(bad, 1.)]]);
        assert!(p.validate_limits().unwrap_err().contains("coordinates"));
        assert!(engine::generate(&p).is_err());
    }
    for path in [vec![], vec![Point::default()], vec![Point::default(); 2]] {
        assert!(
            project(vec![path])
                .validate_limits()
                .unwrap_err()
                .contains("distinct")
        );
    }
    let mut p = project(vec![l_shape()]);
    for kind in [StitchKind::Tatami, StitchKind::Satin] {
        p.objects[0].settings.kind = kind;
        assert!(
            engine::generate(&p)
                .unwrap_err()
                .contains("require Running")
        );
    }
    p.objects[0].settings.kind = StitchKind::Running;
    p.objects[0].contours = vec![l_shape()];
    assert!(p.validate_limits().unwrap_err().contains("must not mix"));
}

#[test]
fn path_nodes_share_project_budget_with_contours_and_rails() {
    let mut p = project(vec![vec![Point::new(0., 0.), Point::new(1., 0.)]; 50_001]);
    assert!(p.validate_limits().unwrap_err().contains("100,000"));
    p.objects[0].running_paths = vec![l_shape()];
    p.objects[0].settings.length_mm = f64::NAN;
    assert!(engine::generate(&p).unwrap_err().contains("length"));
}
