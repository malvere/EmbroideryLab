//! Conservative physical-scale classification of newly traced flat artwork.
//!
//! A Satin decision always creates paired rails and validates actual sections;
//! skeleton width alone never changes the enum to Satin. Unsupported topology
//! retains an editable Tatami region with an object-specific explanation.
use crate::{
    Object, PlanWarning, Point, Project, SatinColumn, StitchKind, axis_edit, centerline, engine,
};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};

#[path = "autodigitize_color_order.rs"]
mod color_order;
pub use color_order::batch_fresh_raster_colors;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AutoProfile {
    pub running_max_width_mm: f64,
    pub satin_min_width_mm: f64,
    pub satin_max_width_mm: f64,
    pub min_detail_area_mm2: f64,
    pub min_stroke_length_mm: f64,
    pub skeleton_pixel_mm: f64,
    pub max_axis_pixels: usize,
    pub max_skeleton_pixels_per_object: usize,
    pub max_objects: usize,
}

impl Default for AutoProfile {
    fn default() -> Self {
        Self {
            running_max_width_mm: 0.65,
            satin_min_width_mm: 0.8,
            satin_max_width_mm: 8.,
            min_detail_area_mm2: 0.15,
            min_stroke_length_mm: 1.,
            skeleton_pixel_mm: 0.15,
            max_axis_pixels: 1024,
            max_skeleton_pixels_per_object: 500_000,
            max_objects: 256,
        }
    }
}

impl AutoProfile {
    pub fn validate(&self) -> Result<(), String> {
        if ![
            self.running_max_width_mm,
            self.satin_min_width_mm,
            self.satin_max_width_mm,
            self.min_detail_area_mm2,
            self.min_stroke_length_mm,
            self.skeleton_pixel_mm,
        ]
        .into_iter()
        .all(|v| v.is_finite() && v > 0.)
            || self.running_max_width_mm >= self.satin_min_width_mm
            || self.satin_min_width_mm >= self.satin_max_width_mm
            || self.satin_max_width_mm > 12.
            || !(0.005..=1.).contains(&self.skeleton_pixel_mm)
            || !(16..=2046).contains(&self.max_axis_pixels)
            || !(256..=4_000_000).contains(&self.max_skeleton_pixels_per_object)
            || !(1..=4096).contains(&self.max_objects)
        {
            return Err(
                "Invalid automatic stitch profile physical thresholds or work budget".into(),
            );
        }
        Ok(())
    }
}

pub fn classify_project(project: &mut Project, cancel: &AtomicBool) -> Vec<PlanWarning> {
    classify_project_with_profile(project, cancel, &AutoProfile::default())
}

/// Only intended for a freshly traced candidate, before user overrides are
/// restored. Cancel leaves the entire passed project unchanged.
pub fn classify_project_with_profile(
    project: &mut Project,
    cancel: &AtomicBool,
    profile: &AutoProfile,
) -> Vec<PlanWarning> {
    let mut warnings = Vec::new();
    if let Err(message) = profile.validate() {
        return vec![PlanWarning {
            object_id: 0,
            point: None,
            message,
        }];
    }
    if let Err(message) = project.machine_profile.validate() {
        return vec![PlanWarning {
            object_id: 0,
            point: None,
            message,
        }];
    }
    let machine_max_stitch_mm = project.machine_profile.max_stitch_mm;
    let mut objects = project.objects.clone();
    for (index, object) in objects.iter_mut().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            return vec![PlanWarning {
                object_id: 0,
                point: None,
                message: "Automatic classification cancelled; document retained".into(),
            }];
        }
        if !object.visible
            || !object.running_paths.is_empty()
            || object.satin_column.is_some()
            || object.settings.kind != StitchKind::Tatami
        {
            continue;
        }
        if index >= profile.max_objects {
            warn(
                &mut warnings,
                object,
                "Automatic classification work budget reached; retained Tatami. Classify smaller groups or edit the stitch type manually.",
            );
            continue;
        }
        if let Err(message) = classify_object(object, profile, machine_max_stitch_mm, cancel) {
            warn(&mut warnings, object, &message);
        }
    }
    if cancel.load(Ordering::Relaxed) {
        return vec![PlanWarning {
            object_id: 0,
            point: None,
            message: "Automatic classification cancelled; document retained".into(),
        }];
    }
    project.objects = objects;
    // Keep the original source region identity, but make the automatic result
    // the baseline for detecting subsequent manual overrides during retracing.
    if let Some(recipe) = &mut project.raster_recipe {
        for identity in &mut recipe.objects {
            if let Some(object) = project.objects.iter().find(|o| o.id == identity.object_id) {
                identity.original_fingerprint = crate::persistence::object_fingerprint(object);
            }
        }
        recipe.issues.extend(warnings.iter().cloned());
    }
    warnings
}

fn warn(warnings: &mut Vec<PlanWarning>, object: &Object, message: &str) {
    warnings.push(PlanWarning {
        object_id: object.id,
        point: object.contours.iter().flatten().copied().next(),
        message: message.into(),
    });
}

fn classify_object(
    object: &mut Object,
    profile: &AutoProfile,
    machine_max_stitch_mm: f64,
    cancel: &AtomicBool,
) -> Result<(), String> {
    let rings = object.flattened_contours();
    let area = filled_area(&rings);
    if area < profile.min_detail_area_mm2 {
        return Err(format!(
            "Detail area {area:.3} mm² is below profile minimum {:.3} mm²; retained geometry. Enlarge or choose a manual stitch type.",
            profile.min_detail_area_mm2
        ));
    }
    let points: Vec<_> = rings.iter().flatten().copied().collect();
    let min = Point::new(
        points.iter().map(|p| p.x).fold(f64::INFINITY, f64::min),
        points.iter().map(|p| p.y).fold(f64::INFINITY, f64::min),
    );
    let max = Point::new(
        points.iter().map(|p| p.x).fold(f64::NEG_INFINITY, f64::max),
        points.iter().map(|p| p.y).fold(f64::NEG_INFINITY, f64::max),
    );
    let perimeter: f64 = rings
        .iter()
        .map(|ring| {
            ring.iter()
                .zip(ring.iter().cycle().skip(1))
                .take(ring.len())
                .map(|(a, b)| a.distance(*b))
                .sum::<f64>()
        })
        .sum();
    let mean_width_hint = 2. * area / perimeter.max(1e-9);
    let bbox_width = (max.x - min.x).min(max.y - min.y);
    object.settings.auto_angle = true;

    if let Some((column, width, length)) = rectangular_column(&rings) {
        if length < profile.min_stroke_length_mm {
            return Err(
                "Stroke is too short for reliable automatic Satin or Running; retained Tatami"
                    .into(),
            );
        }
        if width <= profile.running_max_width_mm {
            let path = vec![
                column.left[0].lerp(column.right[0], 0.5),
                column.left[1].lerp(column.right[1], 0.5),
            ];
            return use_running(object, vec![path], profile.skeleton_pixel_mm);
        }
        if width >= profile.satin_min_width_mm
            && width <= profile.satin_max_width_mm
            && length >= width * 2.
        {
            return use_satin(object, column, profile, machine_max_stitch_mm);
        }
        if width > profile.satin_max_width_mm && length >= width * 2. {
            return Err(format!(
                "Stroke width {width:.2} mm exceeds automatic Satin maximum {:.2} mm; retained Tatami instead of subdividing wide diagonal stitches",
                profile.satin_max_width_mm
            ));
        }
        return Ok(());
    }
    // For an annular border, the existing closed-column model can use the
    // actual outer/inner source boundaries directly. No inferred offset changes
    // occupancy. The normal column validator rejects folds, ambiguous seams,
    // excessive widths and crossings of the protected inner opening.
    if rings.len() == 2 && mean_width_hint <= profile.satin_max_width_mm {
        let column = closed_boundary_column(&rings)
            .ok_or("Closed border exceeds editable rail limits; retained Tatami")?;
        let stations =
            engine::satin_stations(&column, object.settings.spacing_mm).map_err(|e| {
                format!(
                    "Closed Satin correspondence rejected: {e}; retained Tatami with its opening"
                )
            })?;
        let min_width = stations
            .iter()
            .map(|(a, b)| a.distance(*b))
            .fold(f64::INFINITY, f64::min);
        if min_width < profile.satin_min_width_mm {
            return Err("Closed border is below automatic Satin minimum width; retained Tatami. Choose Running or adjust rails manually.".into());
        }
        return use_satin(object, column, profile, machine_max_stitch_mm);
    }
    if mean_width_hint > profile.satin_max_width_mm
        || (bbox_width > profile.satin_max_width_mm * 2.
            && mean_width_hint > profile.satin_max_width_mm * 0.65)
    {
        return Ok(());
    }
    let pixel = profile
        .skeleton_pixel_mm
        .min(mean_width_hint.max(0.02) / 3.);
    let columns = ((max.x - min.x) / pixel).ceil() as usize + 2;
    let rows = ((max.y - min.y) / pixel).ceil() as usize + 2;
    if columns > profile.max_axis_pixels
        || rows > profile.max_axis_pixels
        || columns.saturating_mul(rows) > profile.max_skeleton_pixels_per_object
    {
        return Err("Narrow-region axis analysis exceeds profile work budget; retained Tatami. Split the region or use manual Satin rails.".into());
    }
    let axes = centerline::extract(object, pixel).map_err(|e| {
        format!("Axis extraction could not classify this region: {e}; retained Tatami")
    })?;
    if cancel.load(Ordering::Relaxed) {
        return Ok(());
    }
    if axes.paths.is_empty() || !axes.isolated_points.is_empty() {
        return Err("Axis extraction produced empty or isolated detail; retained Tatami geometry for manual correction".into());
    }
    let graph = axis_edit::attach(&axes.paths, object, pixel)?;
    let mut axis_object = object.clone();
    axis_object.running_paths = graph.paths();
    axis_object.centerline_graph = Some(graph.clone());
    let sample_count: usize = axes.paths.iter().map(Vec::len).sum();
    let stride = (sample_count / 128).max(1);
    let max_width = axes
        .paths
        .iter()
        .flatten()
        .step_by(stride)
        .filter_map(|&p| axis_edit::width_at(&axis_object, p))
        .fold(0., f64::max);
    let total_length: f64 = axes
        .paths
        .iter()
        .flat_map(|p| p.windows(2))
        .map(|p| p[0].distance(p[1]))
        .sum();
    if max_width <= profile.running_max_width_mm && total_length >= profile.min_stroke_length_mm {
        return use_running(object, axes.paths, pixel);
    }
    if graph.branches.len() != 1 || graph.vertices.iter().any(|v| graph.degree(v.id) > 2) {
        return Err("Narrow region contains axis branches or several strokes; retained Tatami to preserve junction coverage. Split it before Satin conversion.".into());
    }
    let branch = &graph.branches[0];
    if branch.start_vertex == branch.end_vertex {
        return Err("Closed narrow region needs an explicit editable Satin seam; retained Tatami. Use the closed outline tool or choose a seam manually.".into());
    }
    if max_width > profile.satin_max_width_mm {
        return Err(format!(
            "Local width estimate {max_width:.2} mm exceeds profile Satin maximum; retained Tatami"
        ));
    }
    if max_width < profile.satin_min_width_mm {
        return Err("Region lies between Running and Satin profile widths; retained Tatami pending a manual stitch choice".into());
    }
    let columns = axis_edit::semi_auto_satin(&axis_object, 0)
        .map_err(|e| format!("Automatic Satin rails rejected: {e}; retained Tatami"))?;
    if columns.len() != 1 {
        return Err("Automatic Satin requires a single certified column; retained Tatami".into());
    }
    let column = columns.into_iter().next().unwrap();
    let column_area = filled_area(&column.boundary_rings()?);
    // Containment is certified by semi_auto_satin. Do not silently discard caps
    // or branches merely because the narrow core can be expressed as rails.
    if column_area < area * 0.97 {
        return Err(format!(
            "Inferred Satin column covers only {:.1}% of region area; retained Tatami to preserve end caps",
            100. * column_area / area
        ));
    }
    use_satin(object, column, profile, machine_max_stitch_mm)
}

fn use_satin(
    object: &mut Object,
    column: SatinColumn,
    profile: &AutoProfile,
    machine_max_stitch_mm: f64,
) -> Result<(), String> {
    let stations = engine::satin_stations(&column, object.settings.spacing_mm)?;
    let width = stations
        .iter()
        .map(|(a, b)| a.distance(*b))
        .fold(0., f64::max);
    if width > profile.satin_max_width_mm + 1e-8 {
        return Err(format!(
            "Actual Satin cross-section {width:.2} mm exceeds profile maximum; retained Tatami"
        ));
    }
    // A Tatami object's 3.5mm segment cap is not a Satin construction rule.
    // Preserve complete alternating diagonals, for either starting rail, before
    // changing the object type. Both Builder and machine normalization reserve
    // 0.15mm; compare actual quantized diagonals as well as unrounded geometry.
    let mut pairs = stations;
    let exterior_side =
        usize::from(signed_area(&column.right).abs() > signed_area(&column.left).abs());
    for pair in &mut pairs {
        let original = *pair;
        let width = original.0.distance(original.1);
        let comp = object.settings.pull_comp_mm;
        if !column.closed || exterior_side == 0 {
            pair.0 = original.0.lerp(original.1, -comp / width);
        }
        if !column.closed || exterior_side == 1 {
            pair.1 = original.1.lerp(original.0, -comp / width);
        }
    }
    let quantize = |p: Point| Point::new((p.x * 10.).round() / 10., (p.y * 10.).round() / 10.);
    let mut required = 0_f64;
    let mut include = |a: Point, b: Point| {
        required = required
            .max(a.distance(b))
            .max(quantize(a).distance(quantize(b)));
    };
    for &(left, right) in &pairs {
        include(left, right);
    }
    for window in pairs.windows(2) {
        // These are the two possible alternating phases. Include same-rail
        // corners too, bounding the engine's intentional inner-bend insets.
        for a in [window[0].0, window[0].1] {
            for b in [window[1].0, window[1].1] {
                include(a, b);
            }
        }
    }
    if column.closed
        && let (Some(&first), Some(&last)) = (pairs.first(), pairs.last())
    {
        include(last.0, first.1);
        include(last.1, first.0);
    }
    let intact_length_mm = required + 0.15 + 1e-8;
    if intact_length_mm > machine_max_stitch_mm {
        return Err(format!(
            "Required intact Satin diagonal {required:.3} mm plus 0.15 mm grid margin exceeds machine stitch limit {machine_max_stitch_mm:.3} mm; retained Tatami. Use a supported machine limit or constructively split the column."
        ));
    }
    object.contours.clear();
    object.bezier_segments.clear();
    object.running_paths.clear();
    object.centerline_graph = None;
    object.satin_column = Some(column);
    object.settings.kind = StitchKind::Satin;
    object.settings.auto_angle = false;
    object.settings.length_mm = intact_length_mm;
    Ok(())
}

fn use_running(object: &mut Object, paths: Vec<Vec<Point>>, pixel: f64) -> Result<(), String> {
    // A skeleton can contain a valid long stroke and a tiny separate spur.
    // Total axis length alone cannot certify every exported branch. Refuse
    // the automatic conversion before discarding its filled source when even
    // one branch would have no distinct punctures on the actual machine grid.
    let grid_scale = 1. / crate::commands::MACHINE_GRID_MM;
    let grid = |p: Point| {
        (
            (p.x * grid_scale).round() as i64,
            (p.y * grid_scale).round() as i64,
        )
    };
    for (index, path) in paths.iter().enumerate() {
        if let Some(&first) = path.first()
            && path.iter().all(|&point| grid(point) == grid(first))
        {
            return Err(format!(
                "Automatic Running branch {} collapses on the 0.1mm machine grid; retained Tatami source geometry. Enlarge the detail or edit its branches manually.",
                index + 1
            ));
        }
    }
    // Keep the original filled region in the editable graph's immutable source
    // snapshot, then use paths exclusively for generating the Running object.
    let graph = axis_edit::attach(&paths, object, pixel)?;
    object.contours.clear();
    object.bezier_segments.clear();
    object.satin_column = None;
    object.running_paths = graph.paths();
    object.centerline_graph = Some(graph);
    object.settings.kind = StitchKind::Running;
    object.settings.underlay = false;
    object.settings.edge_underlay = false;
    object.settings.zigzag_underlay = false;
    object.settings.auto_angle = false;
    Ok(())
}

fn rectangular_column(rings: &[Vec<Point>]) -> Option<(SatinColumn, f64, f64)> {
    if rings.len() != 1 || rings[0].len() != 4 {
        return None;
    }
    let ring = &rings[0];
    let long = (0..4).max_by(|&a, &b| {
        ring[a]
            .distance(ring[(a + 1) % 4])
            .total_cmp(&ring[b].distance(ring[(b + 1) % 4]))
    })?;
    let [a, b, c, d] = [
        ring[long],
        ring[(long + 1) % 4],
        ring[(long + 2) % 4],
        ring[(long + 3) % 4],
    ];
    let u = Point::new(b.x - a.x, b.y - a.y);
    let v = Point::new(d.x - a.x, d.y - a.y);
    let length = a.distance(b);
    let width = a.distance(d);
    if length <= 1e-8
        || width <= 1e-8
        || (u.x * v.x + u.y * v.y).abs() > 1e-6 * length * width
        || c.distance(Point::new(b.x + v.x, b.y + v.y)) > 1e-6 * length.max(width)
    {
        return None;
    }
    Some((
        SatinColumn {
            left: vec![a, b],
            right: vec![d, c],
            ..SatinColumn::default()
        },
        width,
        length,
    ))
}

fn signed_area(ring: &[Point]) -> f64 {
    ring.iter()
        .zip(ring.iter().cycle().skip(1))
        .take(ring.len())
        .map(|(a, b)| a.x * b.y - b.x * a.y)
        .sum::<f64>()
        * 0.5
}

fn closed_boundary_column(rings: &[Vec<Point>]) -> Option<SatinColumn> {
    if rings.iter().any(|r| r.len() < 3 || r.len() > 1024) {
        return None;
    }
    let (mut outer, mut inner) = if signed_area(&rings[0]).abs() >= signed_area(&rings[1]).abs() {
        (rings[0].clone(), rings[1].clone())
    } else {
        (rings[1].clone(), rings[0].clone())
    };
    let start = (0..outer.len()).min_by(|&a, &b| {
        outer[a]
            .x
            .total_cmp(&outer[b].x)
            .then(outer[a].y.total_cmp(&outer[b].y))
    })?;
    outer.rotate_left(start);
    if signed_area(&outer) * signed_area(&inner) < 0. {
        inner.reverse();
    }
    let target = outer[0];
    let total: f64 = inner
        .iter()
        .zip(inner.iter().cycle().skip(1))
        .take(inner.len())
        .map(|(a, b)| a.distance(*b))
        .sum();
    let mut traversed = 0.;
    let mut closest = (f64::INFINITY, 0.);
    for (&a, &b) in inner
        .iter()
        .zip(inner.iter().cycle().skip(1))
        .take(inner.len())
    {
        let dx = b.x - a.x;
        let dy = b.y - a.y;
        let norm = dx * dx + dy * dy;
        if norm <= 1e-12 {
            return None;
        }
        let t = (((target.x - a.x) * dx + (target.y - a.y) * dy) / norm).clamp(0., 1.);
        let distance = target.distance(a.lerp(b, t));
        if distance < closest.0 {
            closest = (distance, (traversed + t * norm.sqrt()) / total);
        }
        traversed += norm.sqrt();
    }
    // rotate_closed operates on an explicitly closed flattened path, whereas
    // editable source contours omit their repeated final vertex.
    let mut closed_inner = inner.clone();
    closed_inner.push(inner[0]);
    let mut aligned =
        crate::satin_edit::rotate_closed(&closed_inner, closest.1.rem_euclid(1.)).ok()?;
    if aligned.first()?.distance(*aligned.last()?) < 1e-8 {
        aligned.pop();
    }
    Some(SatinColumn {
        left: outer,
        right: aligned,
        closed: true,
        ..SatinColumn::default()
    })
}

fn filled_area(rings: &[Vec<Point>]) -> f64 {
    let contains = |p: Point, ring: &[Point]| {
        let mut inside = false;
        for (&a, &b) in ring
            .iter()
            .zip(ring.iter().cycle().skip(1))
            .take(ring.len())
        {
            if (a.y > p.y) != (b.y > p.y) && p.x < (b.x - a.x) * (p.y - a.y) / (b.y - a.y) + a.x {
                inside = !inside;
            }
        }
        inside
    };
    rings
        .iter()
        .enumerate()
        .map(|(i, ring)| {
            let area = signed_area(ring).abs();
            let depth = rings
                .iter()
                .enumerate()
                .filter(|(j, other)| *j != i && !ring.is_empty() && contains(ring[0], other))
                .count();
            if depth % 2 == 0 { area } else { -area }
        })
        .sum::<f64>()
        .max(0.)
}
