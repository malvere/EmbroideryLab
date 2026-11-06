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

#[path = "raster_preparation_context.rs"]
mod context;

pub const PREPARATION_VERSION: u32 = 3;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum PreparationMode {
    #[default]
    Legacy,
    Gentle,
    Standard,
    Small,
    /// Standard phase 2 plus one bounded contextual sweep of flat microregions.
    Contextual,
}

impl PreparationMode {
    pub fn version(self) -> u32 {
        if self == Self::Contextual { 3 } else { 2 }
    }
    pub fn is_legacy(&self) -> bool {
        *self == Self::Legacy
    }

    /// Width is the maximum permitted local thickness of a candidate, in mm.
    /// Distances use native OKLab units; these are not CIE DeltaE thresholds.
    fn limits(self) -> (f64, f64, f64) {
        match self {
            Self::Legacy => (0., 0., 0.),
            Self::Gentle => (0.35, 0.025, 0.025),
            Self::Standard | Self::Contextual => (0.55, 0.035, 0.035),
            Self::Small => (0.8, 0.045, 0.045),
        }
    }
}

pub(super) struct Prepared {
    pub before: usize,
    pub after: usize,
    pub changed: usize,
    pub uncertain: usize,
    pub diagnostics: PreparationDiagnostics,
}

/// Each pass is a separate population of four-connected components. First
/// reasons partition that population; conditions overlap and MUST NOT be summed.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PreparationDiagnostics {
    pub passes: Vec<PreparationPass>,
    pub next_pass_changed_pixels: usize,
    pub comparison_work: u64,
    pub limit_probe: Option<PreparationPass>,
    pub limit_probe_comparison_work: u64,
    pub context_pass: Option<PreparationPass>,
    /// Diagnostic-only probes have a separate conservative work cap; they do
    /// not consume or reset the historical label-changing comparison budget.
    pub diagnostic_context_work_upper_bound: u64,
    pub diagnostic_context_limit_reached: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PreparationPass {
    pub first_reasons: BTreeMap<String, [usize; 4]>,
    pub conditions: BTreeMap<String, [usize; 4]>,
    pub first_reason_pixels: BTreeMap<String, [usize; 4]>,
    pub condition_pixels: BTreeMap<String, [usize; 4]>,
}

impl PreparationPass {
    fn add(&mut self, reason: &str, pixels: usize) {
        let bin = size_bin(pixels);
        self.first_reasons.entry(reason.into()).or_default()[bin] += 1;
        self.first_reason_pixels.entry(reason.into()).or_default()[bin] += pixels;
    }

    fn condition(&mut self, reason: &str, pixels: usize, applies: bool) {
        if applies {
            self.conditions.entry(reason.into()).or_default()[size_bin(pixels)] += 1;
            self.condition_pixels.entry(reason.into()).or_default()[size_bin(pixels)] += pixels;
        }
    }
}

fn size_bin(pixels: usize) -> usize {
    match pixels {
        1 => 0,
        2 => 1,
        3..=4 => 2,
        _ => 3,
    }
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

// Diagnostic hypothesis H1: a physically bounded island, fully enclosed in a
// locally flat source field. This is deliberately stricter than adjacency.
#[allow(clippy::too_many_arguments)]
fn enclosed_close_target(
    node: &Node,
    nodes: &[Node],
    labels: &[u8],
    samples: &[[f64; 3]],
    palette: &[[f64; 3]],
    dimensions: [usize; 2],
    scale: [f64; 2],
    limits: (f64, f64),
) -> Option<u8> {
    let [w, h] = dimensions;
    let (max_width, bound) = limits;
    if node.edges.len() != 1 || node.width_mm > max_width {
        return None;
    }
    let (&other, edge) = node.edges.first_key_value()?;
    let target = &nodes[other];
    if edge.length_mm + 1e-12 < node.perimeter_mm
        || target.pixels.len() < node.pixels.len().saturating_mul(2).max(4)
    {
        return None;
    }
    let (mut lo, mut hi) = ([usize::MAX; 2], [0; 2]);
    for &i in &node.pixels {
        let xy = [i % w, i / w];
        for axis in 0..2 {
            lo[axis] = lo[axis].min(xy[axis]);
            hi[axis] = hi[axis].max(xy[axis]);
        }
    }
    if (0..2).any(|axis| (hi[axis] - lo[axis] + 1) as f64 * scale[axis] > max_width + 1e-12)
        || lo[0] < 2
        || lo[1] < 2
        || hi[0] + 2 >= w
        || hi[1] + 2 >= h
    {
        return None;
    }
    for y in lo[1] - 2..=hi[1] + 2 {
        for x in lo[0] - 2..=hi[0] + 2 {
            let i = y * w + x;
            if labels[i] != node.label && labels[i] != target.label {
                return None;
            }
            // Original samples, including the complete two-pixel context,
            // must all fit the same fixed palette colour error ball.
            if squared_distance(samples[i], palette[target.label as usize]).sqrt() > bound {
                return None;
            }
        }
    }
    if edge.contrast_sum / edge.samples as f64 > bound {
        return None;
    }
    Some(target.label)
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
    prepare_inner(
        labels, image, palette, protected, pinned, scale, mode, cancel, 3, true, 0,
    )
}

#[allow(clippy::too_many_arguments)]
fn prepare_inner(
    labels: &mut [u8],
    image: &RgbaImage,
    palette: &[[f64; 3]],
    protected: &[bool],
    pinned: usize,
    scale: [f64; 2],
    mode: PreparationMode,
    cancel: Option<&AtomicBool>,
    passes: usize,
    audit_limit: bool,
    initial_work: u64,
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
            diagnostics: PreparationDiagnostics::default(),
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
    let mut comparison_work = initial_work;
    let mut diagnostics = PreparationDiagnostics::default();
    let mut last_changes = 0;
    if mode != PreparationMode::Legacy {
        for _ in 0..passes {
            let mut changes = 0;
            let mut diagnostic = PreparationPass::default();
            // Invalidated dependencies are not evaluated until a rebuilt RAG
            // provides their current size, width, boundary and contrast.
            let mut invalidated = BTreeSet::new();
            let mut order: Vec<_> = (0..nodes.len()).collect();
            order.sort_by_key(|&id| (nodes[id].pixels.len(), id));
            for id in order {
                cancelled(cancel)?;
                let node = &nodes[id];
                let mut lo = [f64::INFINITY; 3];
                let mut hi = [f64::NEG_INFINITY; 3];
                for (offset, &i) in node.pixels.iter().enumerate() {
                    if offset % 4096 == 0 {
                        cancelled(cancel)?;
                    }
                    for c in 0..3 {
                        lo[c] = lo[c].min(samples[i][c]);
                        hi[c] = hi[c].max(samples[i][c]);
                    }
                }
                let flat = squared_distance(lo, hi).sqrt() < 0.025;
                let count = node.pixels.len();
                diagnostic.condition("flat_source", count, flat);
                diagnostic.condition("protected", count, node.protected);
                diagnostic.condition("pinned", count, usize::from(node.label) < pinned);
                diagnostic.condition("width", count, node.width_mm > max_width + 1e-12);
                diagnostic.condition("invalidation", count, invalidated.contains(&id));
                let enough = node
                    .edges
                    .iter()
                    .any(|(&other, _)| nodes[other].pixels.len() >= count.saturating_mul(2).max(4));
                let supported = node.edges.iter().any(|(&other, edge)| {
                    nodes[other].pixels.len() >= count.saturating_mul(2).max(4)
                        && edge.length_mm >= 0.1 * node.perimeter_mm
                });
                diagnostic.condition("insufficient_neighbor", count, !enough);
                diagnostic.condition("weak_boundary", count, enough && !supported);
                let valid_enough = node.edges.iter().any(|(&other, _)| {
                    !invalidated.contains(&other)
                        && nodes[other].pixels.len() >= count.saturating_mul(2).max(4)
                });
                let valid_supported = node.edges.iter().any(|(&other, edge)| {
                    !invalidated.contains(&other)
                        && nodes[other].pixels.len() >= count.saturating_mul(2).max(4)
                        && edge.length_mm >= 0.1 * node.perimeter_mm
                });
                diagnostic.condition("target_invalidation", count, supported && !valid_supported);
                let probe_work = if flat && count <= 4 {
                    count as u64 * 192 * (node.edges.len() as u64 + 1).pow(2)
                } else {
                    0
                };
                let can_probe =
                    diagnostics.diagnostic_context_work_upper_bound + probe_work <= 50_000_000;
                if can_probe {
                    diagnostics.diagnostic_context_work_upper_bound += probe_work;
                } else {
                    diagnostics.diagnostic_context_limit_reached = true;
                }
                diagnostic.condition("diagnostic_limit", count, !can_probe);
                let enclosed = if flat && count <= 4 && can_probe {
                    enclosed_close_target(
                        node,
                        &nodes,
                        labels,
                        &samples,
                        palette,
                        [w, h],
                        scale,
                        (max_width, color_bound),
                    )
                } else {
                    None
                };
                diagnostic.condition("flat_enclosed_close_target", count, enclosed.is_some());
                let micro_aa = if flat && count <= 4 && can_probe {
                    context::micro_aa_pair(
                        node,
                        &nodes,
                        labels,
                        &samples,
                        palette,
                        [w, h],
                        scale,
                        (max_width, residual_bound),
                    )
                } else {
                    None
                };
                diagnostic.condition("flat_micro_aa_pair", count, micro_aa.is_some());
                diagnostic.condition(
                    "flat_micro_aa_one_ring",
                    count,
                    flat && count <= 4
                        && can_probe
                        && context::micro_aa_pair_one_ring(
                            node,
                            &nodes,
                            labels,
                            &samples,
                            palette,
                            [w, h],
                            scale,
                            (max_width, residual_bound),
                        )
                        .is_some(),
                );
                let early = if invalidated.contains(&id) {
                    Some("invalidation")
                } else if node.protected {
                    Some("protected")
                } else if usize::from(node.label) < pinned {
                    Some("pinned")
                } else if node.width_mm > max_width + 1e-12 {
                    Some("width")
                } else if node.edges.is_empty() {
                    Some("no_neighbor")
                } else if comparison_work > 50_000_000 {
                    Some("computation_limit")
                } else {
                    None
                };
                if let Some(reason) = early {
                    diagnostic.add(reason, count);
                    if reason == "computation_limit" {
                        uncertain.insert(node.pixels[0]);
                    }
                    continue;
                }
                // A flat rare colour is evidence of a possible intentional
                // symbol, even when close to its neighbour or a colour mixture.
                if flat {
                    diagnostic.add("flat_source", count);
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
                    diagnostic.add(
                        if changes != before_changes {
                            "reassigned_close"
                        } else {
                            "topology"
                        },
                        count,
                    );
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
                    diagnostic.add("flat_source", count);
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
                    diagnostic.add(
                        if changes != before_changes {
                            "reassigned_aa"
                        } else {
                            "topology"
                        },
                        count,
                    );
                } else {
                    diagnostic.add(
                        if supported && !valid_supported {
                            "invalidation"
                        } else if !valid_enough {
                            "insufficient_neighbor"
                        } else if !valid_supported {
                            "weak_boundary"
                        } else {
                            "color_error"
                        },
                        count,
                    );
                    uncertain.insert(node.pixels[0]);
                }
            }
            diagnostics.passes.push(diagnostic);
            last_changes = changes;
            if changes == 0 {
                break;
            }
            (nodes, _) = graph(labels, &samples, protected, w, h, scale, cancel)?;
        }
    }
    // Keep all Phase 2 decisions first. This new sweep handles only formerly
    // ambiguous flat microcomponents; it never applies the hypothetical fourth
    // ordinary RAG pass. Candidates with invalidated endpoints are retained.
    if mode == PreparationMode::Contextual && audit_limit {
        let mut diagnostic = PreparationPass::default();
        let mut invalidated = BTreeSet::new();
        let mut order: Vec<_> = (0..nodes.len()).collect();
        order.sort_by_key(|&id| (nodes[id].pixels.len(), id));
        for id in order {
            cancelled(cancel)?;
            let node = &nodes[id];
            let count = node.pixels.len();
            let reason = if invalidated.contains(&id) {
                Some("invalidation")
            } else if node.protected {
                Some("protected")
            } else if usize::from(node.label) < pinned {
                Some("pinned")
            } else if node.width_mm > max_width + 1e-12 {
                Some("width")
            } else if count > 4 {
                Some("not_microcomponent")
            } else if node.edges.is_empty() {
                Some("no_neighbor")
            } else if node.edges.keys().any(|other| invalidated.contains(other)) {
                Some("invalidation")
            } else if comparison_work > 50_000_000 {
                Some("computation_limit")
            } else {
                None
            };
            if let Some(reason) = reason {
                diagnostic.add(reason, count);
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
            if squared_distance(lo, hi).sqrt() >= 0.025 {
                diagnostic.add("not_flat", count);
                continue;
            }
            // Conservative bound for the small endpoint/context comparisons.
            comparison_work += count as u64 * 64 * (node.edges.len() as u64 + 1).pow(2);
            if comparison_work > 50_000_000 {
                diagnostic.add("computation_limit", count);
                continue;
            }
            let enclosed = enclosed_close_target(
                node,
                &nodes,
                labels,
                &samples,
                palette,
                [w, h],
                scale,
                (max_width, color_bound),
            );
            let aa = if enclosed.is_none() {
                context::micro_aa_pair_one_ring(
                    node,
                    &nodes,
                    labels,
                    &samples,
                    palette,
                    [w, h],
                    scale,
                    (max_width, residual_bound),
                )
            } else {
                None
            };
            let Some((a, b)) = enclosed.map(|v| (v, v)).or(aa) else {
                diagnostic.add("uncertain_context", count);
                uncertain.insert(node.pixels[0]);
                continue;
            };
            let mut changed = 0;
            for &i in &node.pixels {
                let target = if squared_distance(samples[i], palette[a as usize])
                    <= squared_distance(samples[i], palette[b as usize])
                {
                    a
                } else {
                    b
                };
                if labels[i] == node.label && unambiguous_attachment(labels, i, target, w, h) {
                    labels[i] = target;
                    changed += 1;
                }
            }
            diagnostic.add(
                if changed == 0 {
                    "topology"
                } else if enclosed.is_some() {
                    "reassigned_context_interior"
                } else {
                    "reassigned_context_aa"
                },
                count,
            );
            if changed > 0 {
                invalidated.insert(id);
                invalidated.extend(node.edges.keys().copied());
            }
            // A changed seed is not proof that the whole component resolved.
            // Keep a witness for any source-colour residual before filtering
            // historical seeds below (geometry has its own provenance mask).
            if changed < count
                && let Some(&i) = node.pixels.iter().find(|&&i| labels[i] == node.label)
            {
                uncertain.insert(i);
            }
        }
        diagnostics.context_pass = Some(diagnostic);
        (nodes, _) = graph(labels, &samples, protected, w, h, scale, cancel)?;
        uncertain.retain(|&i| labels[i] == original[i]);
    }
    diagnostics.comparison_work = comparison_work - initial_work;
    if audit_limit && diagnostics.passes.len() == passes && last_changes > 0 {
        let mut probe = labels.to_vec();
        let probed = prepare_inner(
            &mut probe,
            image,
            palette,
            protected,
            pinned,
            scale,
            mode,
            cancel,
            1,
            false,
            comparison_work,
        )?;
        diagnostics.next_pass_changed_pixels = probed.changed;
        diagnostics.limit_probe_comparison_work = probed.diagnostics.comparison_work;
        diagnostics.limit_probe = probed.diagnostics.passes.into_iter().next();
    }
    Ok(Prepared {
        before,
        after: nodes.len(),
        changed: original
            .iter()
            .zip(labels.iter())
            .filter(|(a, b)| a != b)
            .count(),
        uncertain: uncertain.len(),
        diagnostics,
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

    #[test]
    fn contextual_noise_absorption_preserves_opposite_details_and_user_locks() {
        let image = RgbaImage::from_fn(24, 24, |x, y| {
            let gray = match (x, y) {
                (8, 8) => 170,
                (17, 8) => 0,
                (17, 12..=18) => 255,
                _ => 160,
            };
            Rgba([gray, gray, gray, 255])
        });
        let palette: Vec<_> = [160, 170, 0, 255].map(|g| srgb_to_oklab([g; 3])).into();
        let source: Vec<_> = image
            .pixels()
            .map(|p| match p[0] {
                170 => 1,
                0 => 2,
                255 => 3,
                _ => 0,
            })
            .collect();
        for (mode, pinned, protect, scale, expected) in [
            (PreparationMode::Standard, 0, false, 0.2, 0),
            (PreparationMode::Contextual, 0, false, 0.2, 1),
            (PreparationMode::Contextual, 2, false, 0.2, 0),
            (PreparationMode::Contextual, 0, true, 0.2, 0),
            (PreparationMode::Contextual, 0, false, 1., 0),
        ] {
            let mut labels = source.clone();
            let mut protected = vec![false; labels.len()];
            protected[8 * 24 + 8] = protect;
            let report = prepare(
                &mut labels,
                &image,
                &palette,
                &protected,
                pinned,
                [scale; 2],
                mode,
                None,
            )
            .unwrap();
            assert_eq!(report.changed, expected);
            for (i, (&a, &b)) in source.iter().zip(&labels).enumerate() {
                assert!(a == b || (i == 8 * 24 + 8 && b == 0));
            }
            assert!(!labels.contains(&255), "noise must be absorbed, not masked");
        }
    }

    #[test]
    fn contextual_flat_edge_sample_is_distinct_from_an_isolated_symbol() {
        let image = RgbaImage::from_fn(24, 24, |x, y| {
            let gray = if (x == 11 && y == 12) || (x == 5 && y == 5) {
                96
            } else if x < 12 {
                0
            } else {
                255
            };
            Rgba([gray, gray, gray, 255])
        });
        let palette = vec![
            srgb_to_oklab([0; 3]),
            srgb_to_oklab([255; 3]),
            srgb_to_oklab([96; 3]),
        ];
        let source: Vec<_> = image
            .pixels()
            .map(|p| match p[0] {
                0 => 0,
                255 => 1,
                _ => 2,
            })
            .collect();
        let mut standard = source.clone();
        prepare(
            &mut standard,
            &image,
            &palette,
            &vec![false; 576],
            0,
            [0.2; 2],
            PreparationMode::Standard,
            None,
        )
        .unwrap();
        assert_eq!(standard, source);
        let mut labels = source.clone();
        let report = prepare(
            &mut labels,
            &image,
            &palette,
            &vec![false; 576],
            0,
            [0.2; 2],
            PreparationMode::Contextual,
            None,
        )
        .unwrap();
        assert_eq!(report.changed, 1);
        assert_eq!(labels[12 * 24 + 11], 0);
        assert_eq!(
            labels[5 * 24 + 5],
            2,
            "an isolated same-colour symbol has no AA boundary evidence"
        );
        assert_eq!(
            labels.iter().filter(|&&v| v == 1).count(),
            source.iter().filter(|&&v| v == 1).count()
        );
    }
}
