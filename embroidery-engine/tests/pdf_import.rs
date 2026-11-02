use embroidery_lab::pdf_import;
use std::path::PathBuf;

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../tests/fixtures/synthetic_pages.pdf")
}

#[test]
fn portrait_vector_page_has_expected_color_aspect_and_opaque_white_paper() {
    let rendered = pdf_import::render_page(&fixture(), 0, 600).unwrap();
    assert_eq!(rendered.page_count, 2);
    assert_eq!(
        (rendered.image.width(), rendered.image.height()),
        (400, 600)
    );
    let image = rendered.image.into_rgba8();
    assert_eq!(image.get_pixel(10, 10).0, [255, 255, 255, 255]);
    let center = image.get_pixel(200, 300);
    assert!(center[0] > 240 && center[1] < 15 && center[2] < 15);
    assert!(image.pixels().all(|pixel| pixel[3] == 255));
}

#[test]
fn landscape_page_renders_embedded_image_and_page_switching_reuses_runtime() {
    let rendered = pdf_import::render_page(&fixture(), 1, 512).unwrap();
    assert_eq!(rendered.page_count, 2);
    assert_eq!(
        (rendered.image.width(), rendered.image.height()),
        (512, 256)
    );
    let image = rendered.image.into_rgba8();
    let center = image.get_pixel(256, 128);
    assert!(center[0] < 60 && center[1] < 90 && center[2] > 180);
    assert_eq!(image.get_pixel(5, 5).0, [255, 255, 255, 255]);
    let first_again = pdf_import::render_page(&fixture(), 0, 300).unwrap();
    assert_eq!(
        (first_again.image.width(), first_again.image.height()),
        (200, 300)
    );
}

#[test]
fn render_limits_bound_pixels_and_reject_invalid_size_before_opening_input() {
    for size in [16, 2048] {
        let page = pdf_import::render_page(&fixture(), 1, size).unwrap();
        assert_eq!(page.image.width().max(page.image.height()), size);
        assert!(page.image.width() <= 2048 && page.image.height() <= 2048);
    }
    for size in [0, 15, 2049, u32::MAX] {
        assert_eq!(
            pdf_import::render_page(&PathBuf::from("missing.pdf"), 0, size)
                .err()
                .unwrap(),
            "PDF render size must be 16–2048 pixels"
        );
    }
}

#[test]
fn page_indices_outside_document_are_errors_instead_of_another_page() {
    for index in [2, usize::MAX] {
        assert_eq!(
            pdf_import::render_page(&fixture(), index, 128)
                .err()
                .unwrap(),
            "PDF page number must be between 1 and 2"
        );
    }
}

#[test]
fn corrupt_pdf_is_rejected_with_a_recoverable_diagnostic() {
    let path =
        std::env::temp_dir().join(format!("embroidery-corrupt-pdf-{}.pdf", std::process::id()));
    std::fs::write(&path, b"%PDF-1.4\nnot a document").unwrap();
    let error = pdf_import::render_page(&path, 0, 256).err().unwrap();
    std::fs::remove_file(path).unwrap();
    assert_eq!(error, "PDF file is invalid or damaged.");
}

#[test]
fn oversized_pdf_is_rejected_before_reading_or_rendering() {
    let path = std::env::temp_dir().join(format!(
        "embroidery-oversized-pdf-{}.pdf",
        std::process::id()
    ));
    let file = std::fs::File::create(&path).unwrap();
    file.set_len(128 * 1024 * 1024 + 1).unwrap();
    drop(file);
    let error = pdf_import::render_page(&path, 0, 256).err().unwrap();
    std::fs::remove_file(path).unwrap();
    assert_eq!(error, "PDF file exceeds 128MB");
}

#[test]
fn pdf_dispatch_and_selected_page_tracing_embed_png_independent_of_pdf() {
    use embroidery_lab::{
        Project,
        import::{self, RasterOptions},
    };

    let first_page = import::load(&fixture(), 80.).unwrap();
    assert!(!first_page.objects.is_empty());
    let source = first_page.source_image.as_ref().unwrap();
    assert!(source.encoded.starts_with(b"\x89PNG\r\n\x1a\n"));
    assert!(source.height_mm > source.width_mm);

    let rendered = pdf_import::render_page(&fixture(), 1, 256).unwrap();
    let options = RasterOptions {
        max_dimension: 256,
        palette_size: 2,
        min_region_pixels: 1,
        ..RasterOptions::default()
    };
    // This input path deliberately does not exist: decoded-page tracing must
    // retain its rendered PNG without rereading the original PDF.
    let selected = import::load_pixels_with_options(
        &rendered.image,
        &PathBuf::from("missing-original-page-2.pdf"),
        80.,
        &options,
    )
    .unwrap();
    assert!(!selected.objects.is_empty());
    assert!(
        selected
            .objects
            .iter()
            .all(|object| object.color[2] > object.color[0])
    );
    let output = std::env::temp_dir().join(format!(
        "embroidery-pdf-roundtrip-{}.elab",
        std::process::id()
    ));
    selected.save(&output).unwrap();
    let reopened = Project::load(&output).unwrap();
    std::fs::remove_file(output).unwrap();
    let source = reopened.source_image.unwrap();
    assert!(source.encoded.starts_with(b"\x89PNG\r\n\x1a\n"));
    assert_eq!(source.file_name, "missing-original-page-2.pdf");
    assert_eq!(source.width_mm, 80.);
    assert_eq!(source.height_mm, 40.);
    let decoded = source.decode().unwrap();
    assert_eq!((decoded.width(), decoded.height()), (256, 128));
}
