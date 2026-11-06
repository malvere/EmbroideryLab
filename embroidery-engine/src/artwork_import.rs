use super::*;

pub(super) struct PdfPreview {
    pub page_index: usize,
    pub page_count: usize,
    pub image: image::DynamicImage,
}

pub(super) struct PdfJob {
    path: PathBuf,
    page_index: usize,
    receiver: std::sync::mpsc::Receiver<Result<embroidery_lab::pdf_import::RenderedPage, String>>,
    pub cancelled: bool,
}

impl Editor {
    pub(super) fn image_dialog(&mut self, ctx: &egui::Context) {
        if let Some(path) = rfd::FileDialog::new()
            .set_title(self.language.text("Import image…"))
            .add_filter(
                self.language.text("Images (JPEG, PNG, WebP, PDF, SVG)"),
                &["jpg", "jpeg", "png", "webp", "pdf", "svg"],
            )
            .add_filter(self.language.text("JPEG image"), &["jpg", "jpeg"])
            .add_filter(self.language.text("PNG image"), &["png"])
            .add_filter(self.language.text("WebP image"), &["webp"])
            .add_filter(self.language.text("PDF document"), &["pdf"])
            .pick_file()
        {
            if self.dirty {
                self.discard = Some(path);
            } else {
                self.request_open(path, ctx);
            }
        }
    }

    pub(super) fn dropped_artwork(&mut self, ctx: &egui::Context) {
        if self.confirm_new {
            return;
        }
        if self.import_preview.is_some()
            || self.recovery.is_some()
            || self.confirm_close
            || self.discard.is_some()
            || self.pdf_job.is_some()
            || self.centerline_job.is_some()
        {
            return;
        }
        if let Some(path) = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .find_map(|file| file.path.clone())
        }) {
            if self.dirty {
                self.discard = Some(path);
            } else {
                self.request_open(path, ctx);
            }
        }
    }

    pub(super) fn start_pdf_load(&mut self, path: PathBuf, page_index: usize, ctx: &egui::Context) {
        self.cancel_digitize();
        if self.pdf_job.is_some() {
            self.result(Err(
                "Finishing the previous PDF load. Try again shortly.".into()
            ));
            return;
        }
        let (sender, receiver) = std::sync::mpsc::channel();
        let input = path.clone();
        let repaint = ctx.clone();
        std::thread::spawn(move || {
            let result = embroidery_lab::pdf_import::render_page(&input, page_index, 2048);
            let _ = sender.send(result);
            repaint.request_repaint();
        });
        self.pdf_job = Some(PdfJob {
            path,
            page_index,
            receiver,
            cancelled: false,
        });
        self.error = false;
        ctx.request_repaint();
    }

    fn apply_pdf_page(
        &mut self,
        path: PathBuf,
        page_index: usize,
        page: embroidery_lab::pdf_import::RenderedPage,
        ctx: &egui::Context,
    ) {
        self.clear_import_diagnostics();
        let pixels = embroidery_lab::color_selection::thumbnail_rgba(&page.image.to_rgba8(), 480);
        self.raster_options.cleanup = None;
        self.raster_options.background = import::BackgroundMode::TransparentOnly;
        self.cleanup.reset(&page.image.to_rgba8(), None);
        let data = egui::ColorImage::from_rgba_unmultiplied(
            [pixels.width() as usize, pixels.height() as usize],
            pixels.as_raw(),
        );
        self.import_preview = Some(ImportPreview {
            path,
            pixels: std::sync::Arc::new(page.image.to_rgba8()),
            texture: ctx.load_texture("PDF page preview", data, egui::TextureOptions::LINEAR),
            pdf: Some(PdfPreview {
                page_index,
                page_count: page.page_count,
                image: page.image,
            }),
        });
        self.region_preview = None;
        self.mask_texture = None;
        self.error = false;
    }

    pub(super) fn pdf_progress(&mut self, ctx: &egui::Context) {
        let result = self
            .pdf_job
            .as_ref()
            .and_then(|job| match job.receiver.try_recv() {
                Ok(result) => Some(result),
                Err(std::sync::mpsc::TryRecvError::Disconnected) => Some(Err(
                    "PDF loading stopped unexpectedly. Try opening the file again.".into(),
                )),
                Err(std::sync::mpsc::TryRecvError::Empty) => None,
            });
        if let Some(result) = result {
            ctx.request_repaint();
            let job = self.pdf_job.take().unwrap();
            if !job.cancelled {
                match result {
                    Ok(page) => self.apply_pdf_page(job.path, job.page_index, page, ctx),
                    Err(error) => self.result(Err(error)),
                }
            }
        }
        if let Some(job) = &mut self.pdf_job {
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
            if !job.cancelled {
                let language = self.language;
                egui::Modal::new(egui::Id::new("loading-pdf")).show(ctx, |ui| {
                    ui.heading(language.text("Loading PDF…"));
                    ui.label(job.path.file_name().unwrap_or_default().to_string_lossy());
                    ui.spinner();
                    if ui.button(language.text("Cancel")).clicked() {
                        job.cancelled = true;
                    }
                });
            }
        }
    }

    pub(super) fn trace_import_preview(&self) -> Result<Project, String> {
        let preview = self
            .import_preview
            .as_ref()
            .ok_or("Choose an image to import.")?;
        if let Some(pdf) = &preview.pdf {
            let source_name = format!(
                "{} · {} {}.png",
                preview
                    .path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy(),
                self.language.text("Page"),
                pdf.page_index + 1
            );
            import::load_pixels_with_options(
                &pdf.image,
                std::path::Path::new(&source_name),
                self.import_width_mm,
                &self.raster_options,
            )
        } else {
            import::load_image_with_options(
                &preview.path,
                self.import_width_mm,
                &self.raster_options,
            )
        }
    }
}

#[cfg(test)]
#[path = "artwork_import_tests.rs"]
mod layout_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jpeg_and_png_open_in_the_same_protected_import_preview() {
        let ctx = egui::Context::default();
        for filename in ["synthetic_two_colors.jpg", "synthetic_holes.png"] {
            let mut app = Editor::workspace();
            app.project.machine_profile.max_stitch_mm = 8.;
            app.project.machine_profile.hoop_width_mm = Some(200.);
            app.project.machine_profile.dst_trim_encoding = dst::TrimEncoding::KeepThread;
            let machine_profile = app.project.machine_profile.clone();
            let original = serde_json::to_value(&app.project).unwrap();
            let source_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../tests/fixtures")
                .join(filename);
            let original_bytes = std::fs::read(&source_path).unwrap();
            app.request_open(source_path, &ctx);
            assert!(app.import_preview.is_some(), "{}", app.message);
            assert!(app.import_preview.as_ref().unwrap().pdf.is_none());
            assert_eq!(serde_json::to_value(&app.project).unwrap(), original);
            app.recovery = Some(Project::demo());
            app.commit_import(&ctx);
            app.wait_for_digitize();
            assert!(app.import_preview.is_none(), "{}", app.message);
            assert!(app.recovery.is_none());
            assert!(app.project.source_image.is_some());
            assert_eq!(
                app.project.source_image.as_ref().unwrap().encoded,
                original_bytes,
                "GUI must preserve the original encoded JPEG/PNG snapshot"
            );
            assert!(app.dirty && !app.stitch_only);
            assert_eq!(
                app.project.machine_profile, machine_profile,
                "Import must preserve machine limits and trim policy"
            );
        }
    }

    #[test]
    fn mislabeled_webp_and_explicit_webp_use_preview_worker_and_portable_png_source() {
        let ctx = egui::Context::default();
        let mut pixels = image::RgbaImage::from_pixel(24, 20, image::Rgba([230, 20, 30, 255]));
        for y in 4..16 {
            for x in 6..18 {
                pixels.put_pixel(x, y, image::Rgba([245, 245, 245, 255]));
            }
        }
        pixels.put_pixel(0, 0, image::Rgba([10, 70, 190, 128]));
        let mut webp = Vec::new();
        image::codecs::webp::WebPEncoder::new_lossless(&mut webp)
            .encode(pixels.as_raw(), 24, 20, image::ExtendedColorType::Rgba8)
            .unwrap();
        for extension in ["svg (1).jpg", "webp"] {
            let path = std::env::temp_dir().join(format!(
                "embroiderylab-preview-{}-рисунок.{extension}",
                std::process::id()
            ));
            std::fs::write(&path, &webp).unwrap();
            let mut app = Editor::workspace_with_language(Language::Russian);
            let before = serde_json::to_value(&app.project).unwrap();
            app.request_open(path.clone(), &ctx);
            assert!(
                !app.error && app.import_preview.is_some(),
                "{}",
                app.message
            );
            assert_eq!(*app.import_preview.as_ref().unwrap().pixels, pixels);
            assert_eq!(
                serde_json::to_value(&app.project).unwrap(),
                before,
                "Opening the cleanup preview must retain the existing document"
            );
            app.commit_import(&ctx);
            app.wait_for_digitize();
            assert!(
                !app.error && app.import_preview.is_none(),
                "{}",
                app.message
            );
            assert!(!app.stale && !app.stitches.is_empty());
            let source = app.project.source_image.as_ref().unwrap();
            assert_eq!(
                image::guess_format(&source.encoded).unwrap(),
                image::ImageFormat::Png
            );
            assert_eq!(source.decode().unwrap().to_rgba8(), pixels);
            assert_eq!(
                std::fs::read(&path).unwrap(),
                webp,
                "Import must leave the original mislabeled file unchanged"
            );
            std::fs::remove_file(path).unwrap();
        }
    }

    #[test]
    fn selected_pdf_page_is_traced_and_saved_as_a_self_contained_image() {
        let ctx = egui::Context::default();
        let mut app = Editor::workspace();
        let path =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../tests/fixtures/synthetic_pages.pdf");
        let page = embroidery_lab::pdf_import::render_page(&path, 1, 512).unwrap();
        app.apply_pdf_page(path, 1, page, &ctx);
        assert_eq!(
            app.import_preview
                .as_ref()
                .unwrap()
                .pdf
                .as_ref()
                .unwrap()
                .page_index,
            1
        );
        app.preview_regions(&ctx);
        app.wait_for_digitize();
        assert!(app.region_preview.is_some(), "{}", app.message);
        app.commit_import(&ctx);
        app.wait_for_digitize();
        assert!(app.import_preview.is_none(), "{}", app.message);
        assert!(app.project.name.contains("2"));
        assert!(app.project.source_image.as_ref().unwrap().decode().is_ok());
        assert!(app.dirty && !app.stitch_only);
    }

    #[test]
    fn cancelled_pdf_worker_cannot_replace_the_project_or_preview() {
        let ctx = egui::Context::default();
        let mut app = Editor::workspace();
        let original = serde_json::to_value(&app.project).unwrap();
        let (sender, receiver) = std::sync::mpsc::channel();
        app.pdf_job = Some(PdfJob {
            path: PathBuf::from("cancelled.pdf"),
            page_index: 0,
            receiver,
            cancelled: true,
        });
        sender
            .send(Ok(embroidery_lab::pdf_import::RenderedPage {
                image: image::DynamicImage::new_rgba8(32, 32),
                page_count: 2,
            }))
            .unwrap();
        app.pdf_progress(&ctx);
        assert!(app.pdf_job.is_none() && app.import_preview.is_none());
        assert_eq!(serde_json::to_value(&app.project).unwrap(), original);
    }
}
