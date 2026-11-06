//! Render-only geometry cache. Coordinates and colors remain document data.
//! Failed or over-budget fills retain outlines instead of inventing occupancy.
use eframe::egui::{self, Color32, Painter, Pos2, Rect, Vec2};
use embroidery_lab::{Point, Project};
use geo::{
    Area, BooleanOps, Coord, CoordsIter, LineString, MultiPolygon, Polygon, TriangulateEarcut,
    Validation,
};

const MAX_OBJECTS: usize = 4096;
const MAX_POINTS: usize = 100_000;
const MAX_CURVES: usize = 512;
const MAX_FILL_POINTS: usize = 16_384;
const MAX_FILL_RINGS: usize = 512;
const MAX_BOOLEAN_WORK: usize = 25_000_000;
const MAX_TRIANGLES: usize = 200_000;

#[derive(Clone, Debug)]
pub(super) struct SceneObject {
    pub id: u64,
    pub color: [u8; 3],
    pub rings: Vec<Vec<Point>>,
    pub triangles: Vec<[Point; 3]>,
    pub bounds: Option<(Point, Point)>,
}

#[derive(Clone, Debug, Default)]
pub(super) struct Scene {
    /// Visible objects, in the original sewing/layer order.
    pub objects: Vec<SceneObject>,
    pub bounds: Option<(Point, Point)>,
    /// Objects whose fill or geometry cache could not be safely constructed.
    pub issue_count: usize,
}

impl Scene {
    pub fn build(project: &Project) -> Self {
        let mut scene = Self::default();
        // Do not decode embedded artwork or validate stitch parameters here.
        // These limits bound only the untrusted geometry used by this cache.
        let raw_points = project.objects.iter().fold(0usize, |count, object| {
            count
                .saturating_add(object.contours.iter().map(Vec::len).sum::<usize>())
                .saturating_add(object.running_paths.iter().map(Vec::len).sum::<usize>())
                .saturating_add(
                    object
                        .satin_column
                        .as_ref()
                        .map_or(0, |c| c.left.len().saturating_add(c.right.len())),
                )
        });
        let curves = project.objects.iter().fold(0usize, |count, object| {
            count.saturating_add(object.bezier_segments.len())
        });
        let geometry_bounded = raw_points <= MAX_POINTS
            && curves <= MAX_CURVES
            && project.objects.len() <= MAX_OBJECTS;
        let mut remaining_points = MAX_POINTS;
        let mut remaining_triangles = MAX_TRIANGLES;
        for object in project
            .objects
            .iter()
            .filter(|o| o.visible)
            .take(MAX_OBJECTS)
        {
            let coordinates_valid = object
                .contours
                .iter()
                .flatten()
                .chain(object.running_paths.iter().flatten())
                .all(finite)
                && object.bezier_segments.iter().all(|curve| {
                    [curve.start, curve.control1, curve.control2, curve.end]
                        .iter()
                        .all(finite)
                });
            let mut issue = !geometry_bounded || !coordinates_valid;
            let mut rings = if !coordinates_valid {
                Vec::new()
            } else if geometry_bounded {
                object.flattened_contours()
            } else {
                // Preserve complete raw rings when an untrusted input exceeds
                // limits; never close a truncated ring or approximate a curve.
                let mut budget = remaining_points;
                let mut outlines = Vec::new();
                for ring in &object.contours {
                    if ring.len() <= budget {
                        budget -= ring.len();
                        outlines.push(ring.clone());
                    }
                }
                outlines
            };
            let points = rings.iter().map(Vec::len).sum::<usize>();
            if points > remaining_points || rings.iter().flatten().any(|p| !finite(p)) {
                rings.clear();
                issue = true;
            } else {
                remaining_points -= points;
            }
            let expected_rings = object.contours.len()
                + usize::from(!object.bezier_segments.is_empty())
                + object
                    .satin_column
                    .as_ref()
                    .map_or(0, |column| if column.closed { 2 } else { 1 });
            if rings.len() < expected_rings {
                issue = true;
            }
            let bounds = point_bounds(
                rings
                    .iter()
                    .flatten()
                    .chain(object.running_paths.iter().flatten())
                    .filter(|p| finite(p))
                    .copied(),
            );
            let triangles = if issue || rings.is_empty() {
                Vec::new()
            } else {
                match triangulate(&rings, remaining_triangles) {
                    Some(triangles) => {
                        remaining_triangles -= triangles.len();
                        triangles
                    }
                    None => {
                        issue = true;
                        Vec::new()
                    }
                }
            };
            scene.issue_count += usize::from(issue);
            scene.bounds = merge_bounds(scene.bounds, bounds);
            scene.objects.push(SceneObject {
                id: object.id,
                color: object.color,
                rings,
                triangles,
                bounds,
            });
        }
        scene.issue_count += project
            .objects
            .iter()
            .filter(|o| o.visible)
            .count()
            .saturating_sub(MAX_OBJECTS);
        scene
    }

    pub fn paint_fills(&self, painter: &Painter, center: Pos2, scale: f32, clip: Rect) {
        if !center.x.is_finite() || !center.y.is_finite() || !scale.is_finite() || scale <= 0. {
            return;
        }
        let painter = painter.with_clip_rect(clip);
        let screen =
            |point: Point| center + Vec2::new(point.x as f32 * scale, point.y as f32 * scale);
        for object in &self.objects {
            if object.triangles.is_empty()
                || object.bounds.is_some_and(|(min, max)| {
                    !Rect::from_two_pos(screen(min), screen(max)).intersects(clip)
                })
            {
                continue;
            }
            let mut mesh = egui::Mesh::default();
            let color = Color32::from_rgb(object.color[0], object.color[1], object.color[2]);
            for triangle in &object.triangles {
                let first = mesh.vertices.len() as u32;
                for point in triangle {
                    mesh.colored_vertex(screen(*point), color);
                }
                mesh.add_triangle(first, first + 1, first + 2);
            }
            painter.add(egui::Shape::mesh(mesh));
        }
    }
}

fn finite(point: &Point) -> bool {
    point.x.is_finite() && point.y.is_finite() && point.x.abs() <= 9990. && point.y.abs() <= 9990.
}

fn point_bounds(points: impl Iterator<Item = Point>) -> Option<(Point, Point)> {
    points.fold(None, |bounds, point| {
        merge_bounds(bounds, Some((point, point)))
    })
}

fn merge_bounds(a: Option<(Point, Point)>, b: Option<(Point, Point)>) -> Option<(Point, Point)> {
    match (a, b) {
        (Some((a0, a1)), Some((b0, b1))) => Some((
            Point::new(a0.x.min(b0.x), a0.y.min(b0.y)),
            Point::new(a1.x.max(b1.x), a1.y.max(b1.y)),
        )),
        (a, None) => a,
        (None, b) => b,
    }
}

fn triangulate(rings: &[Vec<Point>], triangle_budget: usize) -> Option<Vec<[Point; 3]>> {
    let points = rings.iter().map(Vec::len).sum::<usize>();
    if points > MAX_FILL_POINTS || rings.len() > MAX_FILL_RINGS {
        return None;
    }
    // Geo's Earcut adapter unwraps its internal result. An invalid user draft
    // must not bring down the editor; retain outlines on any failed operation.
    std::panic::catch_unwind(|| {
        let mut area = MultiPolygon::new(Vec::new());
        let mut work = 0usize;
        for ring in rings {
            if ring.len() < 3 {
                return None;
            }
            let polygon = Polygon::new(
                LineString::new(ring.iter().map(|p| Coord { x: p.x, y: p.y }).collect()),
                Vec::new(),
            );
            polygon.check_validation().ok()?;
            if polygon.unsigned_area() <= 1e-12 {
                return None;
            }
            work = work.saturating_add(area.coords_count().max(1).saturating_mul(ring.len()));
            if work > MAX_BOOLEAN_WORK {
                return None;
            }
            // XOR is the model's even-odd rule, independent of ring winding,
            // nesting order, islands, or overlap between editable rings.
            area = area.xor(&polygon);
            if area.coords_count() > MAX_FILL_POINTS {
                return None;
            }
        }
        area.check_validation().ok()?;
        let expected_area = area.unsigned_area();
        let mut triangles = Vec::new();
        let mut actual_area = 0.;
        for polygon in &area {
            for triangle in polygon.earcut_triangles_iter() {
                if triangles.len() >= triangle_budget {
                    return None;
                }
                let [a, b, c] = triangle.to_array().map(|p| Point::new(p.x, p.y));
                if ![a, b, c].iter().all(finite) {
                    return None;
                }
                actual_area += ((b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x)).abs() * 0.5;
                triangles.push([a, b, c]);
            }
        }
        if (actual_area - expected_area).abs() > (expected_area.abs() * 1e-8).max(1e-8) {
            return None;
        }
        Some(triangles)
    })
    .ok()
    .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;
    use embroidery_lab::{Object, Settings, StitchKind};

    fn square(x: f64, y: f64, side: f64) -> Vec<Point> {
        vec![
            Point::new(x, y),
            Point::new(x + side, y),
            Point::new(x + side, y + side),
            Point::new(x, y + side),
        ]
    }

    fn covered(triangles: &[[Point; 3]], p: Point) -> bool {
        triangles.iter().any(|triangle| {
            let cross = |a: Point, b: Point| (b.x - a.x) * (p.y - a.y) - (b.y - a.y) * (p.x - a.x);
            let signs = [
                cross(triangle[0], triangle[1]),
                cross(triangle[1], triangle[2]),
                cross(triangle[2], triangle[0]),
            ];
            signs.iter().all(|&v| v >= -1e-9) || signs.iter().all(|&v| v <= 1e-9)
        })
    }

    fn triangle_area(triangles: &[[Point; 3]]) -> f64 {
        triangles
            .iter()
            .map(|[a, b, c]| ((b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x)).abs() * 0.5)
            .sum()
    }

    #[test]
    fn cached_fill_preserves_holes_nested_islands_color_and_source_geometry() {
        let project = Project {
            objects: vec![Object {
                id: 7,
                color: [240, 240, 240],
                contours: vec![square(4., 4., 2.), square(0., 0., 10.), square(2., 2., 6.)],
                ..Default::default()
            }],
            ..Default::default()
        };
        let before = serde_json::to_value(&project).unwrap();
        let scene = Scene::build(&project);
        assert_eq!(scene.issue_count, 0);
        let object = &scene.objects[0];
        assert_eq!(object.id, 7);
        assert_eq!(object.color, [240, 240, 240]);
        assert!((triangle_area(&object.triangles) - 68.).abs() < 1e-8);
        assert!(covered(&object.triangles, Point::new(1., 1.)));
        assert!(!covered(&object.triangles, Point::new(3., 3.)));
        assert!(covered(&object.triangles, Point::new(5., 5.)));
        assert_eq!(
            scene.bounds,
            Some((Point::new(0., 0.), Point::new(10., 10.)))
        );
        assert_eq!(serde_json::to_value(&project).unwrap(), before);
    }

    #[test]
    fn overlapping_rings_use_even_odd_occupancy_and_invalid_drafts_keep_outlines() {
        let project = Project {
            objects: vec![Object {
                contours: vec![square(0., 0., 10.), square(5., 0., 10.)],
                ..Default::default()
            }],
            ..Default::default()
        };
        let scene = Scene::build(&project);
        assert_eq!(scene.issue_count, 0);
        let triangles = &scene.objects[0].triangles;
        assert!((triangle_area(triangles) - 100.).abs() < 1e-8);
        assert!(covered(triangles, Point::new(2., 5.)) && covered(triangles, Point::new(12., 5.)));
        assert!(!covered(triangles, Point::new(7., 5.)));

        let mut invalid = project;
        invalid.objects[0].contours = vec![vec![
            Point::new(0., 0.),
            Point::new(10., 10.),
            Point::new(0., 10.),
            Point::new(10., 0.),
        ]];
        let scene = Scene::build(&invalid);
        assert_eq!(scene.issue_count, 1);
        assert!(scene.objects[0].triangles.is_empty());
        assert_eq!(scene.objects[0].rings, invalid.objects[0].contours);
    }

    #[test]
    fn hidden_objects_are_excluded_and_open_running_paths_are_not_filled() {
        let project = Project {
            objects: vec![
                Object {
                    visible: false,
                    contours: vec![square(100., 100., 10.)],
                    ..Default::default()
                },
                Object {
                    id: 2,
                    running_paths: vec![vec![Point::new(0., 0.), Point::new(5., 5.)]],
                    settings: Settings {
                        kind: StitchKind::Running,
                        ..Default::default()
                    },
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let scene = Scene::build(&project);
        assert_eq!(scene.issue_count, 0);
        assert_eq!(scene.objects.len(), 1);
        assert!(scene.objects[0].triangles.is_empty() && scene.objects[0].rings.is_empty());
        assert_eq!(scene.bounds, Some((Point::new(0., 0.), Point::new(5., 5.))));
    }
}
