use embroidery_lab::{
    Object, Point, Project, StitchKind,
    autodigitize::{self, AutoProfile},
    engine,
};
use std::sync::atomic::AtomicBool;

fn rectangle(length: f64, width: f64) -> Object {
    Object {
        contours: vec![vec![
            Point::new(0., 0.),
            Point::new(length, 0.),
            Point::new(length, width),
            Point::new(0., width),
        ]],
        ..Object::default()
    }
}
fn classify(object: Object) -> (Project, Vec<embroidery_lab::PlanWarning>) {
    let mut project = Project {
        objects: vec![object],
        ..Project::default()
    };
    let warnings = autodigitize::classify_project(&mut project, &AtomicBool::new(false));
    project.validate_limits().unwrap();
    (project, warnings)
}

#[test]
fn elongated_rectangle_builds_real_paired_rails_with_complete_original_extent() {
    let (project, warnings) = classify(rectangle(30., 3.));
    assert!(warnings.is_empty());
    let object = &project.objects[0];
    assert_eq!(object.settings.kind, StitchKind::Satin);
    let column = object.satin_column.as_ref().unwrap();
    assert_eq!(column.left.len(), 2);
    assert_eq!(column.right.len(), 2);
    assert!(object.contours.is_empty());
    let stations = engine::satin_stations(column, 0.4).unwrap();
    assert!(stations.iter().any(|(a, b)| a.x.min(b.x) == 0.));
    assert!(stations.iter().any(|(a, b)| a.x.max(b.x) == 30.));
    assert!(
        stations
            .iter()
            .all(|(a, b)| (a.distance(*b) - 3.).abs() < 1e-8)
    );
    engine::generate(&project).unwrap();
}

#[test]
fn thin_stroke_becomes_editable_running_and_broad_fill_stays_tatami() {
    let (thin, warnings) = classify(rectangle(20., 0.5));
    assert!(warnings.is_empty());
    assert_eq!(thin.objects[0].settings.kind, StitchKind::Running);
    assert!(thin.objects[0].centerline_graph.is_some());
    let path = &thin.objects[0].running_paths[0];
    assert_eq!(path.len(), 2);
    assert!(path.contains(&Point::new(0., 0.25)) && path.contains(&Point::new(20., 0.25)));
    let repeated = classify(rectangle(20., 0.5)).0;
    assert_eq!(
        thin.objects[0].running_paths,
        repeated.objects[0].running_paths
    );
    engine::generate(&thin).unwrap();
    let (broad, _) = classify(rectangle(20., 20.));
    assert_eq!(broad.objects[0].settings.kind, StitchKind::Tatami);
    assert!(broad.objects[0].satin_column.is_none());
}

#[test]
fn wide_column_and_tiny_detail_have_specific_diagnostics_without_disappearing() {
    let original = rectangle(40., 10.);
    let before = original.contours.clone();
    let (wide, warnings) = classify(original);
    assert_eq!(wide.objects[0].settings.kind, StitchKind::Tatami);
    assert_eq!(wide.objects[0].contours, before);
    assert!(
        warnings
            .iter()
            .any(|w| w.message.contains("exceeds automatic Satin maximum"))
    );
    let (tiny, warnings) = classify(rectangle(0.2, 0.2));
    assert!(!tiny.objects[0].contours.is_empty());
    assert!(warnings[0].message.contains("below profile minimum"));
}

#[test]
fn annular_border_uses_closed_source_rails_and_preserves_inner_opening() {
    let ring = |radius: f64| {
        (0..64)
            .map(|i| {
                let t = i as f64 * std::f64::consts::TAU / 64.;
                Point::new(radius * t.cos(), radius * t.sin())
            })
            .collect::<Vec<_>>()
    };
    let (project, warnings) = classify(Object {
        contours: vec![ring(10.), ring(8.)],
        ..Object::default()
    });
    assert!(warnings.is_empty(), "{warnings:?}");
    let column = project.objects[0].satin_column.as_ref().unwrap();
    assert!(column.closed);
    let stations = engine::satin_stations(column, 0.4).unwrap();
    assert!(
        stations
            .iter()
            .all(|(a, b)| a.lerp(*b, 0.5).distance(Point::default()) > 7.9)
    );
    engine::generate(&project).unwrap();
}

#[test]
fn branched_narrow_region_falls_back_without_losing_junction_coverage() {
    let contour = vec![
        Point::new(0., 0.),
        Point::new(2., 0.),
        Point::new(2., 8.),
        Point::new(8., 8.),
        Point::new(8., 10.),
        Point::new(-6., 10.),
        Point::new(-6., 8.),
        Point::new(0., 8.),
    ];
    let (project, warnings) = classify(Object {
        contours: vec![contour.clone()],
        ..Object::default()
    });
    assert_eq!(project.objects[0].settings.kind, StitchKind::Tatami);
    assert_eq!(project.objects[0].contours, vec![contour]);
    assert!(
        warnings
            .iter()
            .any(|w| w.message.contains("branches") || w.message.contains("rejected")),
        "{warnings:?}"
    );
}

#[test]
fn cancellation_and_existing_manual_geometry_do_not_get_partially_overwritten() {
    let mut project = Project {
        objects: vec![rectangle(20., 2.)],
        ..Project::default()
    };
    let before = serde_json::to_value(&project).unwrap();
    let warnings = autodigitize::classify_project(&mut project, &AtomicBool::new(true));
    assert_eq!(serde_json::to_value(&project).unwrap(), before);
    assert!(warnings[0].message.contains("cancelled"));
    let (mut project, _) = classify(rectangle(20., 2.));
    project.objects[0]
        .satin_column
        .as_mut()
        .unwrap()
        .first_right = true;
    let before = serde_json::to_value(&project).unwrap();
    autodigitize::classify_project(&mut project, &AtomicBool::new(false));
    assert_eq!(serde_json::to_value(&project).unwrap(), before);
}

#[test]
fn profile_threshold_changes_are_physical_and_classification_is_deterministic() {
    let mut project = Project {
        objects: vec![rectangle(20., 0.5)],
        ..Project::default()
    };
    let custom = AutoProfile {
        running_max_width_mm: 0.3,
        satin_min_width_mm: 0.4,
        ..AutoProfile::default()
    };
    autodigitize::classify_project_with_profile(&mut project, &AtomicBool::new(false), &custom);
    assert_eq!(project.objects[0].settings.kind, StitchKind::Satin);
    let a = classify(rectangle(30., 3.)).0;
    let b = classify(rectangle(30., 3.)).0;
    assert_eq!(
        serde_json::to_value(a).unwrap(),
        serde_json::to_value(b).unwrap()
    );
}

fn top_segment_lengths(plan: &embroidery_lab::MachinePlan) -> Vec<f64> {
    let mut previous = Point::default();
    let mut lengths = Vec::new();
    for (stitch, meta) in plan.stitches.iter().zip(&plan.metadata) {
        if stitch.command == embroidery_lab::Command::Stitch
            && meta.role == embroidery_lab::StitchRole::TopFill
        {
            lengths.push(previous.distance(stitch.point()));
        }
        previous = stitch.point();
    }
    lengths
}

#[test]
fn automatic_six_mm_satin_preserves_whole_diagonals_for_both_starting_rails() {
    for machine_cap in [12., 6.2] {
        let mut source = rectangle(30., 6.);
        source.settings.underlay = false;
        source.settings.tie_in = false;
        source.settings.tie_off = false;
        let mut project = Project {
            objects: vec![source],
            ..Project::default()
        };
        project.machine_profile.max_stitch_mm = machine_cap;
        let warnings = autodigitize::classify_project(&mut project, &AtomicBool::new(false));
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(project.objects[0].settings.kind, StitchKind::Satin);
        assert!(project.objects[0].settings.length_mm > 6.15);
        assert!(project.objects[0].settings.length_mm <= machine_cap);
        for first_right in [false, true] {
            project.objects[0]
                .satin_column
                .as_mut()
                .unwrap()
                .first_right = first_right;
            let column = project.objects[0].satin_column.as_ref().unwrap();
            let stations =
                engine::satin_stations(column, project.objects[0].settings.spacing_mm).unwrap();
            let plan = engine::generate_plan(&project).unwrap();
            let lengths = top_segment_lengths(&plan);
            assert_eq!(
                lengths.len(),
                stations.len() + 1,
                "generic subdivision added an interior puncture"
            );
            assert!(
                lengths
                    .iter()
                    .all(|&length| length >= 5.99 && length <= machine_cap)
            );
            assert!(
                plan.stitches
                    .iter()
                    .zip(&plan.metadata)
                    .filter(|(s, m)| s.command == embroidery_lab::Command::Stitch
                        && m.role == embroidery_lab::StitchRole::TopFill)
                    .all(|(s, _)| s.y == 0. || s.y == 6.)
            );
        }
    }
}

#[test]
fn automatic_curved_six_mm_closed_satin_keeps_each_diagonal_intact_through_the_seam() {
    let ring = |radius: f64| {
        (0..96)
            .map(|i| {
                let t = i as f64 * std::f64::consts::TAU / 96.;
                Point::new(radius * t.cos(), radius * t.sin())
            })
            .collect::<Vec<_>>()
    };
    let mut source = Object {
        contours: vec![ring(14.), ring(8.)],
        ..Object::default()
    };
    source.settings.underlay = false;
    source.settings.tie_in = false;
    source.settings.tie_off = false;
    let (mut project, warnings) = classify(source);
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(project.objects[0].settings.kind, StitchKind::Satin);
    assert!(project.objects[0].settings.length_mm > 6.15);
    for first_right in [false, true] {
        project.objects[0]
            .satin_column
            .as_mut()
            .unwrap()
            .first_right = first_right;
        let column = project.objects[0].satin_column.as_ref().unwrap();
        assert!(column.closed);
        let stations =
            engine::satin_stations(column, project.objects[0].settings.spacing_mm).unwrap();
        let plan = engine::generate_plan(&project).unwrap();
        let lengths = top_segment_lengths(&plan);
        assert_eq!(
            lengths.len(),
            stations.len() + 1,
            "curved diagonal or seam was subdivided"
        );
        assert!(
            lengths.iter().all(|&length| length > 4.5),
            "unexpected midpoint in an ordinary Satin diagonal"
        );
    }
}

#[test]
fn automatic_satin_uses_tatami_with_a_reason_when_machine_cannot_sew_intact_diagonals() {
    let source = rectangle(30., 6.);
    let contours = source.contours.clone();
    let mut project = Project {
        objects: vec![source],
        ..Project::default()
    };
    project.machine_profile.max_stitch_mm = 5.;
    let warnings = autodigitize::classify_project(&mut project, &AtomicBool::new(false));
    assert_eq!(project.objects[0].settings.kind, StitchKind::Tatami);
    assert!(project.objects[0].satin_column.is_none());
    assert_eq!(project.objects[0].contours, contours);
    assert_eq!(project.objects[0].settings.length_mm, 3.5);
    assert!(
        warnings
            .iter()
            .any(|w| w.message.contains("Required intact Satin diagonal")
                && w.message.contains("machine stitch limit 5.000 mm"))
    );
}

#[test]
fn automatic_running_rejects_collapsed_skeleton_branches_without_losing_filled_source() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../tests/fixtures/synthetic_two_colors.jpg");
    let mut project = embroidery_lab::import::load_image_with_options(
        &path,
        100.,
        &embroidery_lab::import::RasterOptions::autodigitize(),
    )
    .unwrap();
    project.machine_profile.max_stitch_mm = 8.;
    let source_contours: std::collections::BTreeMap<_, _> = project
        .objects
        .iter()
        .map(|object| (object.id, object.contours.clone()))
        .collect();
    let warnings = autodigitize::classify_project(&mut project, &AtomicBool::new(false));
    let collapsed: Vec<_> = warnings
        .iter()
        .filter(|warning| {
            warning.message.contains("Running branch") && warning.message.contains("collapses")
        })
        .collect();
    assert!(
        !collapsed.is_empty(),
        "The compressed-edge fixture must exercise the grid-collapse fallback"
    );
    for warning in collapsed {
        let retained = project
            .objects
            .iter()
            .find(|object| object.id == warning.object_id)
            .unwrap();
        assert_eq!(retained.settings.kind, StitchKind::Tatami);
        assert!(retained.running_paths.is_empty() && retained.centerline_graph.is_none());
        assert_eq!(retained.contours, source_contours[&retained.id]);
    }
    let plan = engine::generate_plan(&project).unwrap();
    assert!(
        plan.stitches
            .iter()
            .any(|stitch| stitch.command == embroidery_lab::Command::Stitch)
    );

    // Explicit manual Running remains a strict error: automatic fallback must
    // not silently delete a branch or weaken the engine's export guard.
    let mut manual = rectangle(5., 0.2);
    manual.contours.clear();
    manual.settings.kind = StitchKind::Running;
    manual.running_paths = vec![vec![Point::new(2.01, 0.1), Point::new(2.04, 0.1)]];
    let error = engine::generate_plan(&Project {
        objects: vec![manual],
        ..Project::default()
    })
    .unwrap_err();
    assert!(error.contains("running path collapses"), "{error}");
}
