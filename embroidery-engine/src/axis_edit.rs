//! Editable skeleton graph. Branch geometry is canonical; Running paths are a cache.
use crate::{Object, Point, SatinColumn, SatinRung};
use serde::{Deserialize, Serialize};

const EPS: f64 = 1e-8;
const WORK_BUDGET: usize = 5_000_000;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AxisVertex {
    pub id: u64,
    pub point: Point,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AxisBranch {
    pub id: u64,
    pub start_vertex: u64,
    pub end_vertex: u64,
    pub points: Vec<Point>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AxisGraph {
    pub source_object_id: u64,
    /// A snapshot, so editing or deleting the source does not change width silently.
    pub source_contours: Vec<Vec<Point>>,
    pub pixel_size_mm: f64,
    pub vertices: Vec<AxisVertex>,
    pub branches: Vec<AxisBranch>,
}

fn finite(p: &Point) -> bool {
    p.x.is_finite() && p.y.is_finite() && p.x.abs() <= 9990. && p.y.abs() <= 9990.
}

impl AxisGraph {
    pub fn degree(&self, vertex: u64) -> usize {
        self.branches
            .iter()
            .map(|b| usize::from(b.start_vertex == vertex) + usize::from(b.end_vertex == vertex))
            .sum()
    }

    pub fn validate(&self) -> Result<(), String> {
        if !self.pixel_size_mm.is_finite() || !(0.005..=10.).contains(&self.pixel_size_mm) {
            return Err("Invalid centerline graph pixel size".into());
        }
        let nodes = self
            .source_contours
            .iter()
            .map(Vec::len)
            .sum::<usize>()
            .saturating_add(self.branches.iter().map(|b| b.points.len()).sum::<usize>())
            .saturating_add(self.vertices.len());
        if nodes > 200_000 || self.branches.len() > 100_000 {
            return Err("Centerline graph exceeds its geometry budget".into());
        }
        if self.source_contours.is_empty()
            || self
                .source_contours
                .iter()
                .any(|c| c.len() < 3 || c.iter().any(|p| !finite(p)))
        {
            return Err("Centerline graph requires finite source contours".into());
        }
        let mut vertex_ids = std::collections::BTreeMap::new();
        for vertex in &self.vertices {
            if !finite(&vertex.point) || vertex_ids.insert(vertex.id, vertex.point).is_some() {
                return Err("Invalid or duplicate centerline vertex".into());
            }
        }
        let mut branch_ids = std::collections::BTreeSet::new();
        for branch in &self.branches {
            if !branch_ids.insert(branch.id)
                || branch.points.len() < 2
                || branch.points.iter().any(|p| !finite(p))
                || branch.points.windows(2).all(|w| w[0].distance(w[1]) <= EPS)
            {
                return Err("Invalid or duplicate centerline branch".into());
            }
            for (id, point) in [
                (branch.start_vertex, branch.points[0]),
                (branch.end_vertex, *branch.points.last().unwrap()),
            ] {
                if vertex_ids.get(&id) != Some(&point) {
                    return Err("Centerline branch does not meet its shared vertex".into());
                }
            }
        }
        if self.branches.is_empty() {
            return Err("Centerline graph requires at least one branch".into());
        }
        Ok(())
    }

    pub fn paths(&self) -> Vec<Vec<Point>> {
        self.branches.iter().map(|b| b.points.clone()).collect()
    }
}

/// Build stable identities from the exact shared endpoints returned by extraction.
pub fn attach(
    paths: &[Vec<Point>],
    source: &Object,
    pixel_size_mm: f64,
) -> Result<AxisGraph, String> {
    let mut graph = AxisGraph {
        source_object_id: source.id,
        source_contours: source.flattened_contours(),
        pixel_size_mm,
        ..AxisGraph::default()
    };
    let mut vertex_positions = std::collections::BTreeMap::new();
    for (index, points) in paths.iter().enumerate() {
        if points.len() < 2 {
            return Err("A centerline branch requires two points".into());
        }
        let mut ends = [0; 2];
        for (slot, point) in [points[0], *points.last().unwrap()].into_iter().enumerate() {
            let bits = |value: f64| if value == 0. { 0 } else { value.to_bits() };
            let key = (bits(point.x), bits(point.y));
            ends[slot] = if let Some(&id) = vertex_positions.get(&key) {
                id
            } else {
                let id = graph.vertices.len() as u64 + 1;
                graph.vertices.push(AxisVertex { id, point });
                vertex_positions.insert(key, id);
                id
            };
        }
        graph.branches.push(AxisBranch {
            id: index as u64 + 1,
            start_vertex: ends[0],
            end_vertex: ends[1],
            points: points.clone(),
        });
    }
    graph.validate()?;
    Ok(graph)
}

pub fn sync_paths(object: &mut Object) {
    if let Some(graph) = &object.centerline_graph {
        object.running_paths = graph.paths();
    }
}

fn graph(object: &Object) -> Result<&AxisGraph, String> {
    let graph = object
        .centerline_graph
        .as_ref()
        .ok_or("This path has no editable source graph; extract centerlines again")?;
    graph.validate()?;
    Ok(graph)
}

/// Removing a spur also coalesces the two remaining branches at degree-two vertices.
pub fn delete_branch(object: &mut Object, branch_index: usize) -> Result<(), String> {
    // Schema v3 stores independent paths without trustworthy shared-vertex IDs.
    // Deleting one is safe; never infer adjacency or merge them by coordinates.
    if object.centerline_graph.is_none() {
        if object.settings.kind != crate::StitchKind::Running
            || branch_index >= object.running_paths.len()
        {
            return Err("Select a centerline branch".into());
        }
        if object.running_paths.len() == 1 {
            return Err("Delete the object to remove its final centerline branch".into());
        }
        object.running_paths.remove(branch_index);
        return Ok(());
    }
    let mut candidate = graph(object)?.clone();
    if branch_index >= candidate.branches.len() {
        return Err("Select a centerline branch".into());
    }
    if candidate.branches.len() == 1 {
        return Err("Delete the object to remove its final centerline branch".into());
    }
    candidate.branches.remove(branch_index);
    let mut merge_budget = WORK_BUDGET;
    loop {
        if candidate.branches.len() > merge_budget {
            return Err("Centerline merging exceeds the geometry budget".into());
        }
        merge_budget -= candidate.branches.len();
        let mut incident = std::collections::BTreeMap::<u64, Vec<usize>>::new();
        for (index, branch) in candidate.branches.iter().enumerate() {
            incident.entry(branch.start_vertex).or_default().push(index);
            incident.entry(branch.end_vertex).or_default().push(index);
        }
        let Some((&vertex, ids)) = incident
            .iter()
            .find(|(_, ids)| ids.len() == 2 && ids[0] != ids[1])
        else {
            break;
        };
        let mut second = candidate.branches.remove(ids[1]);
        let mut first = candidate.branches.remove(ids[0]);
        if first.start_vertex == vertex {
            first.points.reverse();
            std::mem::swap(&mut first.start_vertex, &mut first.end_vertex);
        }
        if second.end_vertex == vertex {
            second.points.reverse();
            std::mem::swap(&mut second.start_vertex, &mut second.end_vertex);
        }
        first.id = first.id.min(second.id);
        first.end_vertex = second.end_vertex;
        first.points.extend(second.points.into_iter().skip(1));
        candidate.branches.insert(ids[0], first);
    }
    let used: std::collections::BTreeSet<_> = candidate
        .branches
        .iter()
        .flat_map(|b| [b.start_vertex, b.end_vertex])
        .collect();
    candidate.vertices.retain(|v| used.contains(&v.id));
    candidate.validate()?;
    object.centerline_graph = Some(candidate);
    sync_paths(object);
    Ok(())
}

fn cross(a: Point, b: Point) -> f64 {
    a.x * b.y - a.y * b.x
}
fn sub(a: Point, b: Point) -> Point {
    Point::new(a.x - b.x, a.y - b.y)
}
fn point_segment(point: Point, a: Point, b: Point) -> f64 {
    let d = sub(b, a);
    let length2 = d.x * d.x + d.y * d.y;
    if length2 < EPS * EPS {
        return point.distance(a);
    }
    let q = sub(point, a);
    point.distance(a.lerp(b, ((q.x * d.x + q.y * d.y) / length2).clamp(0., 1.)))
}
fn edges(contours: &[Vec<Point>]) -> impl Iterator<Item = (Point, Point)> + '_ {
    contours.iter().flat_map(|ring| {
        ring.iter()
            .copied()
            .zip(ring.iter().copied().cycle().skip(1))
            .take(ring.len())
    })
}
fn inside(point: Point, contours: &[Vec<Point>]) -> bool {
    let mut parity = false;
    for (a, b) in edges(contours) {
        if point_segment(point, a, b) <= EPS {
            return true;
        }
        if (a.y > point.y) != (b.y > point.y)
            && point.x < a.x + (point.y - a.y) * (b.x - a.x) / (b.y - a.y)
        {
            parity = !parity;
        }
    }
    parity
}

/// Segment containment tests all intervals induced by polygon intersections.
fn contained(a: Point, b: Point, contours: &[Vec<Point>], budget: &mut usize) -> bool {
    let edge_count = contours.iter().map(Vec::len).sum::<usize>();
    let endpoints_cost = edge_count.saturating_mul(2);
    if endpoints_cost > *budget {
        return false;
    }
    *budget -= endpoints_cost;
    if !inside(a, contours) || !inside(b, contours) {
        return false;
    }
    let ab = sub(b, a);
    let mut cuts = vec![0., 1.];
    for (c, d) in edges(contours) {
        if *budget == 0 {
            return false;
        }
        *budget -= 1;
        let cd = sub(d, c);
        let denom = cross(ab, cd);
        if denom.abs() > EPS {
            let t = cross(sub(c, a), cd) / denom;
            let u = cross(sub(c, a), ab) / denom;
            if (-EPS..=1. + EPS).contains(&t) && (-EPS..=1. + EPS).contains(&u) {
                cuts.push(t.clamp(0., 1.));
            }
        }
    }
    cuts.sort_by(f64::total_cmp);
    let interval_cost = cuts.len().saturating_sub(1).saturating_mul(edge_count);
    if interval_cost > *budget {
        return false;
    }
    *budget -= interval_cost;
    cuts.windows(2)
        .all(|w| inside(a.lerp(b, (w[0] + w[1]) * 0.5), contours))
}

fn intersects(a: Point, b: Point, c: Point, d: Point) -> bool {
    let ab = sub(b, a);
    let cd = sub(d, c);
    let denom = cross(ab, cd);
    if denom.abs() <= EPS {
        return point_segment(a, c, d) <= EPS
            || point_segment(b, c, d) <= EPS
            || point_segment(c, a, b) <= EPS
            || point_segment(d, a, b) <= EPS;
    }
    let t = cross(sub(c, a), cd) / denom;
    let u = cross(sub(c, a), ab) / denom;
    (-EPS..=1. + EPS).contains(&t) && (-EPS..=1. + EPS).contains(&u)
}

fn geometry_valid(candidate: &AxisGraph) -> Result<(), String> {
    let mut budget = WORK_BUDGET;
    for branch in &candidate.branches {
        for segment in branch.points.windows(2) {
            if segment[0].distance(segment[1]) <= EPS
                || !contained(
                    segment[0],
                    segment[1],
                    &candidate.source_contours,
                    &mut budget,
                )
            {
                return Err("The edit leaves the source region, collapses a segment or exceeds the geometry budget".into());
            }
        }
    }
    for (bi, branch) in candidate.branches.iter().enumerate() {
        for (si, segment) in branch.points.windows(2).enumerate() {
            for (bj, other) in candidate.branches.iter().enumerate().skip(bi) {
                for (sj, compare) in other.points.windows(2).enumerate() {
                    if bi == bj && sj <= si + 1 {
                        continue;
                    }
                    if bi == bj
                        && branch.start_vertex == branch.end_vertex
                        && si == 0
                        && sj + 2 == branch.points.len()
                    {
                        continue;
                    }
                    if budget == 0 {
                        return Err("Centerline topology check exceeds the geometry budget".into());
                    }
                    budget -= 1;
                    if !intersects(segment[0], segment[1], compare[0], compare[1]) {
                        continue;
                    }
                    // A shared graph vertex may touch only at the corresponding segment ends.
                    let shared = [
                        (branch.start_vertex, 0),
                        (branch.end_vertex, branch.points.len() - 2),
                    ]
                    .into_iter()
                    .any(|(vertex, index)| {
                        si == index
                            && [
                                (other.start_vertex, 0),
                                (other.end_vertex, other.points.len() - 2),
                            ]
                            .into_iter()
                            .any(|(v, j)| {
                                vertex == v && sj == j && {
                                    let p = candidate
                                        .vertices
                                        .iter()
                                        .find(|v| v.id == vertex)
                                        .unwrap()
                                        .point;
                                    let away_a = if segment[0] == p {
                                        segment[1]
                                    } else {
                                        segment[0]
                                    };
                                    let away_b = if compare[0] == p {
                                        compare[1]
                                    } else {
                                        compare[0]
                                    };
                                    cross(sub(away_a, p), sub(away_b, p)).abs() > EPS
                                        || (sub(away_a, p).x * sub(away_b, p).x
                                            + sub(away_a, p).y * sub(away_b, p).y)
                                            < 0.
                                }
                            })
                    });
                    if !shared {
                        return Err("The edit introduces intersecting centerline branches".into());
                    }
                }
            }
        }
    }
    Ok(())
}

/// Junction movement updates every incident branch atomically.
pub fn move_node(
    object: &mut Object,
    path_index: usize,
    node: usize,
    point: Point,
) -> Result<(), String> {
    if !finite(&point) {
        return Err("Invalid centerline coordinates".into());
    }
    let mut candidate = graph(object)?.clone();
    let branch = candidate
        .branches
        .get(path_index)
        .ok_or("Select a centerline branch")?;
    if node >= branch.points.len() {
        return Err("Select a centerline node".into());
    }
    let vertex = if node == 0 {
        Some(branch.start_vertex)
    } else if node + 1 == branch.points.len() {
        Some(branch.end_vertex)
    } else {
        None
    };
    if let Some(vertex) = vertex {
        candidate
            .vertices
            .iter_mut()
            .find(|v| v.id == vertex)
            .unwrap()
            .point = point;
        for branch in &mut candidate.branches {
            if branch.start_vertex == vertex {
                branch.points[0] = point;
            }
            if branch.end_vertex == vertex {
                *branch.points.last_mut().unwrap() = point;
            }
        }
    } else {
        candidate.branches[path_index].points[node] = point;
    }
    candidate.validate()?;
    geometry_valid(&candidate)?;
    object.centerline_graph = Some(candidate);
    sync_paths(object);
    Ok(())
}

/// One bounded smoothing pass; corresponding polyline points move at most tolerance.
/// Linear interpolation between them gives the same bound along every segment.
pub fn smooth_branch(
    object: &mut Object,
    branch_index: usize,
    tolerance_mm: f64,
) -> Result<(), String> {
    if !tolerance_mm.is_finite() || !(0.001..=2.).contains(&tolerance_mm) {
        return Err("Smoothing tolerance must be between 0.001 and 2 mm".into());
    }
    let mut candidate = graph(object)?.clone();
    let branch = candidate
        .branches
        .get_mut(branch_index)
        .ok_or("Select a centerline branch")?;
    let before = branch.points.clone();
    if before.len() < 3 {
        return Err("This branch has no intermediate nodes to smooth".into());
    }
    for i in 1..before.len() - 1 {
        let average = before[i - 1].lerp(before[i + 1], 0.5);
        let target = before[i].lerp(average, 0.5);
        let distance = before[i].distance(target);
        branch.points[i] = if distance > tolerance_mm {
            before[i].lerp(target, tolerance_mm / distance)
        } else {
            target
        };
    }
    candidate.validate()?;
    geometry_valid(&candidate)?;
    object.centerline_graph = Some(candidate);
    sync_paths(object);
    Ok(())
}

/// Diagnostic inscribed diameter, not the normal cross-section width of Satin.
pub fn width_at(object: &Object, point: Point) -> Option<f64> {
    let graph = object.centerline_graph.as_ref()?;
    if !finite(&point) || !inside(point, &graph.source_contours) {
        return None;
    }
    edges(&graph.source_contours)
        .map(|(a, b)| point_segment(point, a, b) * 2.)
        .min_by(f64::total_cmp)
}

fn resample(points: &[Point], step: f64) -> Result<Vec<Point>, String> {
    let total: f64 = points.windows(2).map(|w| w[0].distance(w[1])).sum();
    let stations = (total / step).ceil().max(1.) as usize;
    if stations > 1023 {
        return Err("Satin conversion exceeds 1024 cross-sections; split the branch".into());
    }
    let mut result = vec![points[0]];
    let mut segment = 0;
    let mut traversed = 0.;
    for index in 1..stations {
        let distance = total * index as f64 / stations as f64;
        while segment + 2 < points.len()
            && traversed + points[segment].distance(points[segment + 1]) < distance
        {
            traversed += points[segment].distance(points[segment + 1]);
            segment += 1;
        }
        let length = points[segment].distance(points[segment + 1]);
        result.push(points[segment].lerp(points[segment + 1], (distance - traversed) / length));
    }
    result.push(*points.last().unwrap());
    Ok(result)
}

/// Preview one unbranched open stroke using actual normal/boundary intersections.
/// Ambiguous topology is deliberately left for explicit manual splitting/seams.
pub fn semi_auto_satin(object: &Object, branch_index: usize) -> Result<Vec<SatinColumn>, String> {
    let graph = graph(object)?;
    graph.validate()?;
    let branch = graph
        .branches
        .get(branch_index)
        .ok_or("Select a centerline branch")?;
    if branch.start_vertex == branch.end_vertex {
        return Err(
            "Choose an explicit seam or split this closed centerline before Satin conversion"
                .into(),
        );
    }
    if graph.degree(branch.start_vertex) != 1 || graph.degree(branch.end_vertex) != 1 {
        return Err("Resolve or split the centerline junctions before Satin conversion".into());
    }
    geometry_valid(graph)?;
    let stations = resample(&branch.points, graph.pixel_size_mm.clamp(0.1, 0.5))?;
    let mut left = Vec::with_capacity(stations.len());
    let mut right = Vec::with_capacity(stations.len());
    let mut budget = WORK_BUDGET;
    for (i, point) in stations.iter().copied().enumerate() {
        let tangent = sub(
            stations[(i + 1).min(stations.len() - 1)],
            stations[i.saturating_sub(1)],
        );
        let length = tangent.x.hypot(tangent.y);
        if length <= EPS {
            return Err("A zero-length tangent needs manual correction".into());
        }
        let normal = Point::new(-tangent.y / length, tangent.x / length);
        let mut negative = f64::NEG_INFINITY;
        let mut positive = f64::INFINITY;
        for (a, b) in edges(&graph.source_contours) {
            if budget == 0 {
                return Err("Satin cross-section search exceeds the geometry budget".into());
            }
            budget -= 1;
            let edge = sub(b, a);
            let denom = cross(normal, edge);
            if denom.abs() <= EPS {
                continue;
            }
            let t = cross(sub(a, point), edge) / denom;
            let u = cross(sub(a, point), normal) / denom;
            if !(-EPS..=1. + EPS).contains(&u) {
                continue;
            }
            if t > EPS {
                positive = positive.min(t);
            }
            if t < -EPS {
                negative = negative.max(t);
            }
        }
        if !negative.is_finite() || !positive.is_finite() || positive - negative < 0.15 {
            return Err("A narrow or ambiguous cross-section needs manual rail correction".into());
        }
        if positive - negative > 12. {
            return Err("A cross-section exceeds 12 mm; split the stroke or use a fill".into());
        }
        let a = Point::new(point.x + normal.x * positive, point.y + normal.y * positive);
        let b = Point::new(point.x + normal.x * negative, point.y + normal.y * negative);
        if !contained(a, b, &graph.source_contours, &mut budget) {
            return Err(
                "A cross-section crosses a hole or exterior gap; correct the branch manually"
                    .into(),
            );
        }
        left.push(a);
        right.push(b);
    }
    // Preserve every sampled correspondence with normalized arc-length rungs.
    let normalized_stations = |rail: &[Point]| {
        let mut distances = vec![0.];
        for pair in rail.windows(2) {
            distances.push(distances.last().unwrap() + pair[0].distance(pair[1]));
        }
        let total = *distances.last().unwrap();
        distances.into_iter().map(|d| d / total).collect::<Vec<_>>()
    };
    let left_stations = normalized_stations(&left);
    let right_stations = normalized_stations(&right);
    let rungs = left_stations
        .into_iter()
        .zip(right_stations)
        .skip(1)
        .take(left.len() - 2)
        .map(|(left, right)| SatinRung { left, right })
        .collect();
    let column = SatinColumn {
        left,
        right,
        rungs,
        ..SatinColumn::default()
    };
    let mut boundary = column.boundary();
    boundary.push(boundary[0]);
    for i in 0..boundary.len() - 1 {
        if !contained(
            boundary[i],
            boundary[i + 1],
            &graph.source_contours,
            &mut budget,
        ) {
            return Err(
                "The inferred rails leave the source area; split or correct the stroke".into(),
            );
        }
        for j in i + 2..boundary.len() - 1 {
            if i == 0 && j + 2 == boundary.len() {
                continue;
            }
            if budget == 0 {
                return Err("Satin rail validation exceeds the geometry budget".into());
            }
            budget -= 1;
            if intersects(boundary[i], boundary[i + 1], boundary[j], boundary[j + 1]) {
                return Err("The inferred rails intersect; split or correct the stroke".into());
            }
        }
    }
    crate::engine::satin_stations(&column, object.settings.spacing_mm)?;
    Ok(vec![column])
}
