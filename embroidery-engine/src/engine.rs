//! Clean independent baseline generators. Complex fill uses rotated even-odd
//! scanline clipping; transfers crossing a hole remain non-sewing jumps.
use crate::model::*;
const MAX_COMMANDS: usize = 1_000_000;
const MAX_SPANS: usize = 250_000;

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
    segments: &mut Vec<(Point, Point, StitchKind)>,
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
) -> Result<Vec<(Point, Point, StitchKind)>, String> {
    let stations = satin_stations(column, settings.spacing_mm)?;
    let mut segments = Vec::new();
    if settings.underlay || settings.edge_underlay || settings.zigzag_underlay {
        for path in satin_underlay_paths(column, settings)? {
            append_underlay(
                &mut segments,
                &path,
                if modern {
                    (settings.underlay_length_mm - 0.15).min(settings.length_mm - 0.15)
                } else {
                    (settings.length_mm - 0.15).min(2.)
                },
                rings,
            )?;
        }
    }
    let mut contacts = [0_usize; 2];
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
            (edge.lerp(opposite, -settings.pull_comp_mm / width), edge)
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
    Ok(segments)
}

/// PCA direction of the input boundary. A heuristic, not a digitizer's semantic
/// understanding of a satin column or an estimate of fabric distortion.
pub fn automatic_angle(rings: &[Vec<Point>]) -> f64 {
    let points: Vec<_> = rings.iter().flatten().collect();
    if points.is_empty() {
        return 0.;
    }
    let n = points.len() as f64;
    let mx = points.iter().map(|p| p.x).sum::<f64>() / n;
    let my = points.iter().map(|p| p.y).sum::<f64>() / n;
    let xx = points.iter().map(|p| (p.x - mx).powi(2)).sum::<f64>();
    let yy = points.iter().map(|p| (p.y - my).powi(2)).sum::<f64>();
    let xy = points.iter().map(|p| (p.x - mx) * (p.y - my)).sum::<f64>();
    (0.5 * (2. * xy).atan2(xx - yy)).to_degrees()
}

/// Return alternating scanline spans, with holes removed. Compensation extends
/// only the two exterior span ends; internal holes keep their original boundary.
pub fn fill_spans(
    rings: &[Vec<Point>],
    angle_deg: f64,
    spacing: f64,
    compensation: f64,
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
        let y = min_y + spacing * (row as f64 + 0.5);
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
        let last = xs.len().saturating_sub(2);
        for (i, pair) in xs.as_chunks::<2>().0.iter().enumerate() {
            let mut left = pair[0];
            let mut right = pair[1];
            if right - left < 0.05 {
                continue;
            }
            if i == 0 {
                left -= compensation;
            }
            if i * 2 == last {
                right += compensation;
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
            || (distance > 0.05 && (distance > max_length || !safe_connector(cursor, a, rings)))
        {
            add_jump_cost(&mut cost, cursor, a);
            cursor = a;
        } else if distance > 0.05 {
            cursor = a;
        }
        // Builder::line omits movements shorter than 0.05mm. Longer spans
        // always finish exactly at their endpoint, including Tatami tails.
        if cursor.distance(b) >= 0.05 {
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
    compensation: f64,
    mut cursor: Point,
    max_length: f64,
    initial_jump: bool,
) -> Result<Vec<(Point, Point)>, String> {
    struct Span {
        a: Point,
        b: Point,
        left: f64,
        right: f64,
        y: f64,
    }
    let angle = angle_deg.to_radians();
    let raw = fill_spans(rings, angle_deg, spacing, compensation)?;
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
    current: Point,
    color: [u8; 3],
    object_id: u64,
    max_length: f64,
    overflow: bool,
}
impl Builder {
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
        self.current = p;
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
        self.line(near);
        self.line(anchor);
        self.line(near);
        self.line(anchor);
    }
    fn span(
        &mut self,
        start: Point,
        end: Point,
        rings: &[Vec<Point>],
        tatami_phase: Option<(f64, usize)>,
        kind: StitchKind,
    ) {
        if self.current.distance(start) > 0.05 {
            if self.current.distance(start) <= self.max_length
                && safe_connector(self.current, start, rings)
            {
                self.line(start);
            } else {
                self.emit(start, Command::Jump);
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
        self.line(end);
    }
    fn sew_segments(
        &mut self,
        segments: &[(Point, Point, StitchKind)],
        rings: &[Vec<Point>],
        settings: &Settings,
        angle_deg: f64,
        modern_satin: bool,
    ) -> Result<(), String> {
        let angle = angle_deg.to_radians();
        let min_y = rings
            .iter()
            .flatten()
            .map(|&p| rotate(p, -angle).y)
            .fold(f64::INFINITY, f64::min);
        if let Some(&(start, end, _)) = segments.first() {
            self.emit(start, Command::Jump);
            if settings.tie_in {
                self.tie(end);
            }
        }
        for &(a, z, kind) in segments {
            let maximum = self.max_length;
            if modern_satin {
                if kind == StitchKind::Running {
                    self.max_length = maximum.min(settings.underlay_length_mm - 0.15);
                }
                if self.current != a {
                    if safe_connector(self.current, a, rings) {
                        self.path_line(a);
                    } else {
                        self.emit(a, Command::Jump);
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
            } else {
                self.span(a, z, rings, phase, kind);
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
    project.validate_limits()?;
    let mut b = Builder {
        out: Vec::new(),
        current: Point::default(),
        color: [0; 3],
        object_id: 0,
        max_length: 3.5,
        overflow: false,
    };
    let mut has_color = false;
    let mut work_budget = 0_usize;
    for object in project.objects.iter().filter(|o| o.visible) {
        let rings = object.flattened_contours();
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
        if has_color && b.color != object.color {
            b.emit(b.current, Command::ColorChange);
        }
        b.color = object.color;
        b.object_id = object.id;
        // DST rounds both absolute endpoints to 0.1mm. Their relative vector
        // can grow by sqrt(2)*0.1mm, so reserve 0.15mm before subdivision.
        b.max_length = (s.length_mm - 0.15).max(0.15);
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
                b.emit(path[0], Command::Jump);
                if s.tie_in {
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
            let modern = project.version >= 4 || column.uses_v4() || s.uses_v4();
            let segments = paired_satin_segments(column, s, &rings, modern)
                .map_err(|e| format!("{}: {e}", object.name))?;
            b.sew_segments(&segments, &rings, s, 0., modern)?;
            continue;
        }
        let angle = if s.auto_angle {
            automatic_angle(&rings)
        } else {
            s.angle_deg
        };
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
            if s.underlay {
                work_budget = work_budget
                    .saturating_add(estimated_work(angle + 90., (s.spacing_mm * 5.).max(1.5)));
            }
            if work_budget > 100_000_000 {
                return Err(
                    "Project exceeds scanline work budget; simplify contours or increase spacing"
                        .into(),
                );
            }
        }
        let mut segments = Vec::new();
        if s.underlay && s.kind != StitchKind::Running {
            segments.extend(
                routed_fill_spans(
                    &rings,
                    angle + 90.,
                    (s.spacing_mm * 5.).max(1.5),
                    0.,
                    b.current,
                    b.max_length,
                    true,
                )?
                .into_iter()
                .map(|(a, z)| (a, z, StitchKind::Running)),
            );
        }
        // The sparse underlay may end in a sub-0.05mm span omitted by the
        // emitter. Carry its real cursor into the top-fill planner as well.
        let mut fill_cursor = segments.first().map_or(b.current, |&(a, _, _)| a);
        for &(a, z, _) in &segments {
            if fill_cursor.distance(a) > 0.05 {
                fill_cursor = a;
            }
            if fill_cursor.distance(z) >= 0.05 {
                fill_cursor = z;
            }
        }
        match s.kind {
            StitchKind::Running => {
                for ring in &rings {
                    for (&a, &z) in ring
                        .iter()
                        .zip(ring.iter().cycle().skip(1))
                        .take(ring.len())
                    {
                        segments.push((a, z, StitchKind::Running));
                    }
                }
            }
            StitchKind::Tatami => segments.extend(
                routed_fill_spans(
                    &rings,
                    angle,
                    s.spacing_mm,
                    s.pull_comp_mm,
                    fill_cursor,
                    b.max_length,
                    segments.is_empty(),
                )?
                .into_iter()
                .map(|(a, z)| (a, z, StitchKind::Tatami)),
            ),
            kind => segments.extend(
                fill_spans(&rings, angle, s.spacing_mm, s.pull_comp_mm)?
                    .into_iter()
                    .map(|(a, z)| (a, z, kind)),
            ),
        }
        b.sew_segments(&segments, &rings, s, angle, false)?;
    }
    b.emit(b.current, Command::End);
    if b.overflow {
        return Err("Generation exceeds one million commands; increase spacing".into());
    }
    Ok(b.out)
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
        let satin = generate(&p).unwrap();
        assert!(satin.len() > run.len());
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
