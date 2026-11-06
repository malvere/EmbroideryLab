use embroidery_lab::{
    Project,
    import::{self, BackgroundMode, RasterOptions},
};
use std::{
    io::Cursor,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT_FOLDER: AtomicU64 = AtomicU64::new(0);

fn folder() -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "embroidery-raster-format-{}-{}",
        std::process::id(),
        NEXT_FOLDER.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir(&path).unwrap();
    path
}

fn pixels() -> image::RgbaImage {
    image::RgbaImage::from_fn(32, 24, |x, y| {
        if !(2..30).contains(&x) || !(2..22).contains(&y) {
            image::Rgba([0, 0, 0, 0])
        } else if (12..20).contains(&x) && (8..16).contains(&y) {
            image::Rgba([255, 255, 255, 255])
        } else if x < 8 {
            image::Rgba([20, 80, 180, 128])
        } else {
            image::Rgba([200, 20, 30, 255])
        }
    })
}

fn options() -> RasterOptions {
    RasterOptions {
        max_dimension: 32,
        palette_size: 4,
        background: BackgroundMode::TransparentOnly,
        min_region_pixels: 1,
        simplify_tolerance_mm: 0.,
        ..RasterOptions::default()
    }
}

fn assert_source_pixels(project: &Project, expected: &image::RgbaImage, path: &Path) {
    let source = project.source_image.as_ref().unwrap();
    assert_eq!(
        source.file_name,
        path.file_name().unwrap().to_string_lossy()
    );
    assert_eq!(source.decode().unwrap().to_rgba8(), *expected);
    assert_eq!(source.width_mm, 32.);
    assert_eq!(source.height_mm, 24.);
}

#[test]
fn webp_content_under_jpeg_or_webp_names_imports_and_reopens_with_exact_rgba() {
    let folder = folder();
    let expected = pixels();
    let mut webp = Vec::new();
    image::codecs::webp::WebPEncoder::new_lossless(&mut webp)
        .encode(
            expected.as_raw(),
            expected.width(),
            expected.height(),
            image::ExtendedColorType::Rgba8,
        )
        .unwrap();
    for name in ["Ездец.svg (1).jpg", "Эскиз.webp"] {
        let path = folder.join(name);
        std::fs::write(&path, &webp).unwrap();
        assert_eq!(
            import::decode_raster_file(&path).unwrap().to_rgba8(),
            expected
        );
        let project = import::load_image_with_options(&path, 32., &options()).unwrap();
        assert_source_pixels(&project, &expected, &path);
        assert_eq!(
            image::guess_format(&project.source_image.as_ref().unwrap().encoded).unwrap(),
            image::ImageFormat::Png,
            "WebP uses a lossless PNG source compatible with existing ELAB readers"
        );
        let saved = path.with_extension("elab");
        project.save(&saved).unwrap();
        let reopened = Project::load(&saved).unwrap();
        assert_eq!(
            reopened.version, 5,
            "No WebP-specific project schema is needed"
        );
        assert_source_pixels(&reopened, &expected, &path);
        assert_eq!(
            serde_json::to_value(&reopened.objects).unwrap(),
            serde_json::to_value(&project.objects).unwrap()
        );
        assert_source_pixels(&import::load(&path, 32.).unwrap(), &expected, &path);
        assert_eq!(std::fs::read(&path).unwrap(), webp, "Input stays unchanged");
        std::fs::remove_file(saved).unwrap();
        std::fs::remove_file(path).unwrap();
    }
    std::fs::remove_dir(folder).unwrap();
}

#[test]
fn png_content_with_a_jpeg_filename_keeps_original_encoded_source_bytes() {
    let folder = folder();
    let path = folder.join("Цветной рисунок.svg (2).jpg");
    let expected = pixels();
    let mut png = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(expected.clone())
        .write_to(&mut png, image::ImageFormat::Png)
        .unwrap();
    let encoded = png.into_inner();
    std::fs::write(&path, &encoded).unwrap();
    assert_eq!(
        import::decode_raster_file(&path).unwrap().to_rgba8(),
        expected
    );
    let project = import::load_image_with_options(&path, 32., &options()).unwrap();
    assert_source_pixels(&project, &expected, &path);
    assert_eq!(project.source_image.unwrap().encoded, encoded);
    assert_eq!(std::fs::read(&path).unwrap(), encoded);
    std::fs::remove_file(path).unwrap();
    std::fs::remove_dir(folder).unwrap();
}

#[test]
fn jpeg_extension_cannot_disguise_invalid_or_unsupported_image_content() {
    let folder = folder();
    let path = folder.join("unsupported.jpg");
    for bytes in [
        b"not an image".as_slice(),
        b"GIF89a\x20\x00\x18\x00\x00\x00".as_slice(),
    ] {
        std::fs::write(&path, bytes).unwrap();
        let preview_error = import::decode_raster_file(&path).unwrap_err();
        assert!(
            preview_error.contains("must be PNG, JPEG or WebP"),
            "{preview_error}"
        );
        let import_error = import::load_image_with_options(&path, 32., &options()).unwrap_err();
        assert!(
            import_error.contains("must be PNG, JPEG or WebP"),
            "{import_error}"
        );
    }
    std::fs::remove_file(path).unwrap();
    std::fs::remove_dir(folder).unwrap();
}
