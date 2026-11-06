use embroidery_lab::{
    Command, Object, Point, Project, SatinColumn, Settings, StitchKind, dst, engine,
};

fn project(column: SatinColumn) -> Project {
    Project {
        objects: vec![Object {
            name: "Paired satin test".into(),
            satin_column: Some(column),
            settings: Settings {
                kind: StitchKind::Satin,
                underlay: false,
                tie_in: false,
                tie_off: false,
                length_mm: 12.,
                ..Settings::default()
            },
            ..Object::default()
        }],
        ..Project::default()
    }
}
fn straight() -> SatinColumn {
    SatinColumn {
        left: vec![Point::new(0., 0.), Point::new(20., 0.)],
        right: vec![Point::new(0., 4.), Point::new(20., 4.)],
        ..SatinColumn::default()
    }
}
fn arc() -> SatinColumn {
    let rail = |radius: f64| {
        (0..=16)
            .map(|i| {
                let a = i as f64 * std::f64::consts::FRAC_PI_2 / 16.;
                Point::new(radius * a.cos(), radius * a.sin())
            })
            .collect()
    };
    SatinColumn {
        left: rail(8.),
        right: rail(12.),
        ..SatinColumn::default()
    }
}

#[test]
fn straight_column_alternates_rails_with_bounded_peak_advance() {
    let mut p = project(straight());
    p.objects[0].settings.angle_deg = 137.; // Paired geometry determines direction.
    p.objects[0].settings.auto_angle = true;
    let result = engine::generate(&p).unwrap();
    assert_eq!(
        result.iter().filter(|s| s.command == Command::Jump).count(),
        0 // The column starts at the machine origin; zero jumps are normalized away.
    );
    let points: Vec<_> = result
        .iter()
        .filter(|s| s.command == Command::Stitch)
        .map(|s| s.point())
        .collect();
    assert!(points.len() > 90);
    for pair in points.windows(2) {
        assert!((pair[0].y - pair[1].y).abs() > 3.99);
    }
    for side in [0., 4.] {
        let contacts: Vec<_> = points
            .iter()
            .filter(|p| (p.y - side).abs() < 1e-8)
            .collect();
        for pair in contacts.windows(2) {
            assert!(pair[1].x - pair[0].x <= 0.4000001);
        }
    }
    assert!(points.iter().any(|p| p.x.abs() < 1e-8));
    assert!(points.iter().filter(|p| (p.x - 20.).abs() < 1e-8).count() >= 2);
}

#[test]
fn independent_arc_lengths_ignore_rail_vertex_distribution() {
    let ordinary = engine::generate(&project(straight())).unwrap();
    let mut subdivided = straight();
    subdivided.right = vec![
        Point::new(0., 4.),
        Point::new(0.13, 4.),
        Point::new(0.14, 4.),
        Point::new(19.8, 4.),
        Point::new(20., 4.),
    ];
    let nonuniform = engine::generate(&project(subdivided)).unwrap();
    assert_eq!(ordinary.len(), nonuniform.len());
    for (a, b) in ordinary.iter().zip(nonuniform) {
        assert_eq!(a.command, b.command);
        assert!(a.point().distance(b.point()) < 1e-8);
    }
}

#[test]
fn tapered_and_curved_columns_have_local_direction_and_inner_shortening() {
    let mut taper = straight();
    taper.right = vec![Point::new(0., 2.), Point::new(20., 6.)];
    let stations = engine::satin_stations(&taper, 0.4).unwrap();
    assert!((stations[0].0.distance(stations[0].1) - 2.).abs() < 1e-8);
    let last = stations.last().unwrap();
    assert!((last.0.distance(last.1) - 6.).abs() < 1e-8);
    let output = engine::generate(&project(arc())).unwrap();
    for (a, b) in engine::satin_stations(&arc(), 0.4).unwrap() {
        for point in [a, b] {
            assert!((7.95..=12.000001).contains(&point.x.hypot(point.y)));
        }
    }
    let mut previous = Point::default();
    let mut diagonal = false;
    let mut inset = 0;
    for s in output {
        if s.command == Command::Stitch {
            let radius = s.x.hypot(s.y);
            // A quantized endpoint moves by at most sqrt(2)*0.05mm.
            assert!(
                (7.95 - 0.071..=12. + 0.071).contains(&radius),
                "radius={radius}"
            );
            if (8.3..9.).contains(&radius) {
                inset += 1;
            }
            if (s.x - previous.x).abs() > 0.5 && (s.y - previous.y).abs() > 0.5 {
                diagonal = true;
            }
        }
        previous = s.point();
    }
    assert!(diagonal);
    assert!(
        inset > 5,
        "Expected alternating short stitches on crowded inner rail"
    );
}

#[test]
fn invalid_rails_fail_explicitly() {
    let mut cases = vec![
        SatinColumn::default(),
        SatinColumn {
            left: vec![Point::new(0., 0.)],
            right: vec![Point::new(0., 4.)],
            ..SatinColumn::default()
        },
    ];
    let mut reversed = straight();
    reversed.right.reverse();
    cases.push(reversed);
    let mut narrow = straight();
    narrow.right = vec![Point::new(0., 0.01), Point::new(20., 0.01)];
    cases.push(narrow);
    let mut repeated = straight();
    repeated.left.insert(1, Point::new(0., 0.));
    cases.push(repeated);
    let mut crossing = straight();
    crossing.right = vec![
        Point::new(0., 4.),
        Point::new(10., -2.),
        Point::new(20., 4.),
    ];
    cases.push(crossing);
    let mut looped = straight();
    looped.left = vec![
        Point::new(0., 0.),
        Point::new(15., 0.),
        Point::new(5., 0.),
        Point::new(20., 0.),
    ];
    cases.push(looped);
    let mut excessive = straight();
    excessive.left = (0..1025).map(|i| Point::new(i as f64 / 50., 0.)).collect();
    cases.push(excessive);
    for column in cases {
        assert!(engine::generate(&project(column)).is_err());
    }
    let mut mixed = project(straight());
    mixed.objects[0].contours = Project::demo().objects.remove(0).contours;
    assert!(
        engine::generate(&mixed)
            .unwrap_err()
            .contains("must not mix")
    );
}

#[test]
fn editor_default_curved_column_supports_centerwalk() {
    let mut p = project(SatinColumn {
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
    });
    p.objects[0].settings.length_mm = 8.;
    p.objects[0].settings.underlay = true;
    assert!(
        engine::generate(&p)
            .unwrap()
            .iter()
            .filter(|s| s.command == Command::Stitch)
            .count()
            > 100
    );
}

#[test]
fn column_density_underlay_colors_and_quantized_length_constraints() {
    let coarse = engine::generate(&project(straight())).unwrap();
    let mut dense = project(straight());
    dense.objects[0].settings.spacing_mm = 0.2;
    assert!(engine::generate(&dense).unwrap().len() > coarse.len());
    for maximum in [0.3, 3.5, 8.] {
        let mut p = project(arc());
        p.objects[0].settings.length_mm = maximum;
        p.objects[0].settings.underlay = true;
        p.objects[0].settings.tie_in = true;
        p.objects[0].settings.tie_off = true;
        p.objects[0].settings.pull_comp_mm = 0.2;
        let mut second = p.objects[0].clone();
        second.id = 2;
        second.color = [200, 30, 30];
        // Keep two visible legacy columns for this codec/length assertion.
        // Coincident duplicates are now intentionally removed by layer planning.
        let column = second.satin_column.as_mut().unwrap();
        for point in column.left.iter_mut().chain(column.right.iter_mut()) {
            point.x += 100.;
        }
        p.objects.push(second);
        let plan = engine::generate_plan(&p).unwrap();
        let mut previous = Point::default();
        for (stitch, meta) in plan.stitches.iter().zip(&plan.metadata) {
            if stitch.command == Command::Stitch {
                let limit = if meta.role == embroidery_lab::StitchRole::Underlay {
                    p.objects[0].settings.underlay_length_mm
                } else {
                    maximum
                };
                assert!(previous.distance(stitch.point()) <= limit + 1e-8);
            }
            if matches!(stitch.command, Command::Stitch | Command::Jump) {
                previous = stitch.point();
            }
        }
        let decoded = dst::decode(&dst::encode(&plan.stitches, "columns").unwrap()).unwrap();
        assert_eq!(dst::stats(&decoded).color_changes, 1);
        assert!(
            dst::stats(&decoded).max_stitch_mm
                <= maximum.max(p.objects[0].settings.underlay_length_mm) + 1e-8
        );
        assert_eq!(decoded.last().unwrap().command, Command::End);
    }
}

#[test]
fn new_rail_projects_save_schema_two_and_old_projects_still_load() {
    let path =
        std::env::temp_dir().join(format!("embroidery-satin-v2-{}.elab", std::process::id()));
    let p = project(straight());
    p.save(&path).unwrap();
    let saved: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(saved["version"], 2);
    let loaded = Project::load(&path).unwrap();
    assert_eq!(loaded.version, 2);
    assert_eq!(
        loaded.objects[0].satin_column.as_ref().unwrap().left,
        straight().left
    );
    assert_eq!(loaded.objects[0].flattened_contours()[0].len(), 4);
    let legacy_path = path.with_extension("legacy.elab");
    let mut legacy = serde_json::to_value(Project::demo()).unwrap();
    for object in legacy["objects"].as_array_mut().unwrap() {
        object.as_object_mut().unwrap().remove("satin_column");
    }
    std::fs::write(&legacy_path, serde_json::to_vec(&legacy).unwrap()).unwrap();
    let old = Project::load(&legacy_path).unwrap();
    assert_eq!(old.version, 1);
    assert!(old.objects.iter().all(|o| o.satin_column.is_none()));
    assert!(engine::generate(&old).is_ok());
}
