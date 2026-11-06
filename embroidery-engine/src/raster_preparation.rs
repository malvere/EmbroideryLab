//! Deterministic source-bounded preparation of adjacent raster components.
//!
//! No pixels become background, no disconnected container replaces geometry,
//! and palette colours stay fixed. A fresh RAG is constructed after each pass:
//! edge support, source contrast, local width and component size are recomputed.
//! Error bounds are always evaluated against immutable source samples, so a
//! sequence of individually small merges cannot walk along a colour gradient.
use crate::color_selection::{squared_distance, srgb_to_oklab};
use image::RgbaImage;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::atomic::{AtomicBool, Ordering},
};

pub const PREPARATION_VERSION: u32 = 2;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum PreparationMode {
    #[default]
    Legacy,
    Gentle,
    Standard,
    Small,
}

impl PreparationMode {
    pub fn is_legacy(&self) -> bool {
        *self == Self::Legacy
    }

    /// Width is the maximum permitted local thickness of a candidate, in mm.
    /// Distances use native OKLab units; these are not CIE DeltaE thresholds.
    fn limits(self) -> (f64, f64, f64) {
        match self {
            Self::Legacy => (0., 0., 0.),
            Self::Gentle => (0.35, 0.025, 0.025),
            Self::Standard => (0.55, 0.035, 0.035),
            Self::Small => (0.8, 0.045, 0.045),
        }
    }
}

pub(super) struct Prepared {
    pub before: usize,
    pub after: usize,
    pub changed: usize,
    pub uncertain: usize,
}

#[derive(Default)]
struct Edge {
    length_mm: f64,
    contrast_sum: f64,
    samples: usize,
}

struct Node {
    label: u8,
    pixels: Vec<usize>,
    protected: bool,
    perimeter_mm: f64,
    width_mm: f64,
    edges: BTreeMap<usize, Edge>,
}

fn cancelled(cancel: Option<&AtomicBool>) -> Result<(), String> {
    if cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
        Err("Raster preparation cancelled".into())
    } else {
        Ok(())
    }
}

fn neighbors(i: usize, w: usize, h: usize) -> [Option<usize>; 4] {
    let (x, y) = (i % w, i / w);
    [
        x.checked_sub(1).map(|_| i - 1),
        (x + 1 < w).then_some(i + 1),
        y.checked_sub(1).map(|_| i - w),
        (y + 1 < h).then_some(i + w),
    ]
}

/// Threshold decisions belong to the immutable pre-preparation components.
/// Partial boundary reassignment must never turn an accepted component's
/// residual pixels into a newly omitted sub-threshold component.
pub(super) fn retained_provenance(
    labels: &[u8],
    protected: &[bool],
    dimensions: [usize; 2],
    scale: [f64; 2],
    floors: (usize, f64),
    cancel: Option<&AtomicBool>,
) -> Result<Vec<bool>, String> {
    let [w, h] = dimensions;
    let mut visited = vec![false; labels.len()];
    let mut retained = vec![false; labels.len()];
    for seed in 0..labels.len() {
        if seed % 4096 == 0 {
            cancelled(cancel)?;
        }
        if labels[seed] == 255 || visited[seed] {
            continue;
        }
        let mut pixels = vec![seed];
        visited[seed] = true;
        let mut cursor = 0;
        let mut locked = false;
        while cursor < pixels.len() {
            let i = pixels[cursor];
            locked |= protected[i];
            for n in neighbors(i, w, h).into_iter().flatten() {
                if !visited[n] && labels[n] == labels[seed] {
                    visited[n] = true;
                    pixels.push(n);
                }
            }
            cursor += 1;
            if cursor % 4096 == 0 {
                cancelled(cancel)?;
            }
        }
        if locked
            || (pixels.len() >= floors.0
                && pixels.len() as f64 * scale[0] * scale[1] + 1e-12 >= floors.1)
        {
            for i in pixels {
                retained[i] = true;
            }
        }
    }
    Ok(retained)
}

/// Component identities are recomputed from real four-connected label pixels.
fn graph(
    labels: &[u8],
    samples: &[[f64; 3]],
    protected: &[bool],
    w: usize,
    h: usize,
    scale: [f64; 2],
    cancel: Option<&AtomicBool>,
) -> Result<(Vec<Node>, Vec<usize>), String> {
    let mut membership = vec![usize::MAX; labels.len()];
    let mut nodes = Vec::new();
    let mut depth = vec![u32::MAX; labels.len()];
    for seed in 0..labels.len() {
        if seed % 4096 == 0 {
            cancelled(cancel)?;
        }
        if labels[seed] == 255 || membership[seed] != usize::MAX {
            continue;
        }
        let id = nodes.len();
        if id >= 65_536 {
            return Err("Boundary preparation exceeds 65,536 component nodes; reduce trace resolution or use Legacy preparation".into());
        }
        let mut pixels = vec![seed];
        membership[seed] = id;
        let mut cursor = 0;
        let mut locked = false;
        while cursor < pixels.len() {
            let i = pixels[cursor];
            locked |= protected[i];
            for n in neighbors(i, w, h).into_iter().flatten() {
                if labels[n] == labels[seed] && membership[n] == usize::MAX {
                    membership[n] = id;
                    pixels.push(n);
                }
            }
            cursor += 1;
            if cursor % 4096 == 0 {
                cancelled(cancel)?;
            }
        }
        nodes.push(Node {
            label: labels[seed],
            pixels,
            protected: locked,
            perimeter_mm: 0.,
            width_mm: 0.,
            edges: BTreeMap::new(),
        });
    }
    // Multi-source distance from each component boundary. The maximum depth
    // bounds the thickest interior, unlike 2*area/perimeter on branched art.
    let mut queue = Vec::new();
    for i in 0..labels.len() {
        if labels[i] == 255 {
            continue;
        }
        let id = membership[i];
        let mut boundary = false;
        for (axis, n) in neighbors(i, w, h).into_iter().enumerate() {
            if n.is_none_or(|n| membership[n] != id) {
                boundary = true;
                let length = if axis < 2 { scale[1] } else { scale[0] };
                nodes[id].perimeter_mm += length;
                if let Some(n) = n.filter(|&n| labels[n] != 255) {
                    let other = membership[n];
                    let edge = nodes[id].edges.entry(other).or_default();
                    edge.length_mm += length;
                    edge.contrast_sum += squared_distance(samples[i], samples[n]).sqrt();
                    edge.samples += 1;
                }
            }
        }
        if boundary {
            depth[i] = 0;
            queue.push(i);
        }
    }
    let mut cursor = 0;
    while cursor < queue.len() {
        let i = queue[cursor];
        for n in neighbors(i, w, h).into_iter().flatten() {
            if membership[n] == membership[i] && depth[n] == u32::MAX {
                depth[n] = depth[i] + 1;
                queue.push(n);
            }
        }
        cursor += 1;
        if cursor % 4096 == 0 {
            cancelled(cancel)?;
        }
    }
    for (i, &id) in membership.iter().enumerate() {
        if id != usize::MAX {
            nodes[id].width_mm = nodes[id]
                .width_mm
                .max(f64::from(2 * depth[i] + 1) * scale[0].max(scale[1]));
        }
    }
    Ok((nodes, membership))
}

/// Refuse bridges between currently disconnected target islands, including
/// two near-parallel lines. The 8-pixel ring must contain exactly one target
/// four-connected group incident to the centre. This conservative local test
/// can reject a harmless change; it cannot create a new island or bridge.
fn incident_groups(labels: &[u8], i: usize, label: u8, w: usize, h: usize) -> usize {
    let (x, y) = (i % w, i / w);
    let mut present = [false; 9];
    for dy in -1_isize..=1 {
        for dx in -1_isize..=1 {
            if dx == 0 && dy == 0 {
                continue;
            }
            let (nx, ny) = (x as isize + dx, y as isize + dy);
            if nx >= 0 && ny >= 0 && nx < w as isize && ny < h as isize {
                present[((dy + 1) * 3 + dx + 1) as usize] =
                    labels[ny as usize * w + nx as usize] == label;
            }
        }
    }
    let mut seen = [false; 9];
    let mut groups = 0;
    for start in [1, 3, 5, 7] {
        if !present[start] || seen[start] {
            continue;
        }
        groups += 1;
        let mut pending = vec![start];
        seen[start] = true;
        while let Some(n) = pending.pop() {
            for next in neighbors(n, 3, 3).into_iter().flatten() {
                if present[next] && !seen[next] {
                    seen[next] = true;
                    pending.push(next);
                }
            }
        }
    }
    groups
}

fn unambiguous_attachment(labels: &[u8], i: usize, label: u8, w: usize, h: usize) -> bool {
    incident_groups(labels, i, label, w, h) == 1 && incident_groups(labels, i, labels[i], w, h) <= 1
}

#[allow(clippy::too_many_arguments)]
pub(super) fn prepare(
    labels: &mut [u8],
    image: &RgbaImage,
    palette: &[[f64; 3]],
    protected: &[bool],
    pinned: usize,
    scale: [f64; 2],
    mode: PreparationMode,
    cancel: Option<&AtomicBool>,
) -> Result<Prepared, String> {
    cancelled(cancel)?;
    // Disabled preparation never allocates a RAG or an OKLab sample map.
    // Its component totals are filled by the existing tracing traversal.
    if mode == PreparationMode::Legacy {
        return Ok(Prepared {
            before: 0,
            after: 0,
            changed: 0,
            uncertain: 0,
        });
    }
    let (w, h) = (image.width() as usize, image.height() as usize);
    let samples: Vec<_> = image
        .pixels()
        .enumerate()
        .map(|(i, p)| {
            if i % 4096 == 0 {
                cancelled(cancel)?;
            }
            Ok(srgb_to_oklab([p[0], p[1], p[2]]))
        })
        .collect::<Result<_, String>>()?;
    let original = labels.to_vec();
    let (mut nodes, _) = graph(labels, &samples, protected, w, h, scale, cancel)?;
    let before = nodes.len();
    let (max_width, color_bound, residual_bound) = mode.limits();
    let mut uncertain = BTreeSet::new();
    let mut comparison_work = 0_u64;
    if mode != PreparationMode::Legacy {
        for _ in 0..3 {
            let mut changes = 0;
            // Invalidated dependencies are not evaluated until a rebuilt RAG
            // provides their current size, width, boundary and contrast.
            let mut invalidated = BTreeSet::new();
            let mut order: Vec<_> = (0..nodes.len()).collect();
            order.sort_by_key(|&id| (nodes[id].pixels.len(), id));
            for id in order {
                cancelled(cancel)?;
                let node = &nodes[id];
                if invalidated.contains(&id)
                    || node.protected
                    || usize::from(node.label) < pinned
                    || node.width_mm > max_width + 1e-12
                    || node.edges.is_empty()
                {
                    continue;
                }
                if comparison_work > 50_000_000 {
                    uncertain.insert(node.pixels[0]);
                    continue;
                }
                let mut lo = [f64::INFINITY; 3];
                let mut hi = [f64::NEG_INFINITY; 3];
                for &i in &node.pixels {
                    for c in 0..3 {
                        lo[c] = lo[c].min(samples[i][c]);
                        hi[c] = hi[c].max(samples[i][c]);
                    }
                }
                // A flat rare colour is evidence of a possible intentional
                // symbol, even when close to its neighbour or a colour mixture.
                if squared_distance(lo, hi).sqrt() < 0.025 {
                    uncertain.insert(node.pixels[0]);
                    continue;
                }
                let mut targets = Vec::new();
                for (&other, edge) in &node.edges {
                    if invalidated.contains(&other) {
                        continue;
                    }
                    let target = &nodes[other];
                    // Never use another tiny island as the growing target.
                    if target.pixels.len() < node.pixels.len().saturating_mul(2).max(4)
                        || edge.length_mm < 0.1 * node.perimeter_mm
                    {
                        continue;
                    }
                    comparison_work += node.pixels.len() as u64;
                    let max_error = node
                        .pixels
                        .iter()
                        .map(|&i| {
                            squared_distance(samples[i], palette[target.label as usize]).sqrt()
                        })
                        .fold(0., f64::max);
                    let contrast = edge.contrast_sum / edge.samples as f64;
                    if max_error <= color_bound && contrast <= color_bound {
                        targets.push((max_error, target.label));
                    }
                }
                let close_target = targets
                    .iter()
                    .min_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)))
                    .map(|v| v.1);
                if let Some(target) = close_target {
                    // A fixed source bound is checked again in every round;
                    // colour error never accumulates via mutable centroids.
                    let before_changes = changes;
                    for &i in &node.pixels {
                        if labels[i] == node.label
                            && unambiguous_attachment(labels, i, target, w, h)
                        {
                            labels[i] = target;
                            changes += 1;
                        }
                    }
                    if changes != before_changes {
                        invalidated.insert(id);
                        invalidated.extend(node.edges.keys().copied());
                    }
                    continue;
                }
                // AA evidence needs TWO incident endpoint regions, sufficient
                // shared boundary, non-flat original samples, and a source
                // colour lying near their fixed OKLab mixture segment.
                let mut best: Option<(f64, u8, u8)> = None;
                let adjacent: Vec<_> = node
                    .edges
                    .iter()
                    .filter(|(other, edge)| {
                        !invalidated.contains(*other)
                            && nodes[**other].pixels.len()
                                >= node.pixels.len().saturating_mul(2).max(4)
                            && edge.length_mm >= 0.1 * node.perimeter_mm
                    })
                    .map(|(&other, _)| nodes[other].label)
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect();
                // Uniform small symbols/intentional flat stripes are ambiguous,
                // even if their colour happens to sit between their neighbours.
                if squared_distance(lo, hi).sqrt() < 0.025 {
                    uncertain.insert(node.pixels[0]);
                    continue;
                }
                for (n, &a) in adjacent.iter().enumerate() {
                    for &b in &adjacent[n + 1..] {
                        if a == b {
                            continue;
                        }
                        let (aa, bb) = (palette[a as usize], palette[b as usize]);
                        let d = std::array::from_fn::<_, 3, _>(|c| bb[c] - aa[c]);
                        let norm = squared_distance(aa, bb);
                        if norm < 0.12_f64.powi(2) {
                            continue;
                        }
                        comparison_work += node.pixels.len() as u64;
                        if comparison_work > 50_000_000 {
                            break;
                        }
                        let mut residual = 0_f64;
                        let mut min_t = 1_f64;
                        let mut max_t = 0_f64;
                        let mut admissible = true;
                        for &i in &node.pixels {
                            let s = samples[i];
                            let t = (0..3).map(|c| (s[c] - aa[c]) * d[c]).sum::<f64>() / norm;
                            let projected = std::array::from_fn(|c| aa[c] + t.clamp(0., 1.) * d[c]);
                            residual = residual.max(squared_distance(s, projected).sqrt());
                            min_t = min_t.min(t);
                            max_t = max_t.max(t);
                            // At most half the endpoint separation, plus the
                            // explicitly recorded residual to the mixture line.
                            let error = squared_distance(s, aa).min(squared_distance(s, bb)).sqrt();
                            if error > 0.5 * norm.sqrt() + residual_bound {
                                admissible = false;
                                break;
                            }
                        }
                        if admissible
                            && residual <= residual_bound
                            && min_t >= -0.05
                            && max_t <= 1.05
                            && max_t - min_t >= 0.04
                            && best.as_ref().is_none_or(|v| residual < v.0)
                        {
                            best = Some((residual, a, b));
                        }
                    }
                }
                if let Some((_, a, b)) = best {
                    let before_changes = changes;
                    for &i in &node.pixels {
                        if labels[i] != node.label {
                            continue;
                        }
                        let target = if squared_distance(samples[i], palette[a as usize])
                            <= squared_distance(samples[i], palette[b as usize])
                        {
                            a
                        } else {
                            b
                        };
                        if unambiguous_attachment(labels, i, target, w, h) {
                            labels[i] = target;
                            changes += 1;
                        }
                    }
                    if changes != before_changes {
                        invalidated.insert(id);
                        invalidated.extend(node.edges.keys().copied());
                    }
                } else {
                    uncertain.insert(node.pixels[0]);
                }
            }
            if changes == 0 {
                break;
            }
            (nodes, _) = graph(labels, &samples, protected, w, h, scale, cancel)?;
        }
    }
    Ok(Prepared {
        before,
        after: nodes.len(),
        changed: original.iter().zip(labels).filter(|(a, b)| a != b).count(),
        uncertain: uncertain.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    fn stripe(flat: bool) -> (Vec<u8>, RgbaImage, Vec<[f64; 3]>) {
        let image = RgbaImage::from_fn(32, 20, |_, y| {
            let gray = match y {
                0..=8 => 0,
                9 if !flat => 64,
                10 if !flat => 160,
                9 | 10 => 128,
                _ => 255,
            };
            Rgba([gray, gray, gray, 255])
        });
        let labels = (0..640)
            .map(|i| match i / 32 {
                0..=8 => 0,
                9 | 10 => 2,
                _ => 1,
            })
            .collect();
        (
            labels,
            image,
            vec![
                srgb_to_oklab([0; 3]),
                srgb_to_oklab([255; 3]),
                srgb_to_oklab([128; 3]),
            ],
        )
    }

    #[test]
    fn aa_boundary_reassigns_real_pixels_without_background_or_new_geometry() {
        let (mut labels, image, palette) = stripe(false);
        let report = prepare(
            &mut labels,
            &image,
            &palette,
            &[false; 640],
            0,
            [0.1, 0.1],
            PreparationMode::Standard,
            None,
        )
        .unwrap();
        assert_eq!((report.before, report.after, report.changed), (3, 2, 64));
        assert!(labels[..320].iter().all(|&v| v == 0));
        assert!(labels[320..].iter().all(|&v| v == 1));
        assert!(!labels.contains(&255));
        let mut repeat = stripe(false).0;
        prepare(
            &mut repeat,
            &image,
            &palette,
            &[false; 640],
            0,
            [0.1, 0.1],
            PreparationMode::Standard,
            None,
        )
        .unwrap();
        assert_eq!(labels, repeat);
    }

    #[test]
    fn flat_rare_symbol_is_retained_even_when_its_color_is_a_mixture() {
        let (mut labels, image, palette) = stripe(true);
        let original = labels.clone();
        let report = prepare(
            &mut labels,
            &image,
            &palette,
            &[false; 640],
            0,
            [0.1, 0.1],
            PreparationMode::Small,
            None,
        )
        .unwrap();
        assert_eq!(labels, original);
        assert!(report.uncertain > 0);
    }

    #[test]
    fn protection_pinning_and_physical_scale_are_strict_guards() {
        for (protected, pinned, scale) in [(true, 0, 0.1), (false, 3, 0.1), (false, 0, 1.)] {
            let (mut labels, image, palette) = stripe(false);
            let original = labels.clone();
            let mut mask = [false; 640];
            mask[9 * 32 + 4] = protected;
            prepare(
                &mut labels,
                &image,
                &palette,
                &mask,
                pinned,
                [scale, scale],
                PreparationMode::Standard,
                None,
            )
            .unwrap();
            assert_eq!(labels, original);
        }
    }

    #[test]
    fn attachment_does_not_bridge_nearby_lines_or_create_new_islands() {
        let labels = [0, 1, 0, 0, 2, 0, 0, 1, 0];
        assert!(!unambiguous_attachment(&labels, 4, 1, 3, 3));
        assert!(!unambiguous_attachment(&[0; 9], 4, 1, 3, 3));
        assert!(unambiguous_attachment(&[1; 9], 4, 1, 3, 3));
    }

    #[test]
    fn legacy_is_bit_exact_and_cancellation_changes_no_labels() {
        let (mut labels, image, palette) = stripe(false);
        let original = labels.clone();
        let report = prepare(
            &mut labels,
            &image,
            &palette,
            &[false; 640],
            0,
            [0.1, 0.1],
            PreparationMode::Legacy,
            None,
        )
        .unwrap();
        assert_eq!(labels, original);
        assert_eq!(report.changed, 0);
        assert!(
            prepare(
                &mut labels,
                &image,
                &palette,
                &[false; 640],
                0,
                [0.1, 0.1],
                PreparationMode::Standard,
                Some(&AtomicBool::new(true))
            )
            .is_err()
        );
        assert_eq!(labels, original);
    }

    #[test]
    fn near_color_merges_are_bounded_by_original_samples_without_gradient_drift() {
        let image = RgbaImage::from_fn(40, 8, |x, y| {
            let gray = if x < 20 {
                160
            } else {
                160 + (x - 19) as u8 * 2
            };
            let gray = if x >= 20 {
                if y % 2 == 0 { gray - 6 } else { gray + 6 }
            } else {
                gray
            };
            Rgba([gray, gray, gray, 255])
        });
        let palette: Vec<_> = (0..21).map(|i| srgb_to_oklab([160 + i * 2; 3])).collect();
        let mut labels: Vec<_> = (0_usize..320)
            .map(|i| (i % 40).saturating_sub(19) as u8)
            .collect();
        let source = labels.clone();
        let report = prepare(
            &mut labels,
            &image,
            &palette,
            &[false; 320],
            0,
            [0.05, 0.05],
            PreparationMode::Standard,
            None,
        )
        .unwrap();
        assert!(
            report.changed > 0,
            "the source-bound check must exercise actual reassignment"
        );
        for (i, &label) in labels.iter().enumerate() {
            if label != source[i] {
                assert!(
                    squared_distance(
                        srgb_to_oklab([image.get_pixel((i % 40) as u32, (i / 40) as u32)[0]; 3]),
                        palette[label as usize]
                    )
                    .sqrt()
                        <= 0.035 + 1e-12
                );
            }
        }
        assert_ne!(
            labels[39], 0,
            "a chain of weak contrasts must not swallow the entire ramp"
        );
    }
}
