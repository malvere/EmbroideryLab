use embroidery_lab::{
    Project,
    import::{self, RasterOptions},
};
use std::path::{Path, PathBuf};

fn folder(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "embroidery-pixel-import-{}-{name}",
        std::process::id()
    ));
    std::fs::create_dir_all(&path).unwrap();
    path
}

fn artwork() -> image::DynamicImage {
    let mut pixels = image::RgbaImage::from_pixel(64, 48, image::Rgba([255; 4]));
    for y in 6..42 {
        for x in 6..42 {
            if !(18..30).contains(&x) || !(18..30).contains(&y) {
                pixels.put_pixel(x, y, image::Rgba([20, 70, 180, 255]));
            }
        }
    }
    for y in 15..32 {
        for x in 48..58 {
            pixels.put_pixel(x, y, image::Rgba([190, 30, 50, 255]));
        }
    }
    image::DynamicImage::ImageRgba8(pixels)
}

#[test]
fn pixels_and_png_file_share_tracing_without_changing_embedded_file_bytes() {
    let image = artwork();
    let path = folder("equivalence").join("artwork.png");
    image.save(&path).unwrap();
    let options = RasterOptions::default();
    let file = import::load_image_with_options(&path, 80., &options).unwrap();
    let pixels = import::load_pixels_with_options(&image, &path, 80., &options).unwrap();

    assert_eq!(file.name, pixels.name);
    assert_eq!(file.width_mm, pixels.width_mm);
    assert_eq!(
        serde_json::to_value(&file.objects).unwrap(),
        serde_json::to_value(&pixels.objects).unwrap()
    );
    assert!(
        pixels
            .objects
            .iter()
            .any(|object| object.contours.len() == 2)
    );
    assert_eq!(
        file.source_image.as_ref().unwrap().encoded,
        std::fs::read(path).unwrap()
    );
}

#[test]
fn rendered_page_pixels_are_embedded_and_reopen_without_a_source_file() {
    let image = artwork();
    let dir = folder("persistence");
    let source_name = dir.join("document-page-2.pdf");
    assert!(!source_name.exists());
    let project =
        import::load_pixels_with_options(&image, &source_name, 80., &RasterOptions::default())
            .unwrap();
    assert_eq!(project.name, "document-page-2");
    assert_eq!(project.version, 5);
    assert!(project.raster_recipe.is_some());
    let embedded = project.source_image.as_ref().unwrap();
    assert_eq!(embedded.file_name, "document-page-2.pdf");
    assert_eq!(
        image::guess_format(&embedded.encoded).unwrap(),
        image::ImageFormat::Png
    );
    assert_eq!(embedded.decode().unwrap().to_rgba8(), image.to_rgba8());
    let saved = dir.join("page.elab");
    project.save(&saved).unwrap();
    let reopened = Project::load(&saved).unwrap();
    assert_eq!(
        serde_json::to_value(&reopened).unwrap(),
        serde_json::to_value(&project).unwrap()
    );
    assert!(!source_name.exists());
}

#[test]
fn pixel_import_validates_dimensions_width_and_options_before_tracing() {
    let source = Path::new("nonexistent-page.pdf");
    let options = RasterOptions::default();
    assert!(
        import::load_pixels_with_options(
            &image::DynamicImage::new_rgba8(0, 1),
            source,
            80.,
            &options
        )
        .unwrap_err()
        .contains("Raster dimensions")
    );
    assert!(
        import::load_pixels_with_options(&artwork(), source, f64::NAN, &options)
            .unwrap_err()
            .contains("Import width")
    );
    let invalid = RasterOptions {
        palette_size: 0,
        ..options
    };
    assert!(
        import::load_pixels_with_options(&artwork(), source, 80., &invalid)
            .unwrap_err()
            .contains("palette_size")
    );
}

#[test]
fn worker_supplied_original_jpeg_is_preserved_and_rebased_without_png_reencoding() {
    let path = folder("jpeg-source").join("original.jpg");
    artwork().to_rgb8().save(&path).unwrap();
    let source = embroidery_lab::persistence::SourceImage::from_path(&path, 80.).unwrap();
    let original = source.encoded.clone();
    // Frozen preview is derived from exactly these bytes, as required by the
    // public worker API; GUI workers verify a fresh path against their snapshot.
    let decoded = source.decode().unwrap();
    let (project, _) = import::load_pixels_with_source_report_cancellable(
        &decoded,
        &path,
        120.,
        &RasterOptions::default(),
        &std::sync::atomic::AtomicBool::new(false),
        source,
    )
    .unwrap();
    let embedded = project.source_image.as_ref().unwrap();
    assert_eq!(embedded.encoded, original);
    assert_eq!(
        image::guess_format(&embedded.encoded).unwrap(),
        image::ImageFormat::Jpeg
    );
    assert_eq!(embedded.width_mm, 120.);
    assert_eq!(embedded.height_mm, 90.);
    assert_eq!(embedded.decode().unwrap().to_rgba8(), decoded.to_rgba8());
    assert_eq!(project.raster_recipe.as_ref().unwrap().width_mm, 120.);
    let saved = folder("jpeg-source").join("preserved.elab");
    project.save(&saved).unwrap();
    assert_eq!(
        Project::load(&saved).unwrap().source_image.unwrap().encoded,
        original
    );
}

#[test]
fn worker_supplied_source_rejects_dimension_mismatch_and_honors_cancellation() {
    let source = embroidery_lab::persistence::SourceImage::from_image(
        "other.png".into(),
        &image::DynamicImage::new_rgba8(32, 32),
        80.,
    )
    .unwrap();
    let decoded = artwork();
    let path = Path::new("frozen.png");
    let options = RasterOptions::default();
    let error = import::load_pixels_with_source_report_cancellable(
        &decoded,
        path,
        80.,
        &options,
        &std::sync::atomic::AtomicBool::new(false),
        source.clone(),
    )
    .unwrap_err();
    assert!(error.contains("source dimensions do not match the frozen import preview"));
    let error = import::load_pixels_with_source_report_cancellable(
        &decoded,
        path,
        80.,
        &options,
        &std::sync::atomic::AtomicBool::new(true),
        source,
    )
    .unwrap_err();
    assert!(error.contains("cancelled"));
}
