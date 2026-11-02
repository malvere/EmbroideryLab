//! Geometry picking for canvas context menus, in project millimetres.

use embroidery_lab::model::{Point, Project};

/// Pick the closest visible contour or Running path within `tolerance`, then
/// fall back to the topmost filled area. Later objects are drawn on top.
/// Contours are implicitly closed and use the model's even-odd fill rule.
pub(super) fn object_at(project: &Project, point: Point, tolerance: f64) -> Option<usize> {
    if !point.x.is_finite() || !point.y.is_finite() || !tolerance.is_finite() || tolerance < 0. {
        return None;
    }
    let mut boundary = None;
    let mut closest = tolerance;
    let mut filled = None;
    for (index, object) in project.objects.iter().enumerate().rev() {
        if !object.visible {
            continue;
        }
        let rings = object.flattened_contours();
        let mut inside = false;
        for ring in &rings {
            for (&a, &b) in ring
                .iter()
                .zip(ring.iter().cycle().skip(1))
                .take(ring.len())
            {
                let distance = segment_distance(point, a, b);
                if distance <= tolerance && (boundary.is_none() || distance < closest) {
                    boundary = Some(index);
                    closest = distance;
                }
                if (a.y > point.y) != (b.y > point.y)
                    && point.x < (b.x - a.x) * (point.y - a.y) / (b.y - a.y) + a.x
                {
                    inside = !inside;
                }
            }
        }
        for path in &object.running_paths {
            for pair in path.windows(2) {
                let distance = segment_distance(point, pair[0], pair[1]);
                if distance <= tolerance && (boundary.is_none() || distance < closest) {
                    boundary = Some(index);
                    closest = distance;
                }
            }
        }
        // An invalid or unfinished column can have visible, individually valid
        // rails even when it has no valid filled boundary. Keep them pickable
        // so the context menu can repair its nodes.
        if rings.is_empty()
            && let Some(column) = &object.satin_column
        {
            for right in [false, true] {
                if let Ok(rail) = column.flatten_rail(right, 0.02) {
                    for pair in rail.windows(2) {
                        let distance = segment_distance(point, pair[0], pair[1]);
                        if distance <= tolerance && (boundary.is_none() || distance < closest) {
                            boundary = Some(index);
                            closest = distance;
                        }
                    }
                }
            }
        }
        if inside && filled.is_none() {
            filled = Some(index);
        }
    }
    boundary.or(filled)
}

fn segment_distance(point: Point, a: Point, b: Point) -> f64 {
    let dx = b.x - a.x;
    let dy = b.y - a.y;
    let length_squared = dx * dx + dy * dy;
    let t = if length_squared > 0. {
        (((point.x - a.x) * dx + (point.y - a.y) * dy) / length_squared).clamp(0., 1.)
    } else {
        0.
    };
    point.distance(a.lerp(b, t))
}

#[cfg(test)]
mod tests {
    use super::*;
    use embroidery_lab::model::{CubicBezier, Object, SatinColumn};

    fn square(min: f64, max: f64) -> Vec<Point> {
        vec![
            Point::new(min, min),
            Point::new(max, min),
            Point::new(max, max),
            Point::new(min, max),
        ]
    }

    fn project(objects: Vec<Object>) -> Project {
        Project {
            objects,
            ..Default::default()
        }
    }

    fn area(rings: Vec<Vec<Point>>) -> Object {
        Object {
            contours: rings,
            ..Default::default()
        }
    }

    #[test]
    fn picking_respects_holes_and_implicit_closing_edges() {
        let project = project(vec![area(vec![square(0., 20.), square(5., 15.)])]);
        assert_eq!(object_at(&project, Point::new(2., 10.), 0.2), Some(0));
        assert_eq!(object_at(&project, Point::new(10., 10.), 0.2), None);
        assert_eq!(object_at(&project, Point::new(5.1, 10.), 0.2), Some(0));
        assert_eq!(object_at(&project, Point::new(-0.1, 10.), 0.2), Some(0));
        assert_eq!(object_at(&project, Point::new(-0.3, 10.), 0.2), None);
    }

    #[test]
    fn overlapping_fills_and_equal_boundaries_choose_topmost_visible_object() {
        let mut project = project(vec![
            area(vec![square(0., 20.)]),
            area(vec![square(0., 20.)]),
        ]);
        assert_eq!(object_at(&project, Point::new(10., 10.), 0.2), Some(1));
        assert_eq!(object_at(&project, Point::new(0., 10.), 0.2), Some(1));
        project.objects[1].visible = false;
        assert_eq!(object_at(&project, Point::new(10., 10.), 0.2), Some(0));
        assert_eq!(object_at(&project, Point::new(0., 10.), 0.2), Some(0));
    }

    #[test]
    fn nearby_running_paths_take_precedence_over_fill_without_closing_the_path() {
        let project = project(vec![
            Object {
                running_paths: vec![vec![
                    Point::new(2., 2.),
                    Point::new(18., 2.),
                    Point::new(18., 18.),
                ]],
                ..Default::default()
            },
            area(vec![square(0., 20.)]),
        ]);
        assert_eq!(object_at(&project, Point::new(10., 2.1), 0.2), Some(0));
        // The diagonal from the last path node to the first is not sewn.
        assert_eq!(object_at(&project, Point::new(10., 10.), 0.2), Some(1));
    }

    #[test]
    fn closest_visible_path_wins_and_hidden_paths_cannot_be_picked() {
        let path = |y, visible| Object {
            running_paths: vec![vec![Point::new(0., y), Point::new(10., y)]],
            visible,
            ..Default::default()
        };
        let project = project(vec![path(0., true), path(0.5, true), path(0.1, false)]);
        assert_eq!(object_at(&project, Point::new(5., 0.1), 1.), Some(0));
        assert_eq!(object_at(&project, Point::new(5., 0.4), 1.), Some(1));
        assert_eq!(object_at(&project, Point::new(5., 3.), 1.), None);
    }

    #[test]
    fn bezier_and_satin_geometry_use_their_flattened_boundaries() {
        let bezier = Object {
            bezier_segments: vec![CubicBezier {
                start: Point::new(0., 0.),
                control1: Point::new(0., 10.),
                control2: Point::new(10., 10.),
                end: Point::new(10., 0.),
            }],
            ..Default::default()
        };
        let satin = Object {
            satin_column: Some(SatinColumn {
                left: vec![Point::new(20., 0.), Point::new(20., 10.)],
                right: vec![Point::new(24., 0.), Point::new(24., 10.)],
                ..Default::default()
            }),
            ..Default::default()
        };
        let project = project(vec![bezier, satin]);
        assert_eq!(object_at(&project, Point::new(5., 3.), 0.1), Some(0));
        assert_eq!(object_at(&project, Point::new(5., 7.5), 0.1), Some(0));
        assert_eq!(object_at(&project, Point::new(22., 5.), 0.1), Some(1));
        assert_eq!(object_at(&project, Point::new(24.05, 5.), 0.1), Some(1));
        assert_eq!(object_at(&project, Point::new(27., 5.), 0.1), None);
    }

    #[test]
    fn empty_project_and_invalid_coordinates_do_not_pick_objects() {
        assert_eq!(object_at(&Project::default(), Point::new(0., 0.), 1.), None);
        let project = project(vec![area(vec![square(0., 20.)])]);
        assert_eq!(object_at(&project, Point::new(f64::NAN, 5.), 1.), None);
        assert_eq!(object_at(&project, Point::new(5., 5.), -1.), None);
        assert_eq!(object_at(&project, Point::new(5., 5.), f64::INFINITY), None);
    }

    #[test]
    fn one_invalid_satin_rail_does_not_hide_the_other_rail_from_picking() {
        let column = SatinColumn {
            left: vec![Point::new(0., 0.), Point::new(0., 0.)],
            right: vec![Point::new(10., 0.), Point::new(10., 10.)],
            ..Default::default()
        };
        assert!(column.boundary_rings().is_err());
        let project = project(vec![Object {
            satin_column: Some(column),
            ..Default::default()
        }]);
        assert_eq!(object_at(&project, Point::new(10., 3.), 0.2), Some(0));
        assert_eq!(object_at(&project, Point::new(10., 10.), 0.2), Some(0));
        assert_eq!(object_at(&project, Point::new(5., 5.), 0.2), None);
    }
}
