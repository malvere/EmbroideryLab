use embroidery_lab::{Command, Object, Point, Project, Settings, dst, engine};

fn rectangle(x: f64, y: f64, width: f64, height: f64) -> Vec<Point> {
    vec![
        Point::new(x, y),
        Point::new(x + width, y),
        Point::new(x + width, y + height),
        Point::new(x, y + height),
    ]
}
fn project(contours: Vec<Vec<Point>>, compensation: f64) -> Project {
    Project {
        objects: vec![Object {
            contours,
            settings: Settings {
                underlay: false,
                tie_in: false,
                tie_off: false,
                pull_comp_mm: compensation,
                ..Settings::default()
            },
            ..Object::default()
        }],
        ..Project::default()
    }
}

#[test]
fn compensation_does_not_create_jump_on_each_rectangle_row() {
    let uncompensated = engine::generate(&project(vec![rectangle(0., 0., 30., 20.)], 0.)).unwrap();
    let compensated = engine::generate(&project(vec![rectangle(0., 0., 30., 20.)], 0.2)).unwrap();
    assert_eq!(dst::stats(&uncompensated).jumps, 1);
    assert_eq!(dst::stats(&compensated).jumps, 1);
}

#[test]
fn compensation_rejects_new_contacts_across_original_component_gaps() {
    for separate_objects in [false, true] {
        let fixture = |gap: f64, compensation: f64| {
            let mut p = project(
                vec![
                    rectangle(0., 0., 10., 10.),
                    rectangle(10. + gap, 0., 10., 10.),
                ],
                compensation,
            );
            if separate_objects {
                let mut second = p.objects[0].clone();
                second.id = 2;
                second.name = "Right island".into();
                second.color = [200, 20, 10];
                second.contours = vec![p.objects[0].contours.pop().unwrap()];
                p.objects.push(second);
            }
            p
        };
        assert!(engine::generate_plan(&fixture(0.1, 0.)).is_ok());
        assert!(engine::generate_plan(&fixture(1., 0.2)).is_ok());
        for gap in [0.1, 0.4] {
            let p = fixture(gap, 0.2);
            if !separate_objects {
                assert!(engine::fill_spans(&p.objects[0].contours, 0., 0.4, 0.2).is_err());
            }
            let original = p
                .objects
                .iter()
                .map(|o| o.contours.clone())
                .collect::<Vec<_>>();
            let error = engine::generate_plan(&p).expect_err("compensation must preserve the gap");
            assert!(
                error.contains("compensation") && error.contains("gap"),
                "{error}"
            );
            assert!(
                error.contains("Reduce") && error.contains("widen"),
                "{error}"
            );
            assert_eq!(
                p.objects
                    .iter()
                    .map(|o| o.contours.clone())
                    .collect::<Vec<_>>(),
                original
            );
        }
        // Originally touching edges and deliberate overlap retain the layer
        // policy. Only a newly closed original gap is rejected by this guard.
        for gap in [0., -1.] {
            assert!(
                engine::generate_plan(&fixture(gap, 0.2)).is_ok(),
                "gap {gap}"
            );
        }
        // The actual directional geometry, rather than expanded bounding boxes,
        // decides contact. Vertical compensation leaves this horizontal gap.
        let mut vertical = fixture(0.1, 0.2);
        for object in &mut vertical.objects {
            object.settings.angle_deg = 90.;
        }
        assert!(engine::generate_plan(&vertical).is_ok());
    }
}

#[test]
fn compensation_checks_one_sided_cross_object_expansion_in_either_order() {
    for reverse in [false, true] {
        let mut p = project(vec![rectangle(0., 0., 10., 10.)], 0.2);
        let mut second = p.objects[0].clone();
        second.id = 2;
        second.contours = vec![rectangle(10.1, 0., 10., 10.)];
        second.settings.pull_comp_mm = 0.;
        p.objects.push(second);
        if reverse {
            p.objects.reverse();
        }
        assert!(engine::generate_plan(&p).unwrap_err().contains("gap"));
    }
}

#[test]
fn compensation_preserves_gaps_around_nested_islands_using_filled_geometry() {
    let mut p = project(
        vec![
            rectangle(0., 0., 10., 10.),
            rectangle(3., 3., 4., 4.),
            rectangle(3.5, 3.5, 3., 3.),
        ],
        0.2,
    );
    assert!(engine::generate_plan(&p).is_ok());
    p.objects[0].contours[2] = rectangle(3.1, 3.1, 3.8, 3.8);
    assert!(engine::generate_plan(&p).unwrap_err().contains("gap"));
}

#[test]
fn compensation_gap_guard_uses_satin_rail_normals_and_protects_neighboring_fills() {
    use embroidery_lab::{SatinColumn, StitchKind};
    for reverse in [false, true] {
        for satin_compensation in [0., 0.2] {
            let mut p = project(vec![rectangle(0., 4.1, 10., 4.)], 0.);
            p.objects[0].id = 2;
            p.objects[0].name = "Neighbor".into();
            p.objects[0].settings.angle_deg = 90.;
            if satin_compensation == 0. {
                p.objects[0].settings.pull_comp_mm = 0.2;
            }
            let satin = Object {
                id: 1,
                name: "Satin".into(),
                satin_column: Some(SatinColumn {
                    left: vec![Point::new(0., 0.), Point::new(10., 0.)],
                    right: vec![Point::new(0., 4.), Point::new(10., 4.)],
                    ..SatinColumn::default()
                }),
                settings: Settings {
                    kind: StitchKind::Satin,
                    length_mm: 5.,
                    underlay: false,
                    tie_in: false,
                    tie_off: false,
                    pull_comp_mm: satin_compensation,
                    ..Settings::default()
                },
                ..Object::default()
            };
            p.version = 5;
            p.objects.push(satin);
            if reverse {
                p.objects.reverse();
            }
            let error = engine::generate_plan(&p).unwrap_err();
            assert!(error.contains("gap"), "{error}");
            let neighbor = p.objects.iter_mut().find(|o| o.id == 2).unwrap();
            neighbor.contours = vec![rectangle(0., 5., 10., 4.)];
            assert!(engine::generate_plan(&p).is_ok());
        }
    }
}

#[test]
fn every_independent_shell_is_compensated_without_touching_holes() {
    let rings = vec![
        rectangle(0., 0., 12., 10.),
        rectangle(3., 3., 4., 4.),
        rectangle(20., 0., 12., 10.),
    ];
    let spans = engine::fill_spans(&rings, 0., 0.4, 0.2).unwrap();
    let points: Vec<_> = spans.iter().flat_map(|(a, b)| [a, b]).collect();
    for x in [-0.2, 12.2, 19.8, 32.2] {
        assert!(
            points.iter().any(|p| (p.x - x).abs() < 1e-6),
            "missing compensation at {x}"
        );
    }
    assert!(points.iter().any(|p| (p.x - 3.).abs() < 1e-6));
    assert!(points.iter().any(|p| (p.x - 7.).abs() < 1e-6));
    let stitches = engine::generate(&project(rings, 0.2)).unwrap();
    let mut previous = Point::default();
    for stitch in stitches {
        if stitch.command == Command::Stitch {
            for i in 0..=100 {
                let p = previous.lerp(stitch.point(), i as f64 / 100.);
                assert!(
                    !(p.x > 3.000001 && p.x < 6.999999 && p.y > 3.000001 && p.y < 6.999999),
                    "hole sewing at {p:?}"
                );
                assert!(p.x <= 12.200001 || p.x >= 19.799999, "island bridge {p:?}");
            }
        }
        previous = stitch.point();
    }
}

#[test]
fn compensated_rotated_holes_remain_protected_after_dst_quantization() {
    for angle in [0_f64, 27., 90., 137.] {
        let rotate = |p: Point, angle: f64| {
            let (sin, cos) = angle.to_radians().sin_cos();
            Point::new(p.x * cos - p.y * sin, p.x * sin + p.y * cos)
        };
        let rings = vec![rectangle(0., 0., 30., 20.), rectangle(10., 5., 10., 10.)]
            .into_iter()
            .map(|ring| ring.into_iter().map(|p| rotate(p, angle)).collect())
            .collect();
        let mut p = project(rings, 0.2);
        p.objects[0].settings.angle_deg = angle;
        let generated = engine::generate(&p).unwrap();
        let decoded = dst::decode(&dst::encode(&generated, "compensation").unwrap()).unwrap();
        let mut previous = Point::default();
        for stitch in decoded {
            if stitch.command == Command::Stitch {
                let steps = (previous.distance(stitch.point()) / 0.025).ceil().max(1.) as usize;
                for i in 0..=steps {
                    let q = rotate(
                        previous.lerp(stitch.point(), i as f64 / steps as f64),
                        -angle,
                    );
                    // Max rounding displacement is sqrt(2)*0.05mm; 0.075mm
                    // adds numerical slack without depending on render width.
                    assert!(
                        !(q.x > 10.075 && q.x < 19.925 && q.y > 5.075 && q.y < 14.925),
                        "angle {angle}: {q:?}"
                    );
                }
            }
            previous = stitch.point();
        }
    }
}

#[test]
fn tangent_hole_cycles_stay_valid_before_and_after_compensation() {
    let rings = vec![
        rectangle(0., 0., 10., 10.),
        rectangle(2., 2., 3., 3.),
        rectangle(5., 5., 3., 3.),
    ];
    for compensation in [0., 0.2] {
        let p = project(rings.clone(), compensation);
        let plan = engine::generate_plan(&p).unwrap();
        let issues = embroidery_lab::quality::analyze_plan(
            &plan,
            &p,
            embroidery_lab::quality::QualityThresholds::default(),
        );
        assert!(
            !issues
                .iter()
                .any(|issue| issue.kind == embroidery_lab::quality::QualityKind::OutsideGeometry)
        );
        assert!(dst::stats(&plan.stitches).stitches > 0);
    }
}

#[test]
fn auto_angle_is_invariant_to_collinear_boundary_resampling() {
    let ring = rectangle(0., 0., 30., 10.);
    let mut resampled = vec![ring[0]];
    for i in 1..=100 {
        resampled.push(ring[0].lerp(ring[1], i as f64 / 250.));
    }
    resampled.push(ring[1]);
    resampled.extend_from_slice(&ring[2..]);
    assert!(
        (engine::automatic_angle(&[ring]) - engine::automatic_angle(&[resampled])).abs() < 1e-8
    );
}

#[test]
fn thin_regions_do_not_silently_produce_no_top_fill() {
    let p = project(vec![rectangle(0., 0., 4., 0.12)], 0.);
    let stitches = engine::generate(&p).unwrap();
    assert!(dst::stats(&stitches).stitches > 0);
}

#[test]
fn tatami_underlay_parameters_are_independent_and_observable() {
    use embroidery_lab::StitchRole;
    let mut p = project(
        vec![rectangle(0., 0., 30., 20.), rectangle(10., 6., 8., 8.)],
        0.,
    );
    p.objects[0].settings.underlay = true;
    let base = engine::generate_plan(&p).unwrap();
    let count = |plan: &embroidery_lab::MachinePlan| {
        plan.stitches
            .iter()
            .zip(&plan.metadata)
            .filter(|(s, m)| s.command == Command::Stitch && m.role == StitchRole::Underlay)
            .count()
    };
    assert!(count(&base) > 0);
    assert_eq!(base.stitches.len(), base.metadata.len());
    p.objects[0].settings.underlay_spacing_mm = 4.;
    let sparse = engine::generate_plan(&p).unwrap();
    assert!(count(&sparse) < count(&base));
    p.objects[0].settings.underlay_spacing_mm = 2.;
    p.objects[0].settings.underlay_length_mm = 1.5;
    let short = engine::generate_plan(&p).unwrap();
    assert!(count(&short) > count(&base));
    let mut previous = Point::default();
    for (s, m) in short.stitches.iter().zip(&short.metadata) {
        if s.command == Command::Stitch && m.role == StitchRole::Underlay {
            assert!(previous.distance(s.point()) <= 1.5 + 1e-8);
            assert!(
                s.x >= 0.3 - 1e-7 && s.x <= 29.7 + 1e-7 && s.y >= 0.3 - 1e-7 && s.y <= 19.7 + 1e-7
            );
        }
        if matches!(s.command, Command::Stitch | Command::Jump) {
            previous = s.point();
        }
    }
    p.objects[0].settings.underlay_length_mm = 3.;
    p.objects[0].settings.underlay_inset_mm = 1.5;
    let inset = engine::generate_plan(&p).unwrap();
    assert_ne!(
        serde_json::to_value(&base.stitches).unwrap(),
        serde_json::to_value(&inset.stitches).unwrap()
    );
    assert!(
        inset
            .stitches
            .iter()
            .zip(&inset.metadata)
            .filter(|(s, m)| s.command == Command::Stitch && m.role == StitchRole::Underlay)
            .all(|(s, _)| s.x >= 1.5 - 1e-7
                && s.x <= 28.5 + 1e-7
                && s.y >= 1.5 - 1e-7
                && s.y <= 18.5 + 1e-7)
    );
}

#[test]
fn tatami_underlay_direction_is_independent_and_respects_protected_holes() {
    use embroidery_lab::{MachinePlan, StitchRole};
    use std::collections::BTreeMap;

    let mut p = project(
        vec![rectangle(0., 0., 32., 22.), rectangle(12., 7., 8., 8.)],
        0.2,
    );
    p.objects[0].settings.underlay = true;
    let contours = p.objects[0].contours.clone();
    let settings = serde_json::to_value(&p.objects[0].settings).unwrap();
    assert!(settings.get("underlay_angle_deg").is_none());
    let base = engine::generate_plan(&p).unwrap();
    p.objects[0].settings.underlay_angle_deg = Some(90.);
    let explicit_legacy = engine::generate_plan(&p).unwrap();
    assert_eq!(
        serde_json::to_value(&base).unwrap(),
        serde_json::to_value(&explicit_legacy).unwrap(),
        "an omitted angle retains the legacy top + 90 degree path"
    );
    // Compare actual sewn row coverage, independent of travel order or needle
    // subdivision. Changing an underlay route may choose another top entry.
    let top_rows = |plan: &MachinePlan| {
        let grid = |n: f64| (n * 10.).round() as i64;
        let mut rows: BTreeMap<i64, Vec<(i64, i64)>> = BTreeMap::new();
        let mut previous = Point::default();
        for (s, m) in plan.stitches.iter().zip(&plan.metadata) {
            if s.command == Command::Stitch && m.role == StitchRole::TopFill {
                assert_eq!(grid(previous.y), grid(s.y));
                let (x0, x1) = (grid(previous.x), grid(s.x));
                rows.entry(grid(s.y))
                    .or_default()
                    .push((x0.min(x1), x0.max(x1)));
            }
            previous = s.point();
        }
        for spans in rows.values_mut() {
            spans.sort_unstable();
            let mut merged: Vec<(i64, i64)> = Vec::new();
            for &(a, b) in spans.iter() {
                if let Some(last) = merged.last_mut()
                    && a <= last.1
                {
                    last.1 = last.1.max(b);
                } else {
                    merged.push((a, b));
                }
            }
            *spans = merged;
        }
        rows
    };
    let original_top = top_rows(&base);
    for direction in [31_f64, -38.] {
        p.objects[0].settings.underlay_angle_deg = Some(direction);
        let rotated = engine::generate_plan(&p).unwrap();
        assert_eq!(top_rows(&rotated), original_top);
        assert_eq!(p.objects[0].contours, contours);
        let mut changed_settings = serde_json::to_value(&p.objects[0].settings).unwrap();
        changed_settings
            .as_object_mut()
            .unwrap()
            .remove("underlay_angle_deg");
        assert_eq!(changed_settings, settings);
        let (sin, cos) = direction.to_radians().sin_cos();
        let mut previous = Point::default();
        let mut directed_segments = 0;
        for (s, m) in rotated.stitches.iter().zip(&rotated.metadata) {
            if s.command == Command::Stitch && m.role == StitchRole::Underlay {
                let length = previous.distance(s.point());
                assert!(length <= 3. + 1e-8);
                if length > 1. {
                    let perpendicular = -(s.x - previous.x) * sin + (s.y - previous.y) * cos;
                    assert!(perpendicular.abs() <= 0.15, "{direction}: {perpendicular}");
                    directed_segments += 1;
                }
                let steps = (length / 0.025).ceil().max(1.) as usize;
                for step in 0..=steps {
                    let q = previous.lerp(s.point(), step as f64 / steps as f64);
                    assert!(q.x >= 0.229 && q.x <= 31.771 && q.y >= 0.229 && q.y <= 21.771);
                    let dx = (12. - q.x).max(q.x - 20.).max(0.);
                    let dy = (7. - q.y).max(q.y - 15.).max(0.);
                    assert!(
                        dx.hypot(dy) >= 0.229,
                        "underlay crosses the inset hole at {q:?}"
                    );
                }
            }
            previous = s.point();
        }
        assert!(directed_segments > 0);
    }
}

#[test]
fn satin_underlay_remains_rail_driven_when_tatami_angle_is_set() {
    use embroidery_lab::{SatinColumn, StitchKind};
    let mut p = Project {
        version: 5,
        objects: vec![Object {
            satin_column: Some(SatinColumn {
                left: vec![Point::new(0., 0.), Point::new(15., 0.)],
                right: vec![Point::new(0., 4.), Point::new(15., 4.)],
                ..SatinColumn::default()
            }),
            settings: Settings {
                kind: StitchKind::Satin,
                length_mm: 6.,
                edge_underlay: true,
                zigzag_underlay: true,
                ..Settings::default()
            },
            ..Object::default()
        }],
        ..Project::default()
    };
    let base = engine::generate_plan(&p).unwrap();
    p.objects[0].settings.underlay_angle_deg = Some(31.);
    let overridden = engine::generate_plan(&p).unwrap();
    assert_eq!(
        serde_json::to_value(base).unwrap(),
        serde_json::to_value(overridden).unwrap()
    );
}

#[test]
fn separate_thread_cut_intentions_and_safe_lock_roles_are_retained() {
    use embroidery_lab::StitchRole;
    let mut p = project(
        vec![rectangle(0., 0., 10., 8.), rectangle(40., 0., 10., 8.)],
        0.2,
    );
    p.objects[0].settings.tie_in = true;
    p.objects[0].settings.tie_off = true;
    let plan = engine::generate_plan(&p).unwrap();
    assert!(plan.stitches.iter().any(|s| s.command == Command::Trim));
    assert!(
        plan.stitches
            .iter()
            .zip(&plan.metadata)
            .any(|(s, m)| s.command == Command::Stitch && m.role == StitchRole::Lock)
    );
    assert!(
        plan.stitches
            .iter()
            .zip(&plan.metadata)
            .filter(|(s, _)| s.command == Command::Jump)
            .all(|(_, m)| m.role == StitchRole::Travel)
    );
    assert!(
        plan.stitches
            .iter()
            .zip(&plan.metadata)
            .filter(|(s, _)| s.command == Command::Trim)
            .all(|(_, m)| m.role == StitchRole::Control)
    );
}

#[test]
fn quality_top_coverage_cannot_be_supplied_by_underlay() {
    use embroidery_lab::{
        StitchRole,
        quality::{self, QualityKind, QualityThresholds},
    };
    let p = project(vec![rectangle(0., 0., 10., 8.)], 0.);
    let mut plan = engine::generate_plan(&p).unwrap();
    for meta in &mut plan.metadata {
        if meta.role == StitchRole::TopFill {
            meta.role = StitchRole::Underlay;
        }
    }
    assert!(
        quality::analyze_plan(&plan, &p, QualityThresholds::default())
            .iter()
            .any(|i| i.kind == QualityKind::EmptyObject)
    );
    let report = quality::coverage_estimates(&plan, &p, 0.45);
    assert_eq!(report[0].samples, 0);
}

#[test]
fn generation_honors_cancellation_before_changing_a_document() {
    let p = project(vec![rectangle(0., 0., 30., 20.)], 0.);
    let before = serde_json::to_value(&p).unwrap();
    assert!(
        engine::generate_plan_cancellable(&p, &std::sync::atomic::AtomicBool::new(true))
            .unwrap_err()
            .contains("cancelled")
    );
    assert_eq!(serde_json::to_value(&p).unwrap(), before);
}

#[test]
fn optimizer_keeps_overlapping_same_color_layer_order() {
    let mut p = project(vec![rectangle(20., 20., 10., 10.)], 0.);
    p.objects.push(Object {
        id: 2,
        contours: vec![rectangle(0., 0., 30., 30.)],
        ..p.objects[0].clone()
    });
    engine::optimize_order(&mut p);
    assert_eq!(
        p.objects.iter().map(|o| o.id).collect::<Vec<_>>(),
        vec![1, 2]
    );
}

#[test]
fn segment_density_clips_exact_lengths_and_separates_pass_roles() {
    use embroidery_lab::{CommandMeta, MachinePlan, Stitch, StitchRole, quality};
    let entries = [
        (-1.5, 0.5, Command::Jump, StitchRole::Travel),
        (1.5, 0.5, Command::Stitch, StitchRole::TopFill),
        (-0.5, -0.5, Command::Jump, StitchRole::Travel),
        (0.5, 0.5, Command::Stitch, StitchRole::Underlay),
    ];
    let plan = MachinePlan {
        stitches: entries
            .iter()
            .map(|&(x, y, command, _)| Stitch {
                x,
                y,
                command,
                color: [0, 0, 0],
                object_id: 1,
            })
            .collect(),
        metadata: entries
            .iter()
            .map(|entry| CommandMeta {
                role: entry.3,
                pass_id: 1,
            })
            .collect(),
        warnings: Vec::new(),
    };
    let report = quality::segment_density(&plan, 1.);
    assert!(report.complete);
    assert_eq!(report.segments_analyzed, 2);
    assert!((report.total_thread_mm - (3. + 2_f64.sqrt())).abs() < 1e-9);
    assert!((report.cells.iter().map(|cell| cell.top_mm).sum::<f64>() - 3.).abs() < 1e-9);
    assert!(
        (report
            .cells
            .iter()
            .map(|cell| cell.underlay_mm)
            .sum::<f64>()
            - 2_f64.sqrt())
        .abs()
            < 1e-9
    );
    assert!(
        report
            .cells
            .iter()
            .all(|cell| cell.travel_mm == 0. && cell.lock_mm == 0.)
    );
    let negative = report
        .cells
        .iter()
        .find(|cell| cell.min == Point::new(-2., 0.))
        .unwrap();
    assert!((negative.top_mm - 0.5).abs() < 1e-9);
    let shared = report
        .cells
        .iter()
        .find(|cell| cell.min == Point::new(0., 0.))
        .unwrap();
    assert!((shared.top_mm - 1.).abs() < 1e-9);
    assert!((shared.underlay_mm - 2_f64.sqrt() * 0.5).abs() < 1e-9);
}

#[test]
fn repeated_top_segments_are_reported_without_flagging_intentional_locks() {
    use embroidery_lab::{
        CommandMeta, MachinePlan, Stitch, StitchRole,
        quality::{self, QualityKind, QualityThresholds},
    };
    let entries = [
        (1., Command::Jump, StitchRole::Travel),
        (3., Command::Stitch, StitchRole::TopFill),
        (1., Command::Stitch, StitchRole::TopFill),
        (3., Command::Stitch, StitchRole::Lock),
        (1., Command::Stitch, StitchRole::Lock),
    ];
    let plan = MachinePlan {
        stitches: entries
            .iter()
            .map(|&(x, command, _)| Stitch {
                x,
                y: 1.,
                command,
                color: [0, 0, 0],
                object_id: 1,
            })
            .collect(),
        metadata: entries
            .iter()
            .map(|entry| CommandMeta {
                role: entry.2,
                pass_id: 1,
            })
            .collect(),
        warnings: Vec::new(),
    };
    let p = project(vec![rectangle(0., 0., 10., 8.)], 0.);
    assert_eq!(
        quality::analyze_plan(&plan, &p, QualityThresholds::default())
            .iter()
            .filter(|issue| issue.kind == QualityKind::RepeatedTopFill)
            .count(),
        1
    );
}

#[test]
fn source_omission_and_classification_warnings_survive_the_shared_plan() {
    let mut p = project(vec![rectangle(0., 0., 10., 8.)], 0.);
    let mut recipe = embroidery_lab::persistence::RasterRecipe::capture(
        &p,
        embroidery_lab::import::RasterOptions::default(),
    );
    recipe.issues.push(embroidery_lab::PlanWarning {
        object_id: 1,
        point: Some(Point::new(2., 3.)),
        message: "Small feature requires manual review".into(),
    });
    recipe
        .warnings
        .push("Small feature requires manual review".into());
    recipe
        .warnings
        .push("Legacy source omission warning".into());
    p.raster_recipe = Some(recipe);
    let plan = engine::generate_plan(&p).unwrap();
    assert_eq!(
        plan.warnings
            .iter()
            .filter(|w| w.message == "Small feature requires manual review")
            .count(),
        1
    );
    assert!(
        plan.warnings
            .iter()
            .any(|w| w.object_id == 1 && w.point == Some(Point::new(2., 3.)))
    );
    assert!(
        plan.warnings
            .iter()
            .any(|w| w.object_id == 0 && w.message == "Legacy source omission warning")
    );
}

#[test]
fn modern_satin_requires_intact_diagonals_within_the_requested_machine_cap() {
    use embroidery_lab::{SatinColumn, StitchKind, StitchRole};
    let mut p = Project {
        version: 5,
        objects: vec![Object {
            name: "Modern6mm Satin".into(),
            satin_column: Some(SatinColumn {
                left: vec![Point::new(0., 0.), Point::new(20., 0.)],
                right: vec![Point::new(0., 6.), Point::new(20., 6.)],
                ..SatinColumn::default()
            }),
            settings: Settings {
                kind: StitchKind::Satin,
                length_mm: 3.5,
                underlay: true,
                underlay_length_mm: 1.,
                tie_in: false,
                tie_off: false,
                ..Settings::default()
            },
            ..Object::default()
        }],
        ..Project::default()
    };
    let before = serde_json::to_value(&p).unwrap();
    let error = engine::generate_plan(&p).unwrap_err();
    assert!(error.contains("unsplit limit") && error.contains("use Tatami"));
    assert_eq!(serde_json::to_value(&p).unwrap(), before);
    p.objects[0].settings.length_mm = 8.;
    let plan = engine::generate_plan(&p).unwrap();
    let top: Vec<_> = plan
        .stitches
        .iter()
        .zip(&plan.metadata)
        .filter(|(stitch, meta)| {
            stitch.command == Command::Stitch && meta.role == StitchRole::TopFill
        })
        .map(|(stitch, _)| stitch.point())
        .collect();
    assert!(top.len() > 90);
    assert!(
        top.iter()
            .all(|point| point.y.abs() < 1e-8 || (point.y - 6.).abs() < 1e-8),
        "Modern Satin must not add generic midpoint punctures"
    );
    assert!(
        top.windows(2)
            .all(|pair| (pair[0].y - pair[1].y).abs() > 5.99)
    );
    assert!(
        plan.metadata
            .iter()
            .any(|meta| meta.role == StitchRole::Underlay)
    );
    p.machine_profile.max_stitch_mm = 4.;
    assert!(
        engine::generate_plan(&p)
            .unwrap_err()
            .contains("unsplit limit")
    );
    // These source diagonals fit the 6.05mm pre-quantization reserve, but
    // endpoint rounding makes them 6.10mm. They remain physically within the
    // 6.20mm machine cap and must not acquire generic midpoint punctures.
    p.machine_profile.max_stitch_mm = 6.2;
    let column = p.objects[0].satin_column.as_mut().unwrap();
    for point in &mut column.left {
        point.y = 0.04;
    }
    for point in &mut column.right {
        point.y = 6.08;
    }
    let plan = engine::generate_plan(&p).unwrap();
    let mut previous = Point::default();
    for (stitch, meta) in plan.stitches.iter().zip(&plan.metadata) {
        if stitch.command == Command::Stitch {
            assert!(previous.distance(stitch.point()) <= 6.2 + 1e-8);
            if meta.role == StitchRole::TopFill {
                assert!(
                    stitch.y.abs() < 1e-8 || (stitch.y - 6.1).abs() < 1e-8,
                    "Quantization must retain intact Satin top diagonals"
                );
            }
        }
        if matches!(stitch.command, Command::Stitch | Command::Jump) {
            previous = stitch.point();
        }
    }
}
