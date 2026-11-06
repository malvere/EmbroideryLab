use embroidery_lab::{
    Object, Point, Project, StitchKind, autodigitize,
    color_selection::{self, CleanupMask, SelectionCache, SelectionMode},
    import::{self, RasterOptions},
    persistence::RasterRecipe,
};
use image::{DynamicImage, Rgba, RgbaImage};
use std::{path::Path, sync::atomic::AtomicBool};

fn rectangle(id: u64, x: f64, length: f64, width: f64) -> Object {
    Object {
        id,
        contours: vec![vec![
            Point::new(x, 0.),
            Point::new(x + length, 0.),
            Point::new(x + length, width),
            Point::new(x, width),
        ]],
        ..Object::default()
    }
}

fn traced(objects: Vec<Object>) -> Project {
    let mut project = Project {
        objects,
        ..Project::default()
    };
    project.raster_recipe = Some(RasterRecipe::capture(
        &project,
        RasterOptions::autodigitize(),
    ));
    project
}

#[test]
fn hundred_mm_square_keeps_exact_flag_corners_under_point_one_mm_smoothing() {
    let source = DynamicImage::ImageRgba8(RgbaImage::from_pixel(32, 32, Rgba([20, 70, 190, 255])));
    for iterations in [1, 2, 3] {
        let options = RasterOptions {
            simplify_tolerance_mm: 0.1,
            smoothing_iterations: iterations,
            ..RasterOptions::autodigitize()
        };
        let (project, _) =
            import::load_pixels_with_report(&source, Path::new("square.png"), 100., &options)
                .unwrap();
        let rings = &project.objects[0].contours;
        assert_eq!(rings.len(), 1);
        assert_eq!(
            rings[0].len(),
            4,
            "smoothing must not replace flag corners by chamfers"
        );
        for corner in [
            Point::new(0., 0.),
            Point::new(100., 0.),
            Point::new(100., 100.),
            Point::new(0., 100.),
        ] {
            assert!(rings[0].contains(&corner));
        }
    }
}

#[test]
fn explicit_fringe_cleanup_removes_gray128_mixture_but_keeps_art_and_source() {
    let mut source = RgbaImage::from_pixel(48, 32, Rgba([255; 4]));
    // A white/black antialiased exterior: the grey strip is a known 50% sRGB
    // mixture, not an independent artwork colour. Keep a white eye inside.
    for y in 4..28 {
        for x in 9..32 {
            source.put_pixel(x, y, Rgba([0, 0, 0, 255]));
        }
        source.put_pixel(8, y, Rgba([128, 128, 128, 255]));
    }
    for y in 12..20 {
        for x in 15..24 {
            source.put_pixel(x, y, Rgba([255; 4]));
        }
    }
    // Real opaque grey art, including an exterior edge, has a solid grey core.
    for y in 10..22 {
        for x in 36..44 {
            source.put_pixel(x, y, Rgba([128, 128, 128, 255]));
        }
    }
    let original = source.clone();
    let excluded = SelectionCache::new(&source, [0, 0])
        .unwrap()
        .candidates(0.1, SelectionMode::Exterior);
    let protected = vec![false; 48 * 32];
    let preview =
        color_selection::light_halo_preview(&source, &excluded, &protected, 2, 15.).unwrap();
    for y in 5..27 {
        assert!(
            preview[y * 48 + 8],
            "the gray128 antialiased fringe must be excluded"
        );
        assert!(
            !preview[y * 48 + 9],
            "the black core must retain its full thickness"
        );
    }
    assert!(!preview[16 * 48 + 18], "necessary internal white stays");
    assert!(!preview[16 * 48 + 36], "opaque grey artwork exterior stays");
    assert!(!preview[16 * 48 + 40], "opaque grey artwork interior stays");
    let cleanup = CleanupMask::from_masks(48, 32, &preview, &protected).unwrap();
    let options = RasterOptions {
        cleanup: Some(cleanup.clone()),
        ..RasterOptions::autodigitize()
    };
    let (project, _) = import::load_pixels_with_report(
        &DynamicImage::ImageRgba8(source.clone()),
        Path::new("fringe.png"),
        48.,
        &options,
    )
    .unwrap();
    // Only the real detached grey artwork becomes a grey region; the fringe
    // beside the black contour cannot consume another object/thread block.
    let gray: Vec<_> = project
        .objects
        .iter()
        .filter(|object| object.color == [128; 3])
        .collect();
    assert_eq!(gray.len(), 1);
    assert!(
        gray[0]
            .contours
            .iter()
            .flatten()
            .all(|point| point.x >= 36.)
    );
    assert_eq!(source, original);
    assert_eq!(
        project.source_image.unwrap().decode().unwrap().to_rgba8(),
        original
    );
    assert_eq!(cleanup.apply(&source).unwrap().get_pixel(8, 16)[3], 0);

    let mut protected = protected;
    protected[16 * 48 + 8] = true;
    let preview =
        color_selection::light_halo_preview(&source, &excluded, &protected, 2, 40.).unwrap();
    assert!(
        !preview[16 * 48 + 8],
        "explicit protection wins at maximum tolerance"
    );
}

#[test]
fn near_endpoint_aa_shades_do_not_become_gray_threads_or_erase_black_and_white_details() {
    for shade in [1, 32, 128, 252, 254] {
        let mut source = RgbaImage::from_pixel(24, 16, Rgba([255; 4]));
        for y in 3..13 {
            source.put_pixel(4, y, Rgba([shade, shade, shade, 255]));
            for x in 5..20 {
                source.put_pixel(x, y, Rgba([0, 0, 0, 255]));
            }
        }
        for y in 6..10 {
            for x in 10..13 {
                source.put_pixel(x, y, Rgba([255; 4]));
            }
        }
        let excluded = SelectionCache::new(&source, [0, 0])
            .unwrap()
            .candidates(0., SelectionMode::Exterior);
        let mut protected = vec![false; 24 * 16];
        let preview =
            color_selection::light_halo_preview(&source, &excluded, &protected, 2, 15.).unwrap();
        assert!(
            preview[8 * 24 + 4],
            "shade {shade}: intermediate fringe remains"
        );
        assert!(
            !preview[8 * 24 + 5],
            "shade {shade}: opaque black core was erased"
        );
        assert!(
            !preview[8 * 24 + 11],
            "shade {shade}: internal white was erased"
        );
        let options = RasterOptions {
            cleanup: Some(CleanupMask::from_masks(24, 16, &preview, &protected).unwrap()),
            ..RasterOptions::autodigitize()
        };
        let (project, _) = import::load_pixels_with_report(
            &DynamicImage::ImageRgba8(source.clone()),
            Path::new("aa-endpoints.png"),
            24.,
            &options,
        )
        .unwrap();
        assert!(
            project
                .objects
                .iter()
                .all(|object| object.color == [0; 3] || object.color == [255; 3]),
            "shade {shade}: AA introduced a gray thread"
        );
        protected[8 * 24 + 4] = true;
        let protected_preview =
            color_selection::light_halo_preview(&source, &excluded, &protected, 2, 40.).unwrap();
        assert!(
            !protected_preview[8 * 24 + 4],
            "shade {shade}: protection lost"
        );
    }
}

#[test]
fn classified_retrace_keeps_source_identity_manual_priority_and_warning_ids() {
    let raw = traced(vec![
        rectangle(10, 0., 40., 10.),
        rectangle(20, 50., 30., 3.),
    ]);
    let mut previous = raw.clone();
    autodigitize::classify_project(&mut previous, &AtomicBool::new(false));
    previous.layering.enabled = false;
    previous.layering.overlap_mm = 0.35;
    previous.objects[1]
        .satin_column
        .as_mut()
        .unwrap()
        .first_right = true;
    previous.objects[1].settings.angle_deg = 31.;
    let manual = serde_json::to_value(&previous.objects[1]).unwrap();
    let recipe = previous.raster_recipe.clone().unwrap();
    let mut next = traced(vec![rectangle(1, 0., 40., 10.), rectangle(2, 50., 30., 3.)]);
    autodigitize::classify_project(&mut next, &AtomicBool::new(false));
    recipe.preserve_overrides(&previous, &mut next).unwrap();
    assert_eq!(serde_json::to_value(&next.objects[1]).unwrap(), manual);
    assert_eq!(next.layering, previous.layering);
    let warnings = &next.raster_recipe.as_ref().unwrap().issues;
    assert_eq!(
        warnings
            .iter()
            .filter(|issue| issue.message.contains("exceeds automatic Satin maximum"))
            .count(),
        1
    );
    assert!(warnings.iter().all(|issue| issue.object_id == 10));
    next.validate_limits().unwrap();
    // A second preservation must still detect manual overrides against the
    // automatic baseline, rather than silently adopting them as that baseline.
    let identity = next
        .raster_recipe
        .as_ref()
        .unwrap()
        .objects
        .iter()
        .find(|identity| identity.object_id == 20)
        .unwrap();
    assert_ne!(
        identity.original_fingerprint,
        embroidery_lab::persistence::object_fingerprint(&next.objects[1])
    );
}

#[test]
fn changed_and_new_source_regions_keep_their_fresh_running_and_satin_choices() {
    let mut previous = traced(vec![rectangle(1, 0., 20., 20.)]);
    autodigitize::classify_project(&mut previous, &AtomicBool::new(false));
    let recipe = previous.raster_recipe.clone().unwrap();
    let mut next = traced(vec![rectangle(1, 0., 20., 0.5), rectangle(2, 30., 30., 3.)]);
    let captured: Vec<_> = next
        .raster_recipe
        .as_ref()
        .unwrap()
        .objects
        .iter()
        .map(|identity| identity.region_identity)
        .collect();
    autodigitize::classify_project(&mut next, &AtomicBool::new(false));
    recipe.preserve_overrides(&previous, &mut next).unwrap();
    assert_eq!(
        next.objects
            .iter()
            .map(|object| object.settings.kind)
            .collect::<Vec<_>>(),
        vec![StitchKind::Running, StitchKind::Satin]
    );
    assert_eq!(
        next.objects
            .iter()
            .map(|object| object.id)
            .collect::<Vec<_>>(),
        vec![2, 3]
    );
    assert_eq!(
        next.objects[0]
            .centerline_graph
            .as_ref()
            .unwrap()
            .source_object_id,
        2
    );
    for (object, expected_region) in next.objects.iter().zip(captured) {
        let identity = next
            .raster_recipe
            .as_ref()
            .unwrap()
            .objects
            .iter()
            .find(|identity| identity.object_id == object.id)
            .unwrap();
        assert_eq!(identity.region_identity, expected_region);
        assert_eq!(
            identity.original_fingerprint,
            embroidery_lab::persistence::object_fingerprint(object)
        );
    }
    next.validate_limits().unwrap();
}

#[test]
fn reclassification_removes_a_machine_limit_warning_that_has_been_resolved() {
    let mut previous = traced(vec![rectangle(1, 0., 30., 6.)]);
    previous.machine_profile.max_stitch_mm = 5.;
    autodigitize::classify_project(&mut previous, &AtomicBool::new(false));
    assert_eq!(previous.objects[0].settings.kind, StitchKind::Tatami);
    assert!(
        previous
            .raster_recipe
            .as_ref()
            .unwrap()
            .issues
            .iter()
            .any(|issue| issue.message.contains("machine stitch limit"))
    );
    let mut next = traced(vec![rectangle(1, 0., 30., 6.)]);
    next.machine_profile.max_stitch_mm = 12.;
    autodigitize::classify_project(&mut next, &AtomicBool::new(false));
    previous
        .raster_recipe
        .as_ref()
        .unwrap()
        .preserve_overrides(&previous, &mut next)
        .unwrap();
    assert_eq!(next.objects[0].settings.kind, StitchKind::Satin);
    assert!(next.raster_recipe.as_ref().unwrap().issues.is_empty());
}
