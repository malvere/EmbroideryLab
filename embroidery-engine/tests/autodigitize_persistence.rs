use embroidery_lab::{color_selection::CleanupMask, import, model::*, persistence::SourceImage};

fn source() -> Project {
    let pixels = image::RgbaImage::from_fn(32, 32, |x, y| {
        if (4..28).contains(&x) && (4..28).contains(&y) {
            image::Rgba([20, 80, 180, 255])
        } else {
            image::Rgba([255; 4])
        }
    });
    let mut excluded = vec![false; 1024];
    for y in 0..32 {
        for x in 0..32 {
            excluded[y * 32 + x] = x < 4 || y < 4 || x >= 28 || y >= 28;
        }
    }
    let options = import::RasterOptions {
        cleanup: Some(CleanupMask::from_masks(32, 32, &excluded, &[false; 1024]).unwrap()),
        ..import::RasterOptions::autodigitize()
    };
    import::load_pixels_with_options(
        &image::DynamicImage::ImageRgba8(pixels),
        std::path::Path::new("synthetic.png"),
        40.,
        &options,
    )
    .unwrap()
}

#[test]
fn schema_five_retains_original_cleanup_profile_and_manual_geometry() {
    let mut project = source();
    let original = project.source_image.clone().unwrap().encoded;
    project.objects[0].contours[0][0].x += 0.3;
    project.objects[0].settings.angle_deg = 37.;
    project.objects[0].color = [140, 20, 40];
    project.machine_profile.hoop_width_mm = Some(120.);
    project.machine_profile.dst_trim_encoding = embroidery_lab::dst::TrimEncoding::KeepThread;
    let path = std::env::temp_dir().join(format!("autodigitize-v5-{}.elab", std::process::id()));
    project.save(&path).unwrap();
    let opened = Project::load(&path).unwrap();
    assert_eq!(opened.version, 5);
    assert_eq!(opened.source_image.unwrap().encoded, original);
    assert_eq!(opened.machine_profile, project.machine_profile);
    assert_eq!(
        serde_json::to_value(opened.objects).unwrap(),
        serde_json::to_value(project.objects).unwrap()
    );
    assert!(opened.raster_recipe.unwrap().options.cleanup.is_some());
    std::fs::remove_file(path).unwrap();
}

#[test]
fn exact_retrace_preserves_manual_changes_ids_and_order_and_refuses_topology_guess() {
    let mut previous = source();
    let recipe = previous.raster_recipe.clone().unwrap();
    previous.objects[0].settings.angle_deg = 29.;
    previous.objects[0].contours[0][0].x += 0.25;
    let mut next = source();
    recipe.preserve_overrides(&previous, &mut next).unwrap();
    assert_eq!(
        serde_json::to_value(&next.objects).unwrap(),
        serde_json::to_value(&previous.objects).unwrap()
    );
    assert_eq!(
        next.raster_recipe.as_ref().unwrap().objects[0].object_id,
        previous.objects[0].id
    );
    let mut changed = source();
    changed.objects[0].contours[0][0].x += 1.;
    assert!(
        recipe
            .preserve_overrides(&previous, &mut changed)
            .unwrap_err()
            .contains("topology changed")
    );
}

#[test]
fn deleted_imported_objects_do_not_reappear_and_legacy_projects_still_open() {
    let mut previous = source();
    let recipe = previous.raster_recipe.clone().unwrap();
    previous.objects.clear();
    let mut next = source();
    recipe.preserve_overrides(&previous, &mut next).unwrap();
    assert!(next.objects.is_empty());
    for version in 1..=4 {
        let path = std::env::temp_dir().join(format!(
            "autodigitize-legacy-{version}-{}.elab",
            std::process::id()
        ));
        std::fs::write(
            &path,
            format!("{{\"version\":{version},\"name\":\"legacy\",\"objects\":[],\"width_mm\":40}}"),
        )
        .unwrap();
        assert_eq!(Project::load(&path).unwrap().version, version);
        std::fs::remove_file(path).unwrap();
    }
}

#[test]
fn cleanup_cannot_bind_to_a_different_embedded_source_resolution() {
    let mut project = source();
    project.source_image = Some(
        SourceImage::from_image(
            "different.png".into(),
            &image::DynamicImage::new_rgba8(16, 16),
            40.,
        )
        .unwrap(),
    );
    assert!(
        project
            .validate_limits()
            .unwrap_err()
            .contains("dimensions")
    );
}

#[test]
fn manual_merged_geometry_cannot_be_copied_across_a_changed_source_component() {
    let mut original = source();
    let mut second = original.objects[0].clone();
    second.id = 2;
    for point in second.contours.iter_mut().flatten() {
        point.x += 40.;
    }
    original.objects.push(second);
    let recipe = embroidery_lab::persistence::RasterRecipe::capture(
        &original,
        import::RasterOptions::autodigitize(),
    );
    let mut edited = original.clone();
    let other = edited.objects.pop().unwrap();
    edited.objects[0].contours.extend(other.contours);
    let mut changed = original;
    changed.objects[1].contours[0][0].y += 1.;
    assert!(
        recipe
            .preserve_overrides(&edited, &mut changed)
            .unwrap_err()
            .contains("topology changed")
    );
}
