use embroidery_lab::{
    Command, Object, Point, Project, RailHandles, SatinColumn, SatinRung, Settings, StitchKind,
    dst, engine, satin_edit,
};

fn straight() -> SatinColumn {
    SatinColumn {
        left: vec![Point::new(0., 0.), Point::new(20., 0.)],
        right: vec![Point::new(0., 4.), Point::new(20., 4.)],
        ..SatinColumn::default()
    }
}
fn project(column: SatinColumn) -> Project {
    Project {
        objects: vec![Object {
            satin_column: Some(column),
            settings: Settings {
                kind: StitchKind::Satin,
                underlay: false,
                tie_in: false,
                tie_off: false,
                length_mm: 8.,
                ..Settings::default()
            },
            ..Object::default()
        }],
        ..Project::default()
    }
}
fn ring() -> SatinColumn {
    let rail = |radius: f64| {
        (0..64)
            .map(|i| {
                let a = i as f64 * std::f64::consts::TAU / 64.;
                Point::new(radius * a.cos(), radius * a.sin())
            })
            .collect()
    };
    SatinColumn {
        left: rail(8.),
        right: rail(12.),
        closed: true,
        ..SatinColumn::default()
    }
}
fn assert_annulus_sewing(stitches: &[embroidery_lab::Stitch]) {
    let mut previous = Point::default();
    for s in stitches {
        if s.command == Command::Stitch {
            for i in 0..=30 {
                let p = previous.lerp(s.point(), i as f64 / 30.);
                let radius = p.x.hypot(p.y);
                assert!(
                    (7.85..=12.15).contains(&radius),
                    "sewing outside annulus at {p:?}"
                );
            }
        }
        previous = s.point();
    }
}

#[test]
fn insert_remove_move_and_smooth_rails_without_losing_cubic_shape() {
    let mut column = straight();
    column.left_handles = vec![
        RailHandles {
            outgoing: Some(Point::new(6., -2.)),
            ..RailHandles::default()
        },
        RailHandles {
            incoming: Some(Point::new(14., -2.)),
            ..RailHandles::default()
        },
    ];
    let original = satin_edit::rail_curve(&column, false, 0).unwrap();
    let index = satin_edit::insert_node(&mut column, false, 0, 0.5).unwrap();
    assert_eq!(index, 1);
    assert!(column.left[1].distance(Point::new(10., -1.5)) < 1e-8);
    let split = column.flatten_rail(false, 0.005).unwrap();
    let before = original.flatten(0.005);
    for point in split {
        assert!(before.iter().any(|p| p.distance(point) < 0.3));
    }
    let old_incoming = column.left_handles[1].incoming.unwrap();
    let node = column.left[1];
    satin_edit::set_node(&mut column, false, 1, Point::new(node.x + 1., node.y + 2.)).unwrap();
    assert!(
        column.left_handles[1]
            .incoming
            .unwrap()
            .distance(Point::new(old_incoming.x + 1., old_incoming.y + 2.))
            < 1e-8
    );
    satin_edit::set_smooth(&mut column, false, 1, true).unwrap();
    satin_edit::set_handle(&mut column, false, 1, true, Point::new(14., 1.5)).unwrap();
    let h = column.left_handles[1];
    let node = column.left[1];
    let a = h.incoming.unwrap();
    let b = h.outgoing.unwrap();
    assert!(((a.x - node.x) * (b.y - node.y) - (a.y - node.y) * (b.x - node.x)).abs() < 1e-8);
    satin_edit::remove_node(&mut column, false, 1).unwrap();
    assert_eq!(column.left.len(), 2);
    assert!(satin_edit::remove_node(&mut column, false, 0).is_err());
}

#[test]
fn rungs_control_local_direction_and_reject_crossings() {
    let mut column = straight();
    satin_edit::add_rung(&mut column, 0.3, 0.4).unwrap();
    satin_edit::add_rung(&mut column, 0.7, 0.8).unwrap();
    assert!(satin_edit::add_rung(&mut column, 0.5, 0.9).is_err());
    let stations = engine::satin_stations(&column, 0.4).unwrap();
    assert!(stations.iter().any(
        |(l, r)| l.distance(Point::new(6., 0.)) < 1e-8 && r.distance(Point::new(8., 4.)) < 1e-8
    ));
    let mut broken = column.clone();
    broken.rungs[1].right = 0.2;
    assert!(engine::satin_stations(&broken, 0.4).is_err());
    satin_edit::reverse(&mut column).unwrap();
    assert_eq!(column.left[0], Point::new(20., 0.));
    assert!((column.rungs[0].left - 0.3).abs() < 1e-8);
    assert!((column.rungs[0].right - 0.2).abs() < 1e-8);
    engine::generate(&project(column)).unwrap();
}

#[test]
fn split_preserves_boundary_endpoints_and_rung_positions() {
    let mut column = straight();
    column.rungs = vec![
        SatinRung {
            left: 0.25,
            right: 0.3,
        },
        SatinRung {
            left: 0.75,
            right: 0.8,
        },
    ];
    let (a, b) = satin_edit::split(&column, 0.5, 0.5).unwrap();
    assert_eq!(a.left.last(), b.left.first());
    assert_eq!(a.right.last(), b.right.first());
    assert_eq!(a.left.first(), column.left.first());
    assert_eq!(b.right.last(), column.right.last());
    assert!((a.rungs[0].left - 0.5).abs() < 1e-8);
    assert!((b.rungs[0].right - 0.6).abs() < 1e-8);
    assert!(satin_edit::split(&column, 0.5, 0.9).is_err());
}

#[test]
fn closed_satin_has_a_real_hole_and_rotatable_seam() {
    let mut column = ring();
    column.seam = 0.25;
    let rings = column.boundary_rings().unwrap();
    assert_eq!(rings.len(), 2);
    let stations = engine::satin_stations(&column, 0.4).unwrap();
    assert!(stations[0].0.distance(Point::new(0., 8.)) < 1e-7);
    assert!(stations[0].0.distance(stations.last().unwrap().0) < 1e-7);
    assert!(stations[0].1.distance(stations.last().unwrap().1) < 1e-7);
    let stitches = engine::generate(&project(column.clone())).unwrap();
    assert_annulus_sewing(&stitches);
    let decoded = dst::decode(&dst::encode(&stitches, "ring").unwrap()).unwrap();
    assert_annulus_sewing(&decoded);
    satin_edit::reverse(&mut column).unwrap();
    assert!(
        engine::satin_stations(&column, 0.4).unwrap()[0]
            .0
            .distance(Point::new(0., 8.))
            < 1e-7
    );
}

#[test]
fn closed_seam_follows_mapped_rungs_and_first_side_is_respected() {
    let mut column = ring();
    column.rungs.push(SatinRung {
        left: 0.25,
        right: 0.3,
    });
    column.seam = 0.25;
    engine::satin_stations(&column, 0.4).unwrap();
    column.first_right = true;
    let stations = engine::satin_stations(&column, 0.4).unwrap();
    let generated = engine::generate(&project(column)).unwrap();
    assert!(generated[0].point().distance(stations[0].1) < 1e-7);
}

#[test]
fn split_closed_columns_at_rung_seams_preserves_editable_curves_and_holes() {
    for seam in [0., 0.1, 0.25] {
        let mut column = ring();
        column.seam = seam;
        satin_edit::add_rung(&mut column, 0.25, 0.3).unwrap();
        let stations = engine::satin_stations(&column, 0.4).unwrap();
        let (a, b) = satin_edit::split(&column, 0.5, 0.5).unwrap();
        assert!(!a.closed && !b.closed);
        assert!(a.left[0].distance(stations[0].0) < 0.01);
        assert_eq!(a.left.last(), b.left.first());
        assert_eq!(a.right.last(), b.right.first());
        assert!(a.left[0].distance(*b.left.last().unwrap()) < 1e-7);
        assert_annulus_sewing(&engine::generate(&project(a)).unwrap());
        assert_annulus_sewing(&engine::generate(&project(b)).unwrap());
    }
}

#[test]
fn outline_offsets_work_for_open_lines_and_closed_paths() {
    let column =
        satin_edit::outline(&[Point::new(0., 0.), Point::new(20., 0.)], false, 4.).unwrap();
    for (l, r) in engine::satin_stations(&column, 0.4).unwrap() {
        assert!((l.distance(r) - 4.).abs() < 1e-8);
        assert!((l.y + r.y).abs() < 1e-8);
    }
    let source: Vec<_> = (0..64)
        .map(|i| {
            let a = i as f64 * std::f64::consts::TAU / 64.;
            Point::new(10. * a.cos(), 10. * a.sin())
        })
        .collect();
    let column = satin_edit::outline(&source, true, 4.).unwrap();
    assert!(column.closed);
    assert_annulus_sewing(&engine::generate(&project(column)).unwrap());
    let bent = satin_edit::outline(
        &[
            Point::new(0., 0.),
            Point::new(20., 0.),
            Point::new(20., 10.),
        ],
        false,
        4.,
    )
    .unwrap();
    assert!(!bent.rungs.is_empty());
    engine::generate(&project(bent)).unwrap();
    let square = satin_edit::outline(
        &[
            Point::new(0., 0.),
            Point::new(20., 0.),
            Point::new(20., 20.),
            Point::new(0., 20.),
        ],
        true,
        4.,
    )
    .unwrap();
    assert_eq!(square.boundary_rings().unwrap().len(), 2);
    engine::generate(&project(square)).unwrap();
    assert!(
        satin_edit::outline(
            &[
                Point::new(0., 0.),
                Point::new(5., 5.),
                Point::new(0., 5.),
                Point::new(5., 0.)
            ],
            false,
            4.
        )
        .is_err()
    );
}

#[test]
fn underlay_layers_stay_inside_closed_holes_and_respect_quantized_length() {
    for layer in 0..4 {
        let mut p = project(ring());
        let s = &mut p.objects[0].settings;
        s.underlay = layer == 0 || layer == 3;
        s.edge_underlay = layer == 1 || layer == 3;
        s.zigzag_underlay = layer == 2 || layer == 3;
        s.underlay_length_mm = 1.2;
        let paths = engine::satin_underlay_paths(
            p.objects[0].satin_column.as_ref().unwrap(),
            &p.objects[0].settings,
        )
        .unwrap();
        assert_eq!(
            paths.len(),
            if layer == 1 {
                2
            } else if layer == 3 {
                4
            } else {
                1
            }
        );
        let generated = engine::generate(&p).unwrap();
        assert_annulus_sewing(&generated);
        let decoded = dst::decode(&dst::encode(&generated, "layers").unwrap()).unwrap();
        assert_annulus_sewing(&decoded);
        assert!(dst::stats(&decoded).max_stitch_mm <= 8. + 1e-8);
    }
}

#[test]
fn v4_geometry_roundtrips_without_changing_old_schema_defaults() {
    let directory = std::env::temp_dir().join(format!("satin-editor-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("geometry.elab");
    let mut p = project(ring());
    p.objects[0].settings.edge_underlay = true;
    p.save(&path).unwrap();
    let loaded = Project::load(&path).unwrap();
    assert_eq!(loaded.version, 4);
    assert!(loaded.objects[0].satin_column.as_ref().unwrap().closed);
    assert!(loaded.objects[0].settings.edge_underlay);
    let mut old = serde_json::to_value(project(straight())).unwrap();
    old["version"] = 2.into();
    for key in [
        "left_handles",
        "right_handles",
        "rungs",
        "closed",
        "seam",
        "first_right",
    ] {
        old["objects"][0]["satin_column"]
            .as_object_mut()
            .unwrap()
            .remove(key);
    }
    for key in [
        "edge_underlay",
        "zigzag_underlay",
        "underlay_inset_mm",
        "underlay_spacing_mm",
        "underlay_length_mm",
    ] {
        old["objects"][0]["settings"]
            .as_object_mut()
            .unwrap()
            .remove(key);
    }
    std::fs::write(&path, serde_json::to_vec(&old).unwrap()).unwrap();
    let loaded = Project::load(&path).unwrap();
    assert!(!loaded.objects[0].settings.edge_underlay);
    assert_eq!(loaded.objects[0].settings.underlay_spacing_mm, 2.);
    engine::generate(&loaded).unwrap();
}

#[test]
fn s_curve_shortens_both_local_inner_edges_with_equal_total_rail_lengths() {
    let mut column = SatinColumn::default();
    for i in 0..=96 {
        let x = i as f64 * 30. / 96.;
        let a = x * std::f64::consts::TAU / 30.;
        let y = 3. * a.sin();
        let slope = 3. * std::f64::consts::TAU / 30. * a.cos();
        let normal = Point::new(
            -slope / (1. + slope * slope).sqrt(),
            1. / (1. + slope * slope).sqrt(),
        );
        column
            .left
            .push(Point::new(x + normal.x * 2., y + normal.y * 2.));
        column
            .right
            .push(Point::new(x - normal.x * 2., y - normal.y * 2.));
    }
    let length = |path: &[Point]| path.windows(2).map(|p| p[0].distance(p[1])).sum::<f64>();
    let (left_length, right_length) = (length(&column.left), length(&column.right));
    assert!((left_length - right_length).abs() < 1e-8);
    // A few manual cross-sections align the two local bends, independent of
    // the equal overall rail lengths that defeated the previous heuristic.
    for i in (8..96).step_by(8) {
        let left = length(&column.left[..=i]) / left_length;
        let right = length(&column.right[..=i]) / right_length;
        satin_edit::add_rung(&mut column, left, right).unwrap();
    }
    let generated = engine::generate(&project(column.clone())).unwrap();
    let distance = |point: Point, path: &[Point]| {
        path.windows(2)
            .map(|pair| {
                let dx = pair[1].x - pair[0].x;
                let dy = pair[1].y - pair[0].y;
                let t = (((point.x - pair[0].x) * dx + (point.y - pair[0].y) * dy)
                    / (dx * dx + dy * dy))
                    .clamp(0., 1.);
                point.distance(pair[0].lerp(pair[1], t))
            })
            .fold(f64::INFINITY, f64::min)
    };
    let interior: Vec<_> = generated
        .iter()
        .filter(|s| {
            s.command == Command::Stitch
                && distance(s.point(), &column.left).min(distance(s.point(), &column.right)) > 0.2
        })
        .collect();
    assert!(
        interior.iter().any(|s| s.x < 15.),
        "first inner bend has no short stitches"
    );
    assert!(
        interior.iter().any(|s| s.x > 15.),
        "second inner bend has no short stitches"
    );
}

#[test]
fn optimize_order_cannot_increase_actual_jump_count_or_length() {
    let mut p = Project::default();
    for (i, offset) in [80., 0., 55., 25.].into_iter().enumerate() {
        let mut object = project(straight()).objects.remove(0);
        object.id = i as u64 + 1;
        let column = object.satin_column.as_mut().unwrap();
        for point in column.left.iter_mut().chain(&mut column.right) {
            point.x += offset;
        }
        p.objects.push(object);
    }
    let cost = |stitches: &[embroidery_lab::Stitch]| {
        let mut cursor = Point::default();
        let mut count = 0;
        let mut length = 0.;
        for s in stitches {
            if s.command == Command::Jump {
                count += 1;
                length += cursor.distance(s.point());
            }
            cursor = s.point();
        }
        (count, length)
    };
    let before = cost(&engine::generate(&p).unwrap());
    engine::optimize_order(&mut p);
    let after = cost(&engine::generate(&p).unwrap());
    assert!(after.0 <= before.0);
    assert!(after.1 <= before.1 + 1e-8);
}

#[test]
fn legacy_v2_satin_matches_recorded_v04_stitches_exactly() {
    // Independent synthetic baseline copied from outputs/development/satin;
    // includes centerwalk, curved rails, ties and the historical jump policy.
    let project: Project =
        serde_json::from_str(include_str!("../../tests/fixtures/legacy_satin_v2.elab")).unwrap();
    let expected: Vec<embroidery_lab::Stitch> = serde_json::from_str(include_str!(
        "../../tests/fixtures/legacy_satin_v2.stitches.json"
    ))
    .unwrap();
    let actual = engine::generate(&project).unwrap();
    assert_eq!(actual.len(), expected.len());
    for (i, (a, b)) in actual.iter().zip(expected).enumerate() {
        assert_eq!(a.command, b.command, "command {i}");
        assert_eq!(a.object_id, b.object_id);
        assert_eq!(a.color, b.color);
        assert!(
            a.point().distance(b.point()) < 1e-8,
            "point {i}: {a:?} vs {b:?}"
        );
    }
}

#[test]
fn evenly_spaced_rungs_do_not_double_density_and_json_preserves_exact_geometry() {
    let mut column = straight();
    for i in 1..100 {
        satin_edit::add_rung(&mut column, i as f64 / 100., i as f64 / 100.).unwrap();
    }
    let stations = engine::satin_stations(&column, 0.4).unwrap();
    assert_eq!(stations.len(), 101);
    let mut project = project(column.clone());
    project.version = 4;
    // These coordinates appeared in the real semi-automatic save/open failure.
    project.objects[0].satin_column.as_mut().unwrap().left[0].x = 9.399999999999999;
    let loaded: Project = serde_json::from_slice(&serde_json::to_vec(&project).unwrap()).unwrap();
    assert_eq!(
        loaded.objects[0].satin_column.as_ref().unwrap().rungs,
        column.rungs
    );
    assert_eq!(
        loaded.objects[0].satin_column.as_ref().unwrap().left,
        project.objects[0].satin_column.as_ref().unwrap().left
    );
    let original = engine::generate(&project).unwrap();
    let regenerated = engine::generate(&loaded).unwrap();
    assert_eq!(original.len(), regenerated.len());
    for (a, b) in original.iter().zip(regenerated) {
        assert_eq!(a.command, b.command);
        assert_eq!(a.point(), b.point());
    }
}
