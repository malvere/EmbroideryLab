use embroidery_lab::thinning::{BinaryMask, skeletonize};

fn mask(rows: &[&str]) -> BinaryMask {
    BinaryMask::new(
        rows[0].len(),
        rows.len(),
        rows.iter()
            .flat_map(|row| row.bytes().map(|b| u8::from(b == b'#')))
            .collect(),
    )
    .unwrap()
}

// Independent flood-fill topology oracle. The extra outside ring is important
// when an object or hole touches an image edge.
fn topology(input: &BinaryMask) -> (usize, usize) {
    let width = input.width + 2;
    let height = input.height + 2;
    let mut pixels = vec![0; width * height];
    for y in 0..input.height {
        pixels[(y + 1) * width + 1..(y + 1) * width + input.width + 1]
            .copy_from_slice(&input.pixels[y * input.width..(y + 1) * input.width]);
    }
    let mut seen = vec![false; pixels.len()];
    let mut components = [0, 0];
    for start in 0..pixels.len() {
        if seen[start] {
            continue;
        }
        let value = pixels[start];
        components[usize::from(value)] += 1;
        let mut pending = vec![start];
        seen[start] = true;
        while let Some(p) = pending.pop() {
            let x = p % width;
            let y = p / width;
            for dy in -1_isize..=1 {
                for dx in -1_isize..=1 {
                    if dx == 0 && dy == 0 || value == 0 && dx != 0 && dy != 0 {
                        continue;
                    }
                    let nx = x as isize + dx;
                    let ny = y as isize + dy;
                    if nx < 0 || ny < 0 || nx >= width as isize || ny >= height as isize {
                        continue;
                    }
                    let q = ny as usize * width + nx as usize;
                    if !seen[q] && pixels[q] == value {
                        seen[q] = true;
                        pending.push(q);
                    }
                }
            }
        }
    }
    (components[1], components[0] - 1)
}

#[test]
fn exhaustive_four_by_four_masks_preserve_topology_and_reach_a_fixed_point() {
    for bits in 0_u32..(1 << 16) {
        let input =
            BinaryMask::new(4, 4, (0..16).map(|i| ((bits >> i) & 1) as u8).collect()).unwrap();
        let output = skeletonize(&input).unwrap();
        assert_eq!(topology(&input), topology(&output), "mask={bits:#06x}");
        assert!(
            input.pixels.iter().zip(&output.pixels).all(|(a, b)| b <= a),
            "mask={bits:#06x}"
        );
        assert_eq!(skeletonize(&output).unwrap(), output, "mask={bits:#06x}");
    }
}

#[test]
fn singleton_tiny_square_lines_and_empty_mask_do_not_disappear() {
    let singleton = mask(&["#"]);
    assert_eq!(skeletonize(&singleton).unwrap(), singleton);
    let empty = mask(&["...", "..."]);
    assert_eq!(skeletonize(&empty).unwrap(), empty);
    for input in [
        mask(&["##", "##"]),
        mask(&["####"]),
        mask(&["#", "#", "#", "#"]),
        mask(&["#...", ".#..", "..#.", "...#"]),
    ] {
        let output = skeletonize(&input).unwrap();
        assert_eq!(topology(&input), topology(&output));
        assert!(output.pixels.contains(&1));
    }
}

#[test]
fn holed_region_at_the_border_and_disconnected_islands_survive() {
    let input = mask(&[
        "#######...###",
        "#######...###",
        "##...##...###",
        "##...##.....#",
        "##...##.....#",
        "#######.....#",
        "#######.....#",
    ]);
    let output = skeletonize(&input).unwrap();
    assert_eq!(topology(&input), (2, 1));
    assert_eq!(topology(&output), (2, 1));
    assert!(output.pixels.iter().sum::<u8>() < input.pixels.iter().sum::<u8>());
    assert_eq!(skeletonize(&input).unwrap(), output);
}

#[test]
fn thin_branch_endpoints_remain_and_thick_cross_has_four_arms() {
    let thin = mask(&["..#..", "..#..", "#####", "..#..", "..#.."]);
    assert_eq!(skeletonize(&thin).unwrap(), thin);
    let thick = mask(&[
        "....###....",
        "....###....",
        "....###....",
        "....###....",
        "###########",
        "###########",
        "###########",
        "....###....",
        "....###....",
        "....###....",
        "....###....",
    ]);
    let output = skeletonize(&thick).unwrap();
    assert_eq!(topology(&output), (1, 0));
    let points: Vec<_> = output
        .pixels
        .iter()
        .enumerate()
        .filter(|(_, p)| **p == 1)
        .map(|(i, _)| (i % output.width, i / output.width))
        .collect();
    assert!(points.iter().any(|&(x, _)| x <= 1));
    assert!(points.iter().any(|&(x, _)| x >= 9));
    assert!(points.iter().any(|&(_, y)| y <= 1));
    assert!(points.iter().any(|&(_, y)| y >= 9));
}

#[test]
fn practical_512_pixel_solid_object_converges_without_losing_its_component() {
    let input = BinaryMask::new(512, 512, vec![1; 512 * 512]).unwrap();
    let output = skeletonize(&input).unwrap();
    assert_eq!(topology(&output), (1, 0));
    assert!(output.pixels.iter().filter(|&&p| p == 1).count() < 1024);
}

#[test]
fn malformed_masks_fail_without_mutating_the_input() {
    for input in [
        BinaryMask {
            width: 0,
            height: 1,
            pixels: vec![],
        },
        BinaryMask {
            width: 1,
            height: 1,
            pixels: vec![2],
        },
        BinaryMask {
            width: 2,
            height: 1,
            pixels: vec![1],
        },
        BinaryMask {
            width: usize::MAX,
            height: 2,
            pixels: vec![],
        },
        BinaryMask {
            width: 2048,
            height: 2048,
            pixels: vec![],
        },
    ] {
        let before = input.clone();
        assert!(skeletonize(&input).is_err());
        assert_eq!(input, before);
    }
}
