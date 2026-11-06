use embroidery_lab::{
    Command, Point, Project, dst, engine,
    import::{self, BackgroundMode, RasterOptions},
};
use std::{collections::BTreeSet, path::PathBuf};

fn image_path(name: &str) -> PathBuf {
    let folder =
        std::env::temp_dir().join(format!("embroidery-import-options-{}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    folder.join(name)
}
fn options() -> RasterOptions {
    RasterOptions {
        max_dimension: 512,
        palette_size: 8,
        min_region_pixels: 1,
        simplify_tolerance_mm: 0.0,
        ..RasterOptions::default()
    }
}
fn ring_area(ring: &[Point]) -> f64 {
    ring.iter()
        .zip(ring.iter().cycle().skip(1))
        .take(ring.len())
        .map(|(a, b)| a.x * b.y - b.x * a.y)
        .sum::<f64>()
        .abs()
        * 0.5
}
fn colors(project: &Project) -> BTreeSet<[u8; 3]> {
    project.objects.iter().map(|object| object.color).collect()
}

#[test]
fn explicit_background_removes_gold_and_keeps_a_real_hole() {
    let path = image_path("gold-logo.png");
    let gold = [204, 161, 65, 255];
    let mut image = image::RgbaImage::from_pixel(96, 64, image::Rgba(gold));
    for y in 8..56 {
        for x in 8..56 {
            if !(24..40).contains(&x) || !(24..40).contains(&y) {
                image.put_pixel(x, y, image::Rgba([20, 50, 190, 255]));
            }
        }
    }
    for y in 20..40 {
        for x in 70..86 {
            image.put_pixel(x, y, image::Rgba([190, 30, 40, 255]));
        }
    }
    image.save(&path).unwrap();
    let mut setting = options();
    setting.palette_size = 2;
    setting.background = BackgroundMode::Color {
        rgb: [204, 161, 65],
        tolerance: 3.,
    };
    let project = import::load_image_with_options(&path, 96., &setting).unwrap();
    assert_eq!(project.objects.len(), 2);
    assert_eq!(colors(&project).len(), 2);
    assert!(project.objects.iter().any(|o| o.contours.len() == 2));
    assert!(!colors(&project).contains(&[204, 161, 65]));
    setting.background = BackgroundMode::NearWhite;
    assert!(
        import::load_image_with_options(&path, 96., &setting)
            .unwrap()
            .objects
            .len()
            > 2
    );
}

#[test]
fn multiple_explicit_background_colors_are_a_union_without_implicit_white() {
    let path = image_path("multiple-backgrounds.png");
    let mut image = image::RgbaImage::from_pixel(64, 64, image::Rgba([255, 255, 255, 255]));
    for y in 4..60 {
        for x in 4..60 {
            image.put_pixel(x, y, image::Rgba([204, 161, 65, 255]));
        }
    }
    for y in 12..52 {
        for x in 12..52 {
            if !(24..40).contains(&x) || !(24..40).contains(&y) {
                image.put_pixel(x, y, image::Rgba([20, 50, 190, 255]));
            }
        }
    }
    image.save(&path).unwrap();
    let mut setting = options();
    setting.background = BackgroundMode::Colors {
        rgb: vec![[255, 255, 255], [204, 161, 65]],
        tolerance: 3.,
    };
    let p = import::load_image_with_options(&path, 64., &setting).unwrap();
    assert_eq!(p.objects.len(), 1);
    assert_eq!(p.objects[0].contours.len(), 2);
    assert_eq!(p.objects[0].color, [20, 50, 190]);
    setting.background = BackgroundMode::Colors {
        rgb: vec![[204, 161, 65]],
        tolerance: 3.,
    };
    assert!(
        colors(&import::load_image_with_options(&path, 64., &setting).unwrap())
            .contains(&[255, 255, 255])
    );
    setting.background = BackgroundMode::Colors {
        rgb: vec![],
        tolerance: 3.,
    };
    assert!(setting.validate().is_err());
}

#[test]
fn transparent_only_retains_white_art_and_never_upscales_small_input() {
    let path = image_path("white-foreground.png");
    let mut image = image::RgbaImage::new(32, 32);
    for y in 5..25 {
        for x in 2..10 {
            image.put_pixel(x, y, image::Rgba([255, 255, 255, 255]));
        }
    }
    for y in 5..25 {
        for x in 18..28 {
            image.put_pixel(x, y, image::Rgba([30, 70, 190, 255]));
        }
    }
    image.save(&path).unwrap();
    let mut setting = options();
    setting.max_dimension = 2048;
    setting.background = BackgroundMode::TransparentOnly;
    let p = import::load_image_with_options(&path, 32., &setting).unwrap();
    assert_eq!(p.objects.len(), 2);
    assert!(colors(&p).contains(&[255, 255, 255]));
    let white = p
        .objects
        .iter()
        .find(|o| o.color == [255, 255, 255])
        .unwrap();
    assert_eq!(ring_area(&white.contours[0]), 160.);
    assert!(white.contours[0].contains(&Point::new(2., 5.)));
    setting.background = BackgroundMode::NearWhite;
    let p = import::load_image_with_options(&path, 32., &setting).unwrap();
    assert_eq!(p.objects.len(), 1);
    assert!(!colors(&p).contains(&[255, 255, 255]));
}

#[test]
fn high_resolution_retains_sub_default_pixel_letter_gap() {
    let path = image_path("small-letter-gap.png");
    let mut image = image::RgbaImage::from_pixel(1024, 64, image::Rgba([255, 255, 255, 255]));
    for y in 16..48 {
        for x in (400..404).chain(405..409) {
            image.put_pixel(x, y, image::Rgba([10, 10, 10, 255]));
        }
    }
    image.save(&path).unwrap();
    let setting = RasterOptions {
        max_dimension: 1024,
        palette_size: 2,
        ..options()
    };
    let p = import::load_image_with_options(&path, 100., &setting).unwrap();
    assert_eq!(p.objects.len(), 2);
    let mut bounds: Vec<_> = p
        .objects
        .iter()
        .map(|o| {
            let min = o
                .contours
                .iter()
                .flatten()
                .map(|p| p.x)
                .fold(f64::INFINITY, f64::min);
            let max = o
                .contours
                .iter()
                .flatten()
                .map(|p| p.x)
                .fold(f64::NEG_INFINITY, f64::max);
            (min, max)
        })
        .collect();
    bounds.sort_by(|a, b| a.0.total_cmp(&b.0));
    assert_eq!(bounds[0], (400. * 100. / 1024., 404. * 100. / 1024.));
    assert_eq!(bounds[1], (405. * 100. / 1024., 409. * 100. / 1024.));
    assert!((bounds[1].0 - bounds[0].1) < 100. / 256.);
}

#[test]
fn palette_and_minimum_component_are_reproducible_controls() {
    let path = image_path("palette.png");
    let mut image = image::RgbaImage::new(64, 32);
    for (index, color) in [
        [20, 30, 200, 255],
        [210, 25, 30, 255],
        [20, 185, 40, 255],
        [190, 140, 20, 255],
    ]
    .into_iter()
    .enumerate()
    {
        for y in 4..24 {
            for x in index as u32 * 16 + 2..index as u32 * 16 + 12 {
                image.put_pixel(x, y, image::Rgba(color));
            }
        }
    }
    image.put_pixel(1, 30, image::Rgba([20, 30, 200, 255]));
    image.save(&path).unwrap();
    let mut setting = options();
    setting.background = BackgroundMode::TransparentOnly;
    setting.palette_size = 4;
    setting.min_region_pixels = 2;
    let first = import::load_image_with_options(&path, 64., &setting).unwrap();
    let second = import::load_image_with_options(&path, 64., &setting).unwrap();
    assert_eq!(colors(&first).len(), 4);
    assert_eq!(first.objects.len(), 4);
    assert_eq!(
        serde_json::to_string(&first).unwrap(),
        serde_json::to_string(&second).unwrap()
    );
    setting.min_region_pixels = 1;
    assert_eq!(
        import::load_image_with_options(&path, 64., &setting)
            .unwrap()
            .objects
            .len(),
        5
    );
    setting.palette_size = 2;
    assert!(colors(&import::load_image_with_options(&path, 64., &setting).unwrap()).len() <= 2);
}

#[test]
fn border_inference_is_explicit_and_matches_color_mask() {
    let path = image_path("border-background.png");
    let mut image = image::RgbaImage::from_pixel(64, 48, image::Rgba([180, 145, 65, 255]));
    for y in 8..40 {
        for x in 10..54 {
            image.put_pixel(x, y, image::Rgba([30, 80, 190, 255]));
        }
    }
    image.save(&path).unwrap();
    let mut setting = options();
    setting.background = BackgroundMode::Border { tolerance: 4. };
    let inferred = import::load_image_with_options(&path, 64., &setting).unwrap();
    setting.background = BackgroundMode::Color {
        rgb: [180, 145, 65],
        tolerance: 4.,
    };
    let explicit = import::load_image_with_options(&path, 64., &setting).unwrap();
    assert_eq!(
        serde_json::to_string(&inferred.objects).unwrap(),
        serde_json::to_string(&explicit.objects).unwrap()
    );
    // Equivalent masks produce the same geometry; schema v5 deliberately
    // retains the distinct user decisions rather than erasing the recipe.
    assert_eq!(inferred.width_mm, explicit.width_mm);
    assert_eq!(inferred.source_image, explicit.source_image);
    assert_eq!(
        inferred.raster_recipe.as_ref().unwrap().options.background,
        BackgroundMode::Border { tolerance: 4. }
    );
    assert_eq!(
        explicit.raster_recipe.as_ref().unwrap().options.background,
        setting.background
    );
    assert_eq!(inferred.objects.len(), 1);
}

#[test]
fn smoothing_preserves_hole_and_exports_valid_stitches() {
    let path = image_path("smoothed-hole.png");
    let mut image = image::RgbaImage::from_pixel(64, 64, image::Rgba([255, 255, 255, 255]));
    for y in 8..56 {
        for x in 8..56 {
            if !(24..40).contains(&x) || !(24..40).contains(&y) {
                image.put_pixel(x, y, image::Rgba([20, 80, 185, 255]));
            }
        }
    }
    image.save(&path).unwrap();
    let setting = RasterOptions {
        smoothing_iterations: 2,
        ..options()
    };
    let p = import::load_image_with_options(&path, 64., &setting).unwrap();
    assert_eq!(p.objects.len(), 1);
    assert_eq!(p.objects[0].contours.len(), 2);
    let s = dst::decode(&dst::encode(&engine::generate(&p).unwrap(), "Smooth").unwrap()).unwrap();
    let mut previous = Point::default();
    for stitch in s {
        if stitch.command == Command::Stitch {
            let middle = previous.lerp(stitch.point(), 0.5);
            assert!(!(middle.x > 27. && middle.x < 37. && middle.y > 27. && middle.y < 37.));
        }
        previous = stitch.point();
    }
}

#[test]
fn invalid_options_are_rejected_before_reading_a_file() {
    let missing = std::path::Path::new("missing-import-options.png");
    for setting in [
        RasterOptions {
            max_dimension: 4096,
            ..options()
        },
        RasterOptions {
            palette_size: 0,
            ..options()
        },
        RasterOptions {
            smoothing_iterations: 4,
            ..options()
        },
        RasterOptions {
            simplify_tolerance_mm: f64::NAN,
            ..options()
        },
        RasterOptions {
            background: BackgroundMode::Color {
                rgb: [0; 3],
                tolerance: f64::INFINITY,
            },
            ..options()
        },
    ] {
        let error = import::load_image_with_options(missing, 100., &setting).unwrap_err();
        assert!(!error.contains("system cannot find"));
        assert!(setting.validate().is_err());
    }
    assert!(
        serde_json::from_str::<RasterOptions>(r#"{"palette_size":4,"misspelled_option":2}"#)
            .is_err()
    );
    let json = serde_json::to_string(&RasterOptions {
        background: BackgroundMode::Color {
            rgb: [100, 80, 40],
            tolerance: 25.,
        },
        ..options()
    })
    .unwrap();
    assert_eq!(
        serde_json::from_str::<RasterOptions>(&json)
            .unwrap()
            .background,
        BackgroundMode::Color {
            rgb: [100, 80, 40],
            tolerance: 25.
        }
    );
}

#[test]
fn border_without_opaque_samples_and_excessive_source_dimensions_fail_clearly() {
    let path = image_path("transparent-perimeter.png");
    let mut image = image::RgbaImage::new(32, 32);
    for y in 8..24 {
        for x in 8..24 {
            image.put_pixel(x, y, image::Rgba([40, 60, 180, 255]));
        }
    }
    image.save(&path).unwrap();
    let setting = RasterOptions {
        background: BackgroundMode::Border { tolerance: 20. },
        ..options()
    };
    assert!(
        import::load_image_with_options(&path, 32., &setting)
            .unwrap_err()
            .contains("opaque perimeter")
    );
    let path = image_path("wide-source.png");
    image::RgbaImage::new(17000, 2).save(&path).unwrap();
    assert!(
        import::load_image_with_options(&path, 100., &options())
            .unwrap_err()
            .contains("dimensions")
    );
}
