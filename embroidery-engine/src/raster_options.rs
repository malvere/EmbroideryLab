//! Configurable flat-colour raster tracing. No Embird implementation is used.
use super::{name, rgb_distance, simplify_ring, trace_region};
use crate::color_selection::{
    self, CleanupMask, oklab_to_srgb, squared_distance, srgb_f64_to_oklab, srgb_to_oklab,
};
use crate::model::*;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, VecDeque},
    io::Read,
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};

#[path = "raster_preparation.rs"]
mod preparation;
pub use preparation::{PREPARATION_VERSION, PreparationDiagnostics, PreparationMode};

/// Background masks are explicit import decisions. Tolerance is Euclidean
/// distance in ordinary 8-bit RGB coordinates (0..sqrt(3*255^2)), not percent.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum BackgroundMode {
    NearWhite,
    TransparentOnly,
    Color {
        rgb: [u8; 3],
        tolerance: f64,
    },
    /// An explicit union of colour masks; no white/fabric colour is added.
    Colors {
        rgb: Vec<[u8; 3]>,
        tolerance: f64,
    },
    /// Infer the most frequent opaque perimeter colour, then apply a global
    /// colour-distance mask. This does not infer fabric or artwork semantics.
    Border {
        tolerance: f64,
    },
    /// Remove only matching pixels connected to the image perimeter through
    /// four-neighbour matching/transparent pixels. Enclosed matching details
    /// remain foreground; selected colours form one union for connectivity.
    ConnectedColors {
        rgb: Vec<[u8; 3]>,
        tolerance: f64,
    },
    /// Estimate the dominant opaque perimeter colour, then remove only its
    /// four-connected exterior. This preserves enclosed same-colour details.
    ConnectedBorder {
        tolerance: f64,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RasterOptions {
    pub max_dimension: u32,
    pub palette_size: usize,
    pub background: BackgroundMode,
    pub min_region_pixels: usize,
    /// Geometric simplification tolerance after scaling into millimetres.
    pub simplify_tolerance_mm: f64,
    /// Optional corner cutting; topology changes are rejected, not hidden.
    pub smoothing_iterations: u8,
    /// Explicit nondestructive selection in immutable source-image pixels.
    pub cleanup: Option<CleanupMask>,
    /// Physical region threshold; min_region_pixels is a legacy technical floor.
    pub min_region_area_mm2: f64,
    /// Keep canonical grid boundaries where differently labelled regions meet.
    pub preserve_shared_boundaries: bool,
    /// Explicit bounded isolated-label denoising, never transparent-hole closing.
    pub denoise_passes: u8,
    /// User-selected exact thread colours; remaining palette slots are learned.
    pub pinned_colors: Vec<[u8; 3]>,
    /// Experimental, nondestructive boundary preparation. Missing fields in
    /// historical recipes retain the exact legacy segmentation.
    #[serde(skip_serializing_if = "PreparationMode::is_legacy")]
    pub preparation: PreparationMode,
}
impl Default for RasterOptions {
    fn default() -> Self {
        Self {
            max_dimension: 256,
            palette_size: 6,
            background: BackgroundMode::NearWhite,
            min_region_pixels: 6,
            simplify_tolerance_mm: 0.1,
            smoothing_iterations: 0,
            cleanup: None,
            min_region_area_mm2: 0.0,
            preserve_shared_boundaries: true,
            denoise_passes: 0,
            pinned_colors: Vec::new(),
            preparation: PreparationMode::Legacy,
        }
    }
}
impl RasterOptions {
    /// New interactive workflow. Historical library defaults are retained for
    /// existing callers; this entry point never silently discards white art.
    pub fn autodigitize() -> Self {
        Self {
            max_dimension: 512,
            palette_size: 8,
            background: BackgroundMode::TransparentOnly,
            min_region_pixels: 1,
            // At a typical 100mm / 512px import, a single resampled AA/noise
            // pixel can exceed 0.04mm². Keep that below the interactive floor;
            // omissions are reported and the operator can lower this value.
            min_region_area_mm2: 0.1,
            ..Self::default()
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if !(16..=2048).contains(&self.max_dimension) {
            return Err("Raster max_dimension must be 16–2048 pixels".into());
        }
        if !(2..=32).contains(&self.palette_size) {
            return Err("Raster palette_size must be 2–32 colours".into());
        }
        if self.pinned_colors.len() > self.palette_size || self.pinned_colors.len() > 32 {
            return Err("Pinned colours must fit the requested palette size (at most 32)".into());
        }
        for (i, color) in self.pinned_colors.iter().enumerate() {
            if self.pinned_colors[..i].contains(color) {
                return Err("Pinned colours must be distinct".into());
            }
        }
        if !(1..=1_000_000).contains(&self.min_region_pixels) {
            return Err("Raster min_region_pixels must be 1–1,000,000".into());
        }
        if !self.simplify_tolerance_mm.is_finite()
            || !(0.0..=2.0).contains(&self.simplify_tolerance_mm)
        {
            return Err("Raster simplify_tolerance_mm must be finite and 0–2mm".into());
        }
        if self.smoothing_iterations > 3 {
            return Err("Raster smoothing_iterations must be 0–3".into());
        }
        if self.denoise_passes > 2 {
            return Err("Raster denoise_passes must be 0–2".into());
        }
        if !self.min_region_area_mm2.is_finite()
            || !(0.0..=100.0).contains(&self.min_region_area_mm2)
        {
            return Err("Raster min_region_area_mm2 must be finite and 0–100mm²".into());
        }
        if let Some(cleanup) = &self.cleanup {
            cleanup.validate()?;
        }
        match self.background {
            BackgroundMode::Color { tolerance, .. }
            | BackgroundMode::Colors { tolerance, .. }
            | BackgroundMode::Border { tolerance }
            | BackgroundMode::ConnectedColors { tolerance, .. }
            | BackgroundMode::ConnectedBorder { tolerance }
                if !tolerance.is_finite() || !(0.0..=441.7).contains(&tolerance) =>
            {
                return Err(
                    "Background tolerance must be finite Euclidean RGB distance 0–441.7".into(),
                );
            }
            _ => {}
        }
        if let BackgroundMode::Colors { ref rgb, .. }
        | BackgroundMode::ConnectedColors { ref rgb, .. } = self.background
            && !(1..=16).contains(&rgb.len())
        {
            return Err("Background Colors must contain 1–16 explicit RGB colours".into());
        }
        Ok(())
    }
}

#[derive(Default)]
struct Bin {
    count: u64,
    sum: [f64; 3],
}
impl Bin {
    fn add(&mut self, color: [u8; 3]) {
        self.count += 1;
        for (sum, value) in self.sum.iter_mut().zip(color) {
            *sum += f64::from(value);
        }
    }
    fn center(&self) -> [f64; 3] {
        self.sum.map(|sum| sum / self.count as f64)
    }
}
fn bucket(color: [u8; 3]) -> [u8; 3] {
    color.map(|value| value / 8)
}

fn border_color(image: &image::RgbaImage) -> Result<[f64; 3], String> {
    let mut histogram: BTreeMap<[u8; 3], Bin> = BTreeMap::new();
    let (width, height) = image.dimensions();
    for (x, y, pixel) in image.enumerate_pixels() {
        if x != 0 && y != 0 && x + 1 != width && y + 1 != height {
            continue;
        }
        if pixel[3] < 128 {
            continue;
        }
        let color = [pixel[0], pixel[1], pixel[2]];
        histogram.entry(bucket(color)).or_default().add(color);
    }
    histogram.values().max_by_key(|bin|bin.count).map(Bin::center)
        .ok_or_else(||"Border background has no opaque perimeter samples; choose TransparentOnly or an explicit colour".into())
}

fn is_background(pixel: &image::Rgba<u8>, mode: &BackgroundMode, border: Option<[f64; 3]>) -> bool {
    if pixel[3] < 128 {
        return true;
    }
    match *mode {
        BackgroundMode::NearWhite => pixel[0] > 245 && pixel[1] > 245 && pixel[2] > 245,
        BackgroundMode::TransparentOnly => false,
        BackgroundMode::Color { rgb, tolerance } => {
            rgb_distance(
                [pixel[0], pixel[1], pixel[2]].map(f64::from),
                rgb.map(f64::from),
            ) <= tolerance * tolerance
        }
        BackgroundMode::Colors { ref rgb, tolerance }
        | BackgroundMode::ConnectedColors { ref rgb, tolerance } => rgb.iter().any(|color| {
            rgb_distance(
                [pixel[0], pixel[1], pixel[2]].map(f64::from),
                color.map(f64::from),
            ) <= tolerance * tolerance
        }),
        BackgroundMode::Border { tolerance } | BackgroundMode::ConnectedBorder { tolerance } => {
            border.is_some_and(|color| {
                rgb_distance([pixel[0], pixel[1], pixel[2]].map(f64::from), color)
                    <= tolerance * tolerance
            })
        }
    }
}

/// Build the mask once, before palette selection. For connected modes the
/// flood traverses only eligible pixels, never opaque nonmatching barriers or
/// diagonal contacts. Transparent pixels are always excluded from stitching,
/// but an isolated transparent hole does not seed removal of adjacent colours.
fn background_mask(image: &image::RgbaImage, mode: &BackgroundMode) -> Result<Vec<bool>, String> {
    let border = if matches!(
        mode,
        BackgroundMode::Border { .. } | BackgroundMode::ConnectedBorder { .. }
    ) {
        Some(border_color(image)?)
    } else {
        None
    };
    let eligible: Vec<bool> = image
        .pixels()
        .map(|pixel| is_background(pixel, mode, border))
        .collect();
    if !matches!(
        mode,
        BackgroundMode::ConnectedColors { .. } | BackgroundMode::ConnectedBorder { .. }
    ) {
        return Ok(eligible);
    }
    let (width, height) = (image.width() as usize, image.height() as usize);
    let mut mask = vec![false; eligible.len()];
    let mut queue = VecDeque::new();
    // The image-size limits bound this flood to at most 2048² visits.
    for x in 0..width {
        for index in [x, (height - 1) * width + x] {
            if eligible[index] && !mask[index] {
                mask[index] = true;
                queue.push_back(index);
            }
        }
    }
    for y in 0..height {
        for index in [y * width, y * width + width - 1] {
            if eligible[index] && !mask[index] {
                mask[index] = true;
                queue.push_back(index);
            }
        }
    }
    while let Some(index) = queue.pop_front() {
        let (x, y) = (index % width, index / width);
        for neighbor in [
            x.checked_sub(1).map(|nx| y * width + nx),
            (x + 1 < width).then_some(index + 1),
            y.checked_sub(1).map(|ny| ny * width + x),
            (y + 1 < height).then_some(index + width),
        ]
        .into_iter()
        .flatten()
        {
            if eligible[neighbor] && !mask[neighbor] {
                mask[neighbor] = true;
                queue.push_back(neighbor);
            }
        }
    }
    for (background, pixel) in mask.iter_mut().zip(image.pixels()) {
        *background |= pixel[3] < 128;
    }
    Ok(mask)
}

fn palette_from_histogram(
    histogram: &BTreeMap<[u8; 3], Bin>,
    requested: usize,
    pixels: usize,
    cancel: Option<&AtomicBool>,
    pinned: &[[u8; 3]],
) -> Result<Vec<[f64; 3]>, String> {
    let bins: Vec<_> = histogram
        .values()
        .map(|bin| (srgb_f64_to_oklab(bin.center()), bin.count))
        .collect();
    let size = requested.min(bins.len().saturating_add(pinned.len()));
    let work = (bins.len() as u64) * (size as u64) * 12 + (pixels as u64) * (size as u64);
    if work > 150_000_000 {
        return Err("Raster palette work exceeds budget; reduce resolution or colour count".into());
    }
    let mut palette: Vec<_> = pinned.iter().copied().map(srgb_to_oklab).collect();
    let frozen = palette.len();
    while palette.len() < size {
        check_cancel(cancel)?;
        let (chosen, _) = bins
            .iter()
            .max_by(|(a, na), (b, nb)| {
                let score = |color: [f64; 3], count: u64| {
                    let distance = if palette.is_empty() {
                        1.0
                    } else {
                        squared_distance(color, palette[nearest_lab(color, &palette)])
                    };
                    distance * (count as f64).sqrt()
                };
                score(*a, *na).total_cmp(&score(*b, *nb))
            })
            .ok_or("No foreground pixels for palette")?;
        if palette
            .iter()
            .any(|color| squared_distance(*color, *chosen) < 1e-14)
        {
            break;
        }
        palette.push(*chosen);
    }
    for _ in 0..8 {
        check_cancel(cancel)?;
        let mut sums = vec![[0.; 3]; palette.len()];
        let mut counts = vec![0_u64; palette.len()];
        for &(color, count) in &bins {
            let index = nearest_lab(color, &palette);
            for (sum, value) in sums[index].iter_mut().zip(color) {
                *sum += value * (count as f64);
            }
            counts[index] += count;
        }
        for (index, count) in counts.into_iter().enumerate() {
            if index >= frozen && count > 0 {
                palette[index] = sums[index].map(|sum| sum / count as f64);
            }
        }
    }
    Ok(palette
        .into_iter()
        .enumerate()
        .map(|(i, lab)| {
            pinned
                .get(i)
                .copied()
                .unwrap_or_else(|| oklab_to_srgb(lab))
                .map(f64::from)
        })
        .collect())
}

fn nearest_lab(color: [f64; 3], palette: &[[f64; 3]]) -> usize {
    palette
        .iter()
        .enumerate()
        .min_by(|(_, a), (_, b)| {
            squared_distance(color, **a).total_cmp(&squared_distance(color, **b))
        })
        .map_or(0, |(index, _)| index)
}

fn denoise_labels(labels: &mut [u8], w: usize, h: usize, passes: u8, protected: &[bool]) -> usize {
    if w < 3 || h < 3 {
        return 0;
    }
    let mut changed = 0;
    for _ in 0..passes {
        let previous = labels.to_vec();
        for y in 1..h - 1 {
            for x in 1..w - 1 {
                let i = y * w + x;
                if previous[i] == 255 || protected[i] {
                    continue;
                }
                let neighbor = previous[i - 1];
                if neighbor != 255
                    && neighbor != previous[i]
                    && [i + 1, i - w, i + w]
                        .into_iter()
                        .all(|n| previous[n] == neighbor)
                {
                    labels[i] = neighbor;
                    changed += 1;
                }
            }
        }
    }
    changed
}

fn area(ring: &[Point]) -> f64 {
    ring.iter()
        .zip(ring.iter().cycle().skip(1))
        .take(ring.len())
        .map(|(a, b)| a.x * b.y - b.x * a.y)
        .sum::<f64>()
        * 0.5
}
fn cross(a: Point, b: Point, c: Point) -> f64 {
    (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x)
}
fn on_segment(p: Point, a: Point, b: Point) -> bool {
    cross(a, b, p).abs() <= 1e-8
        && p.x >= a.x.min(b.x) - 1e-8
        && p.x <= a.x.max(b.x) + 1e-8
        && p.y >= a.y.min(b.y) - 1e-8
        && p.y <= a.y.max(b.y) + 1e-8
}
fn intersects(a: Point, b: Point, c: Point, d: Point) -> bool {
    if a.x.min(b.x) > c.x.max(d.x) + 1e-8
        || c.x.min(d.x) > a.x.max(b.x) + 1e-8
        || a.y.min(b.y) > c.y.max(d.y) + 1e-8
        || c.y.min(d.y) > a.y.max(b.y) + 1e-8
    {
        return false;
    }
    let (ca, cb, cc, cd) = (
        cross(a, b, c),
        cross(a, b, d),
        cross(c, d, a),
        cross(c, d, b),
    );
    (ca * cb < 0. && cc * cd < 0.)
        || on_segment(c, a, b)
        || on_segment(d, a, b)
        || on_segment(a, c, d)
        || on_segment(b, c, d)
}
/// -1 outside, 0 boundary, 1 inside.
fn location(p: Point, ring: &[Point]) -> i8 {
    let mut inside = false;
    for (&a, &b) in ring
        .iter()
        .zip(ring.iter().cycle().skip(1))
        .take(ring.len())
    {
        if on_segment(p, a, b) {
            return 0;
        }
        if (a.y > p.y) != (b.y > p.y) && p.x < (b.x - a.x) * (p.y - a.y) / (b.y - a.y) + a.x {
            inside = !inside;
        }
    }
    if inside { 1 } else { -1 }
}
fn remove_collinear(ring: &[Point]) -> Vec<Point> {
    let mut out = Vec::new();
    for index in 0..ring.len() {
        let (a, b, c) = (
            ring[(index + ring.len() - 1) % ring.len()],
            ring[index],
            ring[(index + 1) % ring.len()],
        );
        if cross(a, b, c).abs() > 1e-10
            || (b.x - a.x) * (c.x - b.x) + (b.y - a.y) * (c.y - b.y) < 0.
        {
            out.push(b);
        }
    }
    if out.len() < 3 { ring.to_vec() } else { out }
}
fn corner_cut(ring: &[Point], tolerance_mm: f64) -> Vec<Point> {
    let mut out = Vec::with_capacity(ring.len() * 2);
    for (index, &point) in ring.iter().enumerate() {
        let previous = ring[(index + ring.len() - 1) % ring.len()];
        let next = ring[(index + 1) % ring.len()];
        let before = point.distance(previous);
        let after = point.distance(next);
        let dot = (previous.x - point.x) * (next.x - point.x)
            + (previous.y - point.y) * (next.y - point.y);
        // Retain characteristic corners (including re-entrant corners) through
        // 110 degrees. Smoothing an isolated rectangle must not chamfer a flag.
        if tolerance_mm <= 0. || dot >= -0.342_020_143_325_668_7 * before * after {
            out.push(point);
            continue;
        }
        // The original corner and both replacement points fit inside this
        // physical error ball; edge length or input resolution cannot enlarge it.
        let distance = tolerance_mm.min(before * 0.25).min(after * 0.25);
        out.push(point.lerp(previous, distance / before));
        out.push(point.lerp(next, distance / after));
    }
    out
}

fn ring_perimeter(ring: &[Point]) -> f64 {
    ring.iter()
        .zip(ring.iter().cycle().skip(1))
        .take(ring.len())
        .map(|(a, b)| a.distance(*b))
        .sum()
}

fn point_segment_distance(p: Point, a: Point, b: Point) -> f64 {
    let length_squared = (b.x - a.x).powi(2) + (b.y - a.y).powi(2);
    if length_squared <= 1e-20 {
        return p.distance(a);
    }
    let t =
        (((p.x - a.x) * (b.x - a.x) + (p.y - a.y) * (b.y - a.y)) / length_squared).clamp(0., 1.);
    p.distance(a.lerp(b, t))
}

/// Conservative, bidirectional boundary-distance certificate. Distance to a
/// closed polyline is 1-Lipschitz, so midpoint distance + half interval length
/// bounds the entire interval, rather than checking only a few sample points.
fn boundary_within_tolerance(
    original: &[Vec<Point>],
    candidate: &[Vec<Point>],
    tolerance: f64,
    budget: &mut usize,
) -> bool {
    if original == candidate {
        return true;
    }
    if tolerance <= 0. {
        return false;
    }
    for (from, to) in [(original, candidate), (candidate, original)] {
        for (ring, target) in from.iter().zip(to) {
            for (&a, &b) in ring
                .iter()
                .zip(ring.iter().cycle().skip(1))
                .take(ring.len())
            {
                let mut pending = vec![(a, b)];
                while let Some((start, end)) = pending.pop() {
                    let midpoint = start.lerp(end, 0.5);
                    let mut nearest = f64::INFINITY;
                    for (&p, &q) in target
                        .iter()
                        .zip(target.iter().cycle().skip(1))
                        .take(target.len())
                    {
                        if *budget == 0 {
                            return false;
                        }
                        *budget -= 1;
                        // If both interval endpoints are within one target
                        // segment's capsule, convexity certifies the interval.
                        if point_segment_distance(start, p, q) <= tolerance + 1e-10
                            && point_segment_distance(end, p, q) <= tolerance + 1e-10
                        {
                            nearest = -1.;
                            break;
                        }
                        nearest = nearest.min(point_segment_distance(midpoint, p, q));
                    }
                    if nearest < 0. || nearest + start.distance(end) * 0.5 <= tolerance + 1e-10 {
                        continue;
                    }
                    if nearest > tolerance + 1e-10 || start.distance(end) <= 1e-8 {
                        return false;
                    }
                    pending.push((start, midpoint));
                    pending.push((midpoint, end));
                }
            }
        }
    }
    true
}

/// Certify candidate rings with bounded exact segment checks. A candidate with
/// crossings, touching rings, or changed nesting/orientation is
/// rejected. Uncertifiable simplification keeps the original traced boundary;
/// smoothing likewise keeps the last certified boundary with a diagnostic.
fn topology_preserved(
    original: &[Vec<Point>],
    candidate: &[Vec<Point>],
    budget: &mut usize,
) -> bool {
    if original.len() != candidate.len() {
        return false;
    }
    let mut spend = || {
        if *budget == 0 {
            false
        } else {
            *budget -= 1;
            true
        }
    };
    for (before, ring) in original.iter().zip(candidate) {
        if ring.len() < 3
            || ring.len() > 8192
            || area(before) * area(ring) <= 0.
            || area(ring).abs() <= 1e-12
        {
            return false;
        }
        for i in 0..ring.len() {
            for j in i + 1..ring.len() {
                if j == i + 1 || (i == 0 && j + 1 == ring.len()) {
                    continue;
                }
                if !spend()
                    || intersects(
                        ring[i],
                        ring[(i + 1) % ring.len()],
                        ring[j],
                        ring[(j + 1) % ring.len()],
                    )
                {
                    return false;
                }
            }
        }
    }
    for i in 0..candidate.len() {
        for j in i + 1..candidate.len() {
            for (&a, &b) in candidate[i]
                .iter()
                .zip(candidate[i].iter().cycle().skip(1))
                .take(candidate[i].len())
            {
                for (&c, &d) in candidate[j]
                    .iter()
                    .zip(candidate[j].iter().cycle().skip(1))
                    .take(candidate[j].len())
                {
                    if !spend() || intersects(a, b, c, d) {
                        return false;
                    }
                }
            }
            for (inner, outer) in [(i, j), (j, i)] {
                let before = location(original[inner][0], &original[outer]);
                let after = location(candidate[inner][0], &candidate[outer]);
                if before == 0 || before != after {
                    return false;
                }
            }
        }
    }
    true
}

fn prepare_rings(
    raw: Vec<Vec<Point>>,
    scale: f64,
    scale_y: f64,
    options: &RasterOptions,
    budget: &mut usize,
) -> Result<(Vec<Vec<Point>>, Vec<&'static str>), String> {
    let original: Vec<_> = raw
        .into_iter()
        .map(|ring| {
            remove_collinear(&ring)
                .into_iter()
                .map(|p| Point::new(p.x * scale, p.y * scale_y))
                .collect::<Vec<_>>()
        })
        .collect();
    let simplified: Vec<_> = original
        .iter()
        .map(|ring| simplify_ring(ring, options.simplify_tolerance_mm))
        .collect();
    let mut notices = Vec::new();
    let acceptable = |candidate: &[Vec<Point>], budget: &mut usize| {
        topology_preserved(&original, candidate, budget)
            && original.iter().zip(candidate).all(|(before, after)| {
                // A 1% relative guard preserves small features; the additional
                // physical bound prevents scale-dependent loss on large art.
                let allowed = (area(before).abs() * 0.01)
                    .min(options.simplify_tolerance_mm * ring_perimeter(before));
                (area(before).abs() - area(after).abs()).abs() <= allowed + 1e-10
            })
            && boundary_within_tolerance(
                &original,
                candidate,
                options.simplify_tolerance_mm,
                budget,
            )
    };
    let mut rings = if simplified == original || acceptable(&simplified, budget) {
        simplified
    } else {
        notices.push("Raster simplification exceeded the topology, physical boundary-error or 1% area-change limit; retained the original boundary. Reduce tolerance or edit the region manually.");
        original.clone()
    };
    for _ in 0..options.smoothing_iterations {
        let candidate: Vec<_> = rings
            .iter()
            .map(|ring| corner_cut(ring, options.simplify_tolerance_mm))
            .collect();
        if candidate == rings {
            break;
        }
        if !acceptable(&candidate, budget) {
            notices.push("Raster smoothing exceeded the topology, physical boundary-error or 1% area-change limit; retained the last certified boundary and its sharp corners. Use fewer iterations or a smaller tolerance.");
            break;
        }
        rings = candidate;
    }
    Ok((rings, notices))
}

const MAX_RASTER_FILE_BYTES: u64 = 128_000_000;

fn read_raster_snapshot(path: &Path) -> Result<Vec<u8>, String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    if file.metadata().map_err(|e| e.to_string())?.len() > MAX_RASTER_FILE_BYTES {
        return Err("Raster file exceeds 128MB".into());
    }
    // Read from the same handle with a hard bound even if the file grows after
    // metadata was checked. Format, decoded pixels and embedded source all use
    // this immutable snapshot rather than reopening a mutable path.
    let mut encoded = Vec::new();
    file.take(MAX_RASTER_FILE_BYTES + 1)
        .read_to_end(&mut encoded)
        .map_err(|e| e.to_string())?;
    if encoded.len() as u64 > MAX_RASTER_FILE_BYTES {
        return Err("Raster file exceeds 128MB".into());
    }
    Ok(encoded)
}

fn decode_raster_snapshot(
    encoded: &[u8],
    max_alloc: u64,
) -> Result<(image::ImageFormat, image::DynamicImage), String> {
    let reader = || {
        image::ImageReader::new(std::io::Cursor::new(encoded))
            .with_guessed_format()
            .map_err(|e| e.to_string())
    };
    let format = reader()?.format();
    let Some(
        format @ (image::ImageFormat::Png | image::ImageFormat::Jpeg | image::ImageFormat::WebP),
    ) = format
    else {
        return Err("Raster artwork must be PNG, JPEG or WebP; the file contents do not match a supported image format".into());
    };
    let (source_width, source_height) = reader()?.into_dimensions().map_err(|e| e.to_string())?;
    validate_dimensions(source_width, source_height)?;
    let mut decoder = reader()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(16384);
    limits.max_image_height = Some(16384);
    limits.max_alloc = Some(max_alloc);
    decoder.limits(limits);
    let decoded = decoder.decode().map_err(|e| e.to_string())?;
    Ok((format, decoded))
}

/// Decode supported raster artwork by its signature, never its filename. The
/// GUI preview uses the same file/dimension checks as tracing and retains its
/// original 128MiB allocation budget.
pub fn decode_raster_file(path: &Path) -> Result<image::DynamicImage, String> {
    let encoded = read_raster_snapshot(path)?;
    decode_raster_snapshot(&encoded, 128 * 1024 * 1024).map(|(_, image)| image)
}

pub fn load_image_with_options(
    path: &Path,
    width_mm: f64,
    options: &RasterOptions,
) -> Result<Project, String> {
    super::validate_width(width_mm)?;
    options.validate()?;
    let encoded = read_raster_snapshot(path)?;
    let (format, decoded) = decode_raster_snapshot(&encoded, 256_000_000)?;
    let file_name = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let source = if matches!(format, image::ImageFormat::Png | image::ImageFormat::Jpeg)
        && encoded.len() <= crate::persistence::MAX_SOURCE_BYTES
    {
        crate::persistence::SourceImage::from_encoded(file_name, encoded, width_mm)?
    } else {
        // Existing ELAB readers support PNG/JPEG sources. Normalize WebP to
        // lossless PNG without changing decoded RGBA or source coordinates.
        crate::persistence::SourceImage::from_image(file_name, &decoded, width_mm)?
    };
    let (mut project, _) = trace_pixels_with_source_report(
        &decoded,
        path,
        width_mm,
        options,
        None,
        Some(source),
        || {
            crate::persistence::SourceImage::from_image(
                path.file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
                &decoded,
                width_mm,
            )
        },
    )?;
    crate::autodigitize::batch_fresh_raster_colors(&mut project, &AtomicBool::new(false))?;
    Ok(project)
}

/// Trace already-decoded artwork, such as one rendered PDF page. The source
/// name labels the project and embedded PNG; this function never reads it.
/// The encoded source is retained in ELAB, independently of the original file.
pub fn load_pixels_with_options(
    decoded: &image::DynamicImage,
    source_name: &Path,
    width_mm: f64,
    options: &RasterOptions,
) -> Result<Project, String> {
    let (mut project, _) = load_pixels_with_report(decoded, source_name, width_mm, options)?;
    crate::autodigitize::batch_fresh_raster_colors(&mut project, &AtomicBool::new(false))?;
    Ok(project)
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OmittedRegion {
    pub seed: [u32; 2],
    pub pixels: usize,
    pub area_mm2: f64,
    pub reason: String,
}

/// Omission locations are diagnostic samples, not an unbounded second raster.
pub const MAX_OMITTED_REGION_DETAILS: usize = 512;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RasterTraceReport {
    pub source_dimensions: [u32; 2],
    pub trace_dimensions: [u32; 2],
    pub source_nontransparent_pixels: usize,
    pub source_partial_alpha_pixels: usize,
    pub cleanup_excluded_source_pixels: usize,
    /// Existing 8-level-per-channel histogram bins used for palette learning;
    /// no globally blurred or white-composited raster is introduced.
    pub normalization_histogram_bins: usize,
    pub background_pixels: usize,
    pub foreground_pixels: usize,
    pub omitted_regions: Vec<OmittedRegion>,
    pub omitted_region_count: usize,
    pub omitted_pixel_count: usize,
    pub omitted_area_mm2: f64,
    pub omitted_region_details_complete: bool,
    pub palette: Vec<[u8; 3]>,
    /// Regions kept on canonical grid geometry to preserve shared borders.
    pub shared_boundary_regions: usize,
    pub relabeled_noise_pixels: usize,
    pub preparation_version: u32,
    pub components_before_preparation: usize,
    pub components_after_preparation: usize,
    pub contour_rings: usize,
    pub object_count: usize,
    pub preparation_changed_pixels: usize,
    pub preparation_changed_area_mm2: f64,
    pub preparation_uncertain_regions: usize,
    #[serde(default)]
    pub preparation_diagnostics: Box<PreparationDiagnostics>,
    /// Residual components kept because their pre-preparation source region
    /// already passed the user's thresholds. Their small new area is not an
    /// instruction to delete previously accepted source geometry.
    pub preparation_preserved_residual_regions: usize,
    /// Trace-grid palette indices (255 = excluded/transparent), ephemeral:
    /// never written into a project recipe or used as a replacement source.
    pub labels_before_preparation: Vec<u8>,
    pub labels_after_preparation: Vec<u8>,
}

impl Default for RasterTraceReport {
    fn default() -> Self {
        Self {
            source_dimensions: [0; 2],
            trace_dimensions: [0; 2],
            source_nontransparent_pixels: 0,
            source_partial_alpha_pixels: 0,
            cleanup_excluded_source_pixels: 0,
            normalization_histogram_bins: 0,
            background_pixels: 0,
            foreground_pixels: 0,
            omitted_regions: Vec::new(),
            omitted_region_count: 0,
            omitted_pixel_count: 0,
            omitted_area_mm2: 0.,
            omitted_region_details_complete: true,
            palette: Vec::new(),
            shared_boundary_regions: 0,
            relabeled_noise_pixels: 0,
            preparation_version: PREPARATION_VERSION,
            components_before_preparation: 0,
            components_after_preparation: 0,
            contour_rings: 0,
            object_count: 0,
            preparation_changed_pixels: 0,
            preparation_changed_area_mm2: 0.,
            preparation_uncertain_regions: 0,
            preparation_diagnostics: Box::default(),
            preparation_preserved_residual_regions: 0,
            labels_before_preparation: Vec::new(),
            labels_after_preparation: Vec::new(),
        }
    }
}

impl RasterTraceReport {
    fn record_omission(&mut self, seed: [u32; 2], pixels: usize, area_mm2: f64, reason: &str) {
        self.omitted_region_count += 1;
        self.omitted_pixel_count += pixels;
        self.omitted_area_mm2 += area_mm2;
        if self.omitted_regions.len() < MAX_OMITTED_REGION_DETAILS {
            self.omitted_regions.push(OmittedRegion {
                seed,
                pixels,
                area_mm2,
                reason: reason.into(),
            });
        } else {
            self.omitted_region_details_complete = false;
        }
    }
}

/// Tracing plus explicit accounting for every component excluded by thresholds.
pub fn load_pixels_with_report(
    decoded: &image::DynamicImage,
    source_name: &Path,
    width_mm: f64,
    options: &RasterOptions,
) -> Result<(Project, RasterTraceReport), String> {
    load_pixels_with_report_inner(decoded, source_name, width_mm, options, None)
}

pub fn load_pixels_with_report_cancellable(
    decoded: &image::DynamicImage,
    source_name: &Path,
    width_mm: f64,
    options: &RasterOptions,
    cancel: &AtomicBool,
) -> Result<(Project, RasterTraceReport), String> {
    load_pixels_with_report_inner(decoded, source_name, width_mm, options, Some(cancel))
}

/// Trace a worker's frozen pixel snapshot while preserving already-encoded
/// PNG/JPEG bytes. The caller must verify pixel correspondence before calling
/// this function (or derive `decoded` from these exact bytes); dimensions and
/// encoded-source validity are checked here without decoding the source again.
/// Physical metadata is rebased to the requested width, never resampled.
pub fn load_pixels_with_source_report_cancellable(
    decoded: &image::DynamicImage,
    source_name: &Path,
    width_mm: f64,
    options: &RasterOptions,
    cancel: &AtomicBool,
    mut source: crate::persistence::SourceImage,
) -> Result<(Project, RasterTraceReport), String> {
    check_cancel(Some(cancel))?;
    super::validate_width(width_mm)?;
    options.validate()?;
    validate_dimensions(decoded.width(), decoded.height())?;
    let dimensions = image::ImageReader::new(std::io::Cursor::new(source.encoded.as_slice()))
        .with_guessed_format()
        .map_err(|e| e.to_string())?
        .into_dimensions()
        .map_err(|e| e.to_string())?;
    if dimensions != (decoded.width(), decoded.height()) {
        return Err("Embedded source dimensions do not match the frozen import preview; reload the original artwork".into());
    }
    source.width_mm = width_mm;
    source.height_mm = width_mm * f64::from(decoded.height()) / f64::from(decoded.width());
    source.validate()?;
    trace_pixels_with_source_report(
        decoded,
        source_name,
        width_mm,
        options,
        Some(cancel),
        Some(source),
        || {
            crate::persistence::SourceImage::from_image(
                source_name
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
                decoded,
                width_mm,
            )
        },
    )
}

fn load_pixels_with_report_inner(
    decoded: &image::DynamicImage,
    source_name: &Path,
    width_mm: f64,
    options: &RasterOptions,
    cancel: Option<&AtomicBool>,
) -> Result<(Project, RasterTraceReport), String> {
    trace_pixels_with_source_report(
        decoded,
        source_name,
        width_mm,
        options,
        cancel,
        None,
        || {
            crate::persistence::SourceImage::from_image(
                source_name
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
                decoded,
                width_mm,
            )
        },
    )
}

fn validate_dimensions(width: u32, height: u32) -> Result<(), String> {
    if width == 0
        || height == 0
        || width > 16384
        || height > 16384
        || u64::from(width) * u64::from(height) > 32_000_000
    {
        Err("Raster dimensions exceed 16384px per axis or 32 million source pixels".into())
    } else {
        Ok(())
    }
}

fn trace_pixels_with_source_report(
    decoded: &image::DynamicImage,
    source_name: &Path,
    width_mm: f64,
    options: &RasterOptions,
    cancel: Option<&AtomicBool>,
    provided_source: Option<crate::persistence::SourceImage>,
    source: impl FnOnce() -> Result<crate::persistence::SourceImage, String>,
) -> Result<(Project, RasterTraceReport), String> {
    check_cancel(cancel)?;
    super::validate_width(width_mm)?;
    options.validate()?;
    validate_dimensions(decoded.width(), decoded.height())?;
    let rgba = decoded.to_rgba8();
    let (source_nontransparent_pixels, source_partial_alpha_pixels) =
        rgba.pixels()
            .fold((0_usize, 0_usize), |(visible, partial), p| {
                (
                    visible + usize::from(p[3] > 0),
                    partial + usize::from(p[3] > 0 && p[3] < 255),
                )
            });
    // Apply the full source-coordinate mask BEFORE resampling; excluded RGB
    // never participates in palette statistics or premultiplied interpolation.
    let cleaned = if let Some(cleanup) = &options.cleanup {
        cleanup.apply(&rgba)?
    } else {
        rgba
    };
    let cleanup_excluded_source_pixels = if options.cleanup.is_some() {
        source_nontransparent_pixels - cleaned.pixels().filter(|p| p[3] > 0).count()
    } else {
        0
    };
    let image = color_selection::thumbnail_rgba(&cleaned, options.max_dimension);
    check_cancel(cancel)?;
    let (w, h) = (image.width() as usize, image.height() as usize);
    let mut background = background_mask(&image, &options.background)?;
    let mut trace_protected = vec![false; w * h];
    if let Some(cleanup) = &options.cleanup {
        let (_, protected) = cleanup.to_masks()?;
        // A nearest source sample can miss a one-pixel protected mark during
        // downsampling. Map EVERY protected source sample into its trace cell.
        for (index, &is_protected) in protected.iter().enumerate() {
            if index % 4096 == 0 {
                check_cancel(cancel)?;
            }
            if !is_protected || cleaned.as_raw()[index * 4 + 3] == 0 {
                continue;
            }
            let x = (index % cleanup.width as usize) * w / cleanup.width as usize;
            let y = (index / cleanup.width as usize) * h / cleanup.height as usize;
            let trace = y * w + x;
            if image.as_raw()[trace * 4 + 3] < 128 {
                return Err("A protected detail disappears at the selected trace resolution; increase max_dimension before tracing. The existing project and source mask are retained.".into());
            }
            let source_rgb = std::array::from_fn(|c| cleaned.as_raw()[index * 4 + c]);
            let trace_rgb = std::array::from_fn(|c| image.as_raw()[trace * 4 + c]);
            if squared_distance(srgb_to_oklab(source_rgb), srgb_to_oklab(trace_rgb))
                > 0.04_f64.powi(2)
            {
                return Err("A protected detail changes colour during downsampling; increase max_dimension before tracing. The existing project and source mask are retained.".into());
            }
            background[trace] = false;
            trace_protected[trace] = true;
        }
    }
    let mut histogram: BTreeMap<[u8; 3], Bin> = BTreeMap::new();
    let mut foreground = 0;
    for (index, (pixel, &masked)) in image.pixels().zip(&background).enumerate() {
        if index % 4096 == 0 {
            check_cancel(cancel)?;
        }
        if !masked {
            let rgb = [pixel[0], pixel[1], pixel[2]];
            histogram.entry(bucket(rgb)).or_default().add(rgb);
            foreground += 1;
        }
    }
    if histogram.is_empty() {
        return Err("No foreground pixels remain after the selected background mask".into());
    }
    let palette = palette_from_histogram(
        &histogram,
        options.palette_size,
        foreground,
        cancel,
        &options.pinned_colors,
    )?;
    let palette_lab: Vec<_> = palette.iter().copied().map(srgb_f64_to_oklab).collect();
    let mut labels: Vec<u8> = image
        .pixels()
        .zip(&background)
        .enumerate()
        .map(|(index, (pixel, &masked))| {
            if index % 4096 == 0 {
                check_cancel(cancel)?;
            }
            Ok(if masked {
                255
            } else {
                nearest_lab(srgb_to_oklab([pixel[0], pixel[1], pixel[2]]), &palette_lab) as u8
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let relabeled_noise_pixels =
        denoise_labels(&mut labels, w, h, options.denoise_passes, &trace_protected);
    let scale = width_mm / w as f64;
    let scale_y = width_mm * f64::from(decoded.height()) / f64::from(decoded.width()) / h as f64;
    let labels_before_preparation = labels.clone();
    let retained_provenance = if options.preparation == PreparationMode::Legacy {
        None
    } else {
        Some(preparation::retained_provenance(
            &labels,
            &trace_protected,
            [w, h],
            [scale, scale_y],
            (options.min_region_pixels, options.min_region_area_mm2),
            cancel,
        )?)
    };
    let prepared = preparation::prepare(
        &mut labels,
        &image,
        &palette_lab,
        &trace_protected,
        options.pinned_colors.len(),
        [scale, scale_y],
        options.preparation,
        cancel,
    )?;
    let mut visited = vec![false; w * h];
    let mut member = vec![false; w * h];
    let mut queue = VecDeque::new();
    let mut objects = Vec::new();
    let mut contour_notices = Vec::new();
    let mut report = RasterTraceReport {
        source_dimensions: [decoded.width(), decoded.height()],
        trace_dimensions: [image.width(), image.height()],
        source_nontransparent_pixels,
        source_partial_alpha_pixels,
        cleanup_excluded_source_pixels,
        normalization_histogram_bins: histogram.len(),
        background_pixels: w * h - foreground,
        foreground_pixels: foreground,
        relabeled_noise_pixels,
        components_before_preparation: prepared.before,
        components_after_preparation: prepared.after,
        preparation_changed_pixels: prepared.changed,
        preparation_changed_area_mm2: prepared.changed as f64 * scale * scale_y,
        preparation_uncertain_regions: prepared.uncertain,
        preparation_version: options.preparation.version(),
        preparation_diagnostics: Box::new(prepared.diagnostics),
        labels_before_preparation,
        labels_after_preparation: labels.clone(),
        palette: palette
            .iter()
            .map(|rgb| rgb.map(|v| v.round() as u8))
            .collect(),
        ..RasterTraceReport::default()
    };
    let mut node_count = 0;
    let mut topology_budget = 30_000_000;
    for seed in 0..w * h {
        if seed % 1024 == 0 {
            check_cancel(cancel)?;
        }
        if visited[seed] || labels[seed] == 255 {
            continue;
        }
        let label = labels[seed];
        queue.push_back(seed);
        visited[seed] = true;
        let mut pixels = Vec::new();
        while let Some(index) = queue.pop_front() {
            if pixels.len() % 4096 == 0 {
                check_cancel(cancel)?;
            }
            let (x, y) = (index % w, index / w);
            pixels.push((x, y));
            for neighbor in [
                x.checked_sub(1).map(|nx| y * w + nx),
                (x + 1 < w).then_some(index + 1),
                y.checked_sub(1).map(|ny| ny * w + x),
                (y + 1 < h).then_some(index + w),
            ]
            .into_iter()
            .flatten()
            {
                if !visited[neighbor] && labels[neighbor] == label {
                    visited[neighbor] = true;
                    queue.push_back(neighbor);
                }
            }
        }
        let area_mm2 = pixels.len() as f64 * scale * scale_y;
        let below_floor = pixels.len() < options.min_region_pixels
            || area_mm2 + 1e-12 < options.min_region_area_mm2;
        let previously_retained = retained_provenance
            .as_ref()
            .is_some_and(|mask| pixels.iter().any(|&(x, y)| mask[y * w + x]));
        if below_floor && previously_retained {
            report.preparation_preserved_residual_regions += 1;
        }
        if below_floor
            && !previously_retained
            && !pixels.iter().any(|&(x, y)| trace_protected[y * w + x])
        {
            report.record_omission(
                [(seed % w) as u32, (seed / w) as u32],
                pixels.len(),
                area_mm2,
                if area_mm2 + 1e-12 < options.min_region_area_mm2 {
                    "Below minimum physical region area"
                } else {
                    "Below legacy minimum pixel region"
                },
            );
            continue;
        }
        let shared = options.preserve_shared_boundaries
            && pixels.iter().any(|&(x, y)| {
                [
                    x.checked_sub(1).map(|nx| y * w + nx),
                    (x + 1 < w).then_some(y * w + x + 1),
                    y.checked_sub(1).map(|ny| ny * w + x),
                    (y + 1 < h).then_some((y + 1) * w + x),
                ]
                .into_iter()
                .flatten()
                .any(|n| labels[n] != 255 && labels[n] != label)
            });
        for &(x, y) in &pixels {
            member[y * w + x] = true;
        }
        let mut boundary_edges = 0;
        for &(x, y) in &pixels {
            boundary_edges += usize::from(x == 0 || !member[y * w + x - 1])
                + usize::from(x + 1 == w || !member[y * w + x + 1])
                + usize::from(y == 0 || !member[(y - 1) * w + x])
                + usize::from(y + 1 == h || !member[(y + 1) * w + x]);
            if boundary_edges > 200_000 {
                return Err("A raster region exceeds 200,000 boundary edges; reduce resolution or simplify artwork".into());
            }
        }
        let raw = trace_region(&pixels, &member, w, h);
        report.contour_rings += raw.len();
        for &(x, y) in &pixels {
            member[y * w + x] = false;
        }
        if raw.is_empty() {
            if previously_retained {
                return Err("Prepared geometry could not retain a previously accepted source component boundary; the existing project is retained. Use Legacy preparation or increase trace resolution.".into());
            }
            report.record_omission(
                [(seed % w) as u32, (seed / w) as u32],
                pixels.len(),
                area_mm2,
                "Tracer produced no closed boundary",
            );
            continue;
        }
        let mut region_options = options.clone();
        if shared {
            // The label grid is the common boundary graph. Both incident
            // regions reuse its exact edges, with collinear compression only.
            region_options.simplify_tolerance_mm = 0.;
            region_options.smoothing_iterations = 0;
            report.shared_boundary_regions += 1;
        }
        let (rings, notices) =
            prepare_rings(raw, scale, scale_y, &region_options, &mut topology_budget)?;
        node_count += rings.iter().map(Vec::len).sum::<usize>();
        if node_count > 100_000 {
            return Err("Raster tracing exceeds 100,000 editable nodes; increase simplification or reduce resolution".into());
        }
        if objects.len() >= 4096 {
            return Err("Raster tracing exceeds 4096 regions; increase minimum region size or reduce palette size".into());
        }
        let id = objects.len() as u64 + 1;
        contour_notices.extend(notices.into_iter().map(|message| PlanWarning {
            object_id: id,
            point: rings.iter().flatten().copied().next(),
            message: message.into(),
        }));
        objects.push(Object {
            id,
            name: format!("Region {id}"),
            contours: rings,
            color: palette[usize::from(label)].map(|value| value.round() as u8),
            ..Object::default()
        });
    }
    if objects.is_empty() {
        return Err("No foreground region meets min_region_pixels; reduce that threshold or increase resolution".into());
    }
    // Compute the aggregate from integer pixel totals once, avoiding accumulated
    // rounding drift when millions of tiny components use the same pixel area.
    report.omitted_area_mm2 = report.omitted_pixel_count as f64 * scale * scale_y;
    report.object_count = objects.len();
    if options.preparation == PreparationMode::Legacy {
        report.components_before_preparation = objects.len() + report.omitted_region_count;
        report.components_after_preparation = report.components_before_preparation;
    }
    // Preserve spatial/component insertion order. Global RGB sorting would
    // silently become a sewing-order decision unrelated to artwork layers.
    let mut project = Project {
        version: 5,
        name: name(source_name),
        objects,
        width_mm,
        source_image: Some(match provided_source {
            Some(source) => source,
            None => source()?,
        }),
        raster_recipe: None,
        machine_profile: Default::default(),
        layering: Default::default(),
    };
    let mut recipe = crate::persistence::RasterRecipe::capture(&project, options.clone());
    recipe.issues = report
        .omitted_regions
        .iter()
        .map(|region| PlanWarning {
            object_id: 0,
            point: Some(Point::new(
                f64::from(region.seed[0]) * scale,
                f64::from(region.seed[1]) * scale_y,
            )),
            message: format!(
                "{}: {} trace-map pixels ({:.4} mm²)",
                region.reason, region.pixels, region.area_mm2
            ),
        })
        .collect();
    recipe.issues.extend(contour_notices);
    if options.preparation != PreparationMode::Legacy {
        recipe.issues.push(PlanWarning {
            object_id: 0,
            point: None,
            message: format!(
                "Experimental boundary preparation v{}: reassigned {} pixels ({:.4} mm²), {} → {} connected components; {} ambiguous narrow regions retained; {} residual regions preserved by original threshold provenance. Review the changed-area overlay and perform a physical sew-out.",
                options.preparation.version(),
                prepared.changed, report.preparation_changed_area_mm2,
                prepared.before, prepared.after, prepared.uncertain,
                report.preparation_preserved_residual_regions
            ),
        });
    }
    if !report.omitted_region_details_complete {
        let count = report.omitted_region_count;
        let pixels = report.omitted_pixel_count;
        let area = report.omitted_area_mm2;
        let shown = report.omitted_regions.len();
        let aggregated = count - shown;
        recipe.issues.push(PlanWarning {object_id:0,point:None,message:format!(
            "Raster import omitted {count} regions ({pixels} trace-map pixels, {area:.4} mm²). Showing the first {shown} locations; {aggregated} further locations are aggregated. Review minimum-area/pixel thresholds and tracing diagnostics.")});
    }
    if relabeled_noise_pixels > 0 {
        recipe.issues.push(PlanWarning {object_id:0,point:None,message:format!(
            "Explicit isolated-pixel denoise reassigned {relabeled_noise_pixels} trace-map pixels ({:.4} mm²); protected pixels and transparent openings were retained",
            relabeled_noise_pixels as f64*scale*scale_y)});
    }
    project.raster_recipe = Some(recipe);
    project.validate_limits()?;
    check_cancel(cancel)?;
    Ok((project, report))
}

fn check_cancel(cancel: Option<&AtomicBool>) -> Result<(), String> {
    if cancel.is_some_and(|cancel| cancel.load(Ordering::Relaxed)) {
        Err("Raster tracing cancelled".into())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod source_preservation_tests {
    use super::*;

    #[test]
    fn provided_jpeg_bypasses_a_lossless_png_embedding_budget_failure() {
        // A small configurable budget reproduces a JPEG fitting the encoded
        // source limit while the equivalent lossless PNG does not. The real
        // production limit remains MAX_SOURCE_BYTES (12MB).
        let pixels = image::RgbImage::from_fn(64, 48, |x, y| {
            image::Rgb([
                (x.wrapping_mul(73).wrapping_add(y.wrapping_mul(29))) as u8,
                (x.wrapping_mul(13).wrapping_add(y.wrapping_mul(97))) as u8,
                (x.wrapping_mul(151).wrapping_add(y.wrapping_mul(43))) as u8,
            ])
        });
        let mut jpeg = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 8)
            .encode_image(&image::DynamicImage::ImageRgb8(pixels))
            .unwrap();
        let source =
            crate::persistence::SourceImage::from_encoded("noise.jpg".into(), jpeg.clone(), 64.)
                .unwrap();
        let decoded = source.decode().unwrap();
        let mut png = std::io::Cursor::new(Vec::new());
        decoded.write_to(&mut png, image::ImageFormat::Png).unwrap();
        let test_budget = jpeg.len();
        assert!(
            png.get_ref().len() > test_budget,
            "fixture must exercise the JPEG/PNG size difference"
        );
        let fallback_called = std::cell::Cell::new(false);
        let options = RasterOptions {
            max_dimension: 64,
            palette_size: 2,
            min_region_pixels: 1,
            background: BackgroundMode::TransparentOnly,
            ..RasterOptions::default()
        };
        let (project, _) = trace_pixels_with_source_report(
            &decoded,
            Path::new("noise.jpg"),
            64.,
            &options,
            None,
            Some(source),
            || {
                fallback_called.set(true);
                if png.get_ref().len() > test_budget {
                    Err("Lossless PNG exceeds the configured test embedding budget".into())
                } else {
                    crate::persistence::SourceImage::from_image("noise.jpg".into(), &decoded, 64.)
                }
            },
        )
        .unwrap();
        assert!(
            !fallback_called.get(),
            "the PNG encoder must not run before replacing source bytes"
        );
        assert_eq!(project.source_image.unwrap().encoded, jpeg);
    }
}
