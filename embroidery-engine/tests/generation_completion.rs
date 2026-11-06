use embroidery_lab::{
    Command, CommandMeta, MachinePlan, Object, Point, Project, Settings, Stitch, StitchKind,
    StitchRole, engine,
    quality::{self, QualityKind, QualityThresholds},
};

fn rectangle(spacing: f64) -> Project {
    Project {
        objects: vec![Object {
            id: 1,
            contours: vec![vec![
                Point::new(0., 0.),
                Point::new(30., 0.),
                Point::new(30., 20.),
                Point::new(0., 20.),
            ]],
            settings: Settings {
                spacing_mm: spacing,
                underlay: false,
                tie_in: false,
                tie_off: false,
                ..Default::default()
            },
            ..Default::default()
        }],
        ..Default::default()
    }
}

#[test]
fn sparse_exposed_edge_transfers_are_jumps_without_losing_top_fill() {
    let project = rectangle(2.);
    let before = serde_json::to_value(&project).unwrap();
    let plan = engine::generate_plan(&project).unwrap();
    assert!(
        !plan
            .stitches
            .iter()
            .zip(&plan.metadata)
            .any(|(s, m)| s.command == Command::Stitch && m.role == StitchRole::Travel)
    );
    assert!(
        plan.stitches
            .iter()
            .filter(|s| s.command == Command::Jump)
            .count()
            >= 10
    );
    let mut previous = Point::default();
    let mut top_length = 0.;
    for (s, m) in plan.stitches.iter().zip(&plan.metadata) {
        if s.command == Command::Stitch && m.role == StitchRole::TopFill {
            top_length += previous.distance(s.point());
        }
        if matches!(s.command, Command::Stitch | Command::Jump) {
            previous = s.point();
        }
    }
    assert!((top_length - 300.).abs() < 1e-7);
    assert_eq!(before, serde_json::to_value(&project).unwrap());
}

#[test]
fn dense_compensated_rectangle_keeps_continuous_rows() {
    let mut project = rectangle(0.4);
    project.objects[0].settings.pull_comp_mm = 0.2;
    let plan = engine::generate_plan(&project).unwrap();
    assert_eq!(
        plan.stitches
            .iter()
            .filter(|s| s.command == Command::Jump)
            .count(),
        1
    );
}

#[test]
fn repeated_top_across_passes_is_detected_but_explicit_running_repeats_are_not() {
    let mut project = rectangle(0.4);
    let plan = MachinePlan {
        stitches: vec![
            (1., Command::Jump),
            (3., Command::Stitch),
            (1., Command::Stitch),
            (3., Command::Stitch),
        ]
        .into_iter()
        .map(|(x, command)| Stitch {
            x,
            y: 1.,
            command,
            color: [0; 3],
            object_id: 1,
        })
        .collect(),
        metadata: vec![
            CommandMeta {
                role: StitchRole::Travel,
                pass_id: 0,
            },
            CommandMeta {
                role: StitchRole::TopFill,
                pass_id: 1,
            },
            CommandMeta {
                role: StitchRole::TopFill,
                pass_id: 2,
            },
            CommandMeta {
                role: StitchRole::Lock,
                pass_id: 3,
            },
        ],
        warnings: vec![],
    };
    assert_eq!(
        quality::analyze_plan(&plan, &project, QualityThresholds::default())
            .iter()
            .filter(|i| i.kind == QualityKind::RepeatedTopFill)
            .count(),
        1
    );
    project.objects[0].settings.kind = StitchKind::Running;
    assert!(
        !quality::analyze_plan(&plan, &project, QualityThresholds::default())
            .iter()
            .any(|i| i.kind == QualityKind::RepeatedTopFill)
    );
}

#[test]
fn quality_uses_visible_planned_domains_and_recognizes_intentional_full_occlusion() {
    let mut project = rectangle(0.4);
    let mut upper = project.objects[0].clone();
    upper.id = 2;
    upper.color = [210, 20, 30];
    upper.contours = vec![vec![
        Point::new(10., 5.),
        Point::new(20., 5.),
        Point::new(20., 15.),
        Point::new(10., 15.),
    ]];
    project.objects.push(upper);
    let plan = engine::generate_plan(&project).unwrap();
    let reports = quality::coverage_estimates(&plan, &project, 0.45);
    let lower = reports.iter().find(|r| r.object_id == 1).unwrap();
    assert!(lower.complete && lower.samples > 1000);
    assert!(
        lower
            .first_uncovered
            .is_none_or(|p| !(p.x > 10.2 && p.x < 19.8 && p.y > 5.2 && p.y < 14.8))
    );
    assert!(
        !quality::analyze_plan(&plan, &project, QualityThresholds::default())
            .iter()
            .any(|issue| matches!(
                issue.kind,
                QualityKind::OutsideGeometry | QualityKind::EmptyObject
            ))
    );
    project.objects[1].contours = project.objects[0].contours.clone();
    let plan = engine::generate_plan(&project).unwrap();
    assert!(
        !plan
            .stitches
            .iter()
            .any(|s| s.object_id == 1 && s.command == Command::Stitch)
    );
    let issues = quality::analyze_plan(&plan, &project, QualityThresholds::default());
    assert!(
        !issues
            .iter()
            .any(|i| i.object_id == 1 && i.kind == QualityKind::EmptyObject)
    );
    assert!(
        issues
            .iter()
            .any(|i| i.object_id == 1 && i.kind == QualityKind::LayerOcclusion)
    );
}

#[test]
fn export_thread_order_comes_from_sewn_blocks_even_with_hidden_fill_and_duplicate_ids() {
    let mut project = rectangle(0.4);
    project.objects[0].color = [15, 30, 45];
    let mut upper = project.objects[0].clone();
    // Legacy/manual IDs are not necessarily unique. RGB must disambiguate.
    upper.color = [220, 120, 20];
    project.objects.push(upper);
    let plan = engine::generate_plan(&project).unwrap();
    let sidecar = embroidery_lab::commands::thread_sidecar(&project, &plan.stitches);
    let blocks = sidecar["color_blocks"].as_array().unwrap();
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0]["rgb"], serde_json::json!([220, 120, 20]));
    assert_eq!(
        blocks[0]["needle_count"].as_u64().unwrap() as usize,
        plan.stitches
            .iter()
            .filter(|s| s.command == Command::Stitch)
            .count()
    );
    assert_eq!(sidecar["objects"][0]["sewn"], false);
    assert_eq!(sidecar["objects"][1]["sewn"], true);
    assert!(
        !plan
            .stitches
            .iter()
            .any(|s| s.command == Command::ColorChange)
    );
}

#[test]
fn thread_sidecar_retains_empty_control_blocks_and_uses_new_color_after_change() {
    let project = rectangle(0.4);
    let make = |command, color| Stitch {
        command,
        color,
        object_id: 1,
        x: 0.,
        y: 0.,
    };
    let stitches = vec![
        make(Command::ColorChange, [10; 3]),
        make(Command::Stitch, [20; 3]),
        make(Command::ColorChange, [20; 3]),
        make(Command::Stitch, [30; 3]),
        make(Command::End, [30; 3]),
    ];
    let sidecar = embroidery_lab::commands::thread_sidecar(&project, &stitches);
    let blocks = sidecar["color_blocks"].as_array().unwrap();
    assert_eq!(blocks.len(), 3);
    assert_eq!(blocks[0]["rgb"], serde_json::Value::Null);
    assert_eq!(blocks[0]["needle_count"], 0);
    assert_eq!(blocks[1]["rgb"], serde_json::json!([20, 20, 20]));
    assert_eq!(blocks[2]["rgb"], serde_json::json!([30, 30, 30]));
    assert!(
        embroidery_lab::commands::thread_sidecar(&project, &[])["color_blocks"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}
