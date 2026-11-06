use embroidery_lab::{
    Command, LayeringSettings, MachinePlan, Object, Point, Project, SatinColumn, Settings,
    StitchKind, StitchRole, engine,
};

fn rectangle(x: f64, y: f64, w: f64, h: f64) -> Vec<Point> {
    vec![
        Point::new(x, y),
        Point::new(x + w, y),
        Point::new(x + w, y + h),
        Point::new(x, y + h),
    ]
}

fn fill(id: u64, rings: Vec<Vec<Point>>) -> Object {
    Object {
        id,
        name: format!("Layer {id}"),
        contours: rings,
        color: [id as u8 * 40, 50, 150],
        settings: Settings {
            underlay: false,
            tie_in: false,
            tie_off: false,
            ..Settings::default()
        },
        ..Object::default()
    }
}

fn nested() -> Project {
    Project {
        objects: vec![
            fill(1, vec![rectangle(0., 0., 30., 20.)]),
            fill(2, vec![rectangle(10., 5., 10., 10.)]),
        ],
        ..Project::default()
    }
}

fn segments(plan: &MachinePlan, id: u64, role: Option<StitchRole>) -> Vec<(Point, Point)> {
    let mut previous = Point::default();
    let mut result = Vec::new();
    for (stitch, meta) in plan.stitches.iter().zip(&plan.metadata) {
        if stitch.command == Command::Stitch
            && stitch.object_id == id
            && role.is_none_or(|r| r == meta.role)
        {
            result.push((previous, stitch.point()));
        }
        if matches!(stitch.command, Command::Stitch | Command::Jump) {
            previous = stitch.point();
        }
    }
    result
}

fn avoids_rect(segments: &[(Point, Point)], bounds: (f64, f64, f64, f64)) {
    for &(a, b) in segments {
        for index in 0..=20 {
            let p = a.lerp(b, index as f64 / 20.);
            assert!(
                !(p.x > bounds.0 && p.x < bounds.2 && p.y > bounds.1 && p.y < bounds.3),
                "sewn segment {a:?}–{b:?} crosses removed interior at {p:?}"
            );
        }
    }
}

#[test]
fn lower_top_underlay_and_travel_omit_hidden_interior_without_editing_artwork() {
    let mut project = nested();
    project.objects[0].settings.underlay = true;
    let original = serde_json::to_value(&project).unwrap();
    let plan = engine::generate_plan(&project).unwrap();
    avoids_rect(&segments(&plan, 1, None), (10.3, 5.3, 19.7, 14.7));
    assert!(!segments(&plan, 1, Some(StitchRole::Underlay)).is_empty());
    assert_eq!(serde_json::to_value(&project).unwrap(), original);
    let mut disabled = project.clone();
    disabled.layering.enabled = false;
    let legacy = engine::generate_plan(&disabled).unwrap();
    let length = |plan: &MachinePlan| {
        segments(plan, 1, Some(StitchRole::TopFill))
            .iter()
            .map(|(a, b)| a.distance(*b))
            .sum::<f64>()
    };
    assert!(length(&plan) + 100. < length(&legacy));
}

#[test]
fn exact_cutout_and_registration_overlap_have_different_physical_boundaries() {
    let mut project = nested();
    let boundary = |plan: &MachinePlan| {
        segments(plan, 1, Some(StitchRole::TopFill))
            .iter()
            .flat_map(|(a, b)| [*a, *b])
            .filter(|p| p.x > 9. && p.x < 11. && p.y > 6. && p.y < 14.)
            .map(|p| p.x)
            .max_by(f64::total_cmp)
            .unwrap()
    };
    project.layering.overlap_mm = 0.;
    let exact = engine::generate_plan(&project).unwrap();
    project.layering.overlap_mm = 0.2;
    let overlap = engine::generate_plan(&project).unwrap();
    assert!((boundary(&exact) - 10.).abs() < 1e-8);
    assert!((boundary(&overlap) - 10.2).abs() < 1e-8);
}

#[test]
fn shared_edge_overlap_accounts_for_compensation_of_both_colors() {
    let mut project = Project {
        objects: vec![
            fill(1, vec![rectangle(0., 0., 10., 10.)]),
            fill(2, vec![rectangle(10., 0., 10., 10.)]),
        ],
        ..Project::default()
    };
    for object in &mut project.objects {
        object.settings.pull_comp_mm = 0.6;
    }
    let plan = engine::generate_plan(&project).unwrap();
    let boundary = |id: u64, maximum: bool| {
        let xs: Vec<_> = segments(&plan, id, Some(StitchRole::TopFill))
            .iter()
            .flat_map(|(a, b)| [*a, *b])
            .filter(|p| p.y > 1. && p.y < 9.)
            .map(|p| p.x)
            .collect();
        if maximum {
            xs.into_iter().max_by(f64::total_cmp).unwrap()
        } else {
            xs.into_iter().min_by(f64::total_cmp).unwrap()
        }
    };
    assert!((boundary(1, true) - 9.6).abs() < 1e-8);
    assert!((boundary(2, false) - 9.4).abs() < 1e-8);
    assert!((boundary(1, true) - boundary(2, false) - 0.2).abs() < 1e-8);
}

#[test]
fn upper_holes_preserve_lower_fill_and_lower_protected_holes_remain_empty() {
    let mut project = nested();
    project.objects[0].contours.push(rectangle(3., 7., 4., 6.));
    project.objects[1].contours.push(rectangle(12., 7., 6., 6.));
    let plan = engine::generate_plan(&project).unwrap();
    let lower = segments(&plan, 1, None);
    avoids_rect(&lower, (3.1, 7.1, 6.9, 12.9));
    assert!(
        segments(&plan, 1, Some(StitchRole::TopFill))
            .iter()
            .any(|(a, b)| {
                let p = a.lerp(*b, 0.5);
                p.x > 13. && p.x < 17. && p.y > 8. && p.y < 12.
            }),
        "upper hole must expose sewn lower fill"
    );
}

#[test]
fn exterior_and_overlapping_occluders_clip_only_their_real_interiors() {
    let mut project = nested();
    project.objects[1].contours = vec![rectangle(20., -5., 20., 30.)];
    project
        .objects
        .push(fill(3, vec![rectangle(12., 8., 15., 15.)]));
    let plan = engine::generate_plan(&project).unwrap();
    avoids_rect(&segments(&plan, 1, None), (20.3, 0.1, 29.9, 19.9));
    avoids_rect(&segments(&plan, 1, None), (12.3, 8.3, 26.7, 19.9));
}

#[test]
fn layer_order_visibility_and_running_paths_control_occlusion() {
    let mut project = nested();
    project.objects.reverse();
    let plan = engine::generate_plan(&project).unwrap();
    assert!(segments(&plan, 2, None).is_empty());
    assert!(
        plan.warnings
            .iter()
            .any(|w| w.object_id == 2 && w.message.starts_with("Layering:"))
    );
    assert!(
        !plan
            .warnings
            .iter()
            .any(|w| w.object_id == 2 && w.message.contains("no top"))
    );
    assert_eq!(
        plan.stitches
            .iter()
            .filter(|s| s.command == Command::ColorChange)
            .count(),
        0
    );

    let mut project = nested();
    project.objects[1].visible = false;
    let hidden = engine::generate_plan(&project).unwrap();
    project.objects[1].visible = true;
    project.objects[1].settings.kind = StitchKind::Running;
    project.objects[1].contours.clear();
    project.objects[1].running_paths = vec![vec![Point::new(10., 5.), Point::new(20., 15.)]];
    let running = engine::generate_plan(&project).unwrap();
    assert_eq!(
        segments(&hidden, 1, Some(StitchRole::TopFill)),
        segments(&running, 1, Some(StitchRole::TopFill))
    );
}

#[test]
fn identical_covered_shapes_do_not_retain_an_unnecessary_registration_collar() {
    let lower = fill(1, vec![rectangle(0., 0., 30., 20.)]);
    let upper = Object {
        id: 2,
        color: [180, 10, 20],
        ..lower.clone()
    };
    let project = Project {
        objects: vec![lower, upper],
        ..Project::default()
    };
    let plan = engine::generate_plan(&project).unwrap();
    assert!(segments(&plan, 1, None).is_empty());
    assert!(!segments(&plan, 2, Some(StitchRole::TopFill)).is_empty());
    assert!(
        plan.warnings
            .iter()
            .any(|w| w.object_id == 1 && w.message.starts_with("Layering:"))
    );
}

fn satin(id: u64, x: f64, y: f64, w: f64, h: f64) -> Object {
    Object {
        id,
        name: format!("Satin {id}"),
        satin_column: Some(SatinColumn {
            left: vec![Point::new(x, y), Point::new(x, y + h)],
            right: vec![Point::new(x + w, y), Point::new(x + w, y + h)],
            ..SatinColumn::default()
        }),
        contours: vec![],
        settings: Settings {
            kind: StitchKind::Satin,
            length_mm: 12.,
            ..fill(id, vec![]).settings
        },
        ..fill(id, vec![])
    }
}

#[test]
fn paired_satin_occludes_its_actual_strip_and_partial_lower_satin_is_explicit() {
    let mut project = nested();
    project.version = 4;
    project.objects[1] = satin(2, 12., 5., 4., 10.);
    let plan = engine::generate_plan(&project).unwrap();
    avoids_rect(&segments(&plan, 1, None), (12.3, 5.3, 15.7, 14.7));
    assert!(
        segments(&plan, 1, Some(StitchRole::TopFill))
            .iter()
            .any(|(a, b)| a.x.min(b.x) < 10. && a.x.max(b.x) > 10.)
    );

    project.objects = vec![
        satin(1, 8., 2., 4., 16.),
        fill(2, vec![rectangle(9., 7., 10., 10.)]),
    ];
    let error = engine::generate_plan(&project).unwrap_err();
    assert!(
        error.contains("partially covers") && error.contains("Tatami"),
        "{error}"
    );
    project.objects[1].contours = vec![rectangle(0., 0., 30., 30.)];
    let full = engine::generate_plan(&project).unwrap();
    assert!(segments(&full, 1, None).is_empty());
    assert!(
        full.warnings
            .iter()
            .any(|w| w.object_id == 1 && w.message.starts_with("Layering:"))
    );
}

#[test]
fn custom_policy_uses_schema_six_and_legacy_projects_keep_default_policy() {
    let mut project = nested();
    let legacy = serde_json::to_value(&project).unwrap();
    assert!(legacy.get("layering").is_none());
    project.layering = LayeringSettings {
        enabled: false,
        overlap_mm: 0.35,
    };
    let path =
        std::env::temp_dir().join(format!("embroidery-layering-{}.elab", std::process::id()));
    project.save(&path).unwrap();
    let opened = Project::load(&path).unwrap();
    assert_eq!(opened.version, 6);
    assert_eq!(opened.layering, project.layering);
    assert_eq!(
        serde_json::to_value(&opened.objects).unwrap(),
        serde_json::to_value(&project.objects).unwrap()
    );
    for version in 1..=5 {
        std::fs::write(
            &path,
            format!("{{\"version\":{version},\"objects\":[],\"width_mm\":40}}"),
        )
        .unwrap();
        assert_eq!(
            Project::load(&path).unwrap().layering,
            LayeringSettings::default()
        );
    }
    std::fs::remove_file(path).unwrap();
    project.layering.overlap_mm = f64::NAN;
    assert!(project.validate_limits().unwrap_err().contains("overlap"));
}

fn legacy_satin(length_mm: f64) -> Project {
    let mut object = satin(1, 0., 0., 4., 20.);
    object.settings.length_mm = length_mm;
    Project {
        version: 2,
        objects: vec![object],
        ..Project::default()
    }
}

#[test]
fn custom_layer_policy_keeps_exact_satin_plan_and_dst_after_reopening_schema_six() {
    let mut project = legacy_satin(12.);
    project.objects[0].settings.underlay = true;
    project.layering.enabled = false;
    assert_eq!(project.version, 2);
    assert_eq!(project.persisted_schema_version(), 6);
    let before = engine::generate_plan(&project).unwrap();
    let path = std::env::temp_dir().join(format!(
        "layering-satin-roundtrip-{}.elab",
        std::process::id()
    ));
    project.save(&path).unwrap();
    let reopened = Project::load(&path).unwrap();
    assert_eq!(
        project.version, 2,
        "saving must not mutate the editable RAM project"
    );
    assert_eq!(reopened.version, 6);
    let after = engine::generate_plan(&reopened).unwrap();
    assert_eq!(
        serde_json::to_value(&before).unwrap(),
        serde_json::to_value(&after).unwrap()
    );
    assert_eq!(
        embroidery_lab::dst::encode(&before.stitches, "roundtrip").unwrap(),
        embroidery_lab::dst::encode(&after.stitches, "roundtrip").unwrap()
    );
    std::fs::remove_file(path).unwrap();
}

#[test]
fn custom_schema_validates_unsplit_satin_before_save_with_the_same_reopen_error() {
    let mut project = legacy_satin(3.5);
    project.layering.enabled = false;
    let before = engine::generate_plan(&project).unwrap_err();
    assert!(before.contains("unsplit limit"), "{before}");
    let path = std::env::temp_dir().join(format!(
        "layering-satin-rejected-{}.elab",
        std::process::id()
    ));
    project.save(&path).unwrap();
    let reopened = Project::load(&path).unwrap();
    assert_eq!(reopened.version, 6);
    assert_eq!(before, engine::generate_plan(&reopened).unwrap_err());
    std::fs::remove_file(path).unwrap();
}

#[test]
fn unchanged_legacy_satin_retains_schema_two_and_its_original_subdivision_policy() {
    let project = legacy_satin(3.5);
    assert_eq!(project.persisted_schema_version(), 2);
    let before = engine::generate_plan(&project).unwrap();
    assert!(
        before
            .stitches
            .iter()
            .zip(&before.metadata)
            .any(|(s, m)| s.command == Command::Stitch
                && m.role == StitchRole::TopFill
                && s.x > 0.1
                && s.x < 3.9),
        "legacy physical subdivision must remain available to unchanged old projects"
    );
    let path =
        std::env::temp_dir().join(format!("layering-satin-legacy-{}.elab", std::process::id()));
    project.save(&path).unwrap();
    let reopened = Project::load(&path).unwrap();
    assert_eq!(reopened.version, 2);
    let after = engine::generate_plan(&reopened).unwrap();
    assert_eq!(
        serde_json::to_value(&before).unwrap(),
        serde_json::to_value(&after).unwrap()
    );
    assert_eq!(
        embroidery_lab::dst::encode(&before.stitches, "legacy").unwrap(),
        embroidery_lab::dst::encode(&after.stitches, "legacy").unwrap()
    );
    std::fs::remove_file(path).unwrap();
}
