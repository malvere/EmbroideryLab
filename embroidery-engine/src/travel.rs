//! Bounded, render-independent concealment and reuse checks for sewn travel.
//! A 0.45mm effective thread-width planning band is a routing heuristic, not textile coverage proof.
use crate::Point;
use std::collections::{BTreeMap, BTreeSet};

const CELL: f64 = 1.;
const COVER_RADIUS: f64 = 0.225;
const MAX_INDEX_ENTRIES: usize = 1_000_000;
const MAX_COVER_WORK: usize = 2_000_000;
const REUSE_CELL: f64 = 0.25;
const MAX_REUSE_CELLS: usize = 250_000;

#[derive(Default)]
pub(super) struct FutureCover {
    segments: Vec<(Point, Point, usize)>,
    cells: BTreeMap<(i64, i64), Vec<usize>>,
    work: usize,
    complete: bool,
}

fn cells(a: Point, b: Point, radius: f64, grid: f64) -> (i64, i64, i64, i64) {
    (
        ((a.x.min(b.x) - radius) / grid).floor() as i64,
        ((a.y.min(b.y) - radius) / grid).floor() as i64,
        ((a.x.max(b.x) + radius) / grid).floor() as i64,
        ((a.y.max(b.y) + radius) / grid).floor() as i64,
    )
}

impl FutureCover {
    pub(super) fn new(segments: impl IntoIterator<Item = (Point, Point, usize)>) -> Self {
        let mut result = Self {
            complete: true,
            ..Self::default()
        };
        let mut entries = 0usize;
        for (a, b, order) in segments {
            let (x0, y0, x1, y1) = cells(a, b, COVER_RADIUS, CELL);
            let count = (x1 - x0 + 1).saturating_mul(y1 - y0 + 1) as usize;
            if entries.saturating_add(count) > MAX_INDEX_ENTRIES {
                result.complete = false;
                result.cells.clear();
                result.segments.clear();
                break;
            }
            let index = result.segments.len();
            result.segments.push((a, b, order));
            for y in y0..=y1 {
                for x in x0..=x1 {
                    result.cells.entry((x, y)).or_default().push(index);
                }
            }
            entries += count;
        }
        result
    }

    pub(super) fn complete(&self) -> bool {
        self.complete
    }

    /// Union analytic capsule intersections along the WHOLE candidate segment.
    /// No sparse point sampling can jump over an exposed section.
    pub(super) fn covers(
        &mut self,
        a: Point,
        b: Point,
        earliest: usize,
        prior_top: Option<Point>,
    ) -> bool {
        if !self.complete {
            return false;
        }
        let (x0, y0, x1, y1) = cells(a, b, 0., CELL);
        let mut candidates = BTreeSet::new();
        for y in y0..=y1 {
            for x in x0..=x1 {
                if let Some(indices) = self.cells.get(&(x, y)) {
                    candidates.extend(indices.iter().copied());
                }
            }
        }
        self.work = self.work.saturating_add(candidates.len());
        if self.work > MAX_COVER_WORK {
            self.complete = false;
            return false;
        }
        let mut intervals = Vec::new();
        for index in candidates {
            let (c, d, order) = self.segments[index];
            if order >= earliest {
                capsule_intervals(a, b, c, d, COVER_RADIUS, &mut intervals);
            }
        }
        // Prefer future top: at least half of the transfer must lie in its
        // footprint. The departure may follow the just-sewn top groove, using
        // the same 0.45mm full width rather than inflating thread thickness.
        intervals.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
        let (mut future_end, mut future_fraction) = (0_f64, 0_f64);
        for &(lo, hi) in &intervals {
            future_fraction += (hi - lo.max(future_end)).max(0.);
            future_end = future_end.max(hi);
        }
        if future_fraction < 0.5 - 1e-9 {
            return false;
        }
        if let Some(prior) = prior_top {
            capsule_intervals(a, b, prior, a, COVER_RADIUS, &mut intervals);
        }
        intervals.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
        let mut end: f64 = 0.;
        for (lo, hi) in intervals {
            if lo > end + 1e-9 {
                return false;
            }
            end = end.max(hi);
            if end >= 1. - 1e-9 {
                return true;
            }
        }
        false
    }
}

fn quadratic_interval(a: f64, b: f64, c: f64, lo: f64, hi: f64, out: &mut Vec<(f64, f64)>) {
    if a.abs() < 1e-18 {
        if b.abs() < 1e-18 {
            if c <= 1e-12 {
                out.push((lo, hi));
            }
        } else {
            let root = -c / b;
            let (l, h) = if b > 0. {
                (lo, hi.min(root))
            } else {
                (lo.max(root), hi)
            };
            if l <= h {
                out.push((l, h));
            }
        }
        return;
    }
    let disc = b * b - 4. * a * c;
    if disc < -1e-12 {
        return;
    }
    let root = disc.max(0.).sqrt();
    let l = lo.max((-b - root) / (2. * a));
    let h = hi.min((-b + root) / (2. * a));
    if l <= h {
        out.push((l, h));
    }
}

fn capsule_intervals(a: Point, b: Point, c: Point, d: Point, r: f64, out: &mut Vec<(f64, f64)>) {
    let (vx, vy) = (b.x - a.x, b.y - a.y);
    let (wx, wy) = (d.x - c.x, d.y - c.y);
    let length2 = wx * wx + wy * wy;
    let (px, py) = (a.x - c.x, a.y - c.y);
    if length2 < 1e-18 {
        quadratic_interval(
            vx * vx + vy * vy,
            2. * (px * vx + py * vy),
            px * px + py * py - r * r,
            0.,
            1.,
            out,
        );
        return;
    }
    let u0 = (px * wx + py * wy) / length2;
    let du = (vx * wx + vy * wy) / length2;
    let mut stops = vec![0., 1.];
    if du.abs() > 1e-15 {
        for u in [0., 1.] {
            let t = (u - u0) / du;
            if t > 0. && t < 1. {
                stops.push(t);
            }
        }
    }
    stops.sort_by(f64::total_cmp);
    for pair in stops.windows(2) {
        let (lo, hi) = (pair[0], pair[1]);
        let u = u0 + du * (lo + hi) * 0.5;
        if u <= 0. || u >= 1. {
            let endpoint = if u <= 0. { c } else { d };
            let (x, y) = (a.x - endpoint.x, a.y - endpoint.y);
            quadratic_interval(
                vx * vx + vy * vy,
                2. * (x * vx + y * vy),
                x * x + y * y - r * r,
                lo,
                hi,
                out,
            );
        } else {
            let q0 = px * wy - py * wx;
            let qv = vx * wy - vy * wx;
            quadratic_interval(
                qv * qv / length2,
                2. * q0 * qv / length2,
                q0 * q0 / length2 - r * r,
                lo,
                hi,
                out,
            );
        }
    }
}

#[derive(Default)]
pub(super) struct TravelMemory {
    cells: BTreeMap<(i64, i64), (usize, f64)>,
    pub(super) limited: bool,
}
impl TravelMemory {
    pub(super) fn allow(&mut self, a: Point, b: Point) -> bool {
        let (x0, y0, x1, y1) = cells(a, b, 0., REUSE_CELL);
        let mut additions = Vec::new();
        for y in y0..=y1 {
            for x in x0..=x1 {
                let Some(fraction) = clip_cell(a, b, x, y) else {
                    continue;
                };
                let length = a.distance(b) * fraction;
                if length < 1e-8 {
                    continue;
                }
                let (visits, total) = self.cells.get(&(x, y)).copied().unwrap_or_default();
                if visits >= 2 || total + length > 2. * 2_f64.sqrt() * REUSE_CELL + 1e-9 {
                    return false;
                }
                additions.push(((x, y), (visits + 1, total + length)));
            }
        }
        if self.cells.len().saturating_add(additions.len()) > MAX_REUSE_CELLS {
            self.limited = true;
            return false;
        }
        self.cells.extend(additions);
        true
    }
}
fn clip_cell(a: Point, b: Point, x: i64, y: i64) -> Option<f64> {
    let (mut lo, mut hi) = (0_f64, 1_f64);
    for (v, delta, min, max) in [
        (
            a.x,
            b.x - a.x,
            x as f64 * REUSE_CELL,
            (x + 1) as f64 * REUSE_CELL,
        ),
        (
            a.y,
            b.y - a.y,
            y as f64 * REUSE_CELL,
            (y + 1) as f64 * REUSE_CELL,
        ),
    ] {
        if delta.abs() < 1e-15 {
            if v < min || v >= max {
                return None;
            }
        } else {
            let (p, q) = ((min - v) / delta, (max - v) / delta);
            lo = lo.max(p.min(q));
            hi = hi.min(p.max(q));
            if hi <= lo {
                return None;
            }
        }
    }
    Some(hi - lo)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn whole_capsule_union_rejects_an_exposed_middle_and_ignores_past_passes() {
        let a = Point::new(0., 0.);
        let b = Point::new(3., 0.);
        let mut cover = FutureCover::new([(a, a, 0), (b, b, 1)]);
        assert!(!cover.covers(a, b, 0, None));
        let mut cover = FutureCover::new([(a, b, 0)]);
        assert!(cover.covers(a, b, 0, None));
        assert!(!cover.covers(a, b, 1, None));
    }
    #[test]
    fn dense_row_edge_is_concealed_but_sparse_outer_edge_is_not() {
        let mut cover = FutureCover::new([(Point::new(0., 0.4), Point::new(20., 0.4), 0)]);
        assert!(cover.covers(
            Point::new(0., 0.),
            Point::new(0., 0.4),
            0,
            Some(Point::new(20., 0.))
        ));
        assert!(!cover.covers(
            Point::new(0., -1.6),
            Point::new(0., 0.4),
            0,
            Some(Point::new(20., 0.))
        ));
    }
    #[test]
    fn repeated_travel_is_bounded_and_rejected_path_does_not_charge_cells() {
        let mut memory = TravelMemory::default();
        let a = Point::new(0.1, 0.1);
        let b = Point::new(1.1, 0.1);
        assert!(memory.allow(a, b));
        assert!(memory.allow(b, a));
        assert!(!memory.allow(a, b));
        assert!(memory.allow(Point::new(0.1, 1.1), Point::new(1.1, 1.1)));
    }
}
