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
                self.language.text("Images (JPEG, PNG, PDF, SVG)"),
                &["jpg", "jpeg", "png", "pdf", "svg"],
            )
            .add_filter(self.language.text("JPEG image"), &["jpg", "jpeg"])
            .add_filter(self.language.text("PNG image"), &["png"])
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
        let pixels = page.image.thumbnail(480, 480).to_rgba8();
        let data = egui::ColorImage::from_rgba_unmultiplied(
            [pixels.width() as usize, pixels.height() as usize],
            pixels.as_raw(),
        );
        self.import_preview = Some(ImportPreview {
            path,
            pixels,
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
            let original = serde_json::to_value(&app.project).unwrap();
            app.request_open(
                PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("../tests/fixtures")
                    .join(filename),
                &ctx,
            );
            assert!(app.import_preview.is_some(), "{}", app.message);
            assert!(app.import_preview.as_ref().unwrap().pdf.is_none());
            assert_eq!(serde_json::to_value(&app.project).unwrap(), original);
            app.recovery = Some(Project::demo());
            app.commit_import(&ctx);
            assert!(app.import_preview.is_none(), "{}", app.message);
            assert!(app.recovery.is_none());
            assert!(app.project.source_image.is_some());
            assert!(app.dirty && !app.stitch_only);
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
        assert!(app.region_preview.is_some(), "{}", app.message);
        app.commit_import(&ctx);
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
