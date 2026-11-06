//! Visible sewing domains derived from editable layer order. Artwork is never
//! modified. Only the interior of a later filled object occludes lower fill;
//! its holes, running paths and the requested registration overlap remain open.
use super::fill_geometry;
use crate::model::{Object, Point, Project, StitchKind};
use geo::algorithm::{coordinate_position::CoordPos, dimensions::Dimensions};
use geo::{
    Area, BooleanOps, BoundingRect, MultiPolygon, Polygon, PreparedGeometry, Rect, Relate,
    Validation,
};
use std::{
    cell::Cell,
    collections::{BTreeMap, BTreeSet},
    sync::atomic::{AtomicBool, Ordering},
};

const MAX_PAIR_CHECKS: usize = 2_000_000;
const MAX_BOOLEAN_WORK: usize = 500_000_000;
const MAX_SINGLE_BOOLEAN_WORK: usize = 16_000_000;
const MAX_OPERATION_NODES: usize = 200_000;

fn sweep_work(count: usize) -> usize {
    count
        .saturating_mul(count.max(2).ilog2() as usize + 1)
        .saturating_mul(8)
}

fn polygon_nodes(p: &Polygon<f64>) -> usize {
    p.exterior().0.len() + p.interiors().iter().map(|r| r.0.len()).sum::<usize>()
}

fn nodes(area: &MultiPolygon<f64>) -> usize {
    area.0.iter().map(polygon_nodes).sum()
}

fn bounds_overlap(a: Rect<f64>, b: Rect<f64>) -> bool {
    a.min().x < b.max().x && b.min().x < a.max().x && a.min().y < b.max().y && b.min().y < a.max().y
}

fn object_key(object: &Object) -> usize {
    // IDs in legacy/manual projects need not be unique. This address is only
    // an internal lookup key; the plan borrows the project to keep it stable.
    object as *const Object as usize
}

struct Occluder {
    name: String,
    area: MultiPolygon<f64>,
    coverage: Polygon<f64>,
    bounds: Rect<f64>,
    nodes: usize,
}

#[derive(Clone)]
struct PhysicalComponent {
    topology: PreparedGeometry<'static, Polygon<f64>>,
    bounds: Rect<f64>,
    nodes: usize,
}

fn physical_top_domain(
    object: &Object,
    original: &MultiPolygon<f64>,
    version: u32,
    plan: &LayeringPlan<'_>,
) -> Result<MultiPolygon<f64>, String> {
    let settings = &object.settings;
    if !settings.pull_comp_mm.is_finite()
        || !(0.0..=2.0).contains(&settings.pull_comp_mm)
        || !settings.angle_deg.is_finite()
        || !settings.spacing_mm.is_finite()
        || !(0.15..=20.0).contains(&settings.spacing_mm)
        || !settings.length_mm.is_finite()
        || !(0.3..=12.0).contains(&settings.length_mm)
    {
        return Err(format!(
            "{}: invalid fill settings for layer cutout; check spacing, length, angle and compensation",
            object.name
        ));
    }
    if settings.kind == StitchKind::Satin {
        return satin_top_domain(object, version);
    }
    if settings.pull_comp_mm == 0. {
        plan.charge(nodes(original), "copying uncompensated layer geometry")?;
        return Ok(original.clone());
    }
    let angle = if settings.auto_angle {
        super::automatic_angle_choice(
            &object.flattened_contours(),
            settings.spacing_mm,
            settings.length_mm,
        )
        .0
    } else {
        settings.angle_deg
    };
    let mut area = MultiPolygon::new(Vec::new());
    for component in &original.0 {
        plan.check_cancel()?;
        plan.charge(
            sweep_work(polygon_nodes(component)),
            "compensating layer geometry",
        )?;
        area.0
            .extend(fill_geometry::compensated(component, angle, settings.pull_comp_mm)?.0);
    }
    Ok(area)
}

/// Boolean kernels can return a closed boundary walk visiting an exact tangent
/// vertex twice. Split that walk without moving a coordinate or losing an edge.
fn simple_cycles(ring: &geo::LineString<f64>) -> Result<Vec<Vec<Point>>, String> {
    let key = |p: Point| {
        (
            (if p.x == 0. { 0. } else { p.x }).to_bits(),
            (if p.y == 0. { 0. } else { p.y }).to_bits(),
        )
    };
    let mut path = Vec::<Point>::new();
    let mut positions = BTreeMap::new();
    let mut cycles = Vec::new();
    for coord in &ring.0 {
        let point = Point::new(coord.x, coord.y);
        if let Some(&start) = positions.get(&key(point)) {
            if path.len() - start < 3 {
                return Err("Boolean boundary contains a degenerate edge cycle".into());
            }
            cycles.push(path[start..].to_vec());
            for old in &path[start + 1..] {
                positions.remove(&key(*old));
            }
            path.truncate(start + 1);
        } else {
            positions.insert(key(point), path.len());
            path.push(point);
        }
    }
    if path.len() != 1 || cycles.is_empty() {
        return Err("Boolean boundary is not a closed walk".into());
    }
    Ok(cycles)
}

fn valid_boolean(area: MultiPolygon<f64>) -> Result<MultiPolygon<f64>, String> {
    if nodes(&area) > MAX_OPERATION_NODES {
        return Err("Boolean output exceeds 200,000 geometry nodes".into());
    }
    if area.check_validation().is_ok() {
        return Ok(area);
    }
    let before = area.unsigned_area();
    let mut rings = Vec::new();
    for polygon in &area.0 {
        for ring in std::iter::once(polygon.exterior()).chain(polygon.interiors()) {
            rings.extend(simple_cycles(ring)?);
        }
    }
    let repaired = MultiPolygon::new(fill_geometry::components(&rings)?);
    // This only changes representation at exact tangent vertices. Area changes
    // beyond floating arithmetic noise are rejected, never hidden as a repair.
    if (before - repaired.unsigned_area()).abs() > 1e-9 {
        return Err("Splitting tangent Boolean cycles changes filled area".into());
    }
    repaired
        .check_validation()
        .map_err(|e| format!("Invalid Boolean result: {e}"))?;
    Ok(repaired)
}

/// Paired Satin can shorten punctures on crowded inner corners. Derive its
/// conservative occluding rails from the actual planned top endpoints instead
/// of treating the entire editable outline as definitely covered.
pub(crate) fn satin_top_domain(object: &Object, version: u32) -> Result<MultiPolygon<f64>, String> {
    let column = object
        .satin_column
        .as_ref()
        .ok_or("Satin layer requires paired rails")?;
    let modern = version >= 4 || column.uses_v4() || object.settings.uses_v4();
    let (segments, _) = super::paired_satin_segments(
        column,
        &object.settings,
        &object.flattened_contours(),
        modern,
    )?;
    let top: Vec<_> = segments
        .into_iter()
        .filter(|s| s.2 == StitchKind::Satin)
        .collect();
    let Some(first) = top.first() else {
        return Ok(MultiPolygon::new(Vec::new()));
    };
    let first_side = usize::from(column.first_right);
    let mut rails = [Vec::new(), Vec::new()];
    rails[first_side].push(first.0);
    for (index, &(_, endpoint, _)) in top.iter().enumerate() {
        let side = if index + 1 == top.len() {
            let previous = if (index - 1) % 2 == 0 {
                1 - first_side
            } else {
                first_side
            };
            1 - previous
        } else if index % 2 == 0 {
            1 - first_side
        } else {
            first_side
        };
        if rails[side].last() != Some(&endpoint) {
            rails[side].push(endpoint);
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
    fill_geometry::components(&rings).map(MultiPolygon::new)
        .map_err(|e|format!("{}: cannot certify actual Satin layer coverage: {e}. Split the column or disable hidden-fill removal",object.name))
}

/// Shared by generation and diagnostics so neither mistakes intentionally
/// removed lower fill for a missing top pass. Each Boolean operation is bounded
/// and cancellation is checked between operations.
pub(crate) struct LayeringPlan<'a> {
    occluders: Vec<Occluder>,
    cutouts: BTreeMap<usize, Vec<usize>>,
    work: Cell<usize>,
    cancel: &'a AtomicBool,
    _project: &'a Project,
}

impl<'a> LayeringPlan<'a> {
    pub(crate) fn new(project: &'a Project, cancel: &'a AtomicBool) -> Result<Self, String> {
        project.layering.validate()?;
        let generation_version = project.persisted_schema_version();
        let mut plan = Self {
            occluders: Vec::new(),
            cutouts: BTreeMap::new(),
            work: Cell::new(0),
            cancel,
            _project: project,
        };
        if !project.layering.enabled {
            return Ok(plan);
        }
        if project
            .objects
            .iter()
            .filter(|object| {
                object.visible
                    && object.settings.kind != StitchKind::Running
                    && object.running_paths.is_empty()
            })
            .count()
            < 2
        {
            return Ok(plan);
        }
        let mut filled = Vec::new();
        for object in &project.objects {
            plan.check_cancel()?;
            if !object.visible
                || object.settings.kind == StitchKind::Running
                || !object.running_paths.is_empty()
            {
                continue;
            }
            if object.settings.kind == StitchKind::Satin {
                let Some(column) = &object.satin_column else {
                    // The normal generator reports the unsupported Satin draft.
                    continue;
                };
                super::satin_stations(column, object.settings.spacing_mm).map_err(|e| {
                    format!("{}: cannot derive Satin layer coverage: {e}", object.name)
                })?;
            }
            let rings = object.flattened_contours();
            if rings.is_empty() {
                continue;
            }
            let components = fill_geometry::components(&rings)
                .map_err(|e| format!("{}: invalid layer geometry: {e}", object.name))?;
            let original = MultiPolygon::new(components);
            let Some(bounds) = original.bounding_rect() else {
                continue;
            };
            // Candidate bounds include directional compensation conservatively;
            // the exact compensated polygon is clipped only by the caller.
            let pull = object.settings.pull_comp_mm.max(0.);
            filled.push((
                object_key(object),
                object.name.clone(),
                Rect::new(
                    (bounds.min().x - pull, bounds.min().y - pull),
                    (bounds.max().x + pull, bounds.max().y + pull),
                ),
                original,
                object,
            ));
        }
        // Exact topology is evaluated once per physical component pair. Merely
        // overlapping bounds (common for label-map regions inside an outline's
        // hole) must not allocate cutouts or repeatedly consume Boolean budgets.
        let mut physical: Vec<Option<Vec<PhysicalComponent>>> = vec![None; filled.len()];
        let mut proven = BTreeMap::<usize, BTreeSet<(usize, usize)>>::new();
        let mut referenced = BTreeSet::new();
        let mut pairs = 0usize;
        for upper in 0..filled.len() {
            plan.check_cancel()?;
            for lower in 0..upper {
                if lower % 64 == 0 {
                    plan.check_cancel()?;
                }
                if !bounds_overlap(filled[lower].2, filled[upper].2) {
                    continue;
                }
                for index in [lower, upper] {
                    if physical[index].is_none() {
                        let domain = physical_top_domain(
                            filled[index].4,
                            &filled[index].3,
                            generation_version,
                            &plan,
                        )?;
                        let mut components = Vec::with_capacity(domain.0.len());
                        for component in domain.0 {
                            plan.check_cancel()?;
                            let count = polygon_nodes(&component);
                            if count > MAX_OPERATION_NODES {
                                return Err("Physical layer topology exceeds 200,000 nodes; simplify or split the object".into());
                            }
                            plan.charge(
                                sweep_work(count),
                                "preparing cached physical layer topology",
                            )?;
                            let bounds = component
                                .bounding_rect()
                                .ok_or("Empty physical layer coverage")?;
                            components.push(PhysicalComponent {
                                topology: PreparedGeometry::from(component),
                                bounds,
                                nodes: count,
                            });
                        }
                        physical[index] = Some(components);
                    }
                }
                let low = physical[lower]
                    .as_ref()
                    .expect("cached physical lower domain");
                let high = physical[upper]
                    .as_ref()
                    .expect("cached physical upper domain");
                for (high_index, high_component) in high.iter().enumerate() {
                    for low_component in low {
                        if !bounds_overlap(low_component.bounds, high_component.bounds) {
                            continue;
                        }
                        pairs = pairs.saturating_add(1);
                        if pairs > MAX_PAIR_CHECKS {
                            return Err(format!(
                                "Layer topology checks exceed the candidate-pair budget ({pairs}/{MAX_PAIR_CHECKS}); simplify or split the project, or disable hidden-fill removal"
                            ));
                        }
                        plan.charge(
                            low_component
                                .nodes
                                .saturating_add(high_component.nodes)
                                .saturating_mul(8),
                            "querying cached physical layer interior overlap",
                        )?;
                        // Geo's prepared geometry reuses the topology graph,
                        // segment R-tree and self-noding. A query clones/scans
                        // the graphs; their n*log(n) preparation is charged once.
                        let relation = low_component.topology.relate(&high_component.topology);
                        if relation.get(CoordPos::Inside, CoordPos::Inside)
                            == Dimensions::TwoDimensional
                        {
                            let key = (upper, high_index);
                            proven.entry(filled[lower].0).or_default().insert(key);
                            referenced.insert(key);
                            break;
                        }
                    }
                }
            }
        }
        let mut indices = BTreeMap::new();
        for (upper, component_index) in referenced {
            plan.check_cancel()?;
            let component = physical[upper]
                .as_ref()
                .expect("referenced physical upper domain")[component_index]
                .topology
                .geometry();
            let name = &filled[upper].1;
            let count = polygon_nodes(component);
            plan.charge(
                if project.layering.overlap_mm == 0. {
                    count
                } else {
                    sweep_work(count)
                },
                "insetting an overlapping upper component",
            )?;
            let interior = valid_boolean(fill_geometry::inset(component, project.layering.overlap_mm)?)
                    .map_err(|e|format!("{name}: invalid layer overlap interior: {e}. Reduce overlap or simplify the upper layer"))?;
            let bounds = component
                .bounding_rect()
                .ok_or("Empty upper layer coverage")?;
            indices.insert((upper, component_index), plan.occluders.len());
            plan.occluders.push(Occluder {
                name: name.clone(),
                nodes: nodes(&interior),
                area: interior,
                coverage: component.clone(),
                bounds,
            });
        }
        for (id, keys) in proven {
            plan.cutouts
                .insert(id, keys.into_iter().map(|key| indices[&key]).collect());
        }
        Ok(plan)
    }

    fn check_cancel(&self) -> Result<(), String> {
        if self.cancel.load(Ordering::Relaxed) {
            Err("Generation cancelled".into())
        } else {
            Ok(())
        }
    }

    fn charge(&self, amount: usize, phase: &str) -> Result<(), String> {
        let work = self.work.get().saturating_add(amount);
        if amount > MAX_SINGLE_BOOLEAN_WORK || work > MAX_BOOLEAN_WORK {
            return Err(format!(
                "Layer cutout exceeds the geometry budget while {phase} (used {}, operation {amount}, total {work}/{MAX_BOOLEAN_WORK}, single-operation limit {MAX_SINGLE_BOOLEAN_WORK}); simplify or split overlapping objects, or disable hidden-fill removal",
                self.work.get()
            ));
        }
        self.work.set(work);
        Ok(())
    }

    pub(crate) fn is_clipped(&self, object: &Object) -> bool {
        self.cutouts.contains_key(&object_key(object))
    }

    /// Clip an already compensated top domain or an independently inset underlay
    /// domain. In particular, compensation is never reapplied around cutout holes.
    pub(crate) fn clip(
        &self,
        object: &Object,
        area: &MultiPolygon<f64>,
    ) -> Result<MultiPolygon<f64>, String> {
        let Some(cuts) = self.cutouts.get(&object_key(object)) else {
            return Ok(area.clone());
        };
        let mut remaining = area.clone();
        for &index in cuts {
            self.check_cancel()?;
            let upper = &self.occluders[index];
            let mut next = Vec::new();
            for component in remaining.0 {
                let Some(bounds) = component.bounding_rect() else {
                    continue;
                };
                if !bounds_overlap(bounds, upper.bounds) {
                    next.push(component);
                    continue;
                }
                self.charge(
                    sweep_work(
                        polygon_nodes(&component).saturating_add(polygon_nodes(&upper.coverage)),
                    ),
                    "checking complete coverage of a sewing component",
                )?;
                if upper.coverage.relate(&component).is_covers() {
                    // A wholly hidden component needs no registration collar.
                    continue;
                }
                let mut pieces = vec![component];
                for cut in &upper.area.0 {
                    self.check_cancel()?;
                    let Some(cut_bounds) = cut.bounding_rect() else {
                        continue;
                    };
                    let mut cut_pieces = Vec::new();
                    for piece in pieces {
                        if !piece
                            .bounding_rect()
                            .is_some_and(|b| bounds_overlap(b, cut_bounds))
                        {
                            cut_pieces.push(piece);
                            continue;
                        }
                        let count = polygon_nodes(&piece).saturating_add(upper.nodes);
                        if count > MAX_OPERATION_NODES {
                            return Err("Layer cutout input exceeds 200,000 geometry nodes; simplify or split the overlapping objects".into());
                        }
                        self.charge(
                            sweep_work(count),
                            "intersecting a sewing component with its cutout",
                        )?;
                        let intersection = piece.intersection(cut);
                        if intersection.unsigned_area() == 0. {
                            // Exact edge contact is adjacency, not hidden fill.
                            // Do not run a difference on disjoint raster regions.
                            cut_pieces.push(piece);
                            continue;
                        }
                        self.charge(sweep_work(count), "subtracting a proven cutout")?;
                        let clipped = valid_boolean(piece.difference(cut))
                            .map_err(|e|format!("Layer cutout beneath {} is invalid: {e}. Simplify the overlapping objects or disable hidden-fill removal",upper.name))?;
                        cut_pieces.extend(clipped.0);
                    }
                    pieces = cut_pieces;
                }
                next.extend(pieces);
            }
            remaining = MultiPolygon::new(next);
            if remaining.0.is_empty() {
                break;
            }
        }
        Ok(remaining)
    }

    /// Ignore only numerical area noise, never a narrow but real filled strip.
    pub(crate) fn changed(before: &MultiPolygon<f64>, after: &MultiPolygon<f64>) -> bool {
        before.unsigned_area() - after.unsigned_area() > 1e-9
    }

    pub(crate) fn first_point(area: &MultiPolygon<f64>) -> Option<Point> {
        area.0
            .first()?
            .exterior()
            .0
            .first()
            .map(|p| Point::new(p.x, p.y))
    }
}
