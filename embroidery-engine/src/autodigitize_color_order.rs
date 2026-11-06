//! Sewing order for newly traced raster regions, before manual overrides.
//! Positive filled-area overlaps retain their source order. Shared boundaries
//! and islands inside another object's holes do not impose layer dependencies.
use crate::{PlanWarning, Project, engine::fill_geometry};
use geo::{
    BoundingRect, Polygon, PreparedGeometry, Rect, Relate, coordinate_position::CoordPos,
    dimensions::Dimensions,
};
use std::{
    cell::OnceCell,
    collections::BTreeSet,
    sync::atomic::{AtomicBool, Ordering},
};

struct Shape {
    polygon: Polygon<f64>,
    bounds: Rect<f64>,
    nodes: usize,
    prepared: OnceCell<PreparedGeometry<'static, Polygon<f64>>>,
}

impl Shape {
    fn new(polygon: Polygon<f64>) -> Option<Self> {
        let bounds = polygon.bounding_rect()?;
        let nodes = polygon.exterior().0.len()
            + polygon.interiors().iter().map(|r| r.0.len()).sum::<usize>();
        Some(Self {
            polygon,
            bounds,
            nodes,
            prepared: OnceCell::new(),
        })
    }

    fn prepare(&self, budget: &mut Budget) -> Result<(), Failure> {
        if self.prepared.get().is_none() {
            // Preparation builds the topology graph, segment R-tree and
            // self-noding once. Subsequent queries reuse those structures.
            budget.charge(
                self.nodes
                    .saturating_mul(self.nodes.max(2).ilog2() as usize + 1)
                    .saturating_mul(8),
            )?;
            let _ = self
                .prepared
                .set(PreparedGeometry::from(self.polygon.clone()));
        }
        Ok(())
    }
}

struct Component {
    shape: Shape,
    holes: Vec<Shape>,
}

enum Failure {
    Cancelled,
    Retain(String),
}

struct Budget {
    pairs: usize,
    exact: usize,
    max_pairs: usize,
    max_exact: usize,
}

impl Default for Budget {
    fn default() -> Self {
        Self {
            pairs: 0,
            exact: 0,
            max_pairs: 2_000_000,
            // Prepared topology is charged once, then graph query scans are
            // charged linearly. Exact overlap predicates and caps are retained.
            max_exact: 500_000_000,
        }
    }
}

impl Budget {
    fn pair(&mut self) -> Result<(), Failure> {
        self.pairs = self.pairs.saturating_add(1);
        if self.pairs > self.max_pairs {
            return Err(Failure::Retain("pair-check budget reached".into()));
        }
        Ok(())
    }

    fn charge(&mut self, work: usize) -> Result<(), Failure> {
        self.exact = self.exact.saturating_add(work);
        if self.exact > self.max_exact {
            return Err(Failure::Retain("intersection work budget reached".into()));
        }
        Ok(())
    }

    fn query(&mut self, a: &Shape, b: &Shape) -> Result<(), Failure> {
        a.prepare(self)?;
        b.prepare(self)?;
        self.charge(a.nodes.saturating_add(b.nodes).saturating_mul(8))
    }
}

fn poll(cancel: &AtomicBool) -> Result<(), Failure> {
    if cancel.load(Ordering::Relaxed) {
        Err(Failure::Cancelled)
    } else {
        Ok(())
    }
}

fn positive_bounds(a: Rect<f64>, b: Rect<f64>) -> bool {
    a.min().x < b.max().x && b.min().x < a.max().x && a.min().y < b.max().y && b.min().y < a.max().y
}

fn bounds_contain(a: Rect<f64>, b: Rect<f64>) -> bool {
    a.min().x <= b.min().x
        && a.min().y <= b.min().y
        && a.max().x >= b.max().x
        && a.max().y >= b.max().y
}

fn in_hole(
    owner: &Component,
    other: &Component,
    budget: &mut Budget,
    cancel: &AtomicBool,
) -> Result<bool, Failure> {
    for hole in &owner.holes {
        poll(cancel)?;
        budget.charge(1)?;
        if bounds_contain(hole.bounds, other.shape.bounds) {
            budget.query(hole, &other.shape)?;
            let covered = hole
                .prepared
                .get()
                .unwrap()
                .relate(other.shape.prepared.get().unwrap())
                .is_covers();
            poll(cancel)?;
            if covered {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

fn overlaps(
    a: &Component,
    b: &Component,
    budget: &mut Budget,
    cancel: &AtomicBool,
) -> Result<bool, Failure> {
    poll(cancel)?;
    if !positive_bounds(a.shape.bounds, b.shape.bounds) {
        return Ok(false);
    }
    // Large raster exteriors can surround hundreds of differently colored
    // hole islands. Certifying against the containing hole avoids charging
    // every comparison against the much larger whole-object boundary.
    if in_hole(a, b, budget, cancel)? || in_hole(b, a, budget, cancel)? {
        return Ok(false);
    }
    budget.query(&a.shape, &b.shape)?;
    let area_overlap = a
        .shape
        .prepared
        .get()
        .unwrap()
        .relate(b.shape.prepared.get().unwrap())
        .get(CoordPos::Inside, CoordPos::Inside)
        == Dimensions::TwoDimensional;
    poll(cancel)?;
    Ok(area_overlap)
}

fn order(
    project: &Project,
    cancel: &AtomicBool,
    budget: &mut Budget,
) -> Result<Vec<usize>, Failure> {
    poll(cancel)?;
    let count = project.objects.len();
    let mut areas = Vec::with_capacity(count);
    for object in &project.objects {
        poll(cancel)?;
        if !object.visible {
            areas.push(Vec::new());
            continue;
        }
        if !object.running_paths.is_empty() || object.satin_column.is_some() {
            return Err(Failure::Retain(format!(
                "object {} is not an unclassified filled raster region",
                object.id
            )));
        }
        let rings = object.flattened_contours();
        if rings.is_empty()
            || rings
                .iter()
                .flatten()
                .any(|p| !p.x.is_finite() || !p.y.is_finite())
        {
            return Err(Failure::Retain(format!(
                "object {} has invalid filled geometry",
                object.id
            )));
        }
        let polygons = fill_geometry::components(&rings)
            .map_err(|error| Failure::Retain(format!("object {}: {error}", object.id)))?;
        let mut components = Vec::with_capacity(polygons.len());
        for polygon in polygons {
            poll(cancel)?;
            let holes = polygon
                .interiors()
                .iter()
                .map(|ring| {
                    Shape::new(Polygon::new(ring.clone(), Vec::new()))
                        .expect("validated hole has vertices")
                })
                .collect();
            let shape = Shape::new(polygon).ok_or_else(|| {
                Failure::Retain(format!("object {} has empty filled geometry", object.id))
            })?;
            components.push(Component { shape, holes });
        }
        areas.push(components);
    }
    // Sweep the axis with the greatest center spread relative to component
    // size. Sparse imports above 2,000 objects must not spend a theoretical
    // all-pairs budget before any spatial rejection has even happened.
    let mut components: Vec<_> = areas
        .iter()
        .enumerate()
        .flat_map(|(object, pieces)| pieces.iter().map(move |piece| (object, piece)))
        .collect();
    let axis = |rect: Rect<f64>, y: bool| {
        if y {
            (rect.min().y, rect.max().y)
        } else {
            (rect.min().x, rect.max().x)
        }
    };
    let spread = |y| {
        let mut lo = f64::INFINITY;
        let mut hi = f64::NEG_INFINITY;
        let mut extent = 0.;
        for (_, component) in &components {
            let (a, b) = axis(component.shape.bounds, y);
            let center = (a + b) * 0.5;
            lo = lo.min(center);
            hi = hi.max(center);
            extent += b - a;
        }
        (hi - lo) / extent.max(1e-12)
    };
    let y = spread(true) > spread(false);
    components.sort_by(|a, b| {
        axis(a.1.shape.bounds, y)
            .0
            .total_cmp(&axis(b.1.shape.bounds, y).0)
            .then(a.0.cmp(&b.0))
    });
    let mut active: Vec<(usize, &Component)> = Vec::new();
    let mut dependencies = BTreeSet::new();
    for &(object, component) in &components {
        poll(cancel)?;
        let start = axis(component.shape.bounds, y).0;
        active.retain(|&(_, other)| axis(other.shape.bounds, y).1 > start);
        for &(other_object, other) in &active {
            poll(cancel)?;
            if object == other_object {
                continue;
            }
            budget.pair()?;
            let pair = (object.min(other_object), object.max(other_object));
            if !dependencies.contains(&pair) && overlaps(component, other, budget, cancel)? {
                dependencies.insert(pair);
            }
        }
        active.push((object, component));
    }
    let mut after = vec![Vec::new(); count];
    let mut indegree = vec![0usize; count];
    for (earlier, later) in dependencies {
        after[earlier].push(later);
        indegree[later] += 1;
    }
    let mut ready: BTreeSet<usize> = indegree
        .iter()
        .enumerate()
        .filter_map(|(i, &degree)| (degree == 0).then_some(i))
        .collect();
    let mut ordered = Vec::with_capacity(count);
    let mut current_color = None;
    while !ready.is_empty() {
        poll(cancel)?;
        let next = current_color
            .and_then(|color| {
                ready
                    .iter()
                    .copied()
                    .find(|&i| project.objects[i].color == color)
            })
            .or_else(|| ready.first().copied())
            .expect("ready set is nonempty");
        ready.remove(&next);
        ordered.push(next);
        current_color = Some(project.objects[next].color);
        for &successor in &after[next] {
            indegree[successor] -= 1;
            if indegree[successor] == 0 {
                ready.insert(successor);
            }
        }
    }
    poll(cancel)?;
    if ordered.len() != count {
        return Err(Failure::Retain(
            "overlap dependency order is incomplete".into(),
        ));
    }
    Ok(ordered)
}

/// Batch only a newly traced raster candidate, before classification or manual
/// overrides. IDs, geometry, visibility, settings and recipe identities are
/// unchanged. Never invoke this on normal project generation or preserved
/// reimports. Cancellation changes nothing. Uncertified/budget-limited geometry
/// retains the entire original order and returns a persisted diagnostic.
pub fn batch_fresh_raster_colors(
    project: &mut Project,
    cancel: &AtomicBool,
) -> Result<Vec<PlanWarning>, String> {
    batch_with_budget(project, cancel, &mut Budget::default())
}

fn batch_with_budget(
    project: &mut Project,
    cancel: &AtomicBool,
    budget: &mut Budget,
) -> Result<Vec<PlanWarning>, String> {
    match order(project, cancel, budget) {
        Ok(indices) => {
            if cancel.load(Ordering::Relaxed) {
                return Err("Color batching cancelled; document unchanged".into());
            }
            let mut objects: Vec<_> = std::mem::take(&mut project.objects)
                .into_iter()
                .map(Some)
                .collect();
            project.objects = indices
                .into_iter()
                .map(|i| objects[i].take().expect("each index occurs once"))
                .collect();
            Ok(Vec::new())
        }
        Err(Failure::Cancelled) => Err("Color batching cancelled; document unchanged".into()),
        Err(Failure::Retain(reason)) => {
            if cancel.load(Ordering::Relaxed) {
                return Err("Color batching cancelled; document unchanged".into());
            }
            let warning = PlanWarning {
                object_id: 0,
                point: None,
                message: format!(
                    "Color batching skipped: {reason}; original object order retained. Review sewing order manually."
                ),
            };
            if let Some(recipe) = &mut project.raster_recipe {
                recipe.issues.push(warning.clone());
            }
            Ok(vec![warning])
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Object, Point};

    fn object(id: u64, color: [u8; 3], x: f64) -> Object {
        Object {
            id,
            color,
            contours: vec![vec![
                Point::new(x, 0.),
                Point::new(x + 1., 0.),
                Point::new(x + 1., 1.),
                Point::new(x, 1.),
            ]],
            ..Object::default()
        }
    }

    #[test]
    fn exhausted_budget_retains_complete_original_order() {
        let mut project = Project {
            objects: vec![
                object(1, [1, 2, 3], 0.),
                object(2, [4, 5, 6], 0.25),
                object(3, [1, 2, 3], 0.5),
            ],
            ..Project::default()
        };
        let before = serde_json::to_value(&project).unwrap();
        let warnings = batch_with_budget(
            &mut project,
            &AtomicBool::new(false),
            &mut Budget {
                max_pairs: 1,
                ..Budget::default()
            },
        )
        .unwrap();
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].message.contains("budget"));
        assert_eq!(serde_json::to_value(&project).unwrap(), before);
    }

    #[test]
    fn exhausted_exact_budget_does_not_publish_a_partial_order() {
        let mut project = Project {
            objects: vec![
                object(1, [1, 2, 3], 0.),
                object(2, [4, 5, 6], 0.5),
                object(3, [1, 2, 3], 4.),
            ],
            ..Project::default()
        };
        let before = serde_json::to_value(&project).unwrap();
        let warnings = batch_with_budget(
            &mut project,
            &AtomicBool::new(false),
            &mut Budget {
                max_exact: 1,
                ..Budget::default()
            },
        )
        .unwrap();
        assert!(warnings[0].message.contains("intersection"));
        assert_eq!(serde_json::to_value(&project).unwrap(), before);
    }
}
