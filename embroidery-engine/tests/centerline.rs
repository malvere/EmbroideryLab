use embroidery_lab::{Object, Point, SatinColumn, centerline};

fn rectangle(x: f64, y: f64, width: f64, height: f64) -> Vec<Point> {
    vec![
        Point::new(x, y),
        Point::new(x + width, y),
        Point::new(x + width, y + height),
        Point::new(x, y + height),
    ]
}

#[test]
fn long_bar_becomes_open_interior_centerline_deterministically() {
    let object = Object {
        contours: vec![rectangle(10., 20., 30., 4.)],
        ..Object::default()
    };
    let a = centerline::extract(&object, 0.25).unwrap();
    let b = centerline::extract(&object, 0.25).unwrap();
    assert_eq!(a, b);
    assert_eq!(a.paths.len(), 1);
    assert!(a.isolated_points.is_empty());
    let path = &a.paths[0];
    assert!(path.first().unwrap().distance(*path.last().unwrap()) > 20.0);
    assert!(
        path.iter()
            .all(|p| p.x > 10. && p.x < 40. && p.y > 20. && p.y < 24.)
    );
    assert!(path.iter().all(|p| (p.y - 22.).abs() <= 0.26));
    assert!(a.skeleton_pixels < a.foreground_pixels);
    assert_eq!(object.contours[0], rectangle(10., 20., 30., 4.));
}

#[test]
fn ring_retains_hole_and_closed_cycle() {
    let object = Object {
        contours: vec![rectangle(0., 0., 20., 20.), rectangle(4., 4., 12., 12.)],
        ..Object::default()
    };
    let result = centerline::extract(&object, 0.25).unwrap();
    assert_eq!(result.paths.len(), 1);
    let path = &result.paths[0];
    assert_eq!(path.first(), path.last());
    assert!(
        path.iter()
            .all(|p| !(p.x > 4. && p.x < 16. && p.y > 4. && p.y < 16.))
    );
    assert_eq!(result.foreground_pixels, (80 * 80) - (48 * 48));
}

#[test]
fn disjoint_regions_do_not_get_connected_by_implicit_closure() {
    let object = Object {
        contours: vec![rectangle(0., 0., 15., 3.), rectangle(0., 10., 15., 3.)],
        ..Object::default()
    };
    let result = centerline::extract(&object, 0.25).unwrap();
    assert_eq!(result.paths.len(), 2);
    for path in &result.paths {
        assert_ne!(path.first(), path.last());
        let first_y = path[0].y;
        assert!(path.iter().all(|p| (p.y - first_y).abs() <= 0.25));
        assert!(path.iter().all(|p| p.y < 3.) || path.iter().all(|p| p.y > 10.));
    }
}

#[test]
fn paired_rail_boundary_is_a_supported_area() {
    let object = Object {
        satin_column: Some(SatinColumn {
            left: vec![Point::new(0., 0.), Point::new(0., 20.)],
            right: vec![Point::new(3., 0.), Point::new(3., 20.)],
            ..SatinColumn::default()
        }),
        ..Object::default()
    };
    let result = centerline::extract(&object, 0.25).unwrap();
    assert_eq!(result.paths.len(), 1);
    assert!(result.paths[0].iter().all(|p| p.x > 0. && p.x < 3.));
}

#[test]
fn explicit_resolution_and_geometry_limits_are_errors() {
    let object = Object {
        contours: vec![rectangle(0., 0., 100., 10.)],
        ..Object::default()
    };
    for pixel in [0., -1., f64::NAN, f64::INFINITY, 0.001, 11.] {
        assert!(centerline::extract(&object, pixel).is_err());
    }
    assert!(
        centerline::extract(&object, 0.01)
            .unwrap_err()
            .contains("2048")
    );
    assert!(centerline::extract(&Object::default(), 0.25).is_err());
    let invalid = Object {
        contours: vec![vec![Point::new(f64::NAN, 0.); 3]],
        ..Object::default()
    };
    assert!(
        centerline::extract(&invalid, 0.25)
            .unwrap_err()
            .contains("invalid")
    );
}

#[test]
fn raster_too_coarse_for_a_thin_region_returns_error() {
    let object = Object {
        contours: vec![rectangle(0., 0., 10., 0.05)],
        ..Object::default()
    };
    assert!(
        centerline::extract(&object, 1.)
            .unwrap_err()
            .contains("No foreground")
    );
}
