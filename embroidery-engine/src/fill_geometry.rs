//! Fill geometry in millimetres. Artwork, directional compensation and inset
//! underlay are deliberately separate polygons. No artwork nodes are modified.
use crate::model::Point;
use geo::algorithm::{
    buffer::{BufferStyle, LineJoin},
    unary_union,
};
use geo::{
    Area, BooleanOps, BoundingRect, Buffer, Coord, Distance, Euclidean, Intersects, LineString,
    MultiPolygon, Polygon, Rect, Relate, Validation,
};

struct CompensationRegion {
    name: String,
    index: usize,
    original: Polygon<f64>,
    effective: MultiPolygon<f64>,
    bounds: Rect<f64>,
    nodes: usize,
    expanded: bool,
}

/// Reject newly closed gaps without editing either component. Existing shared
/// edges and intentional overlapping layers keep their established policy.
/// The all-zero-compensation path allocates and compares no region geometry.
pub(super) struct CompensationGuard {
    enabled: bool,
    regions: Vec<CompensationRegion>,
    pair_checks: usize,
    exact_work: usize,
}
impl CompensationGuard {
    pub(super) fn new(enabled: bool) -> Self {
        Self {
            enabled,
            regions: Vec::new(),
            pair_checks: 0,
            exact_work: 0,
        }
    }
    pub(super) fn enabled(&self) -> bool {
        self.enabled
    }
    pub(super) fn register(
        &mut self,
        name: &str,
        index: usize,
        original: &Polygon<f64>,
        effective: &MultiPolygon<f64>,
        expanded: bool,
    ) -> Result<(), String> {
        if !self.enabled {
            return Ok(());
        }
        let count = |p: &Polygon<f64>| {
            p.exterior().0.len() + p.interiors().iter().map(|r| r.0.len()).sum::<usize>()
        };
        let nodes = count(original) + effective.0.iter().map(count).sum::<usize>();
        let bounds = effective
            .bounding_rect()
            .ok_or("Empty compensated fill region")?;
        let bounds_touch = |a: Rect<f64>, b: Rect<f64>| {
            a.min().x <= b.max().x + 1e-9
                && a.max().x + 1e-9 >= b.min().x
                && a.min().y <= b.max().y + 1e-9
                && a.max().y + 1e-9 >= b.min().y
        };
        for previous in &self.regions {
            self.pair_checks = self.pair_checks.saturating_add(1);
            if self.pair_checks > 2_000_000 {
                return Err("Compensation gap checks exceed the geometry budget; reduce compensation, simplify or split the project".into());
            }
            if !(expanded || previous.expanded) || !bounds_touch(bounds, previous.bounds) {
                continue;
            }
            self.exact_work = self
                .exact_work
                .saturating_add(nodes.saturating_mul(previous.nodes));
            if self.exact_work > 50_000_000 {
                return Err("Compensation intersections exceed the geometry budget; reduce compensation, simplify or split the project".into());
            }
            // Boundary contact is an existing adjacency, not a newly lost gap.
            // Numerical slack is subnanometre, independent of visual thickness.
            if original.intersects(&previous.original)
                || Euclidean.distance(original, &previous.original) <= 1e-9
            {
                continue;
            }
            let contact = effective.0.iter().any(|a| {
                previous
                    .effective
                    .0
                    .iter()
                    .any(|b| a.intersects(b) || Euclidean.distance(a, b) <= 1e-9)
            });
            if contact {
                return Err(format!(
                    "{name} component {}: pull compensation closes an original gap to {} component {}. Reduce pull compensation or widen the gap; originally separate fill regions must not newly touch or overlap.",
                    index + 1,
                    previous.name,
                    previous.index + 1
                ));
            }
        }
        self.regions.push(CompensationRegion {
            name: name.into(),
            index,
            original: original.clone(),
            effective: effective.clone(),
            bounds,
            nodes,
            expanded,
        });
        Ok(())
    }
}

fn line(points: impl IntoIterator<Item = Point>) -> LineString<f64> {
    LineString::new(
        points
            .into_iter()
            .map(|p| Coord { x: p.x, y: p.y })
            .collect(),
    )
}
pub(crate) fn components(rings: &[Vec<Point>]) -> Result<Vec<Polygon<f64>>, String> {
    let nodes = rings.iter().map(Vec::len).sum::<usize>();
    if nodes > 100_000 || nodes.saturating_mul(rings.len()) > 100_000_000 {
        return Err(
            "Fill component decomposition exceeds geometry budget; simplify or split the object"
                .into(),
        );
    }
    let mut polygons = Vec::new();
    for ring in rings {
        if ring.len() < 3 {
            return Err("A fill ring requires at least three vertices".into());
        }
        let polygon = Polygon::new(line(ring.iter().copied()), Vec::new());
        polygon
            .check_validation()
            .map_err(|e| format!("Invalid fill contour: {e}"))?;
        if polygon.unsigned_area() < 1e-10 {
            return Err("Fill contour has zero area".into());
        }
        polygons.push(polygon);
    }
    // Traced boundaries can meet at a grid vertex. A sequential Boolean XOR
    // may merge two valid tangent hole cycles into a self-touching outer walk.
    // Assemble laminar rings by full polygon containment first, preserving
    // every original edge and the exact even-odd occupancy at tangent points.
    let areas: Vec<_> = polygons.iter().map(Area::unsigned_area).collect();
    let bounds: Vec<_> = polygons
        .iter()
        .map(|p| {
            p.exterior().0.iter().fold(
                (
                    f64::INFINITY,
                    f64::INFINITY,
                    f64::NEG_INFINITY,
                    f64::NEG_INFINITY,
                ),
                |(x0, y0, x1, y1), p| (x0.min(p.x), y0.min(p.y), x1.max(p.x), y1.max(p.y)),
            )
        })
        .collect();
    let mut parents = vec![None; polygons.len()];
    let mut work = 0usize;
    for i in 0..polygons.len() {
        let mut parent_area = f64::INFINITY;
        for j in 0..polygons.len() {
            if i == j || areas[j] <= areas[i] + 1e-10 || areas[j] >= parent_area {
                continue;
            }
            let (a, b) = (bounds[i], bounds[j]);
            if a.0 < b.0 - 1e-9 || a.1 < b.1 - 1e-9 || a.2 > b.2 + 1e-9 || a.3 > b.3 + 1e-9 {
                continue;
            }
            work = work.saturating_add(
                polygons[i]
                    .exterior()
                    .0
                    .len()
                    .saturating_mul(polygons[j].exterior().0.len()),
            );
            if work > 100_000_000 {
                return Err(
                    "Fill nesting exceeds geometry budget; simplify or split the object".into(),
                );
            }
            if polygons[j].relate(&polygons[i]).is_contains() {
                parents[i] = Some(j);
                parent_area = areas[j];
            }
        }
    }
    let depths: Vec<_> = (0..polygons.len())
        .map(|i| {
            let mut parent = parents[i];
            let mut depth = 0usize;
            while let Some(index) = parent {
                depth += 1;
                parent = parents[index];
            }
            depth
        })
        .collect();
    let nested = MultiPolygon::new(
        polygons
            .iter()
            .enumerate()
            .filter(|(i, _)| depths[*i] % 2 == 0)
            .map(|(i, polygon)| {
                Polygon::new(
                    polygon.exterior().clone(),
                    polygons
                        .iter()
                        .enumerate()
                        .filter(|(j, _)| parents[*j] == Some(i))
                        .map(|(_, p)| p.exterior().clone())
                        .collect(),
                )
            })
            .collect(),
    );
    if nested.check_validation().is_ok() {
        return Ok(nested.0);
    }
    // Overlapping, non-laminar editable rings still use their defined even-odd
    // Boolean semantics; invalid output is rejected without deleting geometry.
    let mut area = MultiPolygon::new(Vec::new());
    for polygon in polygons {
        area = area.xor(&polygon);
    }
    area.check_validation()
        .map_err(|e| format!("Invalid even-odd fill region: {e}"))?;
    Ok(area.0)
}
pub(crate) fn rings(area: &MultiPolygon<f64>) -> Vec<Vec<Point>> {
    area.0
        .iter()
        .flat_map(|p| std::iter::once(p.exterior()).chain(p.interiors()))
        .map(|ring| {
            let mut points: Vec<_> = ring.0.iter().map(|p| Point::new(p.x, p.y)).collect();
            if points.first() == points.last() {
                points.pop();
            }
            points
        })
        .filter(|r| r.len() >= 3)
        .collect()
}
pub(super) fn polygon_rings(polygon: &Polygon<f64>) -> Vec<Vec<Point>> {
    rings(&MultiPolygon::new(vec![polygon.clone()]))
}

/// Exact polygonal sweep by a horizontal segment in the fill coordinate frame.
/// Only the exterior shell expands; protected hole geometry is subtracted
/// unchanged. Compensation is millimetres *per side* along the fill direction.
pub(super) fn compensated(
    component: &Polygon<f64>,
    angle: f64,
    amount: f64,
) -> Result<MultiPolygon<f64>, String> {
    if amount == 0. {
        return Ok(MultiPolygon::new(vec![component.clone()]));
    }
    let (dx, dy) = (
        angle.to_radians().cos() * amount,
        angle.to_radians().sin() * amount,
    );
    let shifted = |p: &Coord<f64>, sign: f64| Point::new(p.x + sign * dx, p.y + sign * dy);
    let mut pieces = vec![
        Polygon::new(
            line(component.exterior().0.iter().map(|p| shifted(p, -1.))),
            Vec::new(),
        ),
        Polygon::new(
            line(component.exterior().0.iter().map(|p| shifted(p, 1.))),
            Vec::new(),
        ),
    ];
    for edge in component.exterior().0.windows(2) {
        let points = [
            shifted(&edge[0], -1.),
            shifted(&edge[1], -1.),
            shifted(&edge[1], 1.),
            shifted(&edge[0], 1.),
        ];
        let piece = Polygon::new(line(points), Vec::new());
        if piece.unsigned_area() > 1e-12 {
            pieces.push(piece);
        }
    }
    // unary_union assumes consistently oriented inputs.
    use geo::{Orient, orient::Direction};
    let pieces: Vec<_> = pieces
        .iter()
        .map(|p| p.orient(Direction::Default))
        .collect();
    let exterior_area = unary_union(&pieces);
    let mut boundary_rings = rings(&exterior_area);
    boundary_rings.extend(component.interiors().iter().map(|hole| {
        hole.0[..hole.0.len() - 1]
            .iter()
            .map(|p| Point::new(p.x, p.y))
            .collect()
    }));
    // Preserve distinct tangent holes rather than asking sequential Boolean
    // differences to merge them into a self-touching interior walk.
    let area = MultiPolygon::new(components(&boundary_rings)?);
    area.check_validation()
        .map_err(|e| format!("Invalid compensated fill region: {e}"))?;
    Ok(area)
}

/// Euclidean inset is restricted to underlay, never used as pull compensation.
pub(super) fn inset(component: &Polygon<f64>, amount: f64) -> Result<MultiPolygon<f64>, String> {
    if amount == 0. {
        return Ok(MultiPolygon::new(vec![component.clone()]));
    }
    let area =
        component.buffer_with_style(BufferStyle::new(-amount).line_join(LineJoin::Miter(0.2)));
    area.check_validation()
        .map_err(|e| format!("Invalid underlay inset region: {e}"))?;
    Ok(area)
}

/// Filled-area covariance is invariant to adding collinear boundary vertices.
pub(super) fn area_angle(rings_input: &[Vec<Point>]) -> Option<f64> {
    let components = components(rings_input).ok()?;
    let origin = *rings_input.first()?.first()?;
    let (mut mass, mut mx, mut my, mut xx, mut yy, mut xy) = (0., 0., 0., 0., 0., 0.);
    for component in components {
        for (index, ring) in std::iter::once(component.exterior())
            .chain(component.interiors())
            .enumerate()
        {
            let (mut a, mut sx, mut sy, mut sxx, mut syy, mut sxy) = (0., 0., 0., 0., 0., 0.);
            for edge in ring.0.windows(2) {
                let (x, y) = (edge[0].x - origin.x, edge[0].y - origin.y);
                let (u, v) = (edge[1].x - origin.x, edge[1].y - origin.y);
                let cross = x * v - u * y;
                a += cross / 2.;
                sx += (x + u) * cross / 6.;
                sy += (y + v) * cross / 6.;
                sxx += (x * x + x * u + u * u) * cross / 12.;
                syy += (y * y + y * v + v * v) * cross / 12.;
                sxy += (2. * x * y + x * v + u * y + 2. * u * v) * cross / 24.;
            }
            let sign = a.signum() * if index == 0 { 1. } else { -1. };
            mass += a * sign;
            mx += sx * sign;
            my += sy * sign;
            xx += sxx * sign;
            yy += syy * sign;
            xy += sxy * sign;
        }
    }
    if mass <= 1e-10 {
        return None;
    }
    xx -= mx * mx / mass;
    yy -= my * my / mass;
    xy -= mx * my / mass;
    if (xx - yy).hypot(2. * xy) <= 1e-10 * (xx + yy).abs().max(1.) {
        return Some(0.);
    }
    Some((0.5 * (2. * xy).atan2(xx - yy)).to_degrees())
}
