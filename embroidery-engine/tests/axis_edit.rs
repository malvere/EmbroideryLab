use embroidery_lab::{Object, Point, Project, Settings, StitchKind, axis_edit};

fn p(x: f64, y: f64) -> Point {
    Point::new(x, y)
}
fn source() -> Object {
    Object {
        id: 81,
        contours: vec![vec![p(0., 0.), p(30., 0.), p(30., 10.), p(0., 10.)]],
        ..Object::default()
    }
}
fn axes(paths: Vec<Vec<Point>>) -> Object {
    let source = source();
    Object {
        id: 82,
        centerline_graph: Some(axis_edit::attach(&paths, &source, 0.2).unwrap()),
        running_paths: paths,
        settings: Settings {
            kind: StitchKind::Running,
            ..Settings::default()
        },
        ..Object::default()
    }
}
fn bytes(object: &Object) -> Vec<u8> {
    serde_json::to_vec(object).unwrap()
}

#[test]
fn legacy_independent_paths_can_be_deleted_without_inventing_graph_connections() {
    let first = vec![p(0., 0.), p(2., 0.)];
    let second = vec![p(2., 0.), p(4., 0.)];
    let cycle = vec![p(10., 0.), p(12., 2.), p(10., 4.), p(10., 0.)];
    let mut object = Object {
        running_paths: vec![first.clone(), second.clone(), cycle.clone()],
        settings: Settings {
            kind: StitchKind::Running,
            ..Default::default()
        },
        ..Default::default()
    };
    let before = bytes(&object);
    assert!(axis_edit::delete_branch(&mut object, 7).is_err());
    assert_eq!(bytes(&object), before);
    axis_edit::delete_branch(&mut object, 2).unwrap();
    assert_eq!(object.running_paths, vec![first, second.clone()]);
    assert!(object.centerline_graph.is_none());
    axis_edit::delete_branch(&mut object, 0).unwrap();
    assert_eq!(object.running_paths, vec![second]);
    let before = bytes(&object);
    assert!(axis_edit::delete_branch(&mut object, 0).is_err());
    assert_eq!(bytes(&object), before);
    let mut loop_object = Object {
        running_paths: vec![cycle.clone(), vec![p(30., 0.), p(35., 0.)]],
        settings: Settings {
            kind: StitchKind::Running,
            ..Default::default()
        },
        ..Default::default()
    };
    axis_edit::delete_branch(&mut loop_object, 1).unwrap();
    assert_eq!(loop_object.running_paths, vec![cycle]);
}

#[test]
fn deleting_curved_spur_coalesces_degree_two_without_connecting_separate_stroke() {
    let mut object = axes(vec![
        vec![p(2., 3.), p(4., 4.), p(6., 5.)],
        vec![p(6., 5.), p(8., 5.), p(10., 4.)],
        vec![p(6., 5.), p(6.4, 5.3)],
        vec![p(18., 7.), p(26., 7.)],
    ]);
    let before = object.clone();
    let separate_id = object.centerline_graph.as_ref().unwrap().branches[3].id;
    axis_edit::delete_branch(&mut object, 2).unwrap();
    let graph = object.centerline_graph.as_ref().unwrap();
    assert_eq!(graph.branches.len(), 2);
    assert_eq!(graph.branches[0].id, 1);
    assert_eq!(graph.branches[1].id, separate_id);
    assert_eq!(
        object.running_paths[0],
        vec![p(2., 3.), p(4., 4.), p(6., 5.), p(8., 5.), p(10., 4.)]
    );
    assert_eq!(object.running_paths[1], before.running_paths[3]);
    assert_eq!(graph.paths(), object.running_paths);
    assert!(graph.vertices.iter().all(|v| graph.degree(v.id) == 1));
    assert_eq!(
        before.running_paths.len(),
        4,
        "history snapshot retains the original graph"
    );
}

#[test]
fn moving_t_junction_updates_all_incident_ends_and_failed_move_is_atomic() {
    let mut object = axes(vec![
        vec![p(2., 5.), p(6., 5.)],
        vec![p(6., 5.), p(10., 5.)],
        vec![p(6., 5.), p(6., 8.)],
    ]);
    axis_edit::move_node(&mut object, 0, 1, p(6.5, 5.5)).unwrap();
    assert_eq!(object.running_paths[0][1], p(6.5, 5.5));
    assert_eq!(object.running_paths[1][0], p(6.5, 5.5));
    assert_eq!(object.running_paths[2][0], p(6.5, 5.5));
    let before = bytes(&object);
    assert!(axis_edit::move_node(&mut object, 1, 0, p(40., 5.)).is_err());
    assert_eq!(bytes(&object), before);
    assert!(
        axis_edit::semi_auto_satin(&object, 0)
            .unwrap_err()
            .contains("junction")
    );
}

#[test]
fn smoothing_preserves_endpoints_and_has_pointwise_and_segment_error_bound() {
    let mut object = axes(vec![vec![
        p(2., 5.),
        p(4., 5.4),
        p(6., 4.6),
        p(8., 5.4),
        p(10., 5.),
    ]]);
    let before = object.running_paths[0].clone();
    axis_edit::smooth_branch(&mut object, 0, 0.1).unwrap();
    let after = &object.running_paths[0];
    assert_eq!(before.len(), after.len());
    assert_eq!(before.first(), after.first());
    assert_eq!(before.last(), after.last());
    assert_ne!(before, *after);
    for (a, b) in before.iter().zip(after) {
        assert!(a.distance(*b) <= 0.100000001);
    }
    // Same segment parameter is an explicit correspondence, bounding both directions.
    for (original, changed) in before.windows(2).zip(after.windows(2)) {
        for t in [0., 0.25, 0.5, 0.75, 1.] {
            assert!(
                original[0]
                    .lerp(original[1], t)
                    .distance(changed[0].lerp(changed[1], t))
                    <= 0.100000001
            );
        }
    }
}

#[test]
fn smoothing_that_cuts_a_concavity_is_rejected_without_mutation() {
    let source = Object {
        contours: vec![vec![
            p(0., 0.),
            p(4., 0.),
            p(4., 2.),
            p(2., 2.),
            p(2., 4.),
            p(0., 4.),
        ]],
        ..Object::default()
    };
    let paths = vec![vec![p(3., 2.), p(2., 2.), p(2., 3.)]];
    let mut object = axes(paths.clone());
    object.centerline_graph = Some(axis_edit::attach(&paths, &source, 0.2).unwrap());
    let before = bytes(&object);
    assert!(axis_edit::smooth_branch(&mut object, 0, 0.5).is_err());
    assert_eq!(bytes(&object), before);
}

#[test]
fn ring_stays_closed_after_smoothing_and_requires_an_explicit_satin_seam() {
    let mut object = axes(vec![vec![
        p(3., 3.),
        p(8., 3.),
        p(8., 7.),
        p(3., 7.),
        p(3., 3.),
    ]]);
    let graph = object.centerline_graph.as_ref().unwrap();
    assert_eq!(graph.branches[0].start_vertex, graph.branches[0].end_vertex);
    assert_eq!(graph.degree(graph.branches[0].start_vertex), 2);
    axis_edit::smooth_branch(&mut object, 0, 0.1).unwrap();
    assert_eq!(
        object.running_paths[0].first(),
        object.running_paths[0].last()
    );
    assert!(
        axis_edit::semi_auto_satin(&object, 0)
            .unwrap_err()
            .contains("seam")
    );
    let before = bytes(&object);
    assert!(axis_edit::delete_branch(&mut object, 0).is_err());
    assert_eq!(bytes(&object), before);
}

#[test]
fn width_uses_saved_boundary_and_normal_intersections_form_real_rails() {
    let mut object = axes(vec![vec![p(2., 5.), p(28., 5.)]]);
    assert_eq!(axis_edit::width_at(&object, p(15., 5.)), Some(10.));
    assert_eq!(axis_edit::width_at(&object, p(15., 2.)), Some(4.));
    assert_eq!(axis_edit::width_at(&object, p(31., 5.)), None);
    let columns = axis_edit::semi_auto_satin(&object, 0).unwrap();
    assert_eq!(columns.len(), 1);
    assert!(columns[0].left.iter().all(|p| (p.y - 10.).abs() < 1e-7));
    assert!(columns[0].right.iter().all(|p| p.y.abs() < 1e-7));
    // An extra island must stay independent and is not used as the opposing rail.
    object
        .centerline_graph
        .as_mut()
        .unwrap()
        .source_contours
        .push(vec![p(10., 20.), p(20., 20.), p(20., 24.), p(10., 24.)]);
    let separated = axis_edit::semi_auto_satin(&object, 0).unwrap();
    assert!(separated[0].left.iter().all(|p| (p.y - 10.).abs() < 1e-7));
}

#[test]
fn saved_graph_roundtrip_preserves_source_ids_shared_vertices_and_canonical_paths() {
    let mut object = axes(vec![
        vec![p(2., 5.), p(6., 5.)],
        vec![p(6., 5.), p(10., 5.)],
        vec![p(6., 5.), p(6., 8.)],
    ]);
    axis_edit::move_node(&mut object, 0, 1, p(6.5, 5.)).unwrap();
    let project = Project {
        version: 4,
        objects: vec![object],
        ..Project::default()
    };
    let path = std::env::temp_dir().join(format!(
        "axis-edit-{}-{}.elab",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    project.save(&path).unwrap();
    let restored = Project::load(&path).unwrap();
    std::fs::remove_file(&path).unwrap();
    let graph = restored.objects[0].centerline_graph.as_ref().unwrap();
    assert_eq!(graph.source_object_id, 81);
    assert_eq!(graph.source_contours, source().contours);
    assert_eq!(graph.pixel_size_mm, 0.2);
    assert_eq!(graph.paths(), restored.objects[0].running_paths);
    assert_eq!(
        serde_json::to_vec(&restored).unwrap(),
        serde_json::to_vec(&project).unwrap()
    );
}

#[test]
fn graph_validation_rejects_broken_shared_vertices_and_duplicate_ids() {
    let object = axes(vec![
        vec![p(2., 5.), p(6., 5.)],
        vec![p(6., 5.), p(10., 5.)],
    ]);
    let mut graph = object.centerline_graph.unwrap();
    graph.branches[1].points[0] = p(6.1, 5.);
    assert!(graph.validate().is_err());
    graph.branches[1].points[0] = p(6., 5.);
    graph.branches[1].id = graph.branches[0].id;
    assert!(graph.validate().is_err());
}
