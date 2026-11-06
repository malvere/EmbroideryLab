//! Deterministic generation from editable millimetre geometry. Complex fill
//! uses rotated even-odd clipping; protected holes retain non-sewing transfers.
//! Annotated passes are normalized once for preview, diagnostics and export.
use crate::model::*;
#[path = "fill_geometry.rs"]
pub(crate) mod fill_geometry;
#[path = "layering.rs"]
pub(crate) mod layering;
#[path = "travel.rs"]
mod travel;
const MAX_COMMANDS: usize = 1_000_000;
const MAX_SPANS: usize = 250_000;
type SewingSegment = (Point, Point, StitchKind);
#[derive(Clone, Copy)]
struct PassOptions {
    modern_satin: bool,
    role: StitchRole,
    initial_jump: bool,
}

fn rotate(p: Point, angle: f64) -> Point {
    let (c, s) = (angle.cos(), angle.sin());
    Point::new(p.x * c - p.y * s, p.x * s + p.y * c)
}
fn inside(p: Point, rings: &[Vec<Point>]) -> bool {
    let mut value = false;
    for ring in rings {
        for (a, b) in ring
            .iter()
            .zip(ring.iter().cycle().skip(1))
            .take(ring.len())
        {
            if (a.y > p.y) != (b.y > p.y) && p.x < (b.x - a.x) * (p.y - a.y) / (b.y - a.y) + a.x {
                value = !value;
            }
        }
    }
    value
}
fn safe_connector(a: Point, b: Point, rings: &[Vec<Point>]) -> bool {
    // Reject every interior intersection with a boundary, including very narrow
    // holes that a sampled containment check could skip.
    let cross = |a: Point, b: Point| a.x * b.y - a.y * b.x;
    let r = Point::new(b.x - a.x, b.y - a.y);
    for ring in rings {
        for (&c, &d) in ring
            .iter()
            .zip(ring.iter().cycle().skip(1))
            .take(ring.len())
        {
            let s = Point::new(d.x - c.x, d.y - c.y);
            let denom = cross(r, s);
            if denom.abs() > 1e-10 {
                let ca = Point::new(c.x - a.x, c.y - a.y);
                let t = cross(ca, s) / denom;
                let u = cross(ca, r) / denom;
                if (1e-7..1. - 1e-7).contains(&t) && (-1e-7..=1. + 1e-7).contains(&u) {
                    return false;
                }
            }
        }
    }
    let middle = a.lerp(b, 0.5);
    if inside(middle, rings) {
        return true;
    }
    // A connector lying on the outer contour is a valid edge run. The parity
    // test alone classifies one of two opposite rectangle edges as outside.
    rings.iter().any(|ring| {
        ring.iter()
            .zip(ring.iter().cycle().skip(1))
            .take(ring.len())
            .any(|(&c, &d)| {
                let length = c.distance(d);
                length > 1e-9 && (c.distance(middle) + middle.distance(d) - length).abs() < 1e-8
            })
    })
}

/// Diagnostic containment tolerance is the maximum endpoint displacement on
/// the 0.1mm grid (sqrt(2)*0.05mm), plus numerical slack. It is independent of
/// line thickness in either renderer.
pub(crate) fn diagnostic_regions(
    object: &Object,
    role: StitchRole,
    layering: Option<&layering::LayeringPlan<'_>>,
) -> Result<Vec<Vec<Vec<Point>>>, String> {
    use geo::Buffer;
    let rings = object.flattened_contours();
    let angle = if object.settings.auto_angle {
        automatic_angle_choice(
            &rings,
            object.settings.spacing_mm,
            object.settings.length_mm,
        )
        .0
    } else {
        object.settings.angle_deg
    };
    fill_geometry::components(&rings)?
        .iter()
        .map(|component| {
            let area = if role == StitchRole::Underlay {
                fill_geometry::inset(component, object.settings.underlay_inset_mm)?
            } else if object.settings.kind == StitchKind::Tatami {
                fill_geometry::compensated(component, angle, object.settings.pull_comp_mm)?
            } else {
                geo::MultiPolygon::new(vec![component.clone()])
            };
            let area = if let Some(layering) = layering {
                layering.clip(object, &area)?
            } else {
                area
            };
            Ok(fill_geometry::rings(&area.buffer(0.071)))
        })
        .collect()
}
pub(crate) fn diagnostic_connector(a: Point, b: Point, regions: &[Vec<Vec<Point>>]) -> bool {
    regions.iter().any(|rings| safe_connector(a, b, rings))
}
pub(crate) fn diagnostic_inside(point: Point, rings: &[Vec<Point>]) -> bool {
    inside(point, rings)
}

struct ArcRail<'a> {
    points: &'a [Point],
    distances: Vec<f64>,
    length: f64,
}
impl<'a> ArcRail<'a> {
    fn new(points: &'a [Point]) -> Result<Self, String> {
        if !(2..=8193).contains(&points.len()) {
            return Err("Flattened satin rail requires 2–8193 nodes".into());
        }
        if points
            .iter()
            .any(|p| !p.x.is_finite() || !p.y.is_finite() || p.x.abs() > 9990. || p.y.abs() > 9990.)
        {
            return Err("Satin rail contains invalid coordinates".into());
        }
        let mut distances = vec![0.];
        let mut length = 0.;
        for pair in points.windows(2) {
            let step = pair[0].distance(pair[1]);
            if step < 1e-7 {
                return Err("Satin rail has repeated consecutive nodes".into());
            }
            length += step;
            distances.push(length);
        }
        if length < 0.15 {
            return Err("Satin rail is shorter than 0.15mm".into());
        }
        Ok(Self {
            points,
            distances,
            length,
        })
    }
    fn sample(&self, t: f64) -> Point {
        if t <= 0. {
            return self.points[0];
        }
        if t >= 1. {
            return self.points[self.points.len() - 1];
        }
        let distance = t * self.length;
        let end = self
            .distances
            .partition_point(|&d| d < distance)
            .clamp(1, self.points.len() - 1);
        let fraction =
            (distance - self.distances[end - 1]) / (self.distances[end] - self.distances[end - 1]);
        self.points[end - 1].lerp(self.points[end], fraction)
    }
}
fn signed_area(ring: &[Point]) -> f64 {
    ring.iter()
        .zip(ring.iter().cycle().skip(1))
        .take(ring.len())
        .map(|(a, b)| a.x * b.y - b.x * a.y)
        .sum::<f64>()
        * 0.5
}
fn segments_touch(a: Point, b: Point, c: Point, d: Point) -> bool {
    let orient =
        |a: Point, b: Point, c: Point| (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x);
    let on_segment = |a: Point, b: Point, p: Point| {
        orient(a, b, p).abs() < 1e-8
            && p.x >= a.x.min(b.x) - 1e-8
            && p.x <= a.x.max(b.x) + 1e-8
            && p.y >= a.y.min(b.y) - 1e-8
            && p.y <= a.y.max(b.y) + 1e-8
    };
    let (ab_c, ab_d, cd_a, cd_b) = (
        orient(a, b, c),
        orient(a, b, d),
        orient(c, d, a),
        orient(c, d, b),
    );
    (ab_c * ab_d < -1e-16 && cd_a * cd_b < -1e-16)
        || on_segment(a, b, c)
        || on_segment(a, b, d)
        || on_segment(c, d, a)
        || on_segment(c, d, b)
}

/// Arc-length correspondence with optional monotone direction rungs. Closed
/// columns retain two independent boundary rings, including their unsewn hole.
pub fn satin_stations(column: &SatinColumn, spacing: f64) -> Result<Vec<(Point, Point)>, String> {
    if !spacing.is_finite() || !(0.15..=20.).contains(&spacing) {
        return Err("Satin spacing must be 0.15–20mm".into());
    }
    if !column.seam.is_finite() || !(0.0..1.0).contains(&column.seam) {
        return Err("Satin seam must be a normalized position in [0, 1)".into());
    }
    let mut flat = column.flattened()?;
    let mut anchors = satin_anchors(column)?;
    if column.closed {
        let right_seam = map_satin_fraction(&anchors, column.seam);
        flat.left = crate::satin_edit::rotate_closed(&flat.left, column.seam)?;
        flat.right = crate::satin_edit::rotate_closed(&flat.right, right_seam)?;
        anchors = anchors
            .into_iter()
            .filter(|&(l, _)| l < 1. - 1e-9)
            .filter_map(|(l, r)| {
                let l = (l - column.seam).rem_euclid(1.);
                let r = (r - right_seam).rem_euclid(1.);
                (l > 1e-9 && l < 1. - 1e-9).then_some((l, r))
            })
            .collect();
        anchors.push((0., 0.));
        anchors.push((1., 1.));
        anchors.sort_by(|a, b| a.0.total_cmp(&b.0));
    }
    let left = ArcRail::new(&flat.left)?;
    let right = ArcRail::new(&flat.right)?;
    let ratio = left.length.min(right.length) / left.length.max(right.length);
    if ratio < 0.2 {
        return Err(
            "Satin rail length ratio is below 0.2; split the tight or ambiguous column".into(),
        );
    }
    let mut positions = vec![(0., 0.)];
    for pair in anchors.windows(2) {
        let ((la, ra), (lb, rb)) = (pair[0], pair[1]);
        let ratio = ((lb - la) * left.length).max((rb - ra) * right.length) / (spacing * 0.5);
        // Hand-positioned rungs frequently land exactly one station apart.
        // Keep a few arithmetic ulps from doubling that interval's density.
        let count = (ratio - if column.uses_v4() { 1e-10 } else { 0. })
            .ceil()
            .max(1.) as usize;
        if positions.len().saturating_add(count) > 100_001 {
            return Err(
                "Satin column exceeds 100000 stations; increase spacing or split it".into(),
            );
        }
        positions.extend((1..=count).map(|i| {
            let t = i as f64 / count as f64;
            (la + (lb - la) * t, ra + (rb - ra) * t)
        }));
    }
    let count = positions.len() - 1;
    let rings = if column.closed {
        vec![
            flat.left[..flat.left.len() - 1].to_vec(),
            flat.right[..flat.right.len() - 1].to_vec(),
        ]
    } else {
        vec![flat.boundary()]
    };
    let nodes = rings.iter().map(Vec::len).sum::<usize>();
    let validation_count = count.saturating_add(nodes).saturating_add(1);
    if count > 100_000
        || nodes
            .saturating_mul(nodes)
            .saturating_add(validation_count.saturating_mul(nodes).saturating_mul(3))
            > 100_000_000
    {
        return Err(
            "Satin column exceeds geometry work budget; simplify rails or increase spacing".into(),
        );
    }
    for (r, ring) in rings.iter().enumerate() {
        for i in 0..ring.len() {
            for (s, other) in rings.iter().enumerate().skip(r) {
                for j in 0..other.len() {
                    if r == s && (j <= i + 1 || (i == 0 && j == ring.len() - 1)) {
                        continue;
                    }
                    if segments_touch(
                        ring[i],
                        ring[(i + 1) % ring.len()],
                        other[j],
                        other[(j + 1) % other.len()],
                    ) {
                        return Err("Satin rails cross, self-touch, or run in opposite directions; split or correct the column".into());
                    }
                }
            }
        }
    }
    if rings.iter().any(|ring| signed_area(ring).abs() < 1e-5) {
        return Err("Satin column has no usable area".into());
    }
    // Add rail vertex stations for validation, without adding needle punctures:
    // the width vector is linear between these breakpoints, allowing an exact
    // minimum-width check throughout each piece of the correspondence.
    let mut fractions: Vec<f64> = positions.iter().map(|p| p.0).collect();
    fractions.extend(left.distances.iter().map(|d| d / left.length));
    let inverse: Vec<_> = anchors.iter().map(|&(l, r)| (r, l)).collect();
    fractions.extend(
        right
            .distances
            .iter()
            .map(|d| map_satin_fraction(&inverse, d / right.length)),
    );
    fractions.sort_by(f64::total_cmp);
    fractions.dedup_by(|a, b| (*a - *b).abs() < 1e-10);
    let mut previous: Option<(Point, Point)> = None;
    let mut orientation: Option<f64> = None;
    for t in fractions {
        let pair = (
            left.sample(t),
            right.sample(map_satin_fraction(&anchors, t)),
        );
        if pair.0.distance(pair.1) > 12. + 1e-8 {
            return Err(
                "Satin width exceeds 12mm; split the column constructively or use Tatami".into(),
            );
        }
        if pair.0.distance(pair.1) < 0.15 - 1e-9 || !safe_connector(pair.0, pair.1, &rings) {
            return Err("Satin correspondence leaves the column or width is below 0.15mm; split or correct rails".into());
        }
        if let Some((a, b)) = previous {
            let v = Point::new(b.x - a.x, b.y - a.y);
            let end = Point::new(pair.1.x - pair.0.x, pair.1.y - pair.0.y);
            let dv = Point::new(end.x - v.x, end.y - v.y);
            let norm = dv.x * dv.x + dv.y * dv.y;
            let u = if norm > 1e-12 {
                (-(v.x * dv.x + v.y * dv.y) / norm).clamp(0., 1.)
            } else {
                0.
            };
            let area = signed_area(&[a, pair.0, pair.1, b]);
            let sign = *orientation.get_or_insert(area.signum());
            if v.lerp(end, u).distance(Point::default()) < 0.15 - 1e-9
                || segments_touch(a, b, pair.0, pair.1)
                || area * sign <= 1e-12
            {
                return Err(
                    "Satin cross-sections fold or collapse; split the column at the corner".into(),
                );
            }
        }
        previous = Some(pair);
    }
    Ok(positions
        .into_iter()
        .map(|(l, r)| (left.sample(l), right.sample(r)))
        .collect())
}

fn satin_anchors(column: &SatinColumn) -> Result<Vec<(f64, f64)>, String> {
    if column.rungs.len() > 1024 {
        return Err("Satin column exceeds 1024 direction rungs".into());
    }
    let mut anchors = vec![(0., 0.)];
    for rung in &column.rungs {
        if !rung.left.is_finite()
            || !rung.right.is_finite()
            || !(0.0..1.0).contains(&rung.left)
            || !(0.0..1.0).contains(&rung.right)
            || rung.left < 1e-8
            || rung.right < 1e-8
        {
            return Err("Satin rungs require interior normalized positions in (0, 1)".into());
        }
        anchors.push((rung.left, rung.right));
    }
    anchors.sort_by(|a, b| a.0.total_cmp(&b.0));
    anchors.push((1., 1.));
    if anchors
        .windows(2)
        .any(|p| p[1].0 - p[0].0 <= 1e-8 || p[1].1 - p[0].1 <= 1e-8)
    {
        return Err("Satin rungs cross or repeat: they must advance along both rails".into());
    }
    Ok(anchors)
}

fn map_satin_fraction(anchors: &[(f64, f64)], value: f64) -> f64 {
    let end = anchors
        .partition_point(|p| p.0 < value)
        .clamp(1, anchors.len() - 1);
    let (a, b) = (anchors[end - 1], anchors[end]);
    a.1 + (b.1 - a.1) * ((value - a.0) / (b.0 - a.0)).clamp(0., 1.)
}

/// Independent previewable underlay paths, in sewing order: centerwalk, edges,
/// then zigzag. Every segment stays inside the column, including closed holes.
pub fn satin_underlay_paths(
    column: &SatinColumn,
    settings: &Settings,
) -> Result<Vec<Vec<Point>>, String> {
    if !settings.underlay_inset_mm.is_finite()
        || !(0.0..=5.0).contains(&settings.underlay_inset_mm)
        || !settings.underlay_spacing_mm.is_finite()
        || !(0.3..=20.0).contains(&settings.underlay_spacing_mm)
        || !settings.underlay_length_mm.is_finite()
        || !(0.3..=12.0).contains(&settings.underlay_length_mm)
    {
        return Err("Underlay requires inset 0–5mm, spacing 0.3–20mm and length 0.3–12mm".into());
    }
    let stations = satin_stations(column, settings.spacing_mm)?;
    let rings = column.boundary_rings()?;
    let mut paths = Vec::new();
    if settings.underlay {
        paths.push(stations.iter().map(|(a, b)| a.lerp(*b, 0.5)).collect());
    }
    let inset_pair = |a: Point, b: Point| {
        let width = a.distance(b);
        // Retain a usable cross-section on very narrow portions.
        let inset = settings.underlay_inset_mm.min((width - 0.1).max(0.) * 0.5);
        (a.lerp(b, inset / width), b.lerp(a, inset / width))
    };
    if settings.edge_underlay {
        let pairs: Vec<_> = stations.iter().map(|&(a, b)| inset_pair(a, b)).collect();
        paths.push(pairs.iter().map(|p| p.0).collect());
        paths.push(pairs.iter().rev().map(|p| p.1).collect());
    }
    if settings.zigzag_underlay {
        let sparse = satin_stations(column, settings.underlay_spacing_mm)?;
        let mut path = Vec::with_capacity(sparse.len() + 1);
        let first = usize::from(column.first_right);
        let initial = inset_pair(sparse[0].0, sparse[0].1);
        path.push(if first == 0 { initial.0 } else { initial.1 });
        for (i, &(a, b)) in sparse.iter().enumerate() {
            let pair = inset_pair(a, b);
            path.push(if (i + first) % 2 == 0 { pair.1 } else { pair.0 });
        }
        paths.push(path);
    }
    if paths
        .iter()
        .any(|path| path.windows(2).any(|p| !safe_connector(p[0], p[1], &rings)))
    {
        return Err("Satin underlay leaves the column; reduce underlay spacing, correct rails or split the corner".into());
    }
    Ok(paths)
}

fn append_underlay(
    segments: &mut Vec<SewingSegment>,
    path: &[Point],
    maximum: f64,
    rings: &[Vec<Point>],
) -> Result<(), String> {
    let Some(&first) = path.first() else {
        return Ok(());
    };
    let mut anchor = first;
    for i in 1..path.len() {
        let next = path[i];
        if i + 1 < path.len()
            && anchor.distance(next) < maximum
            && safe_connector(anchor, next, rings)
        {
            continue;
        }
        let previous = path[i - 1];
        if !safe_connector(anchor, next, rings) && anchor.distance(previous) > 1e-8 {
            if !safe_connector(anchor, previous, rings) {
                return Err("Satin underlay shortcut leaves coverage".into());
            }
            segments.push((anchor, previous, StitchKind::Running));
            anchor = previous;
        }
        if !safe_connector(anchor, next, rings) {
            return Err("Satin underlay leaves coverage".into());
        }
        if anchor.distance(next) > 1e-8 {
            segments.push((anchor, next, StitchKind::Running));
        }
        anchor = next;
    }
    Ok(())
}

fn paired_satin_segments(
    column: &SatinColumn,
    settings: &Settings,
    rings: &[Vec<Point>],
    modern: bool,
) -> Result<(Vec<SewingSegment>, bool), String> {
    let stations = satin_stations(column, settings.spacing_mm)?;
    let limited_inset = (settings.edge_underlay || settings.zigzag_underlay)
        && stations
            .iter()
            .any(|(a, b)| a.distance(*b) < 2. * settings.underlay_inset_mm + 0.1);
    let mut segments = Vec::new();
    if settings.underlay || settings.edge_underlay || settings.zigzag_underlay {
        for path in satin_underlay_paths(column, settings)? {
            append_underlay(
                &mut segments,
                &path,
                if modern {
                    (settings.underlay_length_mm - 0.15).max(0.15)
                } else {
                    (settings.length_mm - 0.15).min(2.)
                },
                rings,
            )?;
        }
    }
    let mut contacts = [0_usize; 2];
    let exterior_side = if column.closed {
        usize::from(signed_area(&column.right).abs() > signed_area(&column.left).abs())
    } else {
        0
    };
    let lengths: [f64; 2] = [&column.left, &column.right]
        .map(|rail| rail.windows(2).map(|p| p[0].distance(p[1])).sum());
    let mut prior: [Option<Point>; 2] = [None, None];
    let endpoint = |pair: (Point, Point),
                    side: usize,
                    contacts: &mut [usize; 2],
                    prior: &mut [Option<Point>; 2],
                    force_edge: bool| {
        let (edge, opposite) = if side == 0 { pair } else { (pair.1, pair.0) };
        let width = edge.distance(opposite);
        let crowded = prior[side].is_some_and(|p| p.distance(edge) < settings.spacing_mm * 0.8);
        prior[side] = Some(edge);
        contacts[side] += 1;
        let shorten = if modern {
            contacts[side] > 2
        } else {
            lengths[side] < lengths[1 - side] * 0.85
        };
        if !force_edge && crowded && shorten && contacts[side].is_multiple_of(2) {
            // Distribute alternate inner-bend punctures inside the strip.
            // This bounded heuristic is not a general satin corner solver.
            let inset = edge.lerp(opposite, (width * 0.18).min(0.8) / width);
            (inset, inset)
        } else {
            let compensation = if column.closed && side != exterior_side {
                0.
            } else {
                settings.pull_comp_mm
            };
            (edge.lerp(opposite, -compensation / width), edge)
        }
    };
    let first = usize::from(column.first_right);
    let (mut previous, mut previous_base) =
        endpoint(stations[0], first, &mut contacts, &mut prior, true);
    for (i, &pair) in stations.iter().enumerate() {
        let side = if i % 2 == 0 { 1 - first } else { first };
        let (next, base) = endpoint(pair, side, &mut contacts, &mut prior, false);
        if !safe_connector(previous_base, base, rings) {
            return Err(
                "Satin diagonal exits the column; split or add clearer rail geometry".into(),
            );
        }
        segments.push((previous, next, StitchKind::Satin));
        previous = next;
        previous_base = base;
    }
    // Complete the far end with a crossbar rather than leaving one rail short.
    let last = *stations.last().unwrap();
    let last_side = if (stations.len() - 1) % 2 == 0 {
        1 - first
    } else {
        first
    };
    let (final_point, _) = endpoint(last, 1 - last_side, &mut contacts, &mut prior, true);
    segments.push((previous, final_point, StitchKind::Satin));
    Ok((segments, limited_inset))
}

/// Conservative filled envelope through the same certified rail stations used
/// for top stitches. The protected inner rail of a closed column stays fixed.
/// Keep the source polygon as well: stations can shortcut a curved boundary,
/// but that must not hide adjacency or a neighboring compensated component.
fn satin_compensation_domain(
    column: &SatinColumn,
    settings: &Settings,
    source: &[geo::Polygon<f64>],
) -> Result<geo::MultiPolygon<f64>, String> {
    let mut effective = source.to_vec();
    if settings.pull_comp_mm == 0. {
        return Ok(geo::MultiPolygon::new(effective));
    }
    let exterior_side =
        usize::from(signed_area(&column.right).abs() > signed_area(&column.left).abs());
    let stations = satin_stations(column, settings.spacing_mm)?;
    let mut rails = [
        Vec::with_capacity(stations.len()),
        Vec::with_capacity(stations.len()),
    ];
    for (left, right) in stations {
        for (side, rail) in rails.iter_mut().enumerate() {
            let (edge, opposite) = if side == 0 {
                (left, right)
            } else {
                (right, left)
            };
            let amount = if column.closed && side != exterior_side {
                0.
            } else {
                settings.pull_comp_mm
            };
            rail.push(edge.lerp(opposite, -amount / edge.distance(opposite)));
        }
    }
    let rings = if column.closed {
        for rail in &mut rails {
            if rail.first() == rail.last() {
                rail.pop();
            }
        }
        Vec::from(rails)
    } else {
        vec![
            rails[0]
                .iter()
                .copied()
                .chain(rails[1].iter().rev().copied())
                .collect(),
        ]
    };
    effective.extend(fill_geometry::components(&rings).map_err(|e| format!(
        "Cannot certify compensated Satin gaps: {e}. Reduce pull compensation or split the column constructively"
    ))?);
    Ok(geo::MultiPolygon::new(effective))
}

/// Bounded fill-direction search seeded by filled-area moments (including holes
/// and islands). Adding collinear boundary vertices cannot bias its direction.
pub fn automatic_angle(rings: &[Vec<Point>]) -> f64 {
    automatic_angle_choice(rings, 0.4, 3.5).0
}

/// Evaluate a small deterministic direction set with the active physical
/// spacing and length. The manual angle never calls this estimator.
fn automatic_angle_choice(rings: &[Vec<Point>], spacing: f64, length: f64) -> (f64, bool) {
    // Work budgets and scores use geometric corners, so merely adding points
    // to an existing straight edge cannot change the selected strategy.
    let canonical_rings: Vec<Vec<Point>> = rings
        .iter()
        .map(|ring| {
            if ring.len() < 3 {
                return ring.clone();
            }
            ring.iter()
                .enumerate()
                .filter_map(|(index, &point)| {
                    let before = ring[(index + ring.len() - 1) % ring.len()];
                    let after = ring[(index + 1) % ring.len()];
                    let (x, y) = (point.x - before.x, point.y - before.y);
                    let (u, v) = (after.x - point.x, after.y - point.y);
                    let collinear = (x * v - y * u).abs() <= 1e-10 * (x.hypot(y) + u.hypot(v))
                        && x * u + y * v >= 0.;
                    (!collinear).then_some(point)
                })
                .collect()
        })
        .collect();
    let rings = canonical_rings.as_slice();
    let principal = fill_geometry::area_angle(rings).unwrap_or(0.);
    let canonical = |angle: f64| {
        let angle = angle.rem_euclid(180.);
        if angle < 1e-8 || 180. - angle < 1e-8 {
            0.
        } else {
            angle
        }
    };
    let fallback = canonical(principal);
    if !spacing.is_finite() || spacing <= 0. || !length.is_finite() || length <= 0.15 {
        return (fallback, true);
    }
    let nodes = rings.iter().map(Vec::len).sum::<usize>();
    let bounds = rings.iter().flatten().fold(
        (
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ),
        |(x0, y0, x1, y1), p| (x0.min(p.x), y0.min(p.y), x1.max(p.x), y1.max(p.y)),
    );
    let diameter = (bounds.2 - bounds.0).hypot(bounds.3 - bounds.1);
    if !diameter.is_finite() || nodes == 0 {
        return (fallback, true);
    }
    let mut candidates = vec![fallback, canonical(principal + 90.), 0., 45., 90., 135.];
    candidates.sort_by(f64::total_cmp);
    candidates.dedup_by(|a, b| (*a - *b).abs() < 1e-8);
    // Bound the complete candidate search, not just each scan individually.
    if nodes
        .saturating_mul((diameter / spacing).ceil() as usize)
        .saturating_mul(candidates.len())
        > 4_000_000
    {
        return (fallback, true);
    }
    let cursor = Point::new((bounds.0 + bounds.2) * 0.5, (bounds.1 + bounds.3) * 0.5);
    let cancel = std::sync::atomic::AtomicBool::new(false);
    let maximum = (length - 0.15).max(0.15);
    let mut work = 0usize;
    let mut best: Option<(f64, f64)> = None;
    for candidate in candidates {
        let Ok(raw) = raw_fill_spans(rings, candidate, spacing, None) else {
            return (fallback, true);
        };
        work = work.saturating_add(raw.len().saturating_mul(nodes).saturating_mul(12));
        if work > 4_000_000 {
            return (fallback, true);
        }
        let Ok(spans) =
            routed_fill_spans(rings, candidate, spacing, cursor, maximum, true, &cancel)
        else {
            return (fallback, true);
        };
        let travel = fill_travel_cost(&spans, rings, cursor, maximum, true);
        let rows_and_short_spans = spans
            .iter()
            .map(|(a, b)| {
                let distance = a.distance(*b);
                distance / maximum + 0.7 + 2. * (1. - distance / maximum).max(0.)
            })
            .sum::<f64>();
        let score = travel.jump_mm * 2. + travel.jump_records as f64 * 8. + rows_and_short_spans;
        // Quantizing the score suppresses irrelevant arithmetic differences
        // from collinear edge subdivision. Ties choose the smaller angle.
        let score = (score * 100_000.).round() / 100_000.;
        if best.is_none_or(|(old, _)| score < old) {
            best = Some((score, candidate));
        }
    }
    (best.map_or(fallback, |(_, angle)| angle), false)
}

/// Return alternating scanline spans, with holes removed. Directional pull
/// compensation expands each connected exterior separately and subtracts its
/// unchanged protected holes; it never edits the source contour.
pub fn fill_spans(
    rings: &[Vec<Point>],
    angle_deg: f64,
    spacing: f64,
    compensation: f64,
) -> Result<Vec<(Point, Point)>, String> {
    if spacing <= 0. || !spacing.is_finite() || !angle_deg.is_finite() {
        return Err("Invalid scanline spacing or angle".into());
    }
    if !compensation.is_finite() || !(0.0..=2.).contains(&compensation) {
        return Err("Fill compensation must be 0–2mm per side".into());
    }
    if compensation > 0. {
        let mut spans = Vec::new();
        let mut guard = fill_geometry::CompensationGuard::new(true);
        for (index, component) in fill_geometry::components(rings)?.into_iter().enumerate() {
            let compensated = fill_geometry::compensated(&component, angle_deg, compensation)?;
            guard.register("Fill", index, &component, &compensated, true)?;
            spans.extend(raw_fill_spans(
                &fill_geometry::rings(&compensated),
                angle_deg,
                spacing,
                None,
            )?);
            if spans.len() > MAX_SPANS {
                return Err("Fill exceeds 250,000 spans".into());
            }
        }
        return Ok(spans);
    }
    raw_fill_spans(rings, angle_deg, spacing, None)
}

fn raw_fill_spans(
    rings: &[Vec<Point>],
    angle_deg: f64,
    spacing: f64,
    cancel: Option<&std::sync::atomic::AtomicBool>,
) -> Result<Vec<(Point, Point)>, String> {
    if spacing <= 0. || !spacing.is_finite() || !angle_deg.is_finite() {
        return Err("Invalid scanline spacing or angle".into());
    }
    let angle = angle_deg.to_radians();
    let rotated: Vec<Vec<_>> = rings
        .iter()
        .map(|r| r.iter().map(|&p| rotate(p, -angle)).collect())
        .collect();
    let min_y = rotated
        .iter()
        .flatten()
        .map(|p| p.y)
        .fold(f64::INFINITY, f64::min);
    let max_y = rotated
        .iter()
        .flatten()
        .map(|p| p.y)
        .fold(f64::NEG_INFINITY, f64::max);
    let mut spans = Vec::new();
    if !min_y.is_finite() || !max_y.is_finite() {
        return Ok(Vec::new());
    }
    let count = ((max_y - min_y) / spacing).ceil() as usize;
    let nodes = rings.iter().map(Vec::len).sum::<usize>();
    if count > 100_000 || nodes.saturating_mul(count) > 100_000_000 {
        return Err(
            "Fill exceeds scanline complexity budget; simplify contours or increase spacing".into(),
        );
    }
    for row in 0..count {
        if row % 64 == 0 && cancel.is_some_and(|c| c.load(std::sync::atomic::Ordering::Relaxed)) {
            return Err("Generation cancelled".into());
        }
        // A thin component still receives one central row. It must not vanish
        // because its height is less than the first half-spacing offset.
        let y = if max_y - min_y < spacing {
            (min_y + max_y) * 0.5
        } else {
            min_y + spacing * (row as f64 + 0.5)
        };
        if y >= max_y {
            break;
        }
        let mut xs = Vec::new();
        for ring in &rotated {
            for (a, b) in ring
                .iter()
                .zip(ring.iter().cycle().skip(1))
                .take(ring.len())
            {
                if (a.y <= y && b.y > y) || (b.y <= y && a.y > y) {
                    xs.push(a.x + (y - a.y) * (b.x - a.x) / (b.y - a.y));
                }
            }
        }
        xs.sort_by(f64::total_cmp);
        let mut row_spans = Vec::new();
        for pair in xs.as_chunks::<2>().0 {
            let left = pair[0];
            let right = pair[1];
            if right - left < 1e-7 {
                continue;
            }
            let (a, b) = (
                rotate(Point::new(left, y), angle),
                rotate(Point::new(right, y), angle),
            );
            row_spans.push(if row % 2 == 0 { (a, b) } else { (b, a) });
        }
        if row % 2 == 1 {
            row_spans.reverse();
        }
        spans.extend(row_spans);
        if spans.len() > MAX_SPANS {
            return Err("Fill exceeds 250,000 spans; simplify contours or increase spacing".into());
        }
    }
    Ok(spans)
}

#[derive(Default)]
struct FillTravelCost {
    jump_records: usize,
    jump_mm: f64,
    end: Point,
}
fn add_jump_cost(cost: &mut FillTravelCost, start: Point, end: Point) {
    // Measure the actual 0.1mm movement grid and 121-unit axis limit used by
    // DST, including the rounding of intermediate records on long transfers.
    let start = (
        (start.x * 10.).round() as i32,
        (start.y * 10.).round() as i32,
    );
    let end = ((end.x * 10.).round() as i32, (end.y * 10.).round() as i32);
    let delta = (end.0 - start.0, end.1 - start.1);
    let parts = ((delta.0.abs().max(delta.1.abs()) + 120) / 121).max(1);
    cost.jump_records = cost.jump_records.saturating_add(parts as usize);
    if cost.jump_records > MAX_COMMANDS {
        cost.jump_mm = f64::INFINITY;
        return;
    }
    let mut previous = start;
    for part in 1..=parts {
        let next = (
            start.0 + (f64::from(delta.0) * f64::from(part) / f64::from(parts)).round() as i32,
            start.1 + (f64::from(delta.1) * f64::from(part) / f64::from(parts)).round() as i32,
        );
        cost.jump_mm += f64::from(next.0 - previous.0).hypot(f64::from(next.1 - previous.1)) * 0.1;
        previous = next;
    }
}
fn fill_travel_cost(
    spans: &[(Point, Point)],
    rings: &[Vec<Point>],
    mut cursor: Point,
    max_length: f64,
    initial_jump: bool,
) -> FillTravelCost {
    let mut cost = FillTravelCost::default();
    for (index, &(a, b)) in spans.iter().enumerate() {
        let distance = cursor.distance(a);
        if (index == 0 && initial_jump)
            || (distance > 1e-9 && (distance > max_length || !safe_connector(cursor, a, rings)))
        {
            add_jump_cost(&mut cost, cursor, a);
            cursor = a;
        } else if distance > 1e-9 {
            cursor = a;
        }
        // Span endpoints are intentional and retained before quantization.
        if cursor.distance(b) > 1e-9 {
            cursor = b;
        }
    }
    cost.end = cursor;
    cost
}

/// Partition adjacent scanlines into chains through unambiguous one-to-one
/// overlap. A split or merge starts a new block: crossing every hole on every
/// row creates unnecessary long jumps and leaves each side fragmented.
fn routed_fill_spans(
    rings: &[Vec<Point>],
    angle_deg: f64,
    spacing: f64,
    mut cursor: Point,
    max_length: f64,
    initial_jump: bool,
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<Vec<(Point, Point)>, String> {
    struct Span {
        a: Point,
        b: Point,
        left: f64,
        right: f64,
        y: f64,
    }
    let angle = angle_deg.to_radians();
    // Callers supply the already selected top/underlay geometry. Routing never
    // changes artwork or applies a second technological compensation.
    let raw = raw_fill_spans(rings, angle_deg, spacing, Some(cancel))?;
    let Some(&(final_start, final_end)) = raw.last() else {
        return Ok(raw);
    };
    // Avoid adding expensive visibility scoring to already complex geometry.
    // The original traversal remains valid when this optional budget is hit.
    if raw.len().saturating_mul(rings.iter().map(Vec::len).sum()) > 10_000_000 {
        return Ok(raw);
    }
    let raw_cost = fill_travel_cost(&raw, rings, cursor, max_length, initial_jump);
    let mut rows: Vec<Vec<Span>> = Vec::new();
    for &(mut a, mut b) in &raw {
        let (mut ar, mut br) = (rotate(a, -angle), rotate(b, -angle));
        if ar.x > br.x {
            std::mem::swap(&mut a, &mut b);
            std::mem::swap(&mut ar, &mut br);
        }
        if rows
            .last()
            .is_none_or(|row| (row[0].y - ar.y).abs() > spacing * 0.1)
        {
            rows.push(Vec::new());
        }
        rows.last_mut().unwrap().push(Span {
            a,
            b,
            left: ar.x,
            right: br.x,
            y: ar.y,
        });
    }
    let mut chains: Vec<Vec<(Point, Point)>> = Vec::new();
    let mut previous: Vec<Span> = Vec::new();
    let mut previous_chains: Vec<usize> = Vec::new();
    for mut row in rows {
        if cancel.load(std::sync::atomic::Ordering::Relaxed) {
            return Err("Generation cancelled".into());
        }
        row.sort_by(|a, b| a.left.total_cmp(&b.left));
        let mut incoming = vec![0_usize; row.len()];
        let mut outgoing = vec![0_usize; previous.len()];
        let mut parent = vec![0_usize; row.len()];
        if !previous.is_empty() && row[0].y - previous[0].y <= spacing * 1.01 {
            // Both rows contain disjoint sorted intervals. This sweep is linear
            // even when one wide span splits into many narrow spans.
            let (mut i, mut j) = (0, 0);
            while i < previous.len() && j < row.len() {
                let (a, b) = (&previous[i], &row[j]);
                if a.right.min(b.right) - a.left.max(b.left) > 1e-8 {
                    outgoing[i] += 1;
                    incoming[j] += 1;
                    parent[j] = i;
                }
                if a.right <= b.right {
                    i += 1;
                } else {
                    j += 1;
                }
            }
        }
        let mut row_chains = Vec::with_capacity(row.len());
        for (j, span) in row.iter().enumerate() {
            let chain = if incoming[j] == 1 && outgoing[parent[j]] == 1 {
                previous_chains[parent[j]]
            } else {
                chains.push(Vec::new());
                chains.len() - 1
            };
            chains[chain].push((span.a, span.b));
            row_chains.push(chain);
        }
        previous = row;
        previous_chains = row_chains;
    }
    // Finish at the original endpoint and direction so this decision cannot
    // move the entry transfer of the next pass/object somewhere else.
    let terminal = chains
        .iter()
        .position(|chain| {
            let &(a, b) = chain.last().unwrap();
            (a == final_start && b == final_end) || (a == final_end && b == final_start)
        })
        .unwrap();
    let terminal_chain = chains.remove(terminal);
    chains.push(terminal_chain);
    let chain_count = chains.len();
    let original_cursor = cursor;
    let mut ordered = Vec::new();
    // Keep the deterministic creation order of blocks. For each block choose
    // the nearest of its four possible starts; never reorder objects/layers.
    for (chain_index, chain) in chains.into_iter().enumerate() {
        let first = chain[0];
        let last = chain[chain.len() - 1];
        let candidates = [first.0, first.1, last.0, last.1];
        let choice = (0..4)
            .min_by(|&a, &b| {
                candidates[a]
                    .distance(cursor)
                    .total_cmp(&candidates[b].distance(cursor))
            })
            .unwrap();
        let terminal = chain_index + 1 == chain_count;
        let reverse_rows = !terminal && choice >= 2;
        let start_right = if terminal {
            (last.1 == final_start) == (chain.len() % 2 == 1)
        } else {
            choice % 2 == 1
        };
        for index in 0..chain.len() {
            let (a, b) = chain[if reverse_rows {
                chain.len() - 1 - index
            } else {
                index
            }];
            let pair = if (index % 2 == 0) == start_right {
                (b, a)
            } else {
                (a, b)
            };
            cursor = pair.1;
            ordered.push(pair);
        }
    }
    let candidate_cost =
        fill_travel_cost(&ordered, rings, original_cursor, max_length, initial_jump);
    // A topology split is useful only when it reduces fragmentation without
    // increasing the total non-sewing path. Conservative fallback also keeps
    // the exact ending cursor, including the tiny-movement emission rule.
    if candidate_cost.jump_records <= raw_cost.jump_records
        && candidate_cost.jump_mm <= raw_cost.jump_mm + 1e-8
        && candidate_cost.end == raw_cost.end
    {
        Ok(ordered)
    } else {
        Ok(raw)
    }
}

/// Optional greedy ordering within each consecutive same-colour block. The
/// caller must opt in: object order controls overlap and should remain editable.
pub fn optimize_order(project: &mut Project) {
    let original = project.clone();
    let Ok(before) = generate(&original) else {
        return;
    };
    let mut cursor = Point::default();
    let mut start = 0;
    while start < project.objects.len() {
        let color = project.objects[start].color;
        let mut end = start + 1;
        while end < project.objects.len() && project.objects[end].color == color {
            end += 1;
        }
        // A same-colour block can still contain a deliberate layer dependency.
        // Conservatively preserve the complete block when any bounds overlap;
        // optimization remains available for proven spatially separate objects.
        let bounds: Vec<_> = project.objects[start..end]
            .iter()
            .map(|object| {
                object
                    .flattened_contours()
                    .into_iter()
                    .flatten()
                    .chain(object.running_paths.iter().flatten().copied())
                    .fold(
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
        if bounds.iter().enumerate().any(|(i, a)| {
            bounds[i + 1..]
                .iter()
                .any(|b| a.0 <= b.2 && a.2 >= b.0 && a.1 <= b.3 && a.3 >= b.1)
        }) {
            start = end;
            continue;
        }
        for slot in start..end {
            let nearest = (slot..end)
                .min_by(|&a, &b| {
                    let distance = |index: usize| {
                        if let Some(column) = &project.objects[index].satin_column
                            && let Some(start) = column.left.first()
                        {
                            return start.distance(cursor);
                        }
                        if let Some(start) =
                            project.objects[index].running_paths.iter().flatten().next()
                        {
                            return start.distance(cursor);
                        }
                        project.objects[index]
                            .contours
                            .iter()
                            .flatten()
                            .map(|&p| p.distance(cursor))
                            .fold(f64::INFINITY, f64::min)
                    };
                    distance(a).total_cmp(&distance(b))
                })
                .unwrap_or(slot);
            project.objects.swap(slot, nearest);
            let first = project.objects[slot]
                .satin_column
                .as_ref()
                .and_then(|c| c.left.first())
                .or_else(|| project.objects[slot].contours.iter().flatten().next())
                .or_else(|| project.objects[slot].running_paths.iter().flatten().next());
            if let Some(&p) = first {
                cursor = p;
            }
        }
        start = end;
    }
    let jump_cost = |stitches: &[Stitch]| {
        let mut previous = Point::default();
        let mut count = 0_usize;
        let mut distance = 0.;
        for stitch in stitches {
            if stitch.command == Command::Jump {
                count += 1;
                distance += previous.distance(stitch.point());
            }
            previous = stitch.point();
        }
        (count, distance)
    };
    let baseline = jump_cost(&before);
    let acceptable = generate(project).is_ok_and(|after| {
        let candidate = jump_cost(&after);
        candidate.0 <= baseline.0 && candidate.1 <= baseline.1 + 1e-8
    });
    if !acceptable {
        *project = original;
    }
}

struct Builder {
    out: Vec<Stitch>,
    metadata: Vec<CommandMeta>,
    warnings: Vec<PlanWarning>,
    role: StitchRole,
    pass_id: u32,
    current: Point,
    color: [u8; 3],
    object_id: u64,
    max_length: f64,
    machine_max_length: f64,
    trim_jump_mm: f64,
    overflow: bool,
    tie_in_enabled: bool,
    tie_off_enabled: bool,
    last_sewn_start: Option<Point>,
    last_sewn_role: StitchRole,
    thread_active: bool,
    future_top: travel::FutureCover,
    future_order: usize,
    travel_memory: travel::TravelMemory,
}
impl Builder {
    fn prepare_future_top(
        &mut self,
        rings: &[Vec<Point>],
        angle: f64,
        spacing: f64,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> Result<(), String> {
        self.future_top = travel::FutureCover::new(
            raw_fill_spans(rings, angle, spacing, Some(cancel))?
                .into_iter()
                .map(|(a, b)| (a, b, 0)),
        );
        self.future_order = 0;
        Ok(())
    }
    fn concealed_travel(&mut self, target: Point) -> bool {
        let prior = if self.last_sewn_role == StitchRole::TopFill {
            self.last_sewn_start
        } else {
            None
        };
        if !self
            .future_top
            .covers(self.current, target, self.future_order, prior)
        {
            if !self.future_top.complete() {
                self.warn("Travel concealment analysis reached its work budget; explicit jumps preserve all top-fill segments.",Some(target));
            }
            return false;
        }
        if !self.travel_memory.allow(self.current, target) {
            self.warn("Repeated sewn travel was replaced by an explicit jump to limit local thread accumulation.",Some(target));
            return false;
        }
        true
    }
    fn begin_pass(&mut self, role: StitchRole) {
        self.role = role;
        self.pass_id = self.pass_id.saturating_add(1);
    }
    fn warn(&mut self, message: impl Into<String>, point: Option<Point>) {
        let message = message.into();
        if self
            .warnings
            .iter()
            .any(|w| w.object_id == self.object_id && w.message == message)
        {
            return;
        }
        self.warnings.push(PlanWarning {
            object_id: self.object_id,
            point,
            message,
        });
    }
    fn emit(&mut self, p: Point, command: Command) {
        if self.out.len() >= MAX_COMMANDS {
            self.overflow = true;
            return;
        }
        self.out.push(Stitch {
            x: p.x,
            y: p.y,
            command,
            color: self.color,
            object_id: self.object_id,
        });
        self.metadata.push(CommandMeta {
            role: match command {
                Command::Stitch => self.role,
                Command::Jump => StitchRole::Travel,
                _ => StitchRole::Control,
            },
            pass_id: self.pass_id,
        });
        if command == Command::Stitch {
            self.last_sewn_start = Some(self.current);
            self.last_sewn_role = self.role;
            self.thread_active = true;
        } else if matches!(command, Command::Trim | Command::ColorChange) {
            self.thread_active = false;
        }
        self.current = p;
    }
    fn jump_to(&mut self, start: Point, next: Point, rings: &[Vec<Point>]) -> bool {
        // A jump is not a trim. Only long transfers carry a separate explicit
        // cut intention; exporters may leave that intention in a sidecar.
        let cut = self.thread_active && self.current.distance(start) > self.trim_jump_mm;
        if cut {
            if self.tie_off_enabled
                && let Some(prior) = self.last_sewn_start
                && safe_connector(self.current, prior, rings)
            {
                self.tie(prior);
            }
            self.emit(self.current, Command::Trim);
        }
        self.emit(start, Command::Jump);
        let tied = cut && self.tie_in_enabled && safe_connector(start, next, rings);
        if tied {
            self.tie(next);
        }
        tied
    }
    fn line(&mut self, end: Point) {
        let start = self.current;
        let n = (start.distance(end) / self.max_length - 1e-10)
            .ceil()
            .max(1.0) as usize;
        for i in 1..=n {
            if self.overflow {
                break;
            }
            let p = start.lerp(end, i as f64 / n as f64);
            if p.distance(self.current) >= 0.05 {
                self.emit(p, Command::Stitch);
            }
        }
    }
    // Explicit open-path vertices are intentional bends. Preserve them before
    // machine quantization, even below the fill emitter's 0.05mm filter.
    fn path_line(&mut self, end: Point) {
        let start = self.current;
        let n = (start.distance(end) / self.max_length - 1e-10)
            .ceil()
            .max(1.0) as usize;
        for i in 1..=n {
            if self.overflow {
                break;
            }
            let p = if i == n {
                end
            } else {
                start.lerp(end, i as f64 / n as f64)
            };
            if p != self.current {
                self.emit(p, Command::Stitch);
            }
        }
    }
    fn tie(&mut self, direction: Point) {
        let anchor = self.current;
        let distance = anchor.distance(direction);
        if distance < 0.1 {
            return;
        }
        let near = anchor.lerp(direction, (0.6 / distance).min(0.5));
        let role = self.role;
        self.role = StitchRole::Lock;
        self.line(near);
        self.line(anchor);
        self.line(near);
        self.line(anchor);
        self.role = role;
    }
    fn span(
        &mut self,
        start: Point,
        end: Point,
        rings: &[Vec<Point>],
        tatami_phase: Option<(f64, usize)>,
        kind: StitchKind,
        retain_endpoint: bool,
    ) {
        if self.current.distance(start) > if retain_endpoint { 1e-9 } else { 0.05 } {
            if self.current.distance(start) <= self.max_length
                && safe_connector(self.current, start, rings)
                && self.concealed_travel(start)
            {
                let role = self.role;
                self.role = StitchRole::Travel;
                if retain_endpoint {
                    self.path_line(start);
                } else {
                    self.line(start);
                }
                self.role = role;
            } else {
                self.jump_to(start, end, rings);
            }
        }
        if kind == StitchKind::Tatami
            && let Some((angle, row)) = tatami_phase
        {
            // Anchor punctures to a common physical grid, rather than the
            // direction-dependent start of each span. The four-row stagger
            // avoids paired rows sharing all their interior needle holes.
            let a = rotate(start, -angle);
            let z = rotate(end, -angle);
            let step = self.max_length;
            let phase = [0., 0.5, 0.25, 0.75][row % 4] * step;
            let forward = z.x > a.x;
            let mut x = if forward {
                ((a.x - phase) / step).floor() * step + phase + step
            } else {
                ((a.x - phase) / step).ceil() * step + phase - step
            };
            while if forward {
                x < z.x - 0.2
            } else {
                x > z.x + 0.2
            } {
                if self.overflow {
                    break;
                }
                if (x - a.x).abs() >= 0.2 {
                    self.line(rotate(Point::new(x, a.y), angle));
                }
                x += if forward { step } else { -step };
            }
        }
        if retain_endpoint {
            self.path_line(end);
        } else {
            self.line(end);
        }
    }
    fn sew_segments(
        &mut self,
        segments: &[SewingSegment],
        rings: &[Vec<Point>],
        settings: &Settings,
        angle_deg: f64,
        options: PassOptions,
    ) -> Result<(), String> {
        let PassOptions {
            modern_satin,
            role,
            initial_jump,
        } = options;
        self.begin_pass(role);
        let angle = angle_deg.to_radians();
        let min_y = rings
            .iter()
            .flatten()
            .map(|&p| rotate(p, -angle).y)
            .fold(f64::INFINITY, f64::min);
        if let Some(&(start, end, _)) = segments.first() {
            let already_tied = initial_jump && self.jump_to(start, end, rings);
            if settings.tie_in && !already_tied {
                self.tie(end);
            }
        }
        let mixed_satin = segments.iter().any(|s| s.2 == StitchKind::Satin);
        if role == StitchRole::TopFill {
            self.future_top = travel::FutureCover::new(
                segments
                    .iter()
                    .enumerate()
                    .filter(|(_, s)| !mixed_satin || s.2 == StitchKind::Satin)
                    .map(|(index, &(a, z, _))| (a, z, index)),
            );
        }
        for (segment_index, &(a, z, kind)) in segments.iter().enumerate() {
            self.future_order = if role == StitchRole::Underlay {
                0
            } else {
                segment_index
            };
            let grid = |p: Point| ((p.x * 10.).round() as i64, (p.y * 10.).round() as i64);
            if role == StitchRole::TopFill && kind != StitchKind::Running && grid(a) == grid(z) {
                self.warn("A top-fill span collapses on the 0.1mm machine grid; this small feature needs manual review or a larger physical size.",Some(a));
            }
            let segment_role = if mixed_satin && kind == StitchKind::Running {
                StitchRole::Underlay
            } else {
                role
            };
            if self.role != segment_role
                || (mixed_satin && segment_role == StitchRole::Underlay && self.current != a)
            {
                // Independent center/edge/zigzag paths are separate passes,
                // even when object and thread colour remain the same.
                self.begin_pass(segment_role);
            }
            let maximum = self.max_length;
            if segment_role == StitchRole::Underlay && (!mixed_satin || modern_satin) {
                self.max_length =
                    (settings.underlay_length_mm.min(self.machine_max_length) - 0.15).max(0.15);
            }
            if modern_satin {
                if kind == StitchKind::Running {
                    self.max_length =
                        (settings.underlay_length_mm.min(self.machine_max_length) - 0.15).max(0.15);
                }
                if self.current != a {
                    if self.current.distance(a) <= self.max_length
                        && safe_connector(self.current, a, rings)
                        && self.concealed_travel(a)
                    {
                        let role = self.role;
                        self.role = StitchRole::Travel;
                        self.path_line(a);
                        self.role = role;
                    } else {
                        self.jump_to(a, z, rings);
                    }
                }
            }
            let phase = if kind == StitchKind::Tatami {
                let row = ((rotate(a, -angle).y - min_y) / settings.spacing_mm - 0.5)
                    .round()
                    .max(0.) as usize;
                Some((angle, row))
            } else {
                None
            };
            if modern_satin && kind == StitchKind::Running {
                self.path_line(z);
            } else if modern_satin && kind == StitchKind::Satin {
                // The complete compensated diagonal was checked against the
                // physical cap before emission. Keep a rail-to-rail top stitch;
                // generic Builder subdivision is not constructive split-satin.
                if self.current.distance(z) > 1e-9 {
                    self.emit(z, Command::Stitch);
                }
            } else {
                self.span(a, z, rings, phase, kind, modern_satin || !mixed_satin);
            }
            self.max_length = maximum;
            if self.overflow {
                return Err("Generation exceeds one million commands; increase spacing".into());
            }
        }
        if settings.tie_off
            && let Some(&(a, _, _)) = segments.last()
        {
            self.tie(a);
        }
        if self.overflow {
            return Err("Generation exceeds one million commands; increase spacing".into());
        }
        Ok(())
    }
}

pub fn generate(project: &Project) -> Result<Vec<Stitch>, String> {
    generate_plan(project).map(|plan| plan.stitches)
}

/// Plan annotations survive through diagnostics and preview, independently of
/// machine formats which cannot store pass roles or editable object geometry.
pub fn generate_plan(project: &Project) -> Result<MachinePlan, String> {
    generate_plan_cancellable(project, &std::sync::atomic::AtomicBool::new(false))
}

pub fn generate_plan_cancellable(
    project: &Project,
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<MachinePlan, String> {
    project.validate_limits()?;
    let generation_version = project.persisted_schema_version();
    let layering = layering::LayeringPlan::new(project, cancel)?;
    let mut intentionally_occluded = std::collections::BTreeSet::new();
    let mut b = Builder {
        out: Vec::new(),
        metadata: Vec::new(),
        warnings: Vec::new(),
        role: StitchRole::TopFill,
        pass_id: 0,
        current: Point::default(),
        color: [0; 3],
        object_id: 0,
        max_length: 3.5,
        machine_max_length: project.machine_profile.max_stitch_mm,
        trim_jump_mm: project.machine_profile.trim_jump_mm,
        overflow: false,
        tie_in_enabled: false,
        tie_off_enabled: false,
        last_sewn_start: None,
        last_sewn_role: StitchRole::Control,
        thread_active: false,
        future_top: Default::default(),
        future_order: 0,
        travel_memory: Default::default(),
    };
    let mut has_color = false;
    let mut work_budget = 0_usize;
    let mut compensation_guard =
        fill_geometry::CompensationGuard::new(project.objects.iter().any(|o| {
            o.visible
                && o.running_paths.is_empty()
                && o.settings.kind != StitchKind::Running
                && o.settings.pull_comp_mm > 0.
        }));
    for object in project.objects.iter().filter(|o| o.visible) {
        if cancel.load(std::sync::atomic::Ordering::Relaxed) {
            return Err("Generation cancelled".into());
        }
        let rings = object.flattened_contours();
        b.object_id = object.id;
        let nodes = rings.iter().map(Vec::len).sum::<usize>();
        if nodes > 100_000 {
            return Err(format!(
                "{}: flattened geometry exceeds 100,000 nodes",
                object.name
            ));
        }
        let s = &object.settings;
        if rings.is_empty() && object.running_paths.is_empty() {
            if s.kind == StitchKind::Satin && object.satin_column.is_some() {
                return Err(format!(
                    "{}: each satin rail requires at least two nodes",
                    object.name
                ));
            }
            b.warn("Visible object has no stitchable geometry; add a contour or remove the empty object.",None);
            continue;
        }
        if rings.iter().any(|r| {
            r.len() < 3
                || r.iter().any(|p| {
                    !p.x.is_finite() || !p.y.is_finite() || p.x.abs() > 9990. || p.y.abs() > 9990.
                })
        }) {
            return Err(format!(
                "{}: contours require >=3 finite mm points within ±9990mm",
                object.name
            ));
        }
        if !s.spacing_mm.is_finite()
            || !(0.15..=20.).contains(&s.spacing_mm)
            || !s.length_mm.is_finite()
            || !(0.3..=12.0).contains(&s.length_mm)
            || !s.angle_deg.is_finite()
            || !s.pull_comp_mm.is_finite()
            || !(0.0..=2.).contains(&s.pull_comp_mm)
            || !s.underlay_inset_mm.is_finite()
            || !(0.0..=5.0).contains(&s.underlay_inset_mm)
            || !s.underlay_spacing_mm.is_finite()
            || !(0.3..=20.0).contains(&s.underlay_spacing_mm)
            || !s.underlay_length_mm.is_finite()
            || !(0.3..=12.0).contains(&s.underlay_length_mm)
        {
            return Err(format!(
                "{}: spacing must be 0.15–20mm, length 0.3–12mm, compensation 0–2mm",
                object.name
            ));
        }
        let prior_color = b.color;
        let prior_has_color = has_color;
        let prior_thread_active = b.thread_active;
        let prior_commands = b.out.len();
        if has_color && b.color != object.color {
            b.emit(b.current, Command::ColorChange);
        }
        b.color = object.color;
        b.object_id = object.id;
        b.tie_in_enabled = s.tie_in;
        b.tie_off_enabled = s.tie_off;
        // DST rounds both absolute endpoints to 0.1mm. Their relative vector
        // can grow by sqrt(2)*0.1mm, so reserve 0.15mm before subdivision.
        b.max_length = (s.length_mm.min(project.machine_profile.max_stitch_mm) - 0.15).max(0.15);
        has_color = true;
        if !object.running_paths.is_empty() {
            // Each branch is a separate open polyline. Explicitly begin every
            // path with Jump, even when a nearby branch could be sewn across.
            // Closing is opt-in by repeating the first point in the geometry.
            for path in &object.running_paths {
                let grid = |p: Point| ((p.x * 10.).round() as i64, (p.y * 10.).round() as i64);
                if path.iter().all(|&p| grid(p) == grid(path[0])) {
                    return Err(format!(
                        "{}: a running path collapses on the 0.1mm export grid; enlarge it or remove that path",
                        object.name
                    ));
                }
                if path.windows(2).any(|pair| grid(pair[0]) == grid(pair[1])) {
                    b.warn("Running path contains bends below the 0.1mm machine grid. Editable nodes are retained, but coincident machine punctures are omitted.",path.first().copied());
                }
                b.begin_pass(StitchRole::TopFill);
                let already_tied = b.jump_to(path[0], path[1], &rings);
                if s.tie_in && !already_tied {
                    b.tie(path[1]);
                }
                for &end in &path[1..] {
                    b.path_line(end);
                    if b.overflow {
                        return Err(
                            "Generation exceeds one million commands; simplify running paths"
                                .into(),
                        );
                    }
                }
                if s.tie_off {
                    b.tie(path[path.len() - 2]);
                }
                if b.overflow {
                    return Err(
                        "Generation exceeds one million commands; simplify running paths".into(),
                    );
                }
            }
            continue;
        }
        if s.kind == StitchKind::Satin
            && let Some(column) = &object.satin_column
        {
            if !object.contours.is_empty() || !object.bezier_segments.is_empty() {
                return Err(format!(
                    "{}: paired-rail satin must not mix polygon or cubic contours in the same object",
                    object.name
                ));
            }
            let rail_length =
                |points: &[Point]| points.windows(2).map(|p| p[0].distance(p[1])).sum::<f64>();
            let stations = (rail_length(&column.left).max(rail_length(&column.right))
                / (s.spacing_mm * 0.5))
                .ceil() as usize;
            work_budget = work_budget
                .saturating_add(nodes.saturating_mul(nodes))
                .saturating_add(
                    stations
                        .saturating_add(nodes)
                        .saturating_add(1)
                        .saturating_mul(nodes)
                        .saturating_mul(5),
                );
            if work_budget > 100_000_000 {
                return Err(
                    "Project exceeds satin geometry work budget; simplify or split columns".into(),
                );
            }
            let modern = generation_version >= 4 || column.uses_v4() || s.uses_v4();
            let (segments, limited_inset) = paired_satin_segments(column, s, &rings, modern)
                .map_err(|e| format!("{}: {e}", object.name))?;
            if modern {
                let maximum = s.length_mm.min(project.machine_profile.max_stitch_mm);
                let unsplit_limit = (maximum - 0.15).max(0.15);
                if let Some((a, z, _)) = segments.iter().find(|(a, z, kind)| {
                    *kind == StitchKind::Satin && a.distance(*z) > unsplit_limit + 1e-8
                }) {
                    return Err(format!(
                        "{}: Satin diagonal {:.2}mm exceeds the unsplit limit {:.2}mm (configured physical cap {:.2}mm, including a 0.15mm quantization reserve). Split the column constructively, increase its stitch-length cap only if the machine permits it, or use Tatami; generic midpoint subdivision is not split-satin.",
                        object.name,
                        a.distance(*z),
                        unsplit_limit,
                        maximum
                    ));
                }
            }
            if limited_inset {
                b.warn("Underlay inset was locally limited to preserve a 0.1mm Satin cross-section in narrow sections. Review underlay and reduce inset if necessary.",rings.first().and_then(|r|r.first()).copied());
            }
            if compensation_guard.enabled() || layering.is_clipped(object) {
                let components = fill_geometry::components(&rings)
                    .map_err(|e| format!("{}: {e}", object.name))?;
                let effective = satin_compensation_domain(column, s, &components)
                    .map_err(|e| format!("{}: {e}", object.name))?;
                for (index, component) in components.iter().enumerate() {
                    compensation_guard.register(
                        &object.name,
                        index,
                        component,
                        &effective,
                        s.pull_comp_mm > 0.,
                    )?;
                }
                if layering.is_clipped(object) {
                    let sewn = layering::satin_top_domain(object, generation_version)?;
                    let visible = layering.clip(object, &sewn)?;
                    if layering::LayeringPlan::changed(&sewn, &visible) {
                        if visible.0.is_empty() {
                            b.warn("Layering: this object is fully hidden by later filled layers; its top and underlay are intentionally omitted. Change layer order or the hidden-fill policy to sew it.",layering::LayeringPlan::first_point(&sewn));
                            intentionally_occluded.insert(object as *const Object as usize);
                            b.out.truncate(prior_commands);
                            b.metadata.truncate(prior_commands);
                            b.color = prior_color;
                            b.thread_active = prior_thread_active;
                            has_color = prior_has_color;
                            continue;
                        }
                        return Err(format!(
                            "{}: a later filled layer partially covers this Satin column. Constructive rails cannot be pruned safely; move the column above that layer, convert it to Tatami, or disable hidden-fill removal",
                            object.name
                        ));
                    }
                }
            }
            let trim_threshold = b.trim_jump_mm;
            // v1–v3 simple columns retain their recorded underlay/thread policy.
            // Advanced columns and edited profiles use the current planner.
            if !modern {
                b.trim_jump_mm = f64::INFINITY;
            }
            b.sew_segments(
                &segments,
                &rings,
                s,
                0.,
                PassOptions {
                    modern_satin: modern,
                    role: StitchRole::TopFill,
                    initial_jump: true,
                },
            )?;
            b.trim_jump_mm = trim_threshold;
            continue;
        }
        let angle = if s.auto_angle {
            let (angle, limited) = automatic_angle_choice(&rings, s.spacing_mm, s.length_mm);
            if limited {
                b.warn("Automatic direction search reached its work budget; using stable filled-area principal direction. Set a manual angle to override it.",rings.first().and_then(|r|r.first()).copied());
            }
            angle
        } else {
            s.angle_deg
        };
        let underlay_angle = s.underlay_angle_deg.unwrap_or(angle + 90.);
        if s.kind != StitchKind::Running {
            let estimated_work = |angle: f64, spacing: f64| {
                let angle = angle.to_radians();
                let mut lo = f64::INFINITY;
                let mut hi = f64::NEG_INFINITY;
                for p in rings.iter().flatten() {
                    let y = rotate(*p, -angle).y;
                    lo = lo.min(y);
                    hi = hi.max(y);
                }
                (((hi - lo) / spacing).ceil() as usize).saturating_mul(nodes)
            };
            work_budget = work_budget.saturating_add(estimated_work(angle, s.spacing_mm));
            if s.underlay || s.zigzag_underlay {
                work_budget = work_budget
                    .saturating_add(estimated_work(underlay_angle, s.underlay_spacing_mm));
            }
            if work_budget > 100_000_000 {
                return Err(
                    "Project exceeds scanline work budget; simplify contours or increase spacing"
                        .into(),
                );
            }
        }
        if s.kind == StitchKind::Satin {
            return Err(format!(
                "{}: Satin requires constructive paired rails; use Create satin copy or choose Tatami",
                object.name
            ));
        }
        if s.kind == StitchKind::Running {
            let mut segments = Vec::new();
            for ring in &rings {
                for (&a, &z) in ring
                    .iter()
                    .zip(ring.iter().cycle().skip(1))
                    .take(ring.len())
                {
                    segments.push((a, z, StitchKind::Running));
                }
            }
            b.sew_segments(
                &segments,
                &rings,
                s,
                angle,
                PassOptions {
                    modern_satin: false,
                    role: StitchRole::TopFill,
                    initial_jump: true,
                },
            )?;
            continue;
        }
        let components =
            fill_geometry::components(&rings).map_err(|e| format!("{}: {e}", object.name))?;
        let mut retained_components = 0usize;
        let mut hidden_components = 0usize;
        for (index, component) in components.into_iter().enumerate() {
            if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                return Err("Generation cancelled".into());
            }
            let original = fill_geometry::polygon_rings(&component);
            let compensated = fill_geometry::compensated(&component, angle, s.pull_comp_mm)?;
            compensation_guard.register(
                &object.name,
                index,
                &component,
                &compensated,
                s.pull_comp_mm > 0.,
            )?;
            let visible = layering.clip(object, &compensated)?;
            if visible.0.is_empty() {
                hidden_components += 1;
                b.warn("Layering: this component is fully hidden by later filled layers; its top and underlay are intentionally omitted. Change layer order or the hidden-fill policy to sew it.",original.first().and_then(|r|r.first()).copied());
                continue;
            }
            retained_components += 1;
            let allowed = fill_geometry::rings(&visible);
            b.prepare_future_top(&allowed, angle, s.spacing_mm, cancel)?;
            let mut underlay_done = false;
            if s.underlay || s.edge_underlay || s.zigzag_underlay {
                let inset = layering.clip(
                    object,
                    &fill_geometry::inset(&component, s.underlay_inset_mm)?,
                )?;
                let underlay_rings = fill_geometry::rings(&inset);
                if underlay_rings.is_empty() {
                    b.warn("Underlay inset removes this narrow component; underlay skipped. Reduce inset or choose a thinner-detail profile.",original.first().and_then(|r|r.first()).copied());
                } else {
                    let mut segments = Vec::new();
                    if s.edge_underlay {
                        for ring in &underlay_rings {
                            for (&a, &z) in ring
                                .iter()
                                .zip(ring.iter().cycle().skip(1))
                                .take(ring.len())
                            {
                                segments.push((a, z, StitchKind::Running));
                            }
                        }
                    }
                    let edge_count = segments.len();
                    if s.underlay || s.zigzag_underlay {
                        segments.extend(
                            routed_fill_spans(
                                &underlay_rings,
                                underlay_angle,
                                s.underlay_spacing_mm,
                                b.current,
                                (s.underlay_length_mm - 0.15).max(0.15),
                                true,
                                cancel,
                            )?
                            .into_iter()
                            .map(|(a, z)| (a, z, StitchKind::Running)),
                        );
                    }
                    if !segments.is_empty() {
                        let mut underlay_settings = s.clone();
                        underlay_settings.tie_off = false;
                        if edge_count > 0 {
                            b.sew_segments(
                                &segments[..edge_count],
                                &underlay_rings,
                                &underlay_settings,
                                underlay_angle,
                                PassOptions {
                                    modern_satin: false,
                                    role: StitchRole::Underlay,
                                    initial_jump: true,
                                },
                            )?;
                            underlay_settings.tie_in = false;
                        }
                        if edge_count < segments.len() {
                            b.sew_segments(
                                &segments[edge_count..],
                                &underlay_rings,
                                &underlay_settings,
                                underlay_angle,
                                PassOptions {
                                    modern_satin: false,
                                    role: StitchRole::Underlay,
                                    initial_jump: edge_count == 0,
                                },
                            )?;
                        }
                        underlay_done = true;
                    }
                }
            }
            let top = routed_fill_spans(
                &allowed,
                angle,
                s.spacing_mm,
                b.current,
                b.max_length,
                !underlay_done,
                cancel,
            )?;
            if top.is_empty() {
                b.warn("This component has no top-fill rows; increase its physical size or decrease row spacing.",original.first().and_then(|r|r.first()).copied());
                continue;
            }
            let mut top_settings = s.clone();
            if underlay_done {
                top_settings.tie_in = false;
            }
            let segments: Vec<_> = top
                .into_iter()
                .map(|(a, z)| (a, z, StitchKind::Tatami))
                .collect();
            b.sew_segments(
                &segments,
                &allowed,
                &top_settings,
                angle,
                PassOptions {
                    modern_satin: false,
                    role: StitchRole::TopFill,
                    initial_jump: !underlay_done,
                },
            )?;
        }
        if hidden_components > 0 && retained_components == 0 {
            intentionally_occluded.insert(object as *const Object as usize);
            b.out.truncate(prior_commands);
            b.metadata.truncate(prior_commands);
            b.color = prior_color;
            b.thread_active = prior_thread_active;
            has_color = prior_has_color;
        }
    }
    b.emit(b.current, Command::End);
    if b.overflow {
        return Err("Generation exceeds one million commands; increase spacing".into());
    }
    let mut plan = crate::commands::normalize_plan_with_limits(
        MachinePlan {
            stitches: b.out,
            metadata: b.metadata,
            warnings: b.warnings,
        },
        &project.machine_profile,
    )?;
    for object in project.objects.iter().filter(|o| o.visible) {
        if intentionally_occluded.contains(&(object as *const Object as usize)) {
            continue;
        }
        if !plan.stitches.iter().zip(&plan.metadata).any(|(s, m)| {
            s.object_id == object.id
                && s.command == Command::Stitch
                && m.role == StitchRole::TopFill
        }) {
            plan.warnings.push(PlanWarning {object_id:object.id,point:object.flattened_contours().first().and_then(|r|r.first()).copied(),message:"Visible object has no top stitches after 0.1mm machine quantization. Increase its size, edit its geometry or exclude it explicitly.".into()});
        }
    }
    if let Some(recipe) = &project.raster_recipe {
        for issue in &recipe.issues {
            if !plan.warnings.iter().any(|known| {
                known.object_id == issue.object_id
                    && known.point == issue.point
                    && known.message == issue.message
            }) {
                plan.warnings.push(issue.clone());
            }
        }
        for message in &recipe.warnings {
            if !plan.warnings.iter().any(|known| known.message == *message) {
                plan.warnings.push(PlanWarning {
                    object_id: 0,
                    point: None,
                    message: message.clone(),
                });
            }
        }
    }
    if project.objects.iter().any(|o| o.visible)
        && !plan.stitches.iter().any(|s| s.command == Command::Stitch)
    {
        return Err("No visible object produces stitches on the 0.1mm machine grid; increase the physical size or edit the reported geometry".into());
    }
    Ok(plan)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn holed_fill_never_sews_through_hole() {
        let mut p = Project::demo();
        p.objects[0].settings.underlay = false;
        p.objects[0].settings.tie_in = false;
        p.objects[0].settings.tie_off = false;
        let stitches = generate(&p).unwrap();
        let mut previous = Point::default();
        for s in stitches {
            if s.command == Command::Stitch {
                let middle = previous.lerp(s.point(), 0.5);
                assert!(
                    !(middle.x > 30.0001
                        && middle.x < 49.9999
                        && middle.y > 22.0001
                        && middle.y < 37.9999),
                    "{middle:?}"
                );
            }
            previous = s.point();
        }
    }
    #[test]
    fn length_and_color_constraints() {
        let mut p = Project::demo();
        // Deliberately keep both identical layers to exercise a color change.
        // Automatic redundant-layer removal has its own geometry regressions.
        p.layering.enabled = false;
        let mut second = p.objects[0].clone();
        second.id = 2;
        second.color = [200, 0, 0];
        p.objects.push(second);
        let stitches = generate(&p).unwrap();
        let st = crate::dst::stats(&stitches);
        assert_eq!(st.color_changes, 1);
        assert!(st.max_stitch_mm <= 3.500001);
        assert_eq!(stitches.last().unwrap().command, Command::End);
    }
    #[test]
    fn invalid_density_rejected() {
        let mut p = Project::demo();
        p.objects[0].settings.spacing_mm = 0.;
        assert!(generate(&p).is_err());
    }
    #[test]
    fn running_closes_and_satin_differs() {
        let mut p = Project::demo();
        p.objects[0].settings.kind = StitchKind::Running;
        let run = generate(&p).unwrap();
        p.objects[0].settings.kind = StitchKind::Satin;
        assert!(run.len() > 1);
        assert!(generate(&p).unwrap_err().contains("paired rails"));
    }
    #[test]
    fn rotated_holes_and_density() {
        let p = Project::demo();
        let rings = p.objects[0].flattened_contours();
        let coarse = fill_spans(&rings, 37., 0.8, 0.).unwrap();
        let fine = fill_spans(&rings, 37., 0.4, 0.).unwrap();
        assert!(fine.len() > coarse.len());
        for (a, b) in fine {
            assert!(inside(a.lerp(b, 0.5), &rings));
        }
    }
    #[test]
    fn rejects_complexity_before_expansion() {
        let ring = vec![
            Point::new(0., 0.),
            Point::new(1., 0.),
            Point::new(1., 20000.),
            Point::new(0., 20000.),
        ];
        assert!(fill_spans(&[ring], 0., 0.15, 0.).is_err());
        let mut project = Project::demo();
        project.objects[0].contours = vec![vec![Point::new(0., 0.); 100_001]];
        assert!(generate(&project).is_err());
    }
}
