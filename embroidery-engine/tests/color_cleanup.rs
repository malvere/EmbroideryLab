use embroidery_lab::{
    Point,
    color_selection::{self, CleanupMask, SelectionCache, SelectionMode, SelectionOp},
    import::{self, BackgroundMode, RasterOptions},
};
use image::{DynamicImage, Rgba, RgbaImage};
use std::{path::Path, sync::atomic::AtomicBool};

fn flag() -> RgbaImage {
    let mut image = RgbaImage::from_pixel(80, 60, Rgba([255; 4]));
    for y in 8..52 {
        for x in 10..70 {
            let p = if x == 10 || x == 69 || y == 8 || y == 51 {
                [0, 0, 0, 255]
            } else if y < 24 {
                [220, 25, 45, 255]
            } else if y < 38 {
                [255; 4]
            } else {
                [20, 50, 190, 255]
            };
            image.put_pixel(x, y, Rgba(p));
        }
    }
    image
}

fn options() -> RasterOptions {
    RasterOptions {
        max_dimension: 512,
        palette_size: 8,
        min_region_pixels: 1,
        min_region_area_mm2: 0.,
        ..RasterOptions::autodigitize()
    }
}

fn contains(rings: &[Vec<Point>], point: Point) -> bool {
    let mut inside = false;
    for ring in rings {
        for (&a, &b) in ring
            .iter()
            .zip(ring.iter().cycle().skip(1))
            .take(ring.len())
        {
            if (a.y > point.y) != (b.y > point.y)
                && point.x < (b.x - a.x) * (point.y - a.y) / (b.y - a.y) + a.x
            {
                inside = !inside;
            }
        }
    }
    inside
}

#[test]
fn exterior_white_is_removed_while_internal_white_and_original_pixels_survive() {
    let image = flag();
    let immutable = image.clone();
    let cache = SelectionCache::new(&image, [1, 1]).unwrap();
    let candidates = cache.candidates(0.1, SelectionMode::Connected);
    assert!(candidates[0]);
    assert!(!candidates[30 * 80 + 40]);
    assert_eq!(candidates, cache.candidates(0.1, SelectionMode::Exterior));
    assert!(cache.candidates(0.1, SelectionMode::Global)[30 * 80 + 40]);
    let cleanup = CleanupMask::from_masks(80, 60, &candidates, &vec![false; 4800]).unwrap();
    let cleaned = cleanup.apply(&image).unwrap();
    assert_eq!(image, immutable);
    assert_eq!(cleaned.get_pixel(1, 1)[3], 0);
    assert_eq!(cleaned.get_pixel(40, 30), &Rgba([255; 4]));
    let mut settings = options();
    settings.palette_size = 4;
    settings.cleanup = Some(cleanup);
    let project = import::load_pixels_with_options(
        &DynamicImage::ImageRgba8(image),
        Path::new("flag.png"),
        80.,
        &settings,
    )
    .unwrap();
    assert_eq!(project.objects.len(), 4);
    assert!(
        project
            .objects
            .iter()
            .any(|o| o.color == [255; 3] && contains(&o.contours, Point::new(40., 30.)))
    );
    assert!(
        !project
            .objects
            .iter()
            .any(|o| contains(&o.contours, Point::new(1., 1.)))
    );
    let recipe = project.raster_recipe.unwrap();
    assert_eq!(recipe.options, settings);
}

#[test]
fn frozen_base_makes_threshold_reversal_and_subtraction_reversible() {
    let image = RgbaImage::from_fn(64, 1, |x, _| {
        Rgba([200 + x as u8 / 2, 200 + x as u8 / 2, 200 + x as u8 / 2, 255])
    });
    let cache = SelectionCache::new(&image, [0, 0]).unwrap();
    let base = vec![true; 64];
    let narrow = cache.candidates(2., SelectionMode::Connected);
    let wide = cache.candidates(8., SelectionMode::Connected);
    assert!(narrow.iter().zip(&wide).all(|(&a, &b)| !a || b));
    let small = color_selection::combine_selection(&base, &narrow, SelectionOp::Subtract).unwrap();
    let large = color_selection::combine_selection(&base, &wide, SelectionOp::Subtract).unwrap();
    assert!(large.iter().zip(&small).all(|(&a, &b)| !a || b));
    assert_eq!(
        small,
        color_selection::combine_selection(
            &base,
            &cache.candidates(2., SelectionMode::Connected),
            SelectionOp::Subtract
        )
        .unwrap()
    );
    assert_eq!(cache.seed, [0, 0]);
    assert_eq!(cache.seed_rgb, [200; 3]);
}

#[test]
fn fixed_range_flood_does_not_drift_along_gradient_or_diagonal_contact() {
    let image = RgbaImage::from_fn(100, 1, |x, _| {
        Rgba([x as u8 * 2, x as u8 * 2, x as u8 * 2, 255])
    });
    let cache = SelectionCache::new(&image, [0, 0]).unwrap();
    assert!(!cache.candidates(10., SelectionMode::Connected)[99]);
    let mut diagonal = RgbaImage::from_pixel(3, 3, Rgba([0, 0, 0, 255]));
    for i in 0..3 {
        diagonal.put_pixel(i, i, Rgba([255; 4]));
    }
    let cache = SelectionCache::new(&diagonal, [0, 0]).unwrap();
    let connected = cache.candidates(0.1, SelectionMode::Connected);
    let exterior = cache.candidates(0.1, SelectionMode::Exterior);
    assert!(connected[0]);
    assert!(!connected[4]);
    assert!(!connected[8]);
    assert!(exterior[0]);
    assert!(!exterior[4]);
    assert!(exterior[8]);
}

#[test]
fn protection_blocks_a_gap_and_restoration_wins_over_exclusion() {
    let mut image = flag();
    image.put_pixel(10, 30, Rgba([255; 4]));
    let cache = SelectionCache::new(&image, [1, 1]).unwrap();
    assert!(cache.candidates(0.1, SelectionMode::Connected)[30 * 80 + 40]);
    let mut protected = vec![false; 4800];
    protected[30 * 80 + 10] = true;
    let mask = cache.candidates_with_protection(0.1, SelectionMode::Connected, &protected);
    assert!(!mask[30 * 80 + 40]);
    let all = vec![true; 4800];
    let cleanup = CleanupMask::from_masks(80, 60, &all, &protected).unwrap();
    assert_eq!(cleanup.apply(&image).unwrap().get_pixel(10, 30)[3], 255);
    color_selection::paint_mask(&mut protected, 80, 60, [40., 30.], 2., true).unwrap();
    assert!(protected[30 * 80 + 40]);
    color_selection::paint_mask(&mut protected, 80, 60, [40., 30.], 2., false).unwrap();
    assert!(!protected[30 * 80 + 40]);
}

#[test]
fn halo_preview_is_local_and_preserves_black_outline_and_protection() {
    let mut image = flag();
    for y in 8..52 {
        image.put_pixel(9, y, Rgba([230, 230, 230, 255]));
    }
    let mut excluded = vec![false; 4800];
    for y in 0..60 {
        for x in 0..9 {
            excluded[y * 80 + x] = true;
        }
    }
    let mut protected = vec![false; 4800];
    protected[20 * 80 + 9] = true;
    let preview =
        color_selection::light_halo_preview(&image, &excluded, &protected, 2, 15.).unwrap();
    assert!(preview[21 * 80 + 9]);
    assert!(!preview[20 * 80 + 9]);
    assert!(!preview[21 * 80 + 10]);
    assert!(!preview[30 * 80 + 40]);
    assert!(!excluded[21 * 80 + 9]);
}

#[test]
fn hidden_rgb_cannot_contaminate_alpha_resize_or_palette() {
    let first = RgbaImage::from_fn(64, 32, |x, _| {
        if x < 32 {
            Rgba([200, 20, 30, 255])
        } else {
            Rgba([0, 255, 0, 0])
        }
    });
    let second = RgbaImage::from_fn(64, 32, |x, _| {
        if x < 32 {
            Rgba([200, 20, 30, 255])
        } else {
            Rgba([255, 0, 255, 0])
        }
    });
    assert_eq!(
        color_selection::thumbnail_rgba(&first, 16),
        color_selection::thumbnail_rgba(&second, 16)
    );
    let settings = RasterOptions {
        max_dimension: 16,
        palette_size: 2,
        ..options()
    };
    let a = import::load_pixels_with_options(
        &DynamicImage::ImageRgba8(first),
        Path::new("alpha.png"),
        64.,
        &settings,
    )
    .unwrap();
    let b = import::load_pixels_with_options(
        &DynamicImage::ImageRgba8(second),
        Path::new("alpha.png"),
        64.,
        &settings,
    )
    .unwrap();
    assert_eq!(
        serde_json::to_value(&a.objects).unwrap(),
        serde_json::to_value(&b.objects).unwrap()
    );
    assert_eq!(a.objects.len(), 1);
    assert_eq!(a.objects[0].color, [200, 20, 30]);
}

#[test]
fn excluded_background_does_not_take_a_palette_slot() {
    let mut image = flag();
    // Replace the useful interior white with red, leaving only exterior white.
    for y in 24..38 {
        for x in 11..69 {
            image.put_pixel(x, y, Rgba([220, 25, 45, 255]));
        }
    }
    let mask = SelectionCache::new(&image, [0, 0])
        .unwrap()
        .candidates(0.1, SelectionMode::Exterior);
    let mut settings = options();
    settings.palette_size = 3;
    settings.cleanup = Some(CleanupMask::from_masks(80, 60, &mask, &vec![false; 4800]).unwrap());
    let (project, report) = import::load_pixels_with_report(
        &DynamicImage::ImageRgba8(image),
        Path::new("palette.png"),
        80.,
        &settings,
    )
    .unwrap();
    assert_eq!(report.palette.len(), 3);
    assert!(!project.objects.iter().any(|o| o.color == [255; 3]));
    for rgb in [[0; 3], [220, 25, 45], [20, 50, 190]] {
        assert!(report.palette.contains(&rgb));
    }
}

#[test]
fn physical_component_threshold_is_consistent_across_source_resolutions() {
    let settings = RasterOptions {
        min_region_area_mm2: 0.5,
        ..options()
    };
    for scale in [1, 2, 4] {
        let mut image = RgbaImage::new(40 * scale, 40 * scale);
        for y in 2 * scale..12 * scale {
            for x in 2 * scale..12 * scale {
                image.put_pixel(x, y, Rgba([30, 70, 180, 255]));
            }
        }
        for y in 25 * scale..26 * scale {
            for x in 25 * scale..26 * scale {
                image.put_pixel(x, y, Rgba([30, 70, 180, 255]));
            }
        }
        let (project, report) = import::load_pixels_with_report(
            &DynamicImage::ImageRgba8(image),
            Path::new("area.png"),
            20.,
            &settings,
        )
        .unwrap();
        assert_eq!(project.objects.len(), 1);
        assert_eq!(report.omitted_regions.len(), 1);
        assert!((report.omitted_regions[0].area_mm2 - 0.25).abs() < 1e-9);
        let issues = project.raster_recipe.unwrap().issues;
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].point, Some(Point::new(12.5, 12.5)));
    }
}

#[test]
fn canonical_shared_boundaries_retain_t_junction_and_occupancy_after_smoothing_request() {
    let image = RgbaImage::from_fn(32, 32, |x, y| {
        Rgba(if x < 16 {
            [220, 30, 40, 255]
        } else if y < 16 {
            [20, 60, 190, 255]
        } else {
            [20, 180, 40, 255]
        })
    });
    let settings = RasterOptions {
        palette_size: 3,
        smoothing_iterations: 3,
        simplify_tolerance_mm: 1.,
        ..options()
    };
    let (project, report) = import::load_pixels_with_report(
        &DynamicImage::ImageRgba8(image),
        Path::new("junction.png"),
        32.,
        &settings,
    )
    .unwrap();
    assert_eq!(project.objects.len(), 3);
    assert_eq!(report.shared_boundary_regions, 3);
    for y in 0..32 {
        for x in 0..32 {
            assert_eq!(
                project
                    .objects
                    .iter()
                    .filter(|o| contains(&o.contours, Point::new(x as f64 + 0.5, y as f64 + 0.5)))
                    .count(),
                1
            );
        }
    }
    assert!(project.objects[1].contours[0].contains(&Point::new(16., 16.)));
    assert!(project.objects[2].contours[0].contains(&Point::new(16., 16.)));
}

#[test]
fn cleanup_encoding_and_invalid_dimensions_are_checked() {
    let excluded = vec![false, true, true, false, true, true];
    let protected = vec![false, false, true, false, false, false];
    let mask = CleanupMask::from_masks(3, 2, &excluded, &protected).unwrap();
    assert_eq!(mask.excluded_runs, vec![[1, 2], [4, 2]]);
    let roundtrip: CleanupMask =
        serde_json::from_str(&serde_json::to_string(&mask).unwrap()).unwrap();
    assert_eq!(roundtrip.to_masks().unwrap(), (excluded, protected));
    assert!(mask.apply(&RgbaImage::new(2, 3)).is_err());
    let invalid = CleanupMask {
        width: 3,
        height: 2,
        excluded_runs: vec![[5, 2]],
        protected_runs: vec![],
    };
    assert!(invalid.validate().is_err());
}

#[test]
fn oklab_conversion_and_logical_gesture_scale_have_known_limits() {
    for rgb in [
        [255; 3],
        [0; 3],
        [255, 0, 0],
        [20, 60, 190],
        [230, 220, 120],
    ] {
        assert_eq!(
            color_selection::oklab_to_srgb(color_selection::srgb_to_oklab(rgb)),
            rgb
        );
    }
    let white = color_selection::srgb_to_oklab([255; 3]);
    assert!((white[0] - 1.).abs() < 1e-6);
    assert!(white[1].abs() < 1e-6);
    assert!(white[2].abs() < 1e-6);
    assert_eq!(color_selection::gesture_tolerance(5., 0.), 5.);
    assert_eq!(color_selection::gesture_tolerance(5., 3.), 5.);
    assert!((color_selection::gesture_tolerance(5., 103.) - 17.).abs() < 1e-5);
    assert_eq!(color_selection::gesture_tolerance(5., 10000.), 100.);
}

#[test]
fn tracing_honors_cancel_before_document_construction_and_new_profile_is_explicit() {
    assert_eq!(
        RasterOptions::default().background,
        BackgroundMode::NearWhite
    );
    assert_eq!(
        RasterOptions::autodigitize().background,
        BackgroundMode::TransparentOnly
    );
    let cancelled = AtomicBool::new(true);
    let result = import::load_pixels_with_report_cancellable(
        &DynamicImage::ImageRgba8(flag()),
        Path::new("cancel.png"),
        80.,
        &options(),
        &cancelled,
    );
    assert!(result.unwrap_err().contains("cancelled"));
}

#[test]
fn thumbnail_rounding_does_not_change_physical_source_aspect_ratio() {
    let image = DynamicImage::ImageRgba8(RgbaImage::from_pixel(31, 3, Rgba([20, 70, 180, 255])));
    let settings = RasterOptions {
        max_dimension: 16,
        ..options()
    };
    let project =
        import::load_pixels_with_options(&image, Path::new("aspect.png"), 31., &settings).unwrap();
    let max_y = project.objects[0]
        .contours
        .iter()
        .flatten()
        .map(|p| p.y)
        .fold(0., f64::max);
    assert!((max_y - 3.).abs() < 1e-9);
}

#[test]
fn explicit_denoise_reassigns_only_isolated_opaque_labels_and_reports_it() {
    let mut image = RgbaImage::from_pixel(32, 32, Rgba([20, 70, 180, 255]));
    image.put_pixel(5, 5, Rgba([255; 4]));
    image.put_pixel(10, 10, Rgba([255; 4]));
    image.put_pixel(16, 16, Rgba([0; 4]));
    for x in 20..28 {
        image.put_pixel(x, 20, Rgba([255; 4]));
    }
    let mut protected = vec![false; 1024];
    protected[10 * 32 + 10] = true;
    let cleanup = CleanupMask::from_masks(32, 32, &vec![false; 1024], &protected).unwrap();
    let settings = RasterOptions {
        denoise_passes: 1,
        cleanup: Some(cleanup),
        palette_size: 2,
        ..options()
    };
    let (project, report) = import::load_pixels_with_report(
        &DynamicImage::ImageRgba8(image),
        Path::new("denoise.png"),
        32.,
        &settings,
    )
    .unwrap();
    assert_eq!(report.relabeled_noise_pixels, 1);
    assert!(
        project
            .objects
            .iter()
            .any(|o| o.color == [255; 3] && contains(&o.contours, Point::new(10.5, 10.5)))
    );
    assert!(
        project
            .objects
            .iter()
            .any(|o| o.color == [255; 3] && contains(&o.contours, Point::new(24.5, 20.5)))
    );
    assert!(
        !project
            .objects
            .iter()
            .any(|o| contains(&o.contours, Point::new(16.5, 16.5)))
    );
    assert!(
        project
            .raster_recipe
            .unwrap()
            .issues
            .iter()
            .any(|w| w.message.contains("denoise reassigned 1"))
    );
}

#[test]
fn selection_worker_accepts_cooperative_cancellation_at_cache_and_mask_stages() {
    let image = flag();
    let cancelled = AtomicBool::new(true);
    assert!(
        SelectionCache::new_cancellable(&image, [0, 0], &cancelled)
            .unwrap_err()
            .contains("cancelled")
    );
    let cache = SelectionCache::new(&image, [0, 0]).unwrap();
    assert!(
        cache
            .candidates_with_protection_cancellable(5., SelectionMode::Exterior, &[], &cancelled)
            .unwrap_err()
            .contains("cancelled")
    );
}

#[test]
fn pinned_thread_colour_is_exact_while_remaining_slot_is_learned_without_removed_white() {
    let red = [220, 25, 45];
    let blue = [20, 50, 190];
    let mut image = RgbaImage::from_pixel(64, 32, Rgba([255; 4]));
    for y in 5..27 {
        for x in 5..59 {
            let rgb = if x < 22 {
                red
            } else if x < 38 {
                [218, 28, 46]
            } else {
                blue
            };
            image.put_pixel(x, y, Rgba([rgb[0], rgb[1], rgb[2], 255]));
        }
    }
    let mask = SelectionCache::new(&image, [0, 0])
        .unwrap()
        .candidates(0.1, SelectionMode::Exterior);
    let settings = RasterOptions {
        palette_size: 2,
        pinned_colors: vec![red],
        cleanup: Some(CleanupMask::from_masks(64, 32, &mask, &vec![false; 2048]).unwrap()),
        ..options()
    };
    let (project, report) = import::load_pixels_with_report(
        &DynamicImage::ImageRgba8(image),
        Path::new("pinned.png"),
        64.,
        &settings,
    )
    .unwrap();
    assert_eq!(report.palette.len(), 2);
    assert_eq!(report.palette[0], red);
    assert!(report.palette.contains(&blue));
    assert!(!report.palette.contains(&[255; 3]));
    assert!(
        project
            .objects
            .iter()
            .any(|o| o.color == red && contains(&o.contours, Point::new(30., 16.)))
    );
    assert_eq!(
        project.raster_recipe.unwrap().options.pinned_colors,
        vec![red]
    );
}

#[test]
fn fully_pinned_palette_is_fixed_and_invalid_pins_fail_before_tracing() {
    let image = DynamicImage::ImageRgba8(RgbaImage::from_pixel(32, 32, Rgba([150, 150, 150, 255])));
    let settings = RasterOptions {
        palette_size: 2,
        pinned_colors: vec![[0; 3], [255; 3]],
        ..options()
    };
    let (_, report) =
        import::load_pixels_with_report(&image, Path::new("fixed.png"), 32., &settings).unwrap();
    assert_eq!(report.palette, vec![[0; 3], [255; 3]]);
    assert!(
        RasterOptions {
            pinned_colors: vec![[0; 3], [255; 3], [255, 0, 0]],
            ..settings.clone()
        }
        .validate()
        .is_err()
    );
    assert!(
        RasterOptions {
            pinned_colors: vec![[0; 3], [0; 3]],
            ..settings
        }
        .validate()
        .is_err()
    );
}

#[test]
fn diagonal_void_contacts_become_simple_cycles_without_changing_filled_cells() {
    use geo::{Coord, LineString, Polygon, Validation};
    // The central hole touches the missing corner only at a grid vertex.
    // The remaining foreground is one four-connected region with a pinched
    // closed boundary walk. It must not become a self-intersecting ring.
    let mut image = RgbaImage::from_pixel(5, 5, Rgba([20, 70, 180, 255]));
    for (x, y) in [(0, 0), (1, 1), (2, 2)] {
        image.put_pixel(x, y, Rgba([0; 4]));
    }
    let (project, _) = import::load_pixels_with_report(
        &DynamicImage::ImageRgba8(image.clone()),
        Path::new("diagonal-voids.png"),
        5.,
        &options(),
    )
    .unwrap();
    assert_eq!(project.objects.len(), 1);
    for ring in &project.objects[0].contours {
        let mut points: Vec<_> = ring.iter().map(|p| Coord { x: p.x, y: p.y }).collect();
        points.push(points[0]);
        Polygon::new(LineString::new(points), vec![])
            .check_validation()
            .unwrap();
    }
    for y in 0..5 {
        for x in 0..5 {
            assert_eq!(
                contains(
                    &project.objects[0].contours,
                    Point::new(x as f64 + 0.5, y as f64 + 0.5)
                ),
                image.get_pixel(x, y)[3] != 0,
                "cell{x},{y}"
            );
        }
    }
    let mut sewn = project.clone();
    sewn.objects[0].settings.underlay = false;
    embroidery_lab::engine::generate(&sewn).unwrap();
}

#[test]
fn more_than_ten_thousand_omitted_specks_have_exact_totals_and_bounded_locations() {
    let mut image = RgbaImage::new(256, 256);
    for y in 0..32 {
        for x in 0..32 {
            image.put_pixel(x, y, Rgba([20, 70, 180, 255]));
        }
    }
    for y in (0..256).step_by(2) {
        for x in (64..256).step_by(2) {
            image.put_pixel(x, y, Rgba([20, 70, 180, 255]));
        }
    }
    let expected_specks = 128 * 96;
    for physical_threshold in [false, true] {
        let settings = RasterOptions {
            min_region_pixels: if physical_threshold { 1 } else { 2 },
            min_region_area_mm2: if physical_threshold { 2. } else { 0. },
            ..options()
        };
        let (project, report) = import::load_pixels_with_report(
            &DynamicImage::ImageRgba8(image.clone()),
            Path::new("bounded-specks.png"),
            256.,
            &settings,
        )
        .unwrap();
        assert_eq!(project.objects.len(), 1);
        assert_eq!(report.omitted_region_count, expected_specks);
        assert_eq!(report.omitted_pixel_count, expected_specks);
        assert_eq!(report.omitted_area_mm2, expected_specks as f64);
        assert_eq!(
            report.omitted_regions.len(),
            import::MAX_OMITTED_REGION_DETAILS
        );
        assert!(!report.omitted_region_details_complete);
        assert_eq!(report.foreground_pixels, expected_specks + 32 * 32);
        assert!(contains(&project.objects[0].contours, Point::new(16., 16.)));
        assert!(!contains(
            &project.objects[0].contours,
            Point::new(64.5, 0.5)
        ));
        let recipe = project.raster_recipe.as_ref().unwrap();
        assert_eq!(recipe.options, settings);
        assert_eq!(recipe.issues.len(), import::MAX_OMITTED_REGION_DETAILS + 1);
        let summary = &recipe.issues.last().unwrap().message;
        assert!(
            summary.contains("12288 regions")
                && summary.contains("first 512 locations")
                && summary.contains("11776 further locations")
        );
        assert_eq!(
            project
                .source_image
                .as_ref()
                .unwrap()
                .decode()
                .unwrap()
                .to_rgba8(),
            image
        );
        let path = std::env::temp_dir().join(format!(
            "embroidery-bounded-specks-{}-{physical_threshold}.elab",
            std::process::id()
        ));
        project.save(&path).unwrap();
        let reopened = embroidery_lab::Project::load(&path).unwrap();
        assert_eq!(reopened.raster_recipe.as_ref().unwrap().issues.len(), 513);
        assert_eq!(
            serde_json::to_value(&project.objects).unwrap(),
            serde_json::to_value(&reopened.objects).unwrap()
        );
    }
}
