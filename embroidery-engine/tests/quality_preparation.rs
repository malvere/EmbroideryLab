use embroidery_lab::{
    color_selection::CleanupMask,
    import::{self, PreparationMode, RasterOptions},
};
use image::{DynamicImage, Rgba, RgbaImage};
use std::path::Path;

#[test]
fn old_recipe_missing_preparation_retains_legacy_and_new_modes_roundtrip() {
    let old = serde_json::json!({"max_dimension": 512, "palette_size": 8,
        "background": "TransparentOnly", "min_region_pixels": 1,
        "min_region_area_mm2": 0.1});
    let options: RasterOptions = serde_json::from_value(old).unwrap();
    assert_eq!(options.preparation, PreparationMode::Legacy);
    let legacy_json = serde_json::to_value(&options).unwrap();
    assert!(legacy_json.get("preparation").is_none());
    assert_eq!(
        serde_json::from_value::<RasterOptions>(legacy_json).unwrap(),
        options
    );
    for preparation in [
        PreparationMode::Gentle,
        PreparationMode::Standard,
        PreparationMode::Small,
    ] {
        let next = RasterOptions {
            preparation,
            ..options.clone()
        };
        let reread: RasterOptions =
            serde_json::from_slice(&serde_json::to_vec(&next).unwrap()).unwrap();
        assert_eq!(reread, next);
    }
}

#[test]
fn protection_prevents_area_and_pixel_filters_from_deleting_a_rare_symbol() {
    let mut image = RgbaImage::from_pixel(32, 32, Rgba([70, 80, 90, 0]));
    for y in 14..17 {
        for x in 14..17 {
            image.put_pixel(x, y, Rgba([245, 10, 130, 255]));
        }
    }
    let mut protected = vec![false; 1024];
    protected[15 * 32 + 15] = true;
    let options = RasterOptions {
        cleanup: Some(CleanupMask::from_masks(32, 32, &[false; 1024], &protected).unwrap()),
        min_region_pixels: 100,
        min_region_area_mm2: 100.,
        preparation: PreparationMode::Standard,
        ..RasterOptions::autodigitize()
    };
    let (project, report) = import::load_pixels_with_report(
        &DynamicImage::ImageRgba8(image),
        Path::new("rare-protected.png"),
        32.,
        &options,
    )
    .unwrap();
    assert_eq!(project.objects.len(), 1);
    assert_eq!(report.omitted_region_count, 0);
    assert_eq!(report.components_before_preparation, 1);
    assert_eq!(report.components_after_preparation, 1);
    assert_eq!(report.preparation_changed_pixels, 0);
    assert_eq!(report.object_count, 1);
    assert_eq!(report.contour_rings, 1);
}

#[test]
fn hidden_rgb_and_partial_alpha_do_not_composite_on_white_or_change_preparation() {
    let make = |hidden| {
        RgbaImage::from_fn(32, 32, |x, y| {
            if (8..24).contains(&x) && (8..24).contains(&y) {
                Rgba([25, 70, 210, if x == 8 { 160 } else { 255 }])
            } else {
                Rgba([hidden, 255 - hidden, hidden / 2, 0])
            }
        })
    };
    let options = RasterOptions {
        preparation: PreparationMode::Standard,
        min_region_area_mm2: 0.,
        ..RasterOptions::autodigitize()
    };
    let (a, ar) = import::load_pixels_with_report(
        &DynamicImage::ImageRgba8(make(12)),
        Path::new("alpha-a.png"),
        32.,
        &options,
    )
    .unwrap();
    let (b, br) = import::load_pixels_with_report(
        &DynamicImage::ImageRgba8(make(240)),
        Path::new("alpha-b.png"),
        32.,
        &options,
    )
    .unwrap();
    assert_eq!(ar.palette, br.palette);
    assert_eq!(ar.labels_after_preparation, br.labels_after_preparation);
    assert_eq!(a.objects[0].color, [25, 70, 210]);
    assert_eq!(b.objects[0].color, a.objects[0].color);
    assert_eq!(
        a.source_image
            .unwrap()
            .decode()
            .unwrap()
            .to_rgba8()
            .get_pixel(8, 8)[3],
        160
    );
}

#[test]
fn a_protected_subpixel_detail_is_explicitly_rejected_instead_of_silently_lost() {
    for opaque_background in [false, true] {
        let mut image = RgbaImage::from_pixel(
            128,
            128,
            Rgba([20, 70, 180, if opaque_background { 255 } else { 0 }]),
        );
        // This sample is not one of the old nearest-source sample positions.
        image.put_pixel(31, 37, Rgba([240, 20, 110, 255]));
        let immutable = image.clone();
        let mut protected = vec![false; 128 * 128];
        let mut excluded = protected.clone();
        protected[37 * 128 + 31] = true;
        excluded[37 * 128 + 31] = true; // protection/restoration takes precedence
        let options = RasterOptions {
            max_dimension: 16,
            cleanup: Some(CleanupMask::from_masks(128, 128, &excluded, &protected).unwrap()),
            preparation: PreparationMode::Standard,
            min_region_area_mm2: 0.,
            ..RasterOptions::autodigitize()
        };
        let error = import::load_pixels_with_report(
            &DynamicImage::ImageRgba8(image.clone()),
            Path::new("protected-subpixel.png"),
            40.,
            &options,
        )
        .unwrap_err();
        assert!(error.contains("protected detail"), "{error}");
        assert!(error.contains("max_dimension"), "{error}");
        assert_eq!(image, immutable);
        let native = RasterOptions {
            max_dimension: 128,
            ..options
        };
        let (_, report) = import::load_pixels_with_report(
            &DynamicImage::ImageRgba8(image),
            Path::new("protected-native.png"),
            40.,
            &native,
        )
        .unwrap();
        assert_eq!(report.omitted_region_count, 0);
        assert_ne!(report.labels_after_preparation[37 * 128 + 31], 255);
    }
}

#[test]
fn preparation_residue_cannot_be_newly_deleted_by_the_original_region_floors() {
    // A varied AA strip has 33 pixels / 0.33mm². Opposing-ownership samples
    // prevent complete reassignment. The bounded three-pass algorithm leaves
    // a residual below 30px / 0.30mm²; it must keep the original acceptance.
    let image = RgbaImage::from_fn(32, 20, |x, y| {
        let gray = match (x, y) {
            (15, 9) => 140,
            (15, 10) => 64,
            (_, 0..=8) => 0,
            (_, 9) => 64,
            (_, 10) => 160,
            _ => 255,
        };
        Rgba([gray, gray, gray, 255])
    });
    let base = RasterOptions {
        palette_size: 3,
        simplify_tolerance_mm: 0.,
        ..RasterOptions::autodigitize()
    };
    let decoded = DynamicImage::ImageRgba8(image);
    for (min_region_pixels, min_region_area_mm2) in [(1, 0.30), (30, 0.10)] {
        let options = RasterOptions {
            min_region_pixels,
            min_region_area_mm2,
            ..base.clone()
        };
        let (_, before) =
            import::load_pixels_with_report(&decoded, Path::new("aa-residue.png"), 3.2, &options)
                .unwrap();
        assert_eq!(before.omitted_pixel_count, 0);
        let prepared = RasterOptions {
            preparation: PreparationMode::Standard,
            ..options
        };
        let (_, after) =
            import::load_pixels_with_report(&decoded, Path::new("aa-residue.png"), 3.2, &prepared)
                .unwrap();
        assert!(after.preparation_changed_pixels > 0);
        assert!(
            after.preparation_preserved_residual_regions > 0,
            "the fixture must exercise a surviving below-floor residual"
        );
        assert_eq!(
            after.omitted_pixel_count, 0,
            "a retained source pixel may be reassigned, never newly filtered as a small residual"
        );
        assert!(!after.labels_after_preparation.contains(&255));
    }
}
