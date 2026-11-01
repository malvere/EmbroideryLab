//! Shared satin geometry operations for the native editor and the CLI.
use crate::model::{CubicBezier, Point, RailHandles, SatinColumn, SatinRung};
use cavalier_contours::polyline::{PlineSource, PlineSourceMut, Polyline};

fn finite(point: Point) -> bool {
    point.x.is_finite() && point.y.is_finite() && point.x.abs() <= 9990. && point.y.abs() <= 9990.
}

fn rail_mut(column: &mut SatinColumn, right: bool) -> (&mut Vec<Point>, &mut Vec<RailHandles>) {
    if right {
        (&mut column.right, &mut column.right_handles)
    } else {
        (&mut column.left, &mut column.left_handles)
    }
}

/// Split a straight or cubic segment without changing its shape.
pub fn insert_node(
    column: &mut SatinColumn,
    right: bool,
    segment: usize,
    t: f64,
) -> Result<usize, String> {
    if !t.is_finite() || !(0.000001..0.999999).contains(&t) {
        return Err("Insert position must lie inside the rail segment".into());
    }
    let closed = column.closed;
    let (nodes, handles) = rail_mut(column, right);
    if nodes.len() >= 1024 || nodes.len() < 2 || segment >= nodes.len() - usize::from(!closed) {
        return Err("Select an existing rail segment; rails allow at most 1024 nodes".into());
    }
    handles.resize(nodes.len(), RailHandles::default());
    let j = (segment + 1) % nodes.len();
    let start = nodes[segment];
    let end = nodes[j];
    let curved = handles[segment].outgoing.is_some() || handles[j].incoming.is_some();
    let (point, middle) = if curved {
        let a = start.lerp(handles[segment].outgoing.unwrap_or(start), t);
        let b = handles[segment]
            .outgoing
            .unwrap_or(start)
            .lerp(handles[j].incoming.unwrap_or(end), t);
        let c = handles[j].incoming.unwrap_or(end).lerp(end, t);
        let d = a.lerp(b, t);
        let e = b.lerp(c, t);
        let point = d.lerp(e, t);
        handles[segment].outgoing = Some(a);
        handles[j].incoming = Some(c);
        (
            point,
            RailHandles {
                incoming: Some(d),
                outgoing: Some(e),
                smooth: true,
            },
        )
    } else {
        (start.lerp(end, t), RailHandles::default())
    };
    let index = segment + 1;
    nodes.insert(index, point);
    handles.insert(index, middle);
    Ok(index)
}

pub fn remove_node(column: &mut SatinColumn, right: bool, index: usize) -> Result<(), String> {
    let minimum = if column.closed { 3 } else { 2 };
    let (nodes, handles) = rail_mut(column, right);
    if index >= nodes.len() || nodes.len() <= minimum {
        return Err(format!("A satin rail needs at least {minimum} nodes"));
    }
    nodes.remove(index);
    if !handles.is_empty() {
        handles.resize(nodes.len() + 1, RailHandles::default());
        handles.remove(index);
    }
    Ok(())
}

pub fn set_node(
    column: &mut SatinColumn,
    right: bool,
    index: usize,
    point: Point,
) -> Result<(), String> {
    if !finite(point) {
        return Err("Invalid rail node coordinates".into());
    }
    let (nodes, handles) = rail_mut(column, right);
    let node = nodes.get_mut(index).ok_or("Select a rail node")?;
    let old = *node;
    if let Some(h) = handles.get_mut(index) {
        for handle in [&mut h.incoming, &mut h.outgoing].into_iter().flatten() {
            handle.x += point.x - old.x;
            handle.y += point.y - old.y;
        }
    }
    *node = point;
    Ok(())
}

pub fn set_smooth(
    column: &mut SatinColumn,
    right: bool,
    index: usize,
    smooth: bool,
) -> Result<(), String> {
    let closed = column.closed;
    let (nodes, handles) = rail_mut(column, right);
    let &p = nodes.get(index).ok_or("Select a rail node")?;
    handles.resize(nodes.len(), RailHandles::default());
    if !smooth {
        handles[index] = RailHandles::default();
        return Ok(());
    }
    let previous = if index > 0 {
        nodes[index - 1]
    } else if closed {
        nodes[nodes.len() - 1]
    } else {
        p
    };
    let next = if index + 1 < nodes.len() {
        nodes[index + 1]
    } else if closed {
        nodes[0]
    } else {
        p
    };
    let length = previous.distance(next);
    if length < 1e-7 {
        return Err("Cannot smooth coincident neighbours".into());
    }
    let unit = Point::new(
        (next.x - previous.x) / length,
        (next.y - previous.y) / length,
    );
    let along = |distance: f64| Point::new(p.x + unit.x * distance, p.y + unit.y * distance);
    handles[index] = RailHandles {
        incoming: (index > 0 || closed).then(|| along(-p.distance(previous) / 3.)),
        outgoing: (index + 1 < nodes.len() || closed).then(|| along(p.distance(next) / 3.)),
        smooth: true,
    };
    Ok(())
}

/// Moving a smooth handle keeps the opposite handle collinear and preserves its length.
pub fn set_handle(
    column: &mut SatinColumn,
    right: bool,
    index: usize,
    outgoing: bool,
    point: Point,
) -> Result<(), String> {
    if !finite(point) {
        return Err("Invalid rail handle coordinates".into());
    }
    let (nodes, handles) = rail_mut(column, right);
    let &node = nodes.get(index).ok_or("Select a rail node")?;
    handles.resize(nodes.len(), RailHandles::default());
    let h = &mut handles[index];
    let opposite = if outgoing { h.incoming } else { h.outgoing };
    let distance = node.distance(point);
    let mirrored = if h.smooth && distance > 1e-9 {
        opposite.map(|p| node.lerp(point, -node.distance(p) / distance))
    } else {
        opposite
    };
    if outgoing {
        h.outgoing = Some(point);
        h.incoming = mirrored;
    } else {
        h.incoming = Some(point);
        h.outgoing = mirrored;
    }
    Ok(())
}

pub fn reverse(column: &mut SatinColumn) -> Result<(), String> {
    if column.left.len() < 2
        || column.right.len() < 2
        || (!column.left_handles.is_empty() && column.left_handles.len() != column.left.len())
        || (!column.right_handles.is_empty() && column.right_handles.len() != column.right.len())
    {
        return Err("Cannot reverse an incomplete satin column".into());
    }
    for (nodes, handles) in [
        (&mut column.left, &mut column.left_handles),
        (&mut column.right, &mut column.right_handles),
    ] {
        // Keep the origin of closed rails fixed so seam/rung fractions transform exactly.
        if column.closed {
            nodes[1..].reverse();
            if !handles.is_empty() {
                handles[1..].reverse();
            }
        } else {
            nodes.reverse();
            handles.reverse();
        }
        for h in handles {
            std::mem::swap(&mut h.incoming, &mut h.outgoing);
        }
    }
    for rung in &mut column.rungs {
        rung.left = 1. - rung.left;
        rung.right = 1. - rung.right;
    }
    column.rungs.reverse();
    if column.closed {
        column.seam = (1. - column.seam).rem_euclid(1.);
    }
    Ok(())
}

pub fn add_rung(column: &mut SatinColumn, left: f64, right: f64) -> Result<usize, String> {
    if !left.is_finite()
        || !right.is_finite()
        || !(0.000001..0.999999).contains(&left)
        || !(0.000001..0.999999).contains(&right)
        || column.rungs.len() >= 1024
    {
        return Err(
            "Rungs need interior rail positions in (0, 1); at most 1024 are allowed".into(),
        );
    }
    let index = column.rungs.partition_point(|r| r.left < left);
    if index > 0
        && (column.rungs[index - 1].left >= left - 1e-8
            || column.rungs[index - 1].right >= right - 1e-8)
        || index < column.rungs.len()
            && (column.rungs[index].left <= left + 1e-8
                || column.rungs[index].right <= right + 1e-8)
    {
        return Err("Rungs must advance along both rails without crossing".into());
    }
    column.rungs.insert(index, SatinRung { left, right });
    Ok(index)
}

pub fn remove_rung(column: &mut SatinColumn, index: usize) -> Result<(), String> {
    if index >= column.rungs.len() {
        return Err("Select a satin rung".into());
    }
    column.rungs.remove(index);
    Ok(())
}

pub fn sample_path(path: &[Point], fraction: f64) -> Result<Point, String> {
    if path.len() < 2
        || path.iter().any(|&p| !finite(p))
        || !fraction.is_finite()
        || !(0.0..=1.0).contains(&fraction)
    {
        return Err("Invalid path or normalized position".into());
    }
    let length: f64 = path.windows(2).map(|p| p[0].distance(p[1])).sum();
    if length < 1e-7 {
        return Err("Path has no usable length".into());
    }
    let target = fraction * length;
    let mut accumulated = 0.;
    for pair in path.windows(2) {
        let step = pair[0].distance(pair[1]);
        if accumulated + step >= target && step > 1e-12 {
            return Ok(pair[0].lerp(pair[1], (target - accumulated) / step));
        }
        accumulated += step;
    }
    Ok(*path.last().unwrap())
}

/// Rotate a closed flattened rail to an arbitrary seam without changing its shape.
pub fn rotate_closed(path: &[Point], seam: f64) -> Result<Vec<Point>, String> {
    if path.len() < 4
        || path[0].distance(*path.last().unwrap()) > 1e-7
        || !seam.is_finite()
        || !(0.0..1.0).contains(&seam)
    {
        return Err("Invalid closed rail or seam".into());
    }
    let total: f64 = path.windows(2).map(|p| p[0].distance(p[1])).sum();
    let point = sample_path(path, seam)?;
    let mut accumulated = 0.;
    let mut out = vec![point];
    let mut rest = Vec::new();
    for pair in path.windows(2) {
        accumulated += pair[0].distance(pair[1]);
        if accumulated > seam * total + 1e-9 {
            out.push(pair[1]);
        } else {
            rest.push(pair[1]);
        }
    }
    out.extend(rest);
    if out.last().is_some_and(|p| p.distance(point) > 1e-8) {
        out.push(point);
    }
    out.dedup_by(|a, b| a.distance(*b) < 1e-8);
    Ok(out)
}

fn curve_parts(curve: &CubicBezier, t: f64) -> (CubicBezier, CubicBezier) {
    let a = curve.start.lerp(curve.control1, t);
    let b = curve.control1.lerp(curve.control2, t);
    let c = curve.control2.lerp(curve.end, t);
    let d = a.lerp(b, t);
    let e = b.lerp(c, t);
    let middle = d.lerp(e, t);
    (
        CubicBezier {
            start: curve.start,
            control1: a,
            control2: d,
            end: middle,
        },
        CubicBezier {
            start: middle,
            control1: e,
            control2: c,
            end: curve.end,
        },
    )
}

type RailPiece = (Vec<Point>, Vec<RailHandles>);

fn open_at_seam(column: &SatinColumn, right: bool, seam: f64) -> Result<RailPiece, String> {
    let (source, handles) = if right {
        (&column.right, &column.right_handles)
    } else {
        (&column.left, &column.left_handles)
    };
    let mut nodes = source.clone();
    nodes.push(source[0]);
    let mut handles = handles.clone();
    handles.resize(source.len(), RailHandles::default());
    handles.push(handles[0]);
    if seam < 1e-9 {
        return Ok((nodes, handles));
    }
    let temporary = SatinColumn {
        left: nodes,
        left_handles: handles,
        ..SatinColumn::default()
    };
    let (before, mut after) = split_editable_rail(&temporary, false, seam)?;
    if let Some(last) = after.1.last_mut() {
        last.outgoing = before.1[0].outgoing;
        last.smooth = before.1[0].smooth;
    }
    after.0.extend_from_slice(&before.0[1..]);
    after.1.extend_from_slice(&before.1[1..]);
    Ok(after)
}

fn split_editable_rail(
    column: &SatinColumn,
    right: bool,
    fraction: f64,
) -> Result<(RailPiece, RailPiece), String> {
    let (nodes, handles) = if right {
        (&column.right, &column.right_handles)
    } else {
        (&column.left, &column.left_handles)
    };
    let curves: Vec<_> = (0..nodes.len() - 1)
        .map(|i| rail_curve(column, right, i))
        .collect::<Result<_, _>>()?;
    let length = |curve: &CubicBezier| {
        curve
            .flatten(0.002)
            .windows(2)
            .map(|p| p[0].distance(p[1]))
            .sum::<f64>()
    };
    let lengths: Vec<_> = curves.iter().map(length).collect();
    let target = lengths.iter().sum::<f64>() * fraction;
    let mut cumulative = 0.;
    let mut segment = 0;
    for (i, step) in lengths.iter().enumerate() {
        segment = i;
        if cumulative + step >= target {
            break;
        }
        cumulative += step;
    }
    let mut all_handles = handles.clone();
    all_handles.resize(nodes.len(), RailHandles::default());
    if (target - cumulative).abs() < 1e-7 && segment > 0 {
        return Ok((
            (nodes[..=segment].to_vec(), all_handles[..=segment].to_vec()),
            (nodes[segment..].to_vec(), all_handles[segment..].to_vec()),
        ));
    }
    if (target - cumulative - lengths[segment]).abs() < 1e-7 && segment + 2 < nodes.len() {
        return Ok((
            (
                nodes[..=segment + 1].to_vec(),
                all_handles[..=segment + 1].to_vec(),
            ),
            (
                nodes[segment + 1..].to_vec(),
                all_handles[segment + 1..].to_vec(),
            ),
        ));
    }
    let curve = &curves[segment];
    let mut low = 0.;
    let mut high = 1.;
    for _ in 0..28 {
        let middle = (low + high) / 2.;
        if length(&curve_parts(curve, middle).0) < target - cumulative {
            low = middle;
        } else {
            high = middle;
        }
    }
    let (a, b) = curve_parts(curve, (low + high) / 2.);
    let mut before_nodes = nodes[..=segment].to_vec();
    before_nodes.push(a.end);
    let mut after_nodes = vec![a.end];
    after_nodes.extend_from_slice(&nodes[segment + 1..]);
    let mut before_handles = all_handles[..=segment].to_vec();
    let mut after_handles = vec![RailHandles {
        outgoing: Some(b.control1),
        ..RailHandles::default()
    }];
    after_handles.extend_from_slice(&all_handles[segment + 1..]);
    before_handles[segment].outgoing = Some(a.control1);
    before_handles.push(RailHandles {
        incoming: Some(a.control2),
        ..RailHandles::default()
    });
    after_handles[1].incoming = Some(b.control2);
    Ok(((before_nodes, before_handles), (after_nodes, after_handles)))
}

pub fn right_fraction(column: &SatinColumn, fraction: f64) -> Result<f64, String> {
    if !fraction.is_finite() || !(0.0..=1.0).contains(&fraction) {
        return Err("Invalid normalized rail position".into());
    }
    let mut anchors = vec![SatinRung::default()];
    anchors.extend_from_slice(&column.rungs);
    anchors.sort_by(|a, b| a.left.total_cmp(&b.left));
    anchors.push(SatinRung {
        left: 1.,
        right: 1.,
    });
    if anchors
        .windows(2)
        .any(|p| p[1].left - p[0].left <= 1e-8 || p[1].right - p[0].right <= 1e-8)
    {
        return Err("Direction rungs cross or repeat".into());
    }
    let index = anchors
        .partition_point(|p| p.left < fraction)
        .clamp(1, anchors.len() - 1);
    let (a, b) = (anchors[index - 1], anchors[index]);
    Ok(a.right + (b.right - a.right) * (fraction - a.left) / (b.left - a.left))
}

pub fn split(
    column: &SatinColumn,
    left: f64,
    right: f64,
) -> Result<(SatinColumn, SatinColumn), String> {
    if !(0.000001..0.999999).contains(&left) || !(0.000001..0.999999).contains(&right) {
        return Err("Split must lie inside both rails".into());
    }
    column.flattened()?;
    let right_seam = right_fraction(column, column.seam)?;
    let (((la, lah), (lb, lbh)), ((ra, rah), (rb, rbh))) = if column.closed {
        let (left_nodes, left_handles) = open_at_seam(column, false, column.seam)?;
        let (right_nodes, right_handles) = open_at_seam(column, true, right_seam)?;
        let opened = SatinColumn {
            left: left_nodes,
            right: right_nodes,
            left_handles,
            right_handles,
            ..SatinColumn::default()
        };
        (
            split_editable_rail(&opened, false, left)?,
            split_editable_rail(&opened, true, right)?,
        )
    } else {
        (
            split_editable_rail(column, false, left)?,
            split_editable_rail(column, true, right)?,
        )
    };
    let mut a = SatinColumn {
        left: la,
        right: ra,
        left_handles: lah,
        right_handles: rah,
        first_right: column.first_right,
        ..SatinColumn::default()
    };
    let mut b = SatinColumn {
        left: lb,
        right: rb,
        left_handles: lbh,
        right_handles: rbh,
        first_right: column.first_right,
        ..SatinColumn::default()
    };
    let mut rungs = column.rungs.clone();
    if column.closed && column.seam > 1e-9 {
        rungs.push(SatinRung::default());
    }
    for rung in &rungs {
        let l = if column.closed {
            (rung.left - column.seam).rem_euclid(1.)
        } else {
            rung.left
        };
        let r = if column.closed {
            (rung.right - right_seam).rem_euclid(1.)
        } else {
            rung.right
        };
        if l.abs() < 1e-8 && r.abs() < 1e-8 {
            continue;
        }
        if l < left - 1e-8 && r < right - 1e-8 {
            a.rungs.push(SatinRung {
                left: l / left,
                right: r / right,
            });
        } else if l > left + 1e-8 && r > right + 1e-8 {
            b.rungs.push(SatinRung {
                left: (l - left) / (1. - left),
                right: (r - right) / (1. - right),
            });
        } else if (l - left).abs() > 1e-8 || (r - right).abs() > 1e-8 {
            return Err(
                "Split crosses an existing direction rung; split at that rung or remove it".into(),
            );
        }
    }
    a.rungs.sort_by(|a, b| a.left.total_cmp(&b.left));
    b.rungs.sort_by(|a, b| a.left.total_cmp(&b.left));
    crate::engine::satin_stations(&a, 0.4)?;
    crate::engine::satin_stations(&b, 0.4)?;
    Ok((a, b))
}

fn align_closed(path: &[Point], target: Point) -> Result<Vec<Point>, String> {
    rotate_closed(path, closest_fraction(path, target)?.rem_euclid(1.))
}

fn closest_fraction(path: &[Point], target: Point) -> Result<f64, String> {
    let total: f64 = path.windows(2).map(|p| p[0].distance(p[1])).sum();
    if total < 1e-7 {
        return Err("Offset rail has no usable length".into());
    }
    let mut best = (f64::INFINITY, 0.);
    let mut length = 0.;
    for pair in path.windows(2) {
        let dx = pair[1].x - pair[0].x;
        let dy = pair[1].y - pair[0].y;
        let norm = dx * dx + dy * dy;
        let step = norm.sqrt();
        if norm > 1e-12 {
            let t =
                (((target.x - pair[0].x) * dx + (target.y - pair[0].y) * dy) / norm).clamp(0., 1.);
            let distance = pair[0].lerp(pair[1], t).distance(target);
            if distance < best.0 {
                best = (distance, (length + step * t) / total);
            }
        }
        length += step;
    }
    Ok(best.1)
}

fn node_normal(nodes: &[Point], index: usize, closed: bool) -> Result<Point, String> {
    let unit_normal = |a: Point, b: Point| {
        let distance = a.distance(b);
        Point::new(-(b.y - a.y) / distance, (b.x - a.x) / distance)
    };
    let previous = if index > 0 {
        index - 1
    } else if closed {
        nodes.len() - 1
    } else {
        0
    };
    let next = if index + 1 < nodes.len() {
        index + 1
    } else if closed {
        0
    } else {
        nodes.len() - 1
    };
    let incoming = (previous != index).then(|| unit_normal(nodes[previous], nodes[index]));
    let outgoing = (next != index).then(|| unit_normal(nodes[index], nodes[next]));
    let normal = match (incoming, outgoing) {
        (Some(a), Some(b)) => Point::new(a.x + b.x, a.y + b.y),
        (Some(a), None) | (None, Some(a)) => a,
        _ => return Err("Outline needs at least two distinct points".into()),
    };
    let length = normal.x.hypot(normal.y);
    if length < 1e-7 {
        return Err("Outline path doubles back; split that corner".into());
    }
    Ok(Point::new(normal.x / length, normal.y / length))
}

/// Create a centred satin outline. Ambiguous or collapsed offsets fail explicitly.
pub fn outline(path: &[Point], closed: bool, width: f64) -> Result<SatinColumn, String> {
    if !width.is_finite()
        || !(0.15..=50.).contains(&width)
        || path.len() < if closed { 3 } else { 2 }
        || path.len() > 1024
        || path.iter().any(|&p| !finite(p))
    {
        return Err("Outline needs a valid path and width 0.15–50 mm".into());
    }
    let mut nodes = path.to_vec();
    if closed && nodes[0].distance(*nodes.last().unwrap()) < 1e-8 {
        nodes.pop();
    }
    if nodes.windows(2).any(|p| p[0].distance(p[1]) < 1e-7) {
        return Err("Outline path has duplicate nodes".into());
    }
    let mut source = if closed {
        Polyline::<f64>::new_closed()
    } else {
        Polyline::<f64>::new()
    };
    for p in &nodes {
        source.add(p.x, p.y, 0.);
    }
    if source.scan_for_self_intersect() {
        return Err("Outline path intersects itself; split or correct it".into());
    }
    let mut rails = Vec::new();
    for offset in [width / 2., -width / 2.] {
        let mut result = source.parallel_offset(offset);
        if result.len() != 1 {
            return Err("Outline offset collapses or separates into several regions; reduce width or split the path".into());
        }
        let result = result.remove(0);
        let flattened = result
            .arcs_to_approx_lines(0.02)
            .ok_or("Cannot flatten outline arcs")?;
        let mut rail: Vec<Point> = flattened
            .iter_vertexes()
            .map(|v| Point::new(v.x, v.y))
            .collect();
        if closed {
            if rail.len() < 3 {
                return Err("Outline offset has collapsed".into());
            }
            rail.push(rail[0]);
            let normal = node_normal(&nodes, 0, true)?;
            let target = Point::new(
                nodes[0].x + normal.x * offset,
                nodes[0].y + normal.y * offset,
            );
            rail = align_closed(&rail, target)?;
            rail.pop();
        } else if rail
            .first()
            .is_some_and(|p| p.distance(nodes[0]) > rail.last().unwrap().distance(nodes[0]))
        {
            rail.reverse();
        }
        rails.push(rail);
    }
    let mut column = SatinColumn {
        left: rails.remove(0),
        right: rails.remove(0),
        closed,
        ..SatinColumn::default()
    };
    let mut left_path = column.left.clone();
    let mut right_path = column.right.clone();
    if closed {
        left_path.push(left_path[0]);
        right_path.push(right_path[0]);
    }
    for i in 1..nodes.len() - usize::from(!closed) {
        let normal = node_normal(&nodes, i, closed)?;
        let left_target = Point::new(
            nodes[i].x + normal.x * width / 2.,
            nodes[i].y + normal.y * width / 2.,
        );
        let right_target = Point::new(
            nodes[i].x - normal.x * width / 2.,
            nodes[i].y - normal.y * width / 2.,
        );
        let left = closest_fraction(&left_path, left_target)?;
        let right = closest_fraction(&right_path, right_target)?;
        if (1e-6..1. - 1e-6).contains(&left) && (1e-6..1. - 1e-6).contains(&right) {
            add_rung(&mut column, left, right)?;
        }
    }
    crate::engine::satin_stations(&column, 0.4)?;
    Ok(column)
}

/// Expose the cubic for drawing editable handle segments consistently.
pub fn rail_curve(
    column: &SatinColumn,
    right: bool,
    segment: usize,
) -> Result<CubicBezier, String> {
    let (nodes, handles) = if right {
        (&column.right, &column.right_handles)
    } else {
        (&column.left, &column.left_handles)
    };
    if segment >= nodes.len().saturating_sub(usize::from(!column.closed)) || nodes.len() < 2 {
        return Err("Invalid rail segment".into());
    }
    let next = (segment + 1) % nodes.len();
    Ok(CubicBezier {
        start: nodes[segment],
        control1: handles
            .get(segment)
            .and_then(|h| h.outgoing)
            .unwrap_or(nodes[segment]),
        control2: handles
            .get(next)
            .and_then(|h| h.incoming)
            .unwrap_or(nodes[next]),
        end: nodes[next],
    })
}
