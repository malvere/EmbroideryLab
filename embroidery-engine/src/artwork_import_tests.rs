use super::*;

fn page() -> embroidery_lab::pdf_import::RenderedPage {
    embroidery_lab::pdf_import::RenderedPage {
        image: image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            360,
            480,
            image::Rgba([255; 4]),
        )),
        page_count: 3,
    }
}

fn label_rect(shape: &egui::epaint::Shape, label: &str) -> Option<Rect> {
    match shape {
        egui::epaint::Shape::Text(text) if text.galley.job.text == label => {
            Some(Rect::from_min_size(text.pos, text.galley.size()))
        }
        egui::epaint::Shape::Vec(shapes) => {
            shapes.iter().find_map(|shape| label_rect(shape, label))
        }
        _ => None,
    }
}

#[test]
fn pdf_import_actions_remain_visible_at_minimum_viewport_in_both_languages() {
    let viewport = Rect::from_min_size(Pos2::ZERO, Vec2::new(1024., 680.));
    for language in [Language::Russian, Language::English] {
        let context = egui::Context::default();
        let mut style = (*context.style()).clone();
        style.visuals = egui::Visuals::light();
        style.spacing.item_spacing = Vec2::new(8., 8.);
        style.spacing.interact_size.y = 28.;
        context.set_style(style);
        let mut app = Editor::workspace_with_language(language);
        app.apply_pdf_page(PathBuf::from("synthetic-pages.pdf"), 1, page(), &context);
        let mut render = || {
            context.run(
                egui::RawInput {
                    screen_rect: Some(viewport),
                    ..Default::default()
                },
                |ctx| app.import_window(ctx),
            )
        };
        let _ = render();
        let _ = render();
        let output = render();
        for label in ["Trace and open", "Cancel"] {
            let rect = output
                .shapes
                .iter()
                .find_map(|clipped| label_rect(&clipped.shape, language.text(label)))
                .unwrap_or_else(|| panic!("Missing {label} in {}", language.code()));
            assert!(
                viewport.contains_rect(rect),
                "{label} is outside the minimum viewport in {}: {rect:?}",
                language.code()
            );
        }
    }
}

#[test]
fn failed_pdf_page_worker_retains_the_document_and_previous_page_preview() {
    let context = egui::Context::default();
    let mut app = Editor::workspace();
    let path = PathBuf::from("synthetic-pages.pdf");
    app.apply_pdf_page(path.clone(), 1, page(), &context);
    let original = serde_json::to_value(&app.project).unwrap();
    let original_pixels = app.import_preview.as_ref().unwrap().pixels.clone();
    let (sender, receiver) = std::sync::mpsc::channel();
    app.pdf_job = Some(PdfJob {
        path: path.clone(),
        page_index: 2,
        receiver,
        cancelled: false,
    });
    sender
        .send(Err("Cannot render this PDF page".into()))
        .unwrap();

    app.pdf_progress(&context);

    assert!(app.pdf_job.is_none());
    assert!(app.error);
    assert_eq!(app.message, "Cannot render this PDF page");
    assert_eq!(serde_json::to_value(&app.project).unwrap(), original);
    let preview = app.import_preview.as_ref().unwrap();
    assert_eq!(preview.path, path);
    assert_eq!(preview.pdf.as_ref().unwrap().page_index, 1);
    assert_eq!(preview.pixels, original_pixels);
}
