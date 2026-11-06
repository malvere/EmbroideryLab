use embroidery_lab::{Object, Point, Project, Settings, engine};

fn rectangle(x: f64, y: f64, w: f64, h: f64) -> Vec<Point> {
    vec![
        Point::new(x, y),
        Point::new(x + w, y),
        Point::new(x + w, y + h),
        Point::new(x, y + h),
    ]
}

fn object(id: u64, contours: Vec<Vec<Point>>) -> Object {
    Object {
        id,
        contours,
        settings: Settings {
            underlay: false,
            tie_in: false,
            tie_off: false,
            ..Settings::default()
        },
        ..Object::default()
    }
}

#[test]
fn sparse_valid_projects_do_not_spend_exact_overlap_budget_on_disjoint_bounds() {
    let project = Project {
        objects: (0..2048)
            .map(|index| {
                object(
                    index + 1,
                    vec![rectangle(
                        (index % 64) as f64 * 2.,
                        (index / 64) as f64 * 2.,
                        1.,
                        1.,
                    )],
                )
            })
            .collect(),
        ..Project::default()
    };
    let visible = engine::generate_plan(&project).unwrap();
    let mut unchanged = project.clone();
    unchanged.layering.enabled = false;
    let plain = engine::generate_plan(&unchanged).unwrap();
    assert_eq!(
        serde_json::to_vec(&visible).unwrap(),
        serde_json::to_vec(&plain).unwrap()
    );
}

#[test]
fn complex_outline_hole_does_not_occlude_the_partitioned_regions_inside_it() {
    let mut objects: Vec<_> = (0..861)
        .map(|index| {
            object(
                index + 1,
                vec![rectangle(
                    (index % 41) as f64 * 10.,
                    (index / 41) as f64 * 10.,
                    8.,
                    8.,
                )],
            )
        })
        .collect();
    // A high-resolution enclosing outline has overlapping bounds with every
    // inner region, but its physical interior contains only the frame. This
    // matches the scale of the private 861-region/6838-node outline regression.
    let mut outline = Vec::new();
    for index in 0..1700 {
        let t = index as f64 / 1700.;
        outline.push(Point::new(-5. + 420. * t, -5.));
    }
    for index in 0..1700 {
        outline.push(Point::new(415., -5. + 220. * index as f64 / 1700.));
    }
    for index in 0..1700 {
        outline.push(Point::new(415. - 420. * index as f64 / 1700., 215.));
    }
    for index in 0..1700 {
        outline.push(Point::new(-5., 215. - 220. * index as f64 / 1700.));
    }
    objects.push(object(862, vec![outline, rectangle(0., 0., 410., 210.)]));
    let project = Project {
        objects,
        ..Project::default()
    };
    let visible = engine::generate_plan(&project).unwrap();
    let mut unchanged = project.clone();
    unchanged.layering.enabled = false;
    let plain = engine::generate_plan(&unchanged).unwrap();
    assert_eq!(
        serde_json::to_vec(&visible).unwrap(),
        serde_json::to_vec(&plain).unwrap()
    );
}
