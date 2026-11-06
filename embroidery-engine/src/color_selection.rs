//! Non-destructive raster selection in immutable source-image coordinates.
//!
//! Similarity is Euclidean OKLab distance multiplied by 100 (not CIE DeltaE).
//! sRGB is linearized before conversion. Connectivity is four-neighbour.
//! A gesture caches distances to one fixed colour, and recomputes each mask
//! against its frozen mouse-down selection; returning the pointer shrinks it.
use image::RgbaImage;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SelectionMode {
    #[default]
    Connected,
    Exterior,
    Global,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SelectionOp {
    #[default]
    Replace,
    Add,
    Subtract,
}

pub fn srgb_to_oklab(rgb: [u8; 3]) -> [f64; 3] {
    srgb_f64_to_oklab(rgb.map(f64::from))
}

/// Input channels have the ordinary sRGB 0–255 scale.
pub fn srgb_f64_to_oklab(rgb: [f64; 3]) -> [f64; 3] {
    let [r, g, b] = rgb.map(|v| {
        let v = (v / 255.).clamp(0., 1.);
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    });
    let l = (0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b).cbrt();
    let m = (0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b).cbrt();
    let s = (0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b).cbrt();
    [
        0.2104542553 * l + 0.7936177850 * m - 0.0040720468 * s,
        1.9779984951 * l - 2.4285922050 * m + 0.4505937099 * s,
        0.0259040371 * l + 0.7827717662 * m - 0.8086757660 * s,
    ]
}

pub fn oklab_to_srgb(lab: [f64; 3]) -> [u8; 3] {
    let [l, a, b] = lab;
    let ll = (l + 0.3963377774 * a + 0.2158037573 * b).powi(3);
    let mm = (l - 0.1055613458 * a - 0.0638541728 * b).powi(3);
    let ss = (l - 0.0894841775 * a - 1.2914855480 * b).powi(3);
    [
        4.0767416621 * ll - 3.3077115913 * mm + 0.2309699292 * ss,
        -1.2684380046 * ll + 2.6097574011 * mm - 0.3413193965 * ss,
        -0.0041960863 * ll - 0.7034186147 * mm + 1.7076147010 * ss,
    ]
    .map(|v| {
        let v = v.clamp(0., 1.);
        let srgb = if v <= 0.0031308 {
            12.92 * v
        } else {
            1.055 * v.powf(1. / 2.4) - 0.055
        };
        (srgb * 255.).round().clamp(0., 255.) as u8
    })
}

pub fn squared_distance(a: [f64; 3], b: [f64; 3]) -> f64 {
    a.into_iter().zip(b).map(|(x, y)| (x - y).powi(2)).sum()
}

/// Logical egui points only: neither canvas zoom nor physical display pixels
/// enter this calculation. A numeric slider can go below the initial value.
pub fn gesture_tolerance(initial: f32, logical_distance: f32) -> f32 {
    (initial + 0.12 * (logical_distance - 3.).max(0.)).clamp(0., 100.)
}

#[derive(Clone, Debug)]
pub struct SelectionCache {
    pub width: u32,
    pub height: u32,
    pub seed: [u32; 2],
    pub seed_rgb: [u8; 3],
    distances: Vec<f32>,
}

impl SelectionCache {
    pub fn new(image: &RgbaImage, seed: [u32; 2]) -> Result<Self, String> {
        Self::new_internal(image, seed, None)
    }

    pub fn new_cancellable(
        image: &RgbaImage,
        seed: [u32; 2],
        cancel: &AtomicBool,
    ) -> Result<Self, String> {
        Self::new_internal(image, seed, Some(cancel))
    }

    fn new_internal(
        image: &RgbaImage,
        seed: [u32; 2],
        cancel: Option<&AtomicBool>,
    ) -> Result<Self, String> {
        check_cancel(cancel)?;
        validate_dimensions(image.width(), image.height())?;
        if seed[0] >= image.width() || seed[1] >= image.height() {
            return Err("Selection seed is outside the source image".into());
        }
        let pixel = image.get_pixel(seed[0], seed[1]);
        if pixel[3] == 0 {
            return Err(
                "Choose an opaque source pixel; fully transparent RGB has no colour meaning".into(),
            );
        }
        let seed_rgb = [pixel[0], pixel[1], pixel[2]];
        let lab = srgb_to_oklab(seed_rgb);
        let distances = image
            .pixels()
            .enumerate()
            .map(|(i, p)| {
                if i % 4096 == 0 {
                    check_cancel(cancel)?;
                }
                Ok(if p[3] == 0 {
                    0.
                } else {
                    (squared_distance(lab, srgb_to_oklab([p[0], p[1], p[2]])).sqrt() * 100.) as f32
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        Ok(Self {
            width: image.width(),
            height: image.height(),
            seed,
            seed_rgb,
            distances,
        })
    }

    pub fn candidates(&self, tolerance: f32, mode: SelectionMode) -> Vec<bool> {
        self.candidates_with_protection(tolerance, mode, &[])
    }

    pub fn candidates_with_protection(
        &self,
        tolerance: f32,
        mode: SelectionMode,
        protected: &[bool],
    ) -> Vec<bool> {
        // The uncancelled path cannot return a cancellation error.
        self.candidates_internal(tolerance, mode, protected, None)
            .unwrap_or_default()
    }

    pub fn candidates_with_protection_cancellable(
        &self,
        tolerance: f32,
        mode: SelectionMode,
        protected: &[bool],
        cancel: &AtomicBool,
    ) -> Result<Vec<bool>, String> {
        self.candidates_internal(tolerance, mode, protected, Some(cancel))
    }

    fn candidates_internal(
        &self,
        tolerance: f32,
        mode: SelectionMode,
        protected: &[bool],
        cancel: Option<&AtomicBool>,
    ) -> Result<Vec<bool>, String> {
        check_cancel(cancel)?;
        let tolerance = if tolerance.is_finite() {
            tolerance.clamp(0., 100.)
        } else {
            0.
        };
        let eligible: Vec<_> = self
            .distances
            .iter()
            .enumerate()
            .map(|(i, &distance)| {
                if i % 4096 == 0 {
                    check_cancel(cancel)?;
                }
                Ok(distance <= tolerance && !protected.get(i).copied().unwrap_or(false))
            })
            .collect::<Result<Vec<_>, String>>()?;
        if mode == SelectionMode::Global {
            return Ok(eligible);
        }
        let w = self.width as usize;
        let h = self.height as usize;
        let mut selected = vec![false; eligible.len()];
        let mut pending = VecDeque::new();
        let mut push = |index: usize| {
            if eligible[index] && !selected[index] {
                selected[index] = true;
                pending.push_back(index);
            }
        };
        match mode {
            SelectionMode::Connected => push(self.seed[1] as usize * w + self.seed[0] as usize),
            SelectionMode::Exterior => {
                for x in 0..w {
                    push(x);
                    push((h - 1) * w + x);
                }
                for y in 0..h {
                    push(y * w);
                    push(y * w + w - 1);
                }
            }
            SelectionMode::Global => unreachable!(),
        }
        let mut visits = 0;
        while let Some(index) = pending.pop_front() {
            if visits % 4096 == 0 {
                check_cancel(cancel)?;
            }
            visits += 1;
            for neighbor in neighbors(index, w, h).into_iter().flatten() {
                if eligible[neighbor] && !selected[neighbor] {
                    selected[neighbor] = true;
                    pending.push_back(neighbor);
                }
            }
        }
        Ok(selected)
    }
}

fn check_cancel(cancel: Option<&AtomicBool>) -> Result<(), String> {
    if cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
        Err("Colour selection cancelled".into())
    } else {
        Ok(())
    }
}

fn neighbors(index: usize, w: usize, h: usize) -> [Option<usize>; 4] {
    let (x, y) = (index % w, index / w);
    [
        x.checked_sub(1).map(|_| index - 1),
        (x + 1 < w).then_some(index + 1),
        y.checked_sub(1).map(|_| index - w),
        (y + 1 < h).then_some(index + w),
    ]
}

pub fn combine_selection(
    base: &[bool],
    candidates: &[bool],
    operation: SelectionOp,
) -> Result<Vec<bool>, String> {
    if base.len() != candidates.len() {
        return Err("Selection mask dimensions differ".into());
    }
    Ok(base
        .iter()
        .zip(candidates)
        .map(|(&a, &b)| match operation {
            SelectionOp::Replace => b,
            SelectionOp::Add => a || b,
            SelectionOp::Subtract => a && !b,
        })
        .collect())
}

/// Source-coordinate run lengths, each [first_pixel, length], row-major.
/// Explicit protection always wins over exclusion; original RGBA is untouched.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CleanupMask {
    pub width: u32,
    pub height: u32,
    pub excluded_runs: Vec<[u32; 2]>,
    pub protected_runs: Vec<[u32; 2]>,
}

impl CleanupMask {
    pub fn from_masks(
        width: u32,
        height: u32,
        excluded: &[bool],
        protected: &[bool],
    ) -> Result<Self, String> {
        validate_dimensions(width, height)?;
        let pixels = width as usize * height as usize;
        if excluded.len() != pixels || protected.len() != pixels {
            return Err("Cleanup mask dimensions differ from source".into());
        }
        Ok(Self {
            width,
            height,
            excluded_runs: encode_runs(excluded),
            protected_runs: encode_runs(protected),
        })
    }

    pub fn validate(&self) -> Result<(), String> {
        validate_dimensions(self.width, self.height)?;
        let pixels = u64::from(self.width) * u64::from(self.height);
        for runs in [&self.excluded_runs, &self.protected_runs] {
            if runs.len() > 1_000_000 {
                return Err("Cleanup mask exceeds one million runs".into());
            }
            let mut end = 0_u64;
            for &[first, length] in runs {
                let next = u64::from(first) + u64::from(length);
                if length == 0 || u64::from(first) < end || next > pixels {
                    return Err("Cleanup mask contains invalid or overlapping runs".into());
                }
                end = next;
            }
        }
        Ok(())
    }

    pub fn to_masks(&self) -> Result<(Vec<bool>, Vec<bool>), String> {
        self.validate()?;
        let n = self.width as usize * self.height as usize;
        let decode = |runs: &[[u32; 2]]| {
            let mut out = vec![false; n];
            for &[first, length] in runs {
                out[first as usize..(first + length) as usize].fill(true);
            }
            out
        };
        Ok((decode(&self.excluded_runs), decode(&self.protected_runs)))
    }

    pub fn apply(&self, source: &RgbaImage) -> Result<RgbaImage, String> {
        if source.dimensions() != (self.width, self.height) {
            return Err("Cleanup mask dimensions differ from saved source; reselect rather than silently rescale".into());
        }
        let (excluded, protected) = self.to_masks()?;
        let mut image = source.clone();
        for ((pixel, excluded), protected) in image.pixels_mut().zip(excluded).zip(protected) {
            if excluded && !protected {
                pixel[3] = 0;
            }
        }
        Ok(image)
    }
}

fn encode_runs(mask: &[bool]) -> Vec<[u32; 2]> {
    let mut runs = Vec::new();
    let mut i = 0;
    while i < mask.len() {
        if !mask[i] {
            i += 1;
            continue;
        }
        let first = i;
        while i < mask.len() && mask[i] {
            i += 1;
        }
        runs.push([first as u32, (i - first) as u32]);
    }
    runs
}

/// Small source-coordinate correction brush for protection or restoration.
/// The GUI records one complete drag as one undo operation.
pub fn paint_mask(
    mask: &mut [bool],
    width: u32,
    height: u32,
    center: [f32; 2],
    radius: f32,
    value: bool,
) -> Result<(), String> {
    validate_dimensions(width, height)?;
    if mask.len() != width as usize * height as usize
        || !radius.is_finite()
        || radius <= 0.
        || !center.into_iter().all(f32::is_finite)
    {
        return Err("Invalid protection brush dimensions or radius".into());
    }
    let x0 = (center[0] - radius).floor().max(0.) as u32;
    let y0 = (center[1] - radius).floor().max(0.) as u32;
    let x1 = (center[0] + radius).ceil().min(width as f32) as u32;
    let y1 = (center[1] + radius).ceil().min(height as f32) as u32;
    for y in y0..y1 {
        for x in x0..x1 {
            if ((x as f32 + 0.5) - center[0]).hypot((y as f32 + 0.5) - center[1]) <= radius {
                mask[y as usize * width as usize + x as usize] = value;
            }
        }
    }
    Ok(())
}

fn validate_dimensions(width: u32, height: u32) -> Result<(), String> {
    if width == 0
        || height == 0
        || width > 16384
        || height > 16384
        || u64::from(width) * u64::from(height) > 32_000_000
    {
        return Err("Selection source exceeds 16384px per axis or 32 million pixels".into());
    }
    Ok(())
}

/// Preview a narrow antialiasing fringe next to an already excluded background.
/// An opaque candidate must be an intermediate mixture of the known light
/// exterior and a nearby, darker foreground core. Merely being grey/light is
/// insufficient: a solid grey region has no darker core and stays intact.
/// Protected pixels and the core itself never change. This explicit operation
/// records only an exclusion mask; the original RGBA remains recoverable.
pub fn light_halo_preview(
    image: &RgbaImage,
    excluded: &[bool],
    protected: &[bool],
    band_pixels: u8,
    tolerance: f32,
) -> Result<Vec<bool>, String> {
    let (w, h) = (image.width() as usize, image.height() as usize);
    if excluded.len() != w * h || protected.len() != w * h {
        return Err("Halo mask dimensions differ from source".into());
    }
    if band_pixels > 8 || !tolerance.is_finite() || !(0. ..=40.).contains(&tolerance) {
        return Err("Halo band must be 0–8 source pixels and OKLab tolerance 0–40".into());
    }
    let mut result = excluded.to_vec();
    let mut depth = vec![u8::MAX; w * h];
    let mut pending = VecDeque::new();
    for (i, &masked) in excluded.iter().enumerate() {
        if masked {
            depth[i] = 0;
            pending.push_back(i);
        }
    }
    while let Some(i) = pending.pop_front() {
        if depth[i] >= band_pixels {
            continue;
        }
        for n in neighbors(i, w, h).into_iter().flatten() {
            if depth[n] != u8::MAX || protected[n] {
                continue;
            }
            let pixel = image.get_pixel((n % w) as u32, (n / w) as u32);
            // Follow the normal from the excluded side into the art. A bounded
            // look-through finds the opaque core beyond up to eight fringe
            // pixels; it never jumps a protected/transparent/background gap.
            let dx = (n % w) as isize - (i % w) as isize;
            let dy = (n / w) as isize - (i / w) as isize;
            let mixture = tolerance > 0.
                && fringe_matches_core(
                    image,
                    excluded,
                    protected,
                    n,
                    [dx, dy],
                    band_pixels,
                    tolerance,
                );
            if pixel[3] == 0 || mixture {
                result[n] = true;
                depth[n] = depth[i] + 1;
                pending.push_back(n);
            }
        }
    }
    Ok(result)
}

fn fringe_matches_core(
    image: &RgbaImage,
    excluded: &[bool],
    protected: &[bool],
    index: usize,
    direction: [isize; 2],
    band_pixels: u8,
    tolerance: f32,
) -> bool {
    let (w, h) = (image.width() as isize, image.height() as isize);
    let (x, y) = ((index as isize) % w, (index as isize) / w);
    let [dx, dy] = direction;
    let at = |step: isize| {
        let (nx, ny) = (x + step * dx, y + step * dy);
        (nx >= 0 && ny >= 0 && nx < w && ny < h).then_some((nx, ny))
    };
    // Use original, explicitly excluded pixels for the reference. Newly added
    // fringe pixels must not progressively drift the background colour.
    let mut background = None;
    for step in 1..=isize::from(band_pixels) {
        let Some((nx, ny)) = at(-step) else { break };
        let n = (ny * w + nx) as usize;
        if protected[n] {
            break;
        }
        if excluded[n] {
            let p = image.get_pixel(nx as u32, ny as u32);
            let lab = srgb_to_oklab([p[0], p[1], p[2]]);
            if p[3] >= 128 && lab[0] >= 0.85 {
                background = Some([p[0], p[1], p[2]]);
            }
            break;
        }
    }
    let Some(background) = background else {
        return false;
    };
    let p = image.get_pixel(x as u32, y as u32);
    let observed = [p[0], p[1], p[2]];
    let observed_lab = srgb_to_oklab(observed);
    let linear = |rgb: [u8; 3]| {
        rgb.map(|v| {
            let v = f64::from(v) / 255.;
            if v <= 0.04045 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        })
    };
    let background_linear = linear(background);
    let observed_linear = linear(observed);
    for step in 1..=isize::from(band_pixels) + 1 {
        let Some((nx, ny)) = at(step) else { break };
        let n = (ny * w + nx) as usize;
        let core = image.get_pixel(nx as u32, ny as u32);
        if excluded[n] || core[3] < 250 {
            break;
        }
        let core_rgb = [core[0], core[1], core[2]];
        let core_lab = srgb_to_oklab(core_rgb);
        if core_lab[0] + 0.05 >= observed_lab[0] {
            continue;
        }
        // A core has local support, rather than being one dark noise pixel.
        if !neighbors(n, w as usize, h as usize)
            .into_iter()
            .flatten()
            .any(|adjacent| {
                if adjacent == index || excluded[adjacent] {
                    return false;
                }
                let q = image.get_pixel(
                    (adjacent % w as usize) as u32,
                    (adjacent / w as usize) as u32,
                );
                q[3] >= 250
                    && squared_distance(core_lab, srgb_to_oklab([q[0], q[1], q[2]])).sqrt() * 100.
                        <= 2.
            })
        {
            continue;
        }
        let core_linear = linear(core_rgb);
        let delta = std::array::from_fn::<_, 3, _>(|c| core_linear[c] - background_linear[c]);
        let norm: f64 = delta.iter().map(|v| v * v).sum();
        if norm <= 1e-8 {
            continue;
        }
        let fraction: f64 = (0..3)
            .map(|c| (observed_linear[c] - background_linear[c]) * delta[c])
            .sum::<f64>()
            / norm;
        // Reject the exact endpoints, rather than reserving a visible grey
        // range near them. Even gray32/252 are genuine intermediate samples
        // of a white/black AA edge. The numerical margin is below any distinct
        // 8-bit neutral sample; core support/lightness/protection checks remain.
        if !(1e-6..=1. - 1e-6).contains(&fraction) {
            continue;
        }
        let residual = (0..3)
            .map(|c| {
                let error = observed_linear[c] - (background_linear[c] + fraction * delta[c]);
                error * error
            })
            .sum::<f64>()
            .sqrt();
        if residual <= 0.008 + f64::from(tolerance.min(40.)) * 0.001 {
            return true;
        }
    }
    false
}

/// Premultiplied-alpha resampling: arbitrary hidden RGB cannot bleed into
/// visible pixels. Source remains unmodified and no upscaling is performed.
pub fn thumbnail_rgba(source: &RgbaImage, max_dimension: u32) -> RgbaImage {
    if source.width() <= max_dimension && source.height() <= max_dimension {
        return source.clone();
    }
    let scale = f64::from(max_dimension) / f64::from(source.width().max(source.height()));
    let width = (f64::from(source.width()) * scale).round().max(1.) as u32;
    let height = (f64::from(source.height()) * scale).round().max(1.) as u32;
    let mut premultiplied =
        image::ImageBuffer::<image::Rgba<f32>, Vec<f32>>::new(source.width(), source.height());
    for (out, pixel) in premultiplied.pixels_mut().zip(source.pixels()) {
        let a = f32::from(pixel[3]) / 255.;
        // image's floating-point sample range is 0..1. Supplying 8-bit RGB
        // magnitudes here makes the resampler clamp every nonzero channel to 1.
        *out = image::Rgba([
            f32::from(pixel[0]) / 255. * a,
            f32::from(pixel[1]) / 255. * a,
            f32::from(pixel[2]) / 255. * a,
            a,
        ]);
    }
    let resized = image::imageops::resize(
        &premultiplied,
        width,
        height,
        image::imageops::FilterType::Triangle,
    );
    image::ImageBuffer::from_fn(width, height, |x, y| {
        let p = resized.get_pixel(x, y);
        if p[3] <= 1e-6 {
            image::Rgba([0; 4])
        } else {
            image::Rgba([
                (p[0] / p[3] * 255.).round().clamp(0., 255.) as u8,
                (p[1] / p[3] * 255.).round().clamp(0., 255.) as u8,
                (p[2] / p[3] * 255.).round().clamp(0., 255.) as u8,
                (p[3] * 255.).round().clamp(0., 255.) as u8,
            ])
        }
    })
}
