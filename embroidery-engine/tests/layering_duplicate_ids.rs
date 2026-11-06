use embroidery_lab::{Command, Object, Point, Project, Settings, engine};

fn rectangle(x: f64, y: f64, w: f64, h: f64) -> Vec<Point> {
    vec![
        Point::new(x, y),
        Point::new(x + w, y),
        Point::new(x + w, y + h),
        Point::new(x, y + h),
    ]
}

#[test]
fn duplicate_legacy_ids_do_not_apply_a_lower_cutout_to_its_upper_object() {
    let lower = Object {
        id: 1,
        name: "Lower".into(),
        contours: vec![rectangle(0., 0., 20., 20.)],
        color: [10, 20, 180],
        settings: Settings {
            underlay: false,
            tie_in: false,
            tie_off: false,
            ..Settings::default()
        },
        ..Object::default()
    };
    let upper = Object {
        id: 1,
        name: "Upper".into(),
        contours: vec![rectangle(5., 5., 10., 10.)],
        color: [200, 10, 20],
        ..lower.clone()
    };
    let project = Project {
        objects: vec![lower, upper],
        ..Project::default()
    };
    let original = serde_json::to_value(&project).unwrap();
    let plan = engine::generate_plan(&project).unwrap();
    let lower_color = project.objects[0].color;
    let upper_color = project.objects[1].color;
    assert!(
        plan.stitches
            .iter()
            .any(|s| s.command == Command::Stitch && s.color == lower_color)
    );
    assert!(
        plan.stitches
            .iter()
            .any(|s| s.command == Command::Stitch && s.color == upper_color)
    );
    let upper_start = plan
        .stitches
        .iter()
        .position(|s| s.command == Command::Stitch && s.color == upper_color)
        .unwrap();
    assert!(
        plan.stitches[upper_start..]
            .iter()
            .all(|s| s.command != Command::Stitch || s.color == upper_color)
    );
    assert_eq!(serde_json::to_value(&project).unwrap(), original);
}

#[test]
fn identical_shapes_with_duplicate_ids_omit_only_the_lower_layer() {
    let lower = Object {
        id: 1,
        contours: vec![rectangle(0., 0., 20., 20.)],
        color: [10, 20, 180],
        settings: Settings {
            underlay: false,
            tie_in: false,
            tie_off: false,
            ..Settings::default()
        },
        ..Object::default()
    };
    let upper = Object {
        color: [200, 10, 20],
        ..lower.clone()
    };
    let project = Project {
        objects: vec![lower, upper],
        ..Project::default()
    };
    let plan = engine::generate_plan(&project).unwrap();
    assert!(
        !plan
            .stitches
            .iter()
            .any(|s| s.command == Command::Stitch && s.color == project.objects[0].color)
    );
    assert!(
        plan.stitches
            .iter()
            .any(|s| s.command == Command::Stitch && s.color == project.objects[1].color)
    );
}
