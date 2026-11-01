use embroidery_lab::{Command, Object, Point, Project, Settings, StitchKind, dst, engine};

fn rectangle(x: f64, y: f64, w: f64, h: f64) -> Vec<Point> {
    vec![
        Point::new(x, y),
        Point::new(x + w, y),
        Point::new(x + w, y + h),
        Point::new(x, y + h),
    ]
}
fn project(rings: Vec<Vec<Point>>, angle: f64, underlay: bool) -> Project {
    Project {
        objects: vec![Object {
            contours: rings,
            settings: Settings {
                kind: StitchKind::Tatami,
                spacing_mm: 0.4,
                length_mm: 3.5,
                angle_deg: angle,
                underlay,
                tie_in: false,
                tie_off: false,
                ..Settings::default()
            },
            ..Object::default()
        }],
        ..Project::default()
    }
}
fn rotate(p: Point, angle: f64) -> Point {
    let angle = angle.to_radians();
    Point::new(
        p.x * angle.cos() - p.y * angle.sin(),
        p.x * angle.sin() + p.y * angle.cos(),
    )
}
fn assert_no_hole_sewing(stitches: &[embroidery_lab::Stitch], angle: f64) {
    let mut previous = Point::default();
    for stitch in stitches {
        if stitch.command == Command::Stitch {
            let steps = (previous.distance(stitch.point()) / 0.025).ceil().max(1.) as usize;
            for index in 0..=steps {
                let p = rotate(
                    previous.lerp(stitch.point(), index as f64 / steps as f64),
                    -angle,
                );
                assert!(
                    !(p.x > 10.000001 && p.x < 19.999999 && p.y > 5.000001 && p.y < 14.999999),
                    "sewn hole at {p:?}"
                );
            }
        }
        previous = stitch.point();
    }
}

#[test]
fn hole_sides_are_filled_as_continuous_blocks() {
    let p = project(
        vec![rectangle(0., 0., 30., 20.), rectangle(10., 5., 10., 10.)],
        0.,
        false,
    );
    let stitches = engine::generate(&p).unwrap();
    let stats = dst::stats(&stitches);
    assert!(stats.stitches > 400);
    assert!(
        stats.jumps <= 4,
        "hole should require block transfers, not a jump on every row: {}",
        stats.jumps
    );
    assert_no_hole_sewing(&stitches, 0.);
}

#[test]
fn disconnected_islands_do_not_interleave_or_sew_empty_space() {
    let p = project(
        vec![rectangle(0., 0., 20., 20.), rectangle(30., 0., 20., 20.)],
        0.,
        false,
    );
    let stitches = engine::generate(&p).unwrap();
    assert_eq!(dst::stats(&stitches).jumps, 2);
    let mut previous = Point::default();
    for stitch in stitches {
        if stitch.command == Command::Stitch {
            for index in 0..=20 {
                let p = previous.lerp(stitch.point(), index as f64 / 20.);
                assert!(
                    p.x <= 20.000001 || p.x >= 29.999999,
                    "sewing between islands at {p:?}"
                );
            }
        }
        previous = stitch.point();
    }
}

#[test]
fn rotated_block_routing_preserves_holes_and_quantized_length() {
    for angle in [0., 27., 90., 137.] {
        for underlay in [false, true] {
            let rings = [rectangle(0., 0., 30., 20.), rectangle(10., 5., 10., 10.)]
                .into_iter()
                .map(|ring| ring.into_iter().map(|p| rotate(p, angle)).collect())
                .collect();
            let p = project(rings, angle, underlay);
            let stitches = engine::generate(&p).unwrap();
            assert_no_hole_sewing(&stitches, angle);
            let decoded = dst::decode(&dst::encode(&stitches, "route").unwrap()).unwrap();
            assert!(dst::stats(&decoded).max_stitch_mm <= 3.5 + 1e-8);
            assert_eq!(decoded.last().unwrap().command, Command::End);
        }
    }
}

#[test]
fn tatami_stagger_uses_physical_rows_and_grid_across_blocks() {
    let mut p = project(
        vec![rectangle(0., 0., 24., 4.), rectangle(30., 0., 24., 4.)],
        0.,
        false,
    );
    p.objects[0].settings.spacing_mm = 1.;
    p.objects[0].settings.length_mm = 4.15; // 4mm sewing grid after DST reserve.
    let stitches = engine::generate(&p).unwrap();
    for (row, phase) in [0., 2., 1., 3.].into_iter().enumerate() {
        for (lo, hi) in [(4., 20.), (34., 50.)] {
            let punctures: Vec<_> = stitches
                .iter()
                .filter(|s| {
                    s.command == Command::Stitch
                        && (s.y - (row as f64 + 0.5)).abs() < 1e-8
                        && s.x > lo + 1e-8
                        && s.x < hi - 1e-8
                })
                .collect();
            assert!(
                punctures.len() >= 2,
                "missing punctures on row {row} in {lo}..{hi}"
            );
            for s in punctures {
                assert!(
                    ((s.x - phase).rem_euclid(4.)).min((phase - s.x).rem_euclid(4.)) < 1e-7,
                    "row {row} phase {phase}: {s:?}"
                );
            }
        }
    }
}

#[test]
fn split_merge_routing_keeps_every_scanline_span() {
    let p = project(
        vec![
            rectangle(0., 0., 40., 24.),
            rectangle(8., 4., 6., 16.),
            rectangle(23., 8., 8., 8.),
        ],
        0.,
        false,
    );
    let stitches = engine::generate(&p).unwrap();
    let spans = engine::fill_spans(&p.objects[0].contours, 0., 0.4, 0.).unwrap();
    let sewn: Vec<_> = stitches
        .windows(2)
        .filter(|pair| pair[1].command == Command::Stitch && (pair[0].y - pair[1].y).abs() < 1e-7)
        .map(|pair| (pair[0].point(), pair[1].point()))
        .collect();
    for (a, b) in spans {
        for fraction in [0.1, 0.5, 0.9] {
            let p = a.lerp(b, fraction);
            assert!(
                sewn.iter().any(|&(c, d)| (c.y - p.y).abs() < 1e-7
                    && p.x >= c.x.min(d.x) - 1e-7
                    && p.x <= c.x.max(d.x) + 1e-7),
                "missing coverage at {p:?}"
            );
        }
    }
}

#[test]
fn fragmented_jpg_does_not_trade_fewer_blocks_for_longer_jump_travel() {
    use embroidery_lab::import::{RasterOptions, load_image_with_options};
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../tests/fixtures/synthetic_two_colors.jpg");
    let options = RasterOptions {
        max_dimension: 512,
        palette_size: 8,
        ..RasterOptions::default()
    };
    let p = load_image_with_options(&source, 100., &options).unwrap();
    let decoded =
        dst::decode(&dst::encode(&engine::generate(&p).unwrap(), "regression").unwrap()).unwrap();
    let stats = dst::stats(&decoded);
    // Independently measured v0.2 CLI baseline for this checked-in synthetic
    // JPG and these exact options: 183 records / 437.9276066979308mm.
    assert!(stats.jumps <= 183, "regressed jump count: {}", stats.jumps);
    assert!(
        stats.jump_length_mm <= 437.9276066979308 + 1e-7,
        "regressed non-sewing distance: {}",
        stats.jump_length_mm
    );
    assert!(stats.max_stitch_mm <= 3.5 + 1e-8);
}
