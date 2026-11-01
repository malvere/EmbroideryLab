use embroidery_lab::{
    Object, Point, Project,
    region_edit::{self, RegionPreview},
};

fn rect(x: f64, y: f64, w: f64, h: f64) -> Vec<Point> {
    vec![
        Point::new(x, y),
        Point::new(x + w, y),
        Point::new(x + w, y + h),
        Point::new(x, y + h),
    ]
}
fn object(id: u64, rings: Vec<Vec<Point>>) -> Object {
    Object {
        id,
        contours: rings,
        ..Object::default()
    }
}
fn project(objects: Vec<Object>) -> Project {
    Project {
        objects,
        width_mm: 20.,
        ..Project::default()
    }
}
fn covered(point: Point, object: &Object) -> bool {
    object.contours.iter().fold(false, |inside, ring| {
        inside
            ^ ring
                .iter()
                .zip(ring.iter().cycle().skip(1))
                .take(ring.len())
                .fold(false, |v, (a, b)| {
                    v ^ ((a.y > point.y) != (b.y > point.y)
                        && point.x < (b.x - a.x) * (point.y - a.y) / (b.y - a.y) + a.x)
                })
    })
}

#[test]
fn mask_keeps_holes_and_individual_region_choices_through_order_changes() {
    let source = project(vec![
        object(1, vec![rect(1., 1., 8., 8.), rect(3., 3., 4., 4.)]),
        object(2, vec![rect(12., 1., 5., 8.)]),
    ]);
    let mut preview = RegionPreview::from_project(source.clone(), 200, 100).unwrap();
    assert!(preview.region_at(Point::new(4., 4.)).is_none());
    let region = preview.region_at(Point::new(2., 2.)).unwrap();
    preview.set_included(region, false).unwrap();
    assert!(!preview.project.objects[0].visible);
    let mut reordered = source;
    reordered.objects.reverse();
    let remapped = preview.apply_overrides_to(reordered).unwrap();
    assert!(
        !remapped
            .project
            .objects
            .iter()
            .find(|o| o.id == 1)
            .unwrap()
            .visible
    );
    assert!(
        remapped
            .project
            .objects
            .iter()
            .find(|o| o.id == 2)
            .unwrap()
            .visible
    );
    assert_eq!(remapped.region_at(Point::new(2., 2.)), Some(region));
    assert_eq!(remapped.overlay().get_pixel(40, 40)[3], 0);
}

#[test]
fn retrace_refuses_to_drop_manual_choices_when_geometry_changes() {
    let source = project(vec![object(1, vec![rect(1., 1., 8., 8.)])]);
    let mut preview = RegionPreview::from_project(source.clone(), 100, 100).unwrap();
    preview.toggle_at(Point::new(2., 2.)).unwrap();
    let mut changed = source;
    changed.objects[0].contours[0][0].x += 0.1;
    assert!(
        preview
            .apply_overrides_to(changed)
            .unwrap_err()
            .contains("topology changed")
    );
    assert!(!preview.project.objects[0].visible);
}

#[test]
fn union_removes_overlap_and_fills_only_covered_part_of_hole() {
    let source = project(vec![
        object(1, vec![rect(0., 0., 10., 10.), rect(3., 3., 4., 4.)]),
        object(2, vec![rect(5., 2., 8., 6.)]),
    ]);
    let merged = region_edit::merge_objects(&source, &[0, 1]).unwrap();
    assert_eq!(merged.objects.len(), 1);
    assert_eq!(merged.objects[0].id, 1);
    for y in 0..40 {
        for x in 0..56 {
            let point = Point::new(x as f64 / 4. + 0.11, y as f64 / 4. + 0.13);
            assert_eq!(
                covered(point, &merged.objects[0]),
                source.objects.iter().any(|o| covered(point, o)),
                "{point:?}"
            );
        }
    }
    assert!(!covered(Point::new(4., 4.), &merged.objects[0]));
    assert!(covered(Point::new(6., 4.), &merged.objects[0]));
}

#[test]
fn straight_cut_through_a_hole_preserves_exact_occupancy_without_overlap() {
    let source = project(vec![object(
        1,
        vec![rect(1., 1., 10., 10.), rect(4., 4., 4., 4.)],
    )]);
    let split =
        region_edit::split_object(&source, 0, &[Point::new(6., -2.), Point::new(6., 14.)]).unwrap();
    assert_eq!(split.objects.len(), 2);
    assert_ne!(split.objects[0].id, split.objects[1].id);
    for y in 0..52 {
        for x in 0..52 {
            let point = Point::new(x as f64 / 4. + 0.11, y as f64 / 4. + 0.13);
            let pieces = split.objects.iter().filter(|o| covered(point, o)).count();
            assert_eq!(
                pieces,
                usize::from(covered(point, &source.objects[0])),
                "{point:?}"
            );
        }
    }
}

#[test]
fn invalid_operations_leave_original_project_unchanged() {
    let source = project(vec![
        object(1, vec![rect(1., 1., 8., 8.)]),
        object(2, vec![rect(10., 1., 8., 8.)]),
    ]);
    let before = serde_json::to_vec(&source).unwrap();
    assert!(
        region_edit::split_object(&source, 0, &[Point::new(2., 2.), Point::new(3., 3.)]).is_err()
    );
    assert!(
        region_edit::split_object(&source, 0, &[Point::new(0., -1.), Point::new(0., 11.)]).is_err()
    );
    assert!(region_edit::merge_objects(&source, &[0, 50]).is_err());
    assert_eq!(serde_json::to_vec(&source).unwrap(), before);
}

#[test]
fn diagonal_cut_checks_actual_boundary_crossings_instead_of_distant_vertices() {
    let source = project(vec![object(1, vec![rect(0., 0., 10., 10.)])]);
    // The cut is near a corner; its endpoints clear the region even though the
    // projections of the remote corners extend well beyond this short segment.
    let result =
        region_edit::split_object(&source, 0, &[Point::new(-0.2, 2.2), Point::new(2.2, -0.2)])
            .unwrap();
    assert_eq!(result.objects.len(), 2);
    assert!(
        result
            .objects
            .iter()
            .any(|o| covered(Point::new(0.5, 0.5), o))
    );
    assert!(
        result
            .objects
            .iter()
            .any(|o| covered(Point::new(8., 8.), o))
    );
}
