//! Additional evidence for tiny flat samples on an existing colour boundary.
//!
//! This is deliberately not an all-purpose antialiasing detector: the immutable
//! OKLab segment is a bounded diagnostic hypothesis, not a physical mixing law.
//! The caller still applies pinned-colour, invalidation and topology guards.
use super::*;

/// Return two incident fixed palette endpoints only when a complete local
/// neighbourhood witnesses their existing boundary. No labels are changed.
/// `limits` is (maximum bounding-box extent in mm, source residual in OKLab).
#[allow(clippy::too_many_arguments)]
pub(super) fn micro_aa_pair(
    node: &Node,
    nodes: &[Node],
    labels: &[u8],
    samples: &[[f64; 3]],
    palette: &[[f64; 3]],
    dimensions: [usize; 2],
    scale: [f64; 2],
    limits: (f64, f64),
) -> Option<(u8, u8)> {
    micro_aa_pair_radius(
        node, nodes, labels, samples, palette, dimensions, scale, limits, 2,
    )
}

/// Diagnostic ablation of the context radius. This does not select or apply a
/// reassignment; production selection remains the caller's explicit decision.
#[allow(clippy::too_many_arguments)]
pub(super) fn micro_aa_pair_one_ring(
    node: &Node,
    nodes: &[Node],
    labels: &[u8],
    samples: &[[f64; 3]],
    palette: &[[f64; 3]],
    dimensions: [usize; 2],
    scale: [f64; 2],
    limits: (f64, f64),
) -> Option<(u8, u8)> {
    micro_aa_pair_radius(
        node, nodes, labels, samples, palette, dimensions, scale, limits, 1,
    )
}

#[allow(clippy::too_many_arguments)]
fn micro_aa_pair_radius(
    node: &Node,
    nodes: &[Node],
    labels: &[u8],
    samples: &[[f64; 3]],
    palette: &[[f64; 3]],
    dimensions: [usize; 2],
    scale: [f64; 2],
    limits: (f64, f64),
    radius: usize,
) -> Option<(u8, u8)> {
    let [w, h] = dimensions;
    let (max_width, residual_bound) = limits;
    if node.protected || node.pixels.is_empty() || node.pixels.len() > 4 {
        return None;
    }
    let (mut left, mut top, mut right, mut bottom) = (w, h, 0, 0);
    for &i in &node.pixels {
        if labels[i] != node.label {
            return None;
        }
        left = left.min(i % w);
        right = right.max(i % w);
        top = top.min(i / w);
        bottom = bottom.max(i / w);
    }
    if (right - left + 1) as f64 * scale[0] > max_width + 1e-12
        || (bottom - top + 1) as f64 * scale[1] > max_width + 1e-12
        || left < radius
        || top < radius
        || right + radius >= w
        || bottom + radius >= h
    {
        return None;
    }
    let adjacent: Vec<_> = node
        .edges
        .iter()
        .filter(|(other, edge)| {
            nodes[**other].pixels.len() >= (2 * node.pixels.len()).max(4)
                && edge.length_mm + 1e-12 >= 0.1 * node.perimeter_mm
        })
        .map(|(&other, _)| nodes[other].label)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let mut best: Option<(f64, u8, u8)> = None;
    for (number, &a) in adjacent.iter().enumerate() {
        for &b in &adjacent[number + 1..] {
            if a == node.label || b == node.label {
                continue;
            }
            let aa = palette[a as usize];
            let bb = palette[b as usize];
            let norm = squared_distance(aa, bb);
            if norm < 0.12_f64.powi(2) {
                continue;
            }
            // A clipped neighbourhood, background, a third colour, or another
            // island of the candidate colour leaves the interpretation open.
            let mut actual_boundary = false;
            let mut complete = true;
            for y in top - radius..=bottom + radius {
                for x in left - radius..=right + radius {
                    let i = y * w + x;
                    if labels[i] == node.label {
                        complete &= node.pixels.contains(&i);
                    } else if labels[i] != a && labels[i] != b {
                        complete = false;
                    }
                    if labels[i] != a && labels[i] != b {
                        continue;
                    }
                    for n in [
                        (x < right + radius).then_some(i + 1),
                        (y < bottom + radius).then_some(i + w),
                    ]
                    .into_iter()
                    .flatten()
                    {
                        actual_boundary |= (labels[i] == a && labels[n] == b)
                            || (labels[i] == b && labels[n] == a);
                    }
                }
            }
            if !complete || !actual_boundary {
                continue;
            }
            let delta = std::array::from_fn::<_, 3, _>(|c| bb[c] - aa[c]);
            let mut residual = 0_f64;
            let mut admissible = true;
            for &i in &node.pixels {
                let s = samples[i];
                let t = (0..3).map(|c| (s[c] - aa[c]) * delta[c]).sum::<f64>() / norm;
                if !(0.15..=0.85).contains(&t) {
                    admissible = false;
                    break;
                }
                let projected = std::array::from_fn(|c| aa[c] + t * delta[c]);
                residual = residual.max(squared_distance(s, projected).sqrt());
                let error = squared_distance(s, aa).min(squared_distance(s, bb)).sqrt();
                if residual > residual_bound || error > 0.5 * norm.sqrt() + residual_bound {
                    admissible = false;
                    break;
                }
            }
            // Adjacent labels are ordered and equal-residual ties retain the
            // first pair, giving deterministic output without mutable means.
            if admissible && best.as_ref().is_none_or(|v| residual < v.0) {
                best = Some((residual, a, b));
            }
        }
    }
    best.map(|(_, a, b)| (a, b))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result(
        labels: &[u8],
        dimensions: [usize; 2],
        candidate_sample: [f64; 3],
        palette: &[[f64; 3]],
        scale: [f64; 2],
    ) -> Option<(u8, u8)> {
        result_radius(labels, dimensions, candidate_sample, palette, scale, 2)
    }

    fn result_radius(
        labels: &[u8],
        dimensions: [usize; 2],
        candidate_sample: [f64; 3],
        palette: &[[f64; 3]],
        scale: [f64; 2],
        radius: usize,
    ) -> Option<(u8, u8)> {
        let samples: Vec<_> = labels
            .iter()
            .map(|&v| {
                if v == 2 {
                    candidate_sample
                } else if v == 255 {
                    [0.; 3]
                } else {
                    palette[v as usize]
                }
            })
            .collect();
        let (nodes, _) = graph(
            labels,
            &samples,
            &vec![false; labels.len()],
            dimensions[0],
            dimensions[1],
            scale,
            None,
        )
        .unwrap();
        let node = nodes.iter().find(|n| n.label == 2).unwrap();
        micro_aa_pair_radius(
            node,
            &nodes,
            labels,
            &samples,
            palette,
            dimensions,
            scale,
            (0.55, 0.035),
            radius,
        )
    }

    #[test]
    fn flat_single_sample_on_an_existing_boundary_has_context_evidence() {
        let mut labels: Vec<_> = (0..49).map(|i| u8::from(i % 7 >= 4)).collect();
        labels[3 * 7 + 3] = 2;
        let palette = [[0.2, 0., 0.], [0.8, 0., 0.], [0.5, 0., 0.]];
        assert_eq!(
            result(&labels, [7, 7], palette[2], &palette, [0.1; 2]),
            Some((0, 1))
        );
        assert_eq!(
            result(&labels, [7, 7], [0.5, 0.1, 0.], &palette, [0.1; 2]),
            None,
            "off-segment contrast is not AA evidence"
        );
    }

    #[test]
    fn isolated_contrast_dot_and_separate_islands_are_not_an_existing_edge() {
        let palette = [[0.2, 0., 0.], [0.8, 0., 0.], [0.5, 0., 0.]];
        let mut labels = vec![0; 49];
        labels[24] = 2;
        assert_eq!(
            result(&labels, [7, 7], palette[2], &palette, [0.1; 2]),
            None
        );
        let mut separate = vec![255; 49];
        for y in 0..7 {
            separate[y * 7] = 0;
            separate[y * 7 + 6] = 1;
        }
        separate[24] = 2;
        assert_eq!(
            result(&separate, [7, 7], palette[2], &palette, [0.1; 2]),
            None
        );
    }

    #[test]
    fn narrow_white_detail_and_physically_large_sample_are_retained() {
        let mut stripe: Vec<_> = (0..64).map(|i| u8::from(i % 8 >= 4)).collect();
        for y in 2..6 {
            stripe[y * 8 + 3] = 2;
        }
        let palette = [[0.1, 0., 0.], [0.6, 0., 0.], [1., 0., 0.]];
        assert_eq!(
            result(&stripe, [8, 8], palette[2], &palette, [0.1; 2]),
            None
        );
        let mut dot: Vec<_> = (0..49).map(|i| u8::from(i % 7 >= 4)).collect();
        dot[24] = 2;
        assert_eq!(
            result(&dot, [7, 7], [0.35, 0., 0.], &palette, [0.6; 2]),
            None
        );
    }

    #[test]
    fn one_ring_ablation_only_ignores_evidence_outside_its_context() {
        let mut labels: Vec<_> = (0..49).map(|i| u8::from(i % 7 >= 4)).collect();
        labels[24] = 2;
        labels[8] = 3; // Two pixels away; outside the one-pixel witness.
        let palette = [[0.2, 0., 0.], [0.8, 0., 0.], [0.5, 0., 0.], [0.1, 0.1, 0.]];
        assert_eq!(
            result(&labels, [7, 7], palette[2], &palette, [0.1; 2]),
            None
        );
        assert_eq!(
            result_radius(&labels, [7, 7], palette[2], &palette, [0.1; 2], 1),
            Some((0, 1))
        );
        // Moving the third colour into the witness must still reject it.
        labels[16] = 3;
        assert_eq!(
            result_radius(&labels, [7, 7], palette[2], &palette, [0.1; 2], 1),
            None
        );
    }

    #[test]
    fn one_ring_requires_real_endpoint_contact_and_source_evidence() {
        let palette = [[0.2, 0., 0.], [0.8, 0., 0.], [1., 0., 0.]];
        let mut labels: Vec<_> = (0..49).map(|i| u8::from(i % 7 >= 4)).collect();
        labels[24] = 2;
        assert_eq!(
            result_radius(&labels, [7, 7], [0.5, 0.1, 0.], &palette, [0.1; 2], 1),
            None,
            "off-segment contrast remains uncertain"
        );
        assert_eq!(
            result_radius(&labels, [7, 7], palette[2], &palette, [0.1; 2], 1),
            None,
            "a white detail is outside the endpoint mixture"
        );
        labels.fill(0);
        labels[24] = 2;
        assert_eq!(
            result_radius(&labels, [7, 7], [0.5, 0., 0.], &palette, [0.1; 2], 1),
            None,
            "an isolated dot has no two-endpoint boundary"
        );
        // Separate base-colour islands and background do not witness a boundary.
        labels.fill(255);
        for y in 0..7 {
            labels[y * 7] = 0;
            labels[y * 7 + 6] = 1;
        }
        labels[24] = 2;
        assert_eq!(
            result_radius(&labels, [7, 7], [0.5, 0., 0.], &palette, [0.1; 2], 1),
            None
        );
    }
}
