//! Explicit region inclusion and topology-preserving vector split/union.
use crate::model::{Object, Point, Project};
use geo::{Area, BooleanOps, Coord, LineString, MultiPolygon, Polygon, Validation};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug)]
pub struct Region {
    /// Deterministic geometry/color identity, independent of object ordering.
    pub id: u64,
    pub object_id: u64,
    pub color: [u8; 3],
    pub included: bool,
    pub pixel_count: usize,
}

#[derive(Clone, Debug)]
pub struct RegionPreview {
    pub project: Project,
    pub regions: Vec<Region>,
    /// Stable region identities; zero denotes unassigned/background pixels.
    pub mask: Vec<u64>,
    pub width: u32,
    pub height: u32,
    overrides: BTreeMap<u64, bool>,
}

impl RegionPreview {
    pub fn from_project(project: Project, width: u32, height: u32) -> Result<Self, String> {
        project.validate_limits()?;
        if width == 0 || height == 0 || width > 2048 || height > 2048 {
            return Err("Region preview dimensions must be 1–2048 pixels per axis".into());
        }
        if !project.width_mm.is_finite() || project.width_mm <= 0. {
            return Err("Region preview requires a positive physical width".into());
        }
        let mut preview = Self {
            project,
            regions: Vec::new(),
            mask: vec![0; width as usize * height as usize],
            width,
            height,
            overrides: BTreeMap::new(),
        };
        let mut work = 0_u64;
        for object in &preview.project.objects {
            let rings = object.flattened_contours();
            if rings.is_empty() {
                return Err("Region preview requires closed area objects".into());
            }
            let id = region_identity(object);
            if preview.regions.iter().any(|region| region.id == id) {
                return Err("Region preview contains duplicate geometry identities".into());
            }
            let mut pixel_count = 0;
            let scale = f64::from(width) / preview.project.width_mm;
            for y in 0..height {
                let world_y = (f64::from(y) + 0.5) / scale;
                let mut intersections = Vec::new();
                for ring in &rings {
                    work += ring.len() as u64;
                    if work > 60_000_000 {
                        return Err(
                            "Region preview exceeds geometry budget; reduce preview resolution"
                                .into(),
                        );
                    }
                    for (&a, &b) in ring
                        .iter()
                        .zip(ring.iter().cycle().skip(1))
                        .take(ring.len())
                    {
                        if (a.y > world_y) != (b.y > world_y) {
                            intersections.push(a.x + (world_y - a.y) * (b.x - a.x) / (b.y - a.y));
                        }
                    }
                }
                intersections.sort_by(f64::total_cmp);
                for pair in intersections.as_chunks::<2>().0 {
                    let from = (pair[0] * scale - 0.5).ceil().clamp(0., f64::from(width)) as u32;
                    let to = (pair[1] * scale - 0.5).ceil().clamp(0., f64::from(width)) as u32;
                    for x in from..to {
                        preview.mask[(y * width + x) as usize] = id;
                        pixel_count += 1;
                    }
                }
            }
            preview.regions.push(Region {
                id,
                object_id: object.id,
                color: object.color,
                included: object.visible,
                pixel_count,
            });
        }
        Ok(preview)
    }

    pub fn set_included(&mut self, id: u64, included: bool) -> Result<(), String> {
        let region = self
            .regions
            .iter_mut()
            .find(|r| r.id == id)
            .ok_or("Unknown region identity")?;
        region.included = included;
        let object = self
            .project
            .objects
            .iter_mut()
            .find(|o| o.id == region.object_id)
            .ok_or("Region object no longer exists")?;
        object.visible = included;
        self.overrides.insert(id, included);
        Ok(())
    }

    pub fn region_at(&self, point_mm: Point) -> Option<u64> {
        let scale = f64::from(self.width) / self.project.width_mm;
        let x = point_mm.x * scale;
        let y = point_mm.y * scale;
        if !x.is_finite()
            || !y.is_finite()
            || x < 0.
            || y < 0.
            || x >= f64::from(self.width)
            || y >= f64::from(self.height)
        {
            return None;
        }
        let id = self.mask[y.floor() as usize * self.width as usize + x.floor() as usize];
        (id != 0).then_some(id)
    }

    pub fn toggle_at(&mut self, point_mm: Point) -> Option<u64> {
        let id = self.region_at(point_mm)?;
        let included = self.regions.iter().find(|r| r.id == id)?.included;
        self.set_included(id, !included).ok()?;
        Some(id)
    }

    pub fn has_overrides(&self) -> bool {
        !self.overrides.is_empty()
    }

    /// Re-tracing may alter segmentation. Refuse to silently discard a manual
    /// decision whose exact geometry/color identity can no longer be matched.
    pub fn apply_overrides_to(&self, project: Project) -> Result<Self, String> {
        let mut next = Self::from_project(project, self.width, self.height)?;
        if self
            .overrides
            .keys()
            .any(|id| !next.regions.iter().any(|r| r.id == *id))
        {
            return Err("Region topology changed; manual region choices cannot be matched. Reset region choices explicitly before rebuilding the mask.".into());
        }
        for (&id, &included) in &self.overrides {
            next.set_included(id, included)?;
        }
        Ok(next)
    }

    pub fn overlay(&self) -> image::RgbaImage {
        let colors: BTreeMap<_, _> = self
            .regions
            .iter()
            .map(|r| {
                (
                    r.id,
                    if r.included {
                        image::Rgba([r.color[0], r.color[1], r.color[2], 155])
                    } else {
                        image::Rgba([120, 120, 120, 55])
                    },
                )
            })
            .collect();
        image::RgbaImage::from_fn(self.width, self.height, |x, y| {
            colors
                .get(&self.mask[(y * self.width + x) as usize])
                .copied()
                .unwrap_or(image::Rgba([0, 0, 0, 0]))
        })
    }
}

pub fn region_identity(object: &Object) -> u64 {
    // FNV-1a over canonical ring sequences: cyclic starting-node position,
    // winding direction and ring order do not affect the identity.
    let mut rings: Vec<Vec<(u64, u64)>> = object
        .flattened_contours()
        .iter()
        .map(|ring| {
            let mut points: Vec<_> = ring
                .iter()
                .map(|p| (p.x.to_bits(), p.y.to_bits()))
                .collect();
            if points.len() > 1 && points.first() == points.last() {
                points.pop();
            }
            let canonical = |points: &[(u64, u64)]| {
                let index = points
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, v)| **v)
                    .map(|(i, _)| i)
                    .unwrap_or(0);
                points[index..]
                    .iter()
                    .chain(&points[..index])
                    .copied()
                    .collect::<Vec<_>>()
            };
            let forward = canonical(&points);
            points.reverse();
            forward.min(canonical(&points))
        })
        .collect();
    rings.sort();
    let mut hash = 0xcbf29ce484222325_u64;
    let mut feed = |byte: u8| {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100000001b3);
    };
    for c in object.color {
        feed(c);
    }
    for ring in rings {
        for b in (ring.len() as u64).to_le_bytes() {
            feed(b);
        }
        for (x, y) in ring {
            for b in x.to_le_bytes().into_iter().chain(y.to_le_bytes()) {
                feed(b);
            }
        }
    }
    hash.max(1)
}

fn object_area(object: &Object) -> Result<MultiPolygon<f64>, String> {
    if object.satin_column.is_some() || !object.running_paths.is_empty() {
        return Err("Region operations require closed contour objects; Satin and Running paths are not regions".into());
    }
    let rings = object.flattened_contours();
    if rings.is_empty() {
        return Err("The object contains no closed region".into());
    }
    let mut result = MultiPolygon::new(Vec::new());
    for ring in rings {
        let polygon = ring_polygon(&ring)?;
        // Every ring toggles occupancy, preserving nested islands and holes
        // independently of ring order and winding direction.
        result = result.xor(&MultiPolygon::new(vec![polygon]));
    }
    if result.0.is_empty() || result.unsigned_area() <= 1e-8 {
        return Err("Region has no positive filled area".into());
    }
    result
        .check_validation()
        .map_err(|e| format!("Invalid region: {e}"))?;
    Ok(result)
}

fn ring_polygon(ring: &[Point]) -> Result<Polygon<f64>, String> {
    if ring.len() < 3 || ring.iter().any(|p| !p.x.is_finite() || !p.y.is_finite()) {
        return Err("Region ring needs three finite points".into());
    }
    let polygon = Polygon::new(
        LineString::new(ring.iter().map(|p| Coord { x: p.x, y: p.y }).collect()),
        Vec::new(),
    );
    polygon
        .check_validation()
        .map_err(|e| format!("Invalid region contour: {e}"))?;
    if polygon.unsigned_area() <= 1e-8 {
        return Err("Region contour has zero area".into());
    }
    Ok(polygon)
}

fn contours(area: &MultiPolygon<f64>) -> Vec<Vec<Point>> {
    area.0
        .iter()
        .flat_map(|polygon| std::iter::once(polygon.exterior()).chain(polygon.interiors()))
        .map(|ring| {
            let mut points: Vec<_> = ring.0.iter().map(|p| Point::new(p.x, p.y)).collect();
            if points.len() > 1 && points.first() == points.last() {
                points.pop();
            }
            points
        })
        .filter(|ring| ring.len() >= 3)
        .collect()
}

/// Union selected same-color regions. The earliest selected object keeps its
/// settings, identity and sewing position; all source geometry stays untouched
/// when validation or a boolean operation fails.
pub fn merge_objects(project: &Project, indices: &[usize]) -> Result<Project, String> {
    project.validate_limits()?;
    let selected: BTreeSet<_> = indices.iter().copied().collect();
    if selected.len() < 2 {
        return Err("Select at least two regions to merge".into());
    }
    let first = *selected.first().unwrap();
    let template = project
        .objects
        .get(first)
        .ok_or("Selected region does not exist")?;
    let mut union = MultiPolygon::new(Vec::new());
    for &index in &selected {
        let object = project
            .objects
            .get(index)
            .ok_or("Selected region does not exist")?;
        if object.color != template.color {
            return Err("Merge requires regions with the same thread color".into());
        }
        union = union.union(&object_area(object)?);
    }
    union
        .check_validation()
        .map_err(|e| format!("Merged region is invalid: {e}"))?;
    let mut merged = template.clone();
    merged.contours = contours(&union);
    merged.bezier_segments.clear();
    merged.centerline_graph = None;
    let mut candidate = project.clone();
    candidate.objects = project
        .objects
        .iter()
        .enumerate()
        .filter_map(|(index, object)| {
            if index == first {
                Some(merged.clone())
            } else if selected.contains(&index) {
                None
            } else {
                Some(object.clone())
            }
        })
        .collect();
    candidate.validate_limits()?;
    Ok(candidate)
}

/// Split with a straight, open cutting line. Its endpoints must extend beyond
/// the region so a short accidental segment cannot cut unrelated geometry.
pub fn split_object(
    project: &Project,
    object_index: usize,
    line: &[Point],
) -> Result<Project, String> {
    project.validate_limits()?;
    if line.len() != 2
        || line[0].distance(line[1]) <= 1e-7
        || line.iter().any(|p| !p.x.is_finite() || !p.y.is_finite())
    {
        return Err(
            "Split requires an open straight line with two distinct finite endpoints".into(),
        );
    }
    let object = project
        .objects
        .get(object_index)
        .ok_or("Selected region does not exist")?;
    let area = object_area(object)?;
    let (a, b) = (line[0], line[1]);
    let delta = Point::new(b.x - a.x, b.y - a.y);
    let length = a.distance(b);
    let u = Point::new(delta.x / length, delta.y / length);
    let n = Point::new(-u.y, u.x);
    let all: Vec<_> = area
        .0
        .iter()
        .flat_map(|p| std::iter::once(p.exterior()).chain(p.interiors()))
        .flat_map(|r| r.0.iter())
        .collect();
    let mut min_t = f64::INFINITY;
    let mut max_t = f64::NEG_INFINITY;
    let mut extent = 1_f64;
    for point in &all {
        let dx = point.x - a.x;
        let dy = point.y - a.y;
        let t = dx * u.x + dy * u.y;
        min_t = min_t.min(t);
        max_t = max_t.max(t);
        extent = extent.max((dx * n.x + dy * n.y).abs());
    }
    let mut cut_min = f64::INFINITY;
    let mut cut_max = f64::NEG_INFINITY;
    for ring in area
        .0
        .iter()
        .flat_map(|p| std::iter::once(p.exterior()).chain(p.interiors()))
    {
        for pair in ring.0.windows(2) {
            let side = |p: Coord<f64>| (p.x - a.x) * n.x + (p.y - a.y) * n.y;
            let (d0, d1) = (side(pair[0]), side(pair[1]));
            if (d0 > 0.) != (d1 > 0.) || d0.abs() < 1e-9 || d1.abs() < 1e-9 {
                let blend = if (d0 - d1).abs() < 1e-12 {
                    0.
                } else {
                    d0 / (d0 - d1)
                };
                let x = pair[0].x + (pair[1].x - pair[0].x) * blend;
                let y = pair[0].y + (pair[1].y - pair[0].y) * blend;
                let t = (x - a.x) * u.x + (y - a.y) * u.y;
                cut_min = cut_min.min(t);
                cut_max = cut_max.max(t);
            }
        }
    }
    if cut_min < -1e-6 || cut_max > length + 1e-6 {
        return Err("Extend the cutting line beyond both sides of the region".into());
    }
    let start = Point::new(a.x + u.x * (min_t - 1.), a.y + u.y * (min_t - 1.));
    let end = Point::new(a.x + u.x * (max_t + 1.), a.y + u.y * (max_t + 1.));
    let far = extent * 2. + 1.;
    let half = |side: f64| {
        ring_polygon(&[
            start,
            end,
            Point::new(end.x + n.x * far * side, end.y + n.y * far * side),
            Point::new(start.x + n.x * far * side, start.y + n.y * far * side),
        ])
    };
    let left = area.intersection(&MultiPolygon::new(vec![half(1.)?]));
    let right = area.intersection(&MultiPolygon::new(vec![half(-1.)?]));
    if left.unsigned_area() <= 1e-7 || right.unsigned_area() <= 1e-7 {
        return Err("The cutting line must cross the filled region".into());
    }
    let total = left.unsigned_area() + right.unsigned_area();
    if (total - area.unsigned_area()).abs() > 1e-5 * area.unsigned_area().max(1.) {
        return Err("Split could not preserve the region area".into());
    }
    let mut parts = Vec::new();
    let mut next_id = project.objects.iter().map(|o| o.id).max().unwrap_or(0);
    for (index, polygon) in left.0.into_iter().chain(right.0).enumerate() {
        let mut part = object.clone();
        if index > 0 {
            next_id = next_id
                .checked_add(1)
                .ok_or("Object identity limit reached")?;
            part.id = next_id;
        }
        part.name = format!("{} · {}", object.name, index + 1);
        part.contours = contours(&MultiPolygon::new(vec![polygon]));
        part.bezier_segments.clear();
        part.centerline_graph = None;
        parts.push(part);
    }
    let mut candidate = project.clone();
    candidate.objects.splice(object_index..=object_index, parts);
    candidate.validate_limits()?;
    Ok(candidate)
}
