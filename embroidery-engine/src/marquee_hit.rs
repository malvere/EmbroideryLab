//! Rectangle picking for visible objects, in project millimetres.

use embroidery_lab::model::{Object, Point, Project};

/// Return every visible object whose drawn boundary, Running path or even-odd
/// filled area intersects the rectangle. Either corner may be supplied first.
/// In particular, bounding-box overlap alone does not select a concavity or hole.
pub(super) fn objects_in_rect(project: &Project, a: Point, b: Point) -> Vec<usize> {
    let Some(rect) = PickRect::new(a, b) else {
        return Vec::new();
    };
    project
        .objects
        .iter()
        .enumerate()
        .filter_map(|(index, object)| {
            (object.visible && intersects_object(rect, object)).then_some(index)
        })
        .collect()
}

#[derive(Clone, Copy)]
struct PickRect {
    min: Point,
    max: Point,
}

impl PickRect {
    fn new(a: Point, b: Point) -> Option<Self> {
        if !finite(a) || !finite(b) {
            return None;
        }
        Some(Self {
            min: Point::new(a.x.min(b.x), a.y.min(b.y)),
            max: Point::new(a.x.max(b.x), a.y.max(b.y)),
        })
    }

    fn contains(self, point: Point) -> bool {
        finite(point)
            && (self.min.x..=self.max.x).contains(&point.x)
            && (self.min.y..=self.max.y).contains(&point.y)
    }

    /// Clip a finite segment against both rectangle slabs. This also includes
    /// zero-length segments and touching the rectangle's edge or corner.
    fn intersects_segment(self, a: Point, b: Point) -> bool {
        if !finite(a) || !finite(b) {
            return false;
        }
        if self.contains(a) || self.contains(b) {
            return true;
        }
        let mut enter = 0_f64;
        let mut leave = 1_f64;
        for (start, end, min, max) in [
            (a.x, b.x, self.min.x, self.max.x),
            (a.y, b.y, self.min.y, self.max.y),
        ] {
            let delta = end - start;
            if !delta.is_finite() {
                return false;
            }
            if delta == 0. {
                if start < min || start > max {
                    return false;
                }
                continue;
            }
            let first = (min - start) / delta;
            let second = (max - start) / delta;
            enter = enter.max(first.min(second));
            leave = leave.min(first.max(second));
            if enter > leave {
                return false;
            }
        }
        true
    }

    fn corners(self) -> [Point; 4] {
        [
            self.min,
            Point::new(self.max.x, self.min.y),
            self.max,
            Point::new(self.min.x, self.max.y),
        ]
    }
}

fn finite(point: Point) -> bool {
    point.x.is_finite() && point.y.is_finite()
}

fn intersects_object(rect: PickRect, object: &Object) -> bool {
    let rings = object.flattened_contours();
    // Invalid rings must not invent a closing edge or change another ring's
    // even-odd fill parity. Valid geometry in the same object stays pickable.
    let rings: Vec<_> = rings
        .iter()
        .filter(|ring| !ring.is_empty() && ring.iter().copied().all(finite))
        .collect();
    if rings.iter().any(|ring| {
        ring.iter()
            .zip(ring.iter().cycle().skip(1))
            .take(ring.len())
            .any(|(&a, &b)| rect.intersects_segment(a, b))
    }) {
        return true;
    }
    if object
        .running_paths
        .iter()
        .flat_map(|path| path.windows(2))
        .any(|pair| rect.intersects_segment(pair[0], pair[1]))
    {
        return true;
    }
    // Keep the valid individual rail of a malformed/unfinished satin column
    // selectable, just as point picking does when no fill can be generated.
    if let Some(column) = &object.satin_column
        && column.boundary_rings().is_err()
    {
        for right in [false, true] {
            if let Ok(rail) = column.flatten_rail(right, 0.02)
                && rail
                    .windows(2)
                    .any(|pair| rect.intersects_segment(pair[0], pair[1]))
            {
                return true;
            }
        }
    }
    // With no intersecting boundaries, a rectangle corner inside the fill
    // means the rectangle lies within the area. All rings share one parity.
    rect.corners().into_iter().any(|point| {
        let mut inside = false;
        for ring in &rings {
            if ring.len() < 3 {
                continue;
            }
            for (&a, &b) in ring
                .iter()
                .zip(ring.iter().cycle().skip(1))
                .take(ring.len())
            {
                if (a.y > point.y) != (b.y > point.y)
                    && point.x < (b.x - a.x) * (point.y - a.y) / (b.y - a.y) + a.x
                {
                    inside = !inside;
                }
            }
        }
        inside
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use embroidery_lab::model::{CubicBezier, SatinColumn};

    fn square(min: f64, max: f64) -> Vec<Point> {
        vec![
            Point::new(min, min),
            Point::new(max, min),
            Point::new(max, max),
            Point::new(min, max),
        ]
    }

    fn area(rings: Vec<Vec<Point>>) -> Object {
        Object {
            contours: rings,
            ..Default::default()
        }
    }

    fn project(objects: Vec<Object>) -> Project {
        Project {
            objects,
            ..Default::default()
        }
    }

    #[test]
    fn picks_all_intersecting_visible_objects_in_project_order_in_either_direction() {
        let mut hidden = area(vec![square(0., 20.)]);
        hidden.visible = false;
        let project = project(vec![
            area(vec![square(0., 20.)]),
            area(vec![square(8., 12.)]),
            area(vec![square(30., 40.)]),
            hidden,
        ]);
        let a = Point::new(6., 6.);
        let b = Point::new(14., 14.);
        assert_eq!(objects_in_rect(&project, a, b), vec![0, 1]);
        assert_eq!(objects_in_rect(&project, b, a), vec![0, 1]);
    }

    #[test]
    fn fill_contains_small_marquee_but_holes_and_bbox_only_overlap_are_not_hits() {
        let project = project(vec![
            area(vec![square(0., 20.), square(5., 15.)]),
            area(vec![vec![
                Point::new(30., 30.),
                Point::new(50., 30.),
                Point::new(30., 50.),
            ]]),
        ]);
        assert!(objects_in_rect(&project, Point::new(8., 8.), Point::new(12., 12.)).is_empty());
        assert_eq!(
            objects_in_rect(&project, Point::new(1., 8.), Point::new(3., 12.)),
            vec![0]
        );
        assert_eq!(
            objects_in_rect(&project, Point::new(4., 8.), Point::new(6., 12.)),
            vec![0]
        );
        assert!(objects_in_rect(&project, Point::new(45., 45.), Point::new(49., 49.)).is_empty());
    }

    #[test]
    fn running_paths_cross_marquee_without_endpoints_inside_and_remain_open() {
        let project = project(vec![Object {
            running_paths: vec![vec![
                Point::new(0., 0.),
                Point::new(20., 0.),
                Point::new(20., 20.),
            ]],
            ..Default::default()
        }]);
        assert_eq!(
            objects_in_rect(&project, Point::new(8., -1.), Point::new(12., 1.)),
            vec![0]
        );
        // No implicit diagonal between the last and first nodes.
        assert!(objects_in_rect(&project, Point::new(8., 8.), Point::new(12., 12.)).is_empty());
        // Touching a segment is included, without a stroke-width tolerance.
        assert_eq!(
            objects_in_rect(&project, Point::new(5., 0.), Point::new(8., 3.)),
            vec![0]
        );
    }

    #[test]
    fn bezier_and_satin_use_flattened_geometry_and_closed_satin_keeps_its_hole() {
        let project = project(vec![
            Object {
                bezier_segments: vec![CubicBezier {
                    start: Point::new(0., 0.),
                    control1: Point::new(0., 10.),
                    control2: Point::new(10., 10.),
                    end: Point::new(10., 0.),
                }],
                ..Default::default()
            },
            Object {
                satin_column: Some(SatinColumn {
                    left: vec![Point::new(20., 0.), Point::new(20., 10.)],
                    right: vec![Point::new(24., 0.), Point::new(24., 10.)],
                    ..Default::default()
                }),
                ..Default::default()
            },
            Object {
                satin_column: Some(SatinColumn {
                    left: square(40., 60.),
                    right: square(45., 55.),
                    closed: true,
                    ..Default::default()
                }),
                ..Default::default()
            },
        ]);
        assert_eq!(
            objects_in_rect(&project, Point::new(4., 7.), Point::new(6., 8.)),
            vec![0]
        );
        assert!(objects_in_rect(&project, Point::new(0., 8.), Point::new(1., 9.)).is_empty());
        assert_eq!(
            objects_in_rect(&project, Point::new(21., 3.), Point::new(23., 7.)),
            vec![1]
        );
        assert!(objects_in_rect(&project, Point::new(48., 48.), Point::new(52., 52.)).is_empty());
        assert_eq!(
            objects_in_rect(&project, Point::new(41., 48.), Point::new(43., 52.)),
            vec![2]
        );
    }

    #[test]
    fn invalid_marquee_and_malformed_geometry_do_not_select_phantom_edges() {
        let project = project(vec![area(vec![vec![
            Point::new(f64::NAN, 0.),
            Point::new(10., 0.),
            Point::new(10., 10.),
        ]])]);
        assert!(objects_in_rect(&project, Point::new(4., 4.), Point::new(6., 6.)).is_empty());
        assert!(
            objects_in_rect(&project, Point::new(f64::INFINITY, 4.), Point::new(6., 6.)).is_empty()
        );
        assert!(
            objects_in_rect(
                &Project::default(),
                Point::new(0., 0.),
                Point::new(10., 10.)
            )
            .is_empty()
        );
    }

    #[test]
    fn valid_rail_of_an_invalid_satin_column_remains_selectable() {
        let project = project(vec![Object {
            satin_column: Some(SatinColumn {
                left: vec![Point::new(0., 0.), Point::new(0., 0.)],
                right: vec![Point::new(10., 0.), Point::new(10., 10.)],
                ..Default::default()
            }),
            ..Default::default()
        }]);
        assert_eq!(
            objects_in_rect(&project, Point::new(9., 3.), Point::new(11., 7.)),
            vec![0]
        );
        assert!(objects_in_rect(&project, Point::new(4., 4.), Point::new(6., 6.)).is_empty());
    }

    #[test]
    fn zero_size_marquee_matches_a_point_on_geometry() {
        let rect = PickRect::new(Point::new(5., 5.), Point::new(5., 5.)).unwrap();
        assert!(rect.contains(Point::new(5., 5.)));
        assert!(rect.intersects_segment(Point::new(0., 0.), Point::new(10., 10.)));
        assert!(!rect.intersects_segment(Point::new(0., 1.), Point::new(10., 11.)));
    }
}
