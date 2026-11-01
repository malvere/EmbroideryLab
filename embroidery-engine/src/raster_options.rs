//! Configurable flat-colour raster tracing. No Embird implementation is used.
use super::{name, nearest, rgb_distance, simplify_ring, trace_region};
use crate::model::*;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, VecDeque},
    path::Path,
};

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
        }
    }
}
impl RasterOptions {
    pub fn validate(&self) -> Result<(), String> {
        if !(16..=2048).contains(&self.max_dimension) {
            return Err("Raster max_dimension must be 16–2048 pixels".into());
        }
        if !(2..=32).contains(&self.palette_size) {
            return Err("Raster palette_size must be 2–32 colours".into());
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
) -> Result<Vec<[f64; 3]>, String> {
    let bins: Vec<_> = histogram
        .values()
        .map(|bin| (bin.center(), bin.count))
        .collect();
    let size = requested.min(bins.len());
    let work = (bins.len() as u64) * (size as u64) * 12 + (pixels as u64) * (size as u64);
    if work > 150_000_000 {
        return Err("Raster palette work exceeds budget; reduce resolution or colour count".into());
    }
    let mut palette = Vec::new();
    for _ in 0..size {
        let (chosen, _) = bins
            .iter()
            .max_by(|(a, na), (b, nb)| {
                let score = |color: [f64; 3], count: u64| {
                    let distance = if palette.is_empty() {
                        1.0
                    } else {
                        rgb_distance(color, palette[nearest(color, &palette)])
                    };
                    distance * (count as f64).sqrt()
                };
                score(*a, *na).total_cmp(&score(*b, *nb))
            })
            .ok_or("No foreground pixels for palette")?;
        if palette
            .iter()
            .any(|color| rgb_distance(*color, *chosen) < 1e-9)
        {
            break;
        }
        palette.push(*chosen);
    }
    for _ in 0..8 {
        let mut sums = vec![[0.; 3]; palette.len()];
        let mut counts = vec![0_u64; palette.len()];
        for &(color, count) in &bins {
            let index = nearest(color, &palette);
            for (sum, value) in sums[index].iter_mut().zip(color) {
                *sum += value * (count as f64);
            }
            counts[index] += count;
        }
        for (index, count) in counts.into_iter().enumerate() {
            if count > 0 {
                palette[index] = sums[index].map(|sum| sum / count as f64);
            }
        }
    }
    Ok(palette)
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
fn corner_cut(ring: &[Point]) -> Vec<Point> {
    let mut out = Vec::with_capacity(ring.len() * 2);
    for (&a, &b) in ring
        .iter()
        .zip(ring.iter().cycle().skip(1))
        .take(ring.len())
    {
        out.push(a.lerp(b, 0.25));
        out.push(a.lerp(b, 0.75));
    }
    out
}

/// Certify candidate rings with bounded exact segment checks. A candidate with
/// crossings, touching rings, changed nesting/orientation, or >50% area loss is
/// rejected. Uncertifiable simplification keeps the original traced boundary;
/// explicitly requested smoothing reports an error instead.
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
            || area(ring).abs() < area(before).abs() * 0.5
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
    options: &RasterOptions,
    budget: &mut usize,
) -> Result<Vec<Vec<Point>>, String> {
    let original: Vec<_> = raw
        .into_iter()
        .map(|ring| {
            remove_collinear(&ring)
                .into_iter()
                .map(|p| Point::new(p.x * scale, p.y * scale))
                .collect::<Vec<_>>()
        })
        .collect();
    let simplified: Vec<_> = original
        .iter()
        .map(|ring| simplify_ring(ring, options.simplify_tolerance_mm))
        .collect();
    let mut rings = if simplified == original || topology_preserved(&original, &simplified, budget)
    {
        simplified
    } else {
        original.clone()
    };
    for _ in 0..options.smoothing_iterations {
        let candidate: Vec<_> = rings.iter().map(|ring| corner_cut(ring)).collect();
        if !topology_preserved(&original, &candidate, budget) {
            return Err("Requested raster smoothing could not preserve contour topology within the geometry budget; use fewer iterations or zero smoothing".into());
        }
        rings = candidate;
    }
    Ok(rings)
}

pub fn load_image_with_options(
    path: &Path,
    width_mm: f64,
    options: &RasterOptions,
) -> Result<Project, String> {
    super::validate_width(width_mm)?;
    options.validate()?;
    if std::fs::metadata(path).map_err(|e| e.to_string())?.len() > 128_000_000 {
        return Err("Raster file exceeds 128MB".into());
    }
    let reader = || {
        image::ImageReader::open(path)
            .map_err(|e| e.to_string())?
            .with_guessed_format()
            .map_err(|e| e.to_string())
    };
    let (source_width, source_height) = reader()?.into_dimensions().map_err(|e| e.to_string())?;
    if source_width == 0
        || source_height == 0
        || source_width > 16384
        || source_height > 16384
        || u64::from(source_width) * u64::from(source_height) > 32_000_000
    {
        return Err("Raster dimensions exceed 16384px per axis or 32 million source pixels".into());
    }
    let mut decoder = reader()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(16384);
    limits.max_image_height = Some(16384);
    limits.max_alloc = Some(256_000_000);
    decoder.limits(limits);
    let decoded = decoder.decode().map_err(|e| e.to_string())?;
    let image =
        if decoded.width() > options.max_dimension || decoded.height() > options.max_dimension {
            decoded
                .thumbnail(options.max_dimension, options.max_dimension)
                .to_rgba8()
        } else {
            decoded.to_rgba8()
        };
    let (w, h) = (image.width() as usize, image.height() as usize);
    let background = background_mask(&image, &options.background)?;
    let mut histogram: BTreeMap<[u8; 3], Bin> = BTreeMap::new();
    let mut foreground = 0;
    for (pixel, &masked) in image.pixels().zip(&background) {
        if !masked {
            let rgb = [pixel[0], pixel[1], pixel[2]];
            histogram.entry(bucket(rgb)).or_default().add(rgb);
            foreground += 1;
        }
    }
    if histogram.is_empty() {
        return Err("No foreground pixels remain after the selected background mask".into());
    }
    let palette = palette_from_histogram(&histogram, options.palette_size, foreground)?;
    let labels: Vec<u8> = image
        .pixels()
        .zip(&background)
        .map(|(pixel, &masked)| {
            if masked {
                255
            } else {
                nearest([pixel[0], pixel[1], pixel[2]].map(f64::from), &palette) as u8
            }
        })
        .collect();
    let mut visited = vec![false; w * h];
    let mut member = vec![false; w * h];
    let mut queue = VecDeque::new();
    let mut objects = Vec::new();
    let scale = width_mm / w as f64;
    let mut node_count = 0;
    let mut topology_budget = 30_000_000;
    for seed in 0..w * h {
        if visited[seed] || labels[seed] == 255 {
            continue;
        }
        let label = labels[seed];
        queue.push_back(seed);
        visited[seed] = true;
        let mut pixels = Vec::new();
        while let Some(index) = queue.pop_front() {
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
        if pixels.len() < options.min_region_pixels {
            continue;
        }
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
        for &(x, y) in &pixels {
            member[y * w + x] = false;
        }
        if raw.is_empty() {
            continue;
        }
        let rings = prepare_rings(raw, scale, options, &mut topology_budget)?;
        node_count += rings.iter().map(Vec::len).sum::<usize>();
        if node_count > 100_000 {
            return Err("Raster tracing exceeds 100,000 editable nodes; increase simplification or reduce resolution".into());
        }
        if objects.len() >= 4096 {
            return Err("Raster tracing exceeds 4096 regions; increase minimum region size or reduce palette size".into());
        }
        let id = objects.len() as u64 + 1;
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
    objects.sort_by_key(|object| object.color);
    let project = Project {
        version: 4,
        name: name(path),
        objects,
        width_mm,
        source_image: Some(
            crate::persistence::SourceImage::from_path(path, width_mm).or_else(|_| {
                crate::persistence::SourceImage::from_image(
                    path.file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned(),
                    &decoded,
                    width_mm,
                )
            })?,
        ),
    };
    project.validate_limits()?;
    Ok(project)
}
