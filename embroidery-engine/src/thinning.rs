//! Independent binary thinning, with foreground 8-connectivity and background
//! 4-connectivity. This is image geometry, not a stitch or automatic satin maker.
//!
//! Uses the Guo--Hall two-subiteration deletion conditions. The mathematical
//! rules are documented in Algorithm 3 of Bolelli and Grana (2019),
//! https://federicobolelli.it/pub_files/2019iciap_thinning.pdf .
//! No installed SDK, binary, decompiled function or external implementation is
//! linked or incorporated.

const MAX_AXIS: usize = 2048;
const MAX_PIXELS: usize = 4_000_000;
// Charge once per foreground neighborhood examined. Return an error, never a
// partially thinned mask, when a very thick/complex input exceeds this budget.
const MAX_NEIGHBORHOOD_CHECKS: usize = 200_000_000;

/// Row-major binary pixels: 1 is foreground and 0 is background.
/// Pixels outside the rectangle are always background.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BinaryMask {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<u8>,
}

impl BinaryMask {
    pub fn new(width: usize, height: usize, pixels: Vec<u8>) -> Result<Self, String> {
        let mask = Self {
            width,
            height,
            pixels,
        };
        mask.validate()?;
        Ok(mask)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.width == 0 || self.height == 0 || self.width > MAX_AXIS || self.height > MAX_AXIS {
            return Err("Skeleton mask axes must be between 1 and 2048 pixels".into());
        }
        let size = self
            .width
            .checked_mul(self.height)
            .ok_or("Skeleton mask size overflow")?;
        if size > MAX_PIXELS {
            return Err("Skeleton mask exceeds 4,000,000 pixels".into());
        }
        if self.pixels.len() != size {
            return Err("Skeleton mask pixel count does not match its dimensions".into());
        }
        if self.pixels.iter().any(|&p| p > 1) {
            return Err("Skeleton mask pixels must be 0 or 1".into());
        }
        Ok(())
    }
}

fn reserved<T>(capacity: usize) -> Result<Vec<T>, String> {
    let mut vector = Vec::new();
    vector
        .try_reserve_exact(capacity)
        .map_err(|_| "Not enough memory for skeleton mask".to_string())?;
    Ok(vector)
}

// Neighbor bit order is N, NE, E, SE, S, SW, W, NW. Keeping the
// cardinal/diagonal names explicit avoids dependence on another library's LUT.
fn removable(bits: usize, second: bool) -> bool {
    let [n, ne, e, se, s, sw, w, nw] = std::array::from_fn::<_, 8, _>(|i| bits & (1 << i) != 0);
    let crossings = usize::from(!n && (ne || e))
        + usize::from(!e && (se || s))
        + usize::from(!s && (sw || w))
        + usize::from(!w && (nw || n));
    let pairs_a =
        usize::from(nw || n) + usize::from(ne || e) + usize::from(se || s) + usize::from(sw || w);
    let pairs_b =
        usize::from(n || ne) + usize::from(e || se) + usize::from(s || sw) + usize::from(w || nw);
    let keep_direction = if second {
        (n || ne || !se) && e
    } else {
        (s || sw || !nw) && w
    };
    crossings == 1 && (2..=3).contains(&pairs_a.min(pairs_b)) && !keep_direction
}

/// Reduce foreground regions to a binary skeleton without pruning branches.
///
/// The returned pixels are a subset of the input; isolated points, components
/// and holes are retained. Topology refers to the raster mask at this
/// resolution, not the original vector outline. No smoothing, width estimate,
/// arbitrary spur removal or guarantee of an exact Euclidean medial axis is
/// implied. Failure leaves the input untouched.
pub fn skeletonize(mask: &BinaryMask) -> Result<BinaryMask, String> {
    skeletonize_with_budget(mask, MAX_NEIGHBORHOOD_CHECKS)
}

fn skeletonize_with_budget(mask: &BinaryMask, budget: usize) -> Result<BinaryMask, String> {
    mask.validate()?;
    let stride = mask.width + 2;
    let padded_size = stride * (mask.height + 2);
    let mut pixels = reserved::<u8>(padded_size)?;
    pixels.resize(padded_size, 0);
    let foreground = mask.pixels.iter().filter(|&&p| p != 0).count();
    let mut active = reserved::<usize>(foreground)?;
    let mut removed = reserved::<usize>(foreground)?;
    for y in 0..mask.height {
        let start = (y + 1) * stride + 1;
        let source = &mask.pixels[y * mask.width..(y + 1) * mask.width];
        pixels[start..start + mask.width].copy_from_slice(source);
        active.extend(
            (0..mask.width)
                .filter(|&x| source[x] != 0)
                .map(|x| start + x),
        );
    }
    // A wholly surrounded pixel cannot satisfy the deletion rules. Foreground
    // only shrinks, so the candidate boundary is monotone: retain survivors and
    // add neighbors of removed pixels. This avoids rescanning thick interiors.
    active.retain(|&p| neighbors(p, stride).iter().any(|&q| pixels[q] == 0));
    let mut queued = reserved::<u8>(padded_size)?;
    queued.resize(padded_size, 0);
    for &p in &active {
        queued[p] = 1;
    }
    let tables: [[bool; 256]; 2] =
        std::array::from_fn(|pass| std::array::from_fn(|bits| removable(bits, pass == 1)));
    let mut checks = 0_usize;
    loop {
        let mut changed = false;
        for table in &tables {
            if active.len() > budget - checks {
                return Err("Skeleton exceeds work budget; reduce its raster resolution".into());
            }
            checks += active.len();
            removed.clear();
            for &p in &active {
                let bits = usize::from(pixels[p - stride])
                    | usize::from(pixels[p - stride + 1]) << 1
                    | usize::from(pixels[p + 1]) << 2
                    | usize::from(pixels[p + stride + 1]) << 3
                    | usize::from(pixels[p + stride]) << 4
                    | usize::from(pixels[p + stride - 1]) << 5
                    | usize::from(pixels[p - 1]) << 6
                    | usize::from(pixels[p - stride - 1]) << 7;
                if table[bits] {
                    removed.push(p);
                }
            }
            // Delete together only after the complete subiteration was read.
            changed |= !removed.is_empty();
            for &p in &removed {
                pixels[p] = 0;
                queued[p] = 0;
            }
            active.retain(|&p| pixels[p] != 0);
            // All new candidates belong to the next stable subiteration.
            for &p in &removed {
                for q in neighbors(p, stride) {
                    if pixels[q] != 0 && queued[q] == 0 {
                        active.push(q);
                        queued[q] = 1;
                    }
                }
            }
        }
        if !changed {
            break;
        }
    }
    let mut result = reserved::<u8>(mask.pixels.len())?;
    for y in 0..mask.height {
        let start = (y + 1) * stride + 1;
        result.extend_from_slice(&pixels[start..start + mask.width]);
    }
    Ok(BinaryMask {
        width: mask.width,
        height: mask.height,
        pixels: result,
    })
}

fn neighbors(p: usize, stride: usize) -> [usize; 8] {
    [
        p - stride,
        p - stride + 1,
        p + 1,
        p + stride + 1,
        p + stride,
        p + stride - 1,
        p - 1,
        p - stride - 1,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    // A deliberately simple full-image reference checks that skipping the
    // interior in the optimized implementation does not alter the algorithm.
    fn full_scan_reference(mask: &BinaryMask) -> BinaryMask {
        let mut current = mask.clone();
        loop {
            let mut changed = false;
            for second in [false, true] {
                let mut next = current.clone();
                for y in 0..mask.height {
                    for x in 0..mask.width {
                        let index = y * mask.width + x;
                        if current.pixels[index] == 0 {
                            continue;
                        }
                        let mut bits = 0;
                        for (bit, (dx, dy)) in [
                            (0_isize, -1_isize),
                            (1, -1),
                            (1, 0),
                            (1, 1),
                            (0, 1),
                            (-1, 1),
                            (-1, 0),
                            (-1, -1),
                        ]
                        .into_iter()
                        .enumerate()
                        {
                            let nx = x as isize + dx;
                            let ny = y as isize + dy;
                            if nx >= 0
                                && ny >= 0
                                && nx < mask.width as isize
                                && ny < mask.height as isize
                            {
                                bits |= usize::from(
                                    current.pixels[ny as usize * mask.width + nx as usize],
                                ) << bit;
                            }
                        }
                        if removable(bits, second) {
                            next.pixels[index] = 0;
                            changed = true;
                        }
                    }
                }
                current = next;
            }
            if !changed {
                return current;
            }
        }
    }

    #[test]
    fn boundary_frontier_matches_full_scan_outputs() {
        for bits in 0..512 {
            let input =
                BinaryMask::new(3, 3, (0..9).map(|i| ((bits >> i) & 1) as u8).collect()).unwrap();
            assert_eq!(skeletonize(&input).unwrap(), full_scan_reference(&input));
        }
        let mut random = 1_u64;
        for sample in 0..512 {
            let input = BinaryMask::new(
                19,
                23,
                (0..19 * 23)
                    .map(|_| {
                        random ^= random << 13;
                        random ^= random >> 7;
                        random ^= random << 17;
                        u8::from(random % 100 < sample % 101)
                    })
                    .collect(),
            )
            .unwrap();
            assert_eq!(
                skeletonize(&input).unwrap(),
                full_scan_reference(&input),
                "sample={sample}"
            );
        }
    }

    #[test]
    fn exhausted_work_budget_returns_an_error_and_leaves_input_intact() {
        let input = BinaryMask::new(8, 8, vec![1; 64]).unwrap();
        let before = input.clone();
        assert!(
            skeletonize_with_budget(&input, 64)
                .unwrap_err()
                .contains("work budget")
        );
        assert_eq!(input, before);
    }
}
