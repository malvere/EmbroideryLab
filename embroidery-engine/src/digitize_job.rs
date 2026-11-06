//! Revision-checked workers. A completed job is applied as one editor operation.
use super::*;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU8, Ordering},
    mpsc,
};

pub(super) enum JobOutput {
    Generated(PreparedPlan),
    Imported(Project, PreparedPlan),
    Preview(
        embroidery_lab::region_edit::RegionPreview,
        MachinePlan,
        import::RasterTraceReport,
        Option<image::RgbaImage>,
    ),
}

pub(super) struct PreparedPlan {
    plan: MachinePlan,
    issues: Vec<embroidery_lab::quality::QualityIssue>,
    summary: embroidery_lab::quality::PlanSummary,
    coverage: Vec<embroidery_lab::quality::CoverageEstimate>,
    segment_density: embroidery_lab::quality::SegmentDensityReport,
}
impl PreparedPlan {
    fn new(project: &Project, plan: MachinePlan) -> Self {
        Self {
            segment_density: embroidery_lab::quality::segment_density(&plan, 1.),
            issues: embroidery_lab::quality::analyze_plan(&plan, project, Default::default()),
            summary: embroidery_lab::quality::plan_summary(&plan),
            coverage: embroidery_lab::quality::coverage_estimates(&plan, project, 0.45),
            plan,
        }
    }
}

pub(super) struct DigitizeJob {
    pub id: u64,
    document_fingerprint: u64,
    import_fingerprint: Option<u64>,
    pub cancelled: Arc<AtomicBool>,
    pub(super) stage: Arc<AtomicU8>,
    pub receiver: mpsc::Receiver<Result<JobOutput, String>>,
}

fn fingerprint(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325, |h, b| {
        (h ^ u64::from(*b)).wrapping_mul(0x100000001b3)
    })
}
fn project_fingerprint(project: &Project) -> u64 {
    fingerprint(&serde_json::to_vec(project).unwrap_or_default())
}

impl Editor {
    pub(super) fn machine_profile_ui(&mut self, ui: &mut egui::Ui) {
        let language = self.language;
        let before = self.project.machine_profile.clone();
        ui.collapsing(language.text("Machine limits and thread cuts"),|ui| {
            ui.label(language.text("Maximum stitch length (mm)"));
            ui.add(egui::DragValue::new(&mut self.project.machine_profile.max_stitch_mm).range(0.3..=12.).speed(0.1));
            ui.label(language.text("Request trim for jumps longer than (mm)"));
            ui.add(egui::DragValue::new(&mut self.project.machine_profile.trim_jump_mm).range(0. ..=500.).speed(0.5));
            for (label,value) in [
                ("Maximum jump (mm)",&mut self.project.machine_profile.max_jump_mm),
                ("Hoop width (mm)",&mut self.project.machine_profile.hoop_width_mm),
                ("Hoop height (mm)",&mut self.project.machine_profile.hoop_height_mm)] {
                let mut enabled=value.is_some();
                ui.horizontal_wrapped(|ui| {
                    if ui.checkbox(&mut enabled,language.text(label)).changed(){*value=if enabled{Some(100.)}else{None};}
                    if let Some(value)=value{ui.add(egui::DragValue::new(value).range(1. ..=9990.).speed(1.));}
                });
            }
            egui::ComboBox::from_id_salt("dst-trim-profile").selected_text(language.text(match self.project.machine_profile.dst_trim_encoding{dst::TrimEncoding::ThreeJump=>"DST trim: three jumps",dst::TrimEncoding::KeepThread=>"DST: retain uncut thread"})).show_ui(ui,|ui| {
                ui.selectable_value(&mut self.project.machine_profile.dst_trim_encoding,dst::TrimEncoding::ThreeJump,language.text("DST trim: three jumps"));
                ui.selectable_value(&mut self.project.machine_profile.dst_trim_encoding,dst::TrimEncoding::KeepThread,language.text("DST: retain uncut thread"));
            });
            ui.small(language.text("DST has no universal trim opcode. Test this profile on your machine; the thread list is saved beside DST."));
        });
        if before != self.project.machine_profile {
            if self.field_before.is_none() {
                let mut snapshot = self.project.clone();
                snapshot.machine_profile = before;
                self.field_before = Some(snapshot);
            }
            self.changed();
        }
    }

    pub(super) fn import_fingerprint(&self) -> u64 {
        fingerprint(
            &serde_json::to_vec(&(
                import::PREPARATION_VERSION,
                &self.raster_options,
                self.import_width_mm,
                self.auto_classify,
                self.cleanup.reimport,
                self.cleanup.reset_manual_decisions,
                self.region_preview.as_ref().map(|p| {
                    p.regions
                        .iter()
                        .map(|r| (r.id, r.included))
                        .collect::<Vec<_>>()
                }),
                self.import_preview.as_ref().map(|p| {
                    (
                        &p.path,
                        p.pixels.dimensions(),
                        p.pdf.as_ref().map(|pdf| pdf.page_index),
                    )
                }),
            ))
            .unwrap_or_default(),
        )
    }

    pub(super) fn cancel_digitize(&mut self) {
        if let Some(job) = self.digitize_job.take() {
            job.cancelled.store(true, Ordering::Relaxed);
        }
        self.job_revision = self.job_revision.wrapping_add(1);
    }

    pub(super) fn generate(&mut self) {
        if self.stitch_only {
            return;
        }
        self.finish_field_edit();
        self.cancel_digitize();
        self.playing = false;
        self.stale = true;
        let project = self.project.clone();
        let document_fingerprint = project_fingerprint(&project);
        let cancel = Arc::new(AtomicBool::new(false));
        let stage = Arc::new(AtomicU8::new(3));
        let (sender, receiver) = mpsc::channel();
        let token = cancel.clone();
        std::thread::spawn(move || {
            let result = engine::generate_plan_cancellable(&project, &token)
                .map(|plan| JobOutput::Generated(PreparedPlan::new(&project, plan)));
            let _ = sender.send(result);
        });
        self.digitize_job = Some(DigitizeJob {
            id: self.job_revision,
            document_fingerprint,
            import_fingerprint: None,
            cancelled: cancel,
            stage,
            receiver,
        });
    }

    pub(super) fn start_trace_job(&mut self, preview_only: bool) {
        if self.import_preview.is_none() {
            return;
        }
        self.finish_field_edit();
        self.cancel_digitize();
        let preview = self.import_preview.as_ref().unwrap();
        let pixels = preview.pixels.clone();
        let saved_source = if self.cleanup.reimport {
            self.project.source_image.clone()
        } else {
            None
        };
        let raster_file = preview.pdf.is_none();
        let source_path = if let Some(pdf) = &preview.pdf {
            preview.path.with_file_name(format!(
                "{} · {} {}.png",
                preview
                    .path
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy(),
                self.language.text("Page"),
                pdf.page_index + 1
            ))
        } else {
            preview.path.clone()
        };
        let options = self.raster_options.clone();
        let machine_profile = self.project.machine_profile.clone();
        let layering = self.project.layering.clone();
        let width = self.import_width_mm;
        let overrides = self.region_preview.clone();
        let previous = if self.cleanup.reimport && !self.cleanup.reset_manual_decisions {
            Some(self.project.clone())
        } else {
            None
        };
        let classify = self.auto_classify;
        let document_fingerprint = project_fingerprint(&self.project);
        let import_fingerprint = Some(self.import_fingerprint());
        let cancel = Arc::new(AtomicBool::new(false));
        let token = cancel.clone();
        let stage = Arc::new(AtomicU8::new(1));
        let worker_stage = stage.clone();
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let result = (|| {
                let image = image::DynamicImage::ImageRgba8((*pixels).clone());
                // Preserve exact original JPEG/PNG bytes when they belong to the
                // frozen preview. This avoids inflating a small JPEG into an oversized PNG.
                let candidate_source = saved_source.or_else(|| {
                    if raster_file {
                        embroidery_lab::persistence::SourceImage::from_path(&source_path, width)
                            .ok()
                    } else {
                        None
                    }
                });
                let verified_source = match candidate_source {
                    Some(source)
                        if source
                            .decode()
                            .is_ok_and(|decoded| decoded.to_rgba8() == *pixels) =>
                    {
                        Some(source)
                    }
                    _ => None,
                };
                if token.load(Ordering::Relaxed) {
                    return Err("Digitizing cancelled. Document unchanged.".into());
                }
                let (mut project, report) = if let Some(source) = verified_source {
                    import::load_pixels_with_source_report_cancellable(
                        &image,
                        &source_path,
                        width,
                        &options,
                        &token,
                        source,
                    )?
                } else {
                    import::load_pixels_with_report_cancellable(
                        &image,
                        &source_path,
                        width,
                        &options,
                        &token,
                    )?
                };
                // Image cleanup never resets the operator's machine limits or trim policy.
                project.machine_profile = machine_profile;
                project.layering = layering;
                if token.load(Ordering::Relaxed) {
                    return Err("Digitizing cancelled. Document unchanged.".into());
                }
                worker_stage.store(2, Ordering::Relaxed);
                if let Some(overrides) = overrides {
                    project = overrides.apply_overrides_to(project)?.project;
                }
                if previous.is_none() {
                    let _warnings = embroidery_lab::autodigitize::batch_fresh_raster_colors(
                        &mut project,
                        &token,
                    )?;
                }
                let regions = if preview_only {
                    let dimensions =
                        embroidery_lab::color_selection::thumbnail_rgba(&pixels, 512).dimensions();
                    let regions = embroidery_lab::region_edit::RegionPreview::from_project(
                        project.clone(),
                        dimensions.0,
                        dimensions.1,
                    )?;
                    Some(regions)
                } else {
                    None
                };
                // Classify the complete fresh candidate first, including new
                // regions created by cleanup. Captured source identities survive
                // constructive Running/Satin conversion; proven manual overrides
                // are then restored with priority over these automatic choices.
                if classify {
                    let _warnings =
                        embroidery_lab::autodigitize::classify_project(&mut project, &token);
                }
                if let Some(previous) = previous {
                    let recipe=previous.raster_recipe.as_ref().ok_or("This older project has no source-region identities. Turn off preservation explicitly to redigitize; your current document is retained.")?;
                    recipe.preserve_overrides(&previous, &mut project)?;
                }
                if token.load(Ordering::Relaxed) {
                    return Err("Digitizing cancelled. Document unchanged.".into());
                }
                worker_stage.store(3, Ordering::Relaxed);
                let plan = engine::generate_plan_cancellable(&project, &token)?;
                if let Some(regions) = regions {
                    let overlay = cleanup_ui::preparation_change_overlay(&report, &pixels);
                    return Ok(JobOutput::Preview(regions, plan, report, overlay));
                }
                let prepared = PreparedPlan::new(&project, plan);
                Ok(JobOutput::Imported(project, prepared))
            })();
            let _ = sender.send(result);
        });
        self.digitize_job = Some(DigitizeJob {
            id: self.job_revision,
            document_fingerprint,
            import_fingerprint,
            cancelled: cancel,
            stage,
            receiver,
        });
        self.error = false;
    }

    pub(super) fn apply_plan(&mut self, prepared: PreparedPlan) {
        let PreparedPlan {
            plan,
            issues,
            summary,
            coverage,
            segment_density,
        } = prepared;
        self.quality_issues = issues;
        self.plan_summary = Some(summary);
        self.coverage = coverage;
        self.segment_density = Some(segment_density);
        self.plan_metadata = plan.metadata;
        self.plan_warnings = plan.warnings;
        self.stitches = plan.stitches;
        self.update_stitch_view();
        self.progress = self.stitches.len();
        self.stale = false;
        self.result(Ok(format!(
            "Generated {} commands. Preview ready.",
            self.stitches.len()
        )));
    }

    pub(super) fn poll_digitize(&mut self, ctx: &egui::Context) {
        let ready = self
            .digitize_job
            .as_ref()
            .and_then(|job| match job.receiver.try_recv() {
                Ok(result) => Some(result),
                Err(mpsc::TryRecvError::Disconnected) => {
                    Some(Err("Digitizing worker stopped. Document unchanged.".into()))
                }
                Err(mpsc::TryRecvError::Empty) => None,
            });
        if let Some(result) = ready {
            let job = self.digitize_job.take().unwrap();
            if job.cancelled.load(Ordering::Relaxed)
                || job.id != self.job_revision
                || job.document_fingerprint != project_fingerprint(&self.project)
                || job.import_fingerprint.is_some_and(|f| {
                    self.import_preview.is_none() || f != self.import_fingerprint()
                })
            {
                self.result(Ok(
                    "Outdated digitizing result discarded. Document unchanged.".into(),
                ));
                return;
            }
            match result {
                Ok(JobOutput::Generated(plan)) => self.apply_plan(plan),
                Ok(JobOutput::Preview(preview, plan, report, overlay)) => {
                    self.region_preview = Some(preview);
                    self.import_plan = Some(plan);
                    self.set_import_trace_report(ctx, report, overlay);
                    self.refresh_mask_texture(ctx);
                    self.cleanup.view = 2;
                    self.import_preview_fingerprint = Some(self.import_fingerprint());
                    self.error = false;
                }
                Ok(JobOutput::Imported(project, plan)) => {
                    let reimport = self.cleanup.reimport;
                    self.snapshot();
                    self.project = project;
                    self.locked_objects.clear();
                    self.stitch_only = false;
                    self.dsb_records = None;
                    self.selected = self
                        .project
                        .objects
                        .iter()
                        .position(|o| o.visible)
                        .unwrap_or(0);
                    self.selection_anchor = None;
                    self.merge_selection.clear();
                    self.clear_selection();
                    self.cancel_draft();
                    if !reimport {
                        self.path = None;
                    }
                    self.dirty = true;
                    self.recovery = None;
                    self.restore_source(ctx);
                    self.fit = true;
                    self.import_preview = None;
                    self.region_preview = None;
                    self.mask_texture = None;
                    self.clear_import_diagnostics();
                    self.cleanup = Default::default();
                    self.apply_plan(plan);
                }
                Err(error) => self.result(Err(error)),
            }
        }
        if self.digitize_job.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
        }
    }

    pub(super) fn digitize_progress(&mut self, ctx: &egui::Context) {
        self.poll_digitize(ctx);
        let Some(job) = &self.digitize_job else {
            return;
        };
        let stage = job.stage.load(Ordering::Relaxed);
        let language = self.language;
        let mut cancel = false;
        let modal_id = egui::Id::new("digitizing-progress");
        egui::Modal::new(modal_id)
            .area(egui::Modal::default_area(modal_id).fade_in(false))
            .frame(egui::Frame::popup(&ctx.style()).fill(self.workspace.theme.tokens().surface))
            .show(ctx,|ui| {
            ui.heading(language.text("Generating embroidery…"));
            ui.label(language.text(match stage{1=>"1 / 3 · Cleanup, palette and tracing",2=>"2 / 3 · Editable regions and stitch types",_=>"3 / 3 · Underlay, top fill and machine commands"}));
            ui.add(egui::ProgressBar::new((stage as f32-0.5)/3.).animate(true));
            ui.small(language.text("The result is applied as one operation. Cancel keeps your document and last valid stitches."));
            if ui.button(language.text("Cancel")).clicked(){cancel=true;}
        });
        if cancel {
            self.cancel_digitize();
            self.result(Ok("Digitizing cancelled. Document unchanged.".into()));
        }
    }

    pub(super) fn redigitize_source(&mut self, ctx: &egui::Context) {
        let Some(source) = self.project.source_image.clone() else {
            return;
        };
        match source.decode() {
            Ok(image) => {
                let pixels = Arc::new(image.to_rgba8());
                let thumbnail = embroidery_lab::color_selection::thumbnail_rgba(&pixels, 480);
                let data = egui::ColorImage::from_rgba_unmultiplied(
                    [thumbnail.width() as usize, thumbnail.height() as usize],
                    thumbnail.as_raw(),
                );
                self.raster_options = self
                    .project
                    .raster_recipe
                    .as_ref()
                    .map(|r| r.options.clone())
                    .unwrap_or_else(import::RasterOptions::autodigitize);
                self.import_width_mm = source.width_mm;
                self.cleanup
                    .reset(&pixels, self.raster_options.cleanup.as_ref());
                self.cleanup.reimport = true;
                self.clear_import_diagnostics();
                self.import_preview = Some(ImportPreview {
                    path: PathBuf::from(source.file_name),
                    pixels,
                    pdf: None,
                    texture: ctx.load_texture("saved source", data, egui::TextureOptions::LINEAR),
                });
                self.region_preview = None;
                self.mask_texture = None;
            }
            Err(error) => self.result(Err(error)),
        }
    }

    #[cfg(test)]
    pub(super) fn wait_for_digitize(&mut self) {
        let ctx = egui::Context::default();
        let start = Instant::now();
        while self.digitize_job.is_some() {
            self.poll_digitize(&ctx);
            assert!(start.elapsed().as_secs() < 60, "worker timed out");
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_retrace_classifies_new_regions_and_keeps_warning_ids_and_layering() {
        let mut pixels = image::RgbaImage::new(96, 48);
        for y in 4..14 {
            for x in 4..44 {
                pixels.put_pixel(x, y, image::Rgba([190, 20, 40, 255]));
            }
        }
        let mut excluded = vec![false; 96 * 48];
        for y in 30..33 {
            for x in 4..34 {
                pixels.put_pixel(x, y, image::Rgba([20, 60, 190, 255]));
                excluded[y as usize * 96 + x as usize] = true;
            }
        }
        let options = import::RasterOptions {
            cleanup: Some(
                embroidery_lab::color_selection::CleanupMask::from_masks(
                    96,
                    48,
                    &excluded,
                    &vec![false; 96 * 48],
                )
                .unwrap(),
            ),
            ..import::RasterOptions::autodigitize()
        };
        let mut project = import::load_pixels_with_report(
            &image::DynamicImage::ImageRgba8(pixels),
            std::path::Path::new("retrace.png"),
            96.,
            &options,
        )
        .unwrap()
        .0;
        let warnings =
            embroidery_lab::autodigitize::classify_project(&mut project, &AtomicBool::new(false));
        assert!(
            warnings
                .iter()
                .any(|warning| warning.message.contains("exceeds automatic Satin maximum"))
        );
        project.layering.enabled = false;
        project.layering.overlap_mm = 0.35;
        let layering = project.layering.clone();
        let original_id = project.objects[0].id;
        let mut app = Editor::workspace();
        app.project = project;
        app.auto_classify = true;
        let ctx = egui::Context::default();
        for reveal_new_region in [false, true] {
            app.redigitize_source(&ctx);
            if reveal_new_region {
                app.raster_options.cleanup = None;
            }
            app.start_trace_job(false);
            app.wait_for_digitize();
            assert!(!app.error && !app.stale, "{}", app.message);
            assert_eq!(app.project.layering, layering);
            assert_eq!(app.project.objects[0].id, original_id);
            assert!(
                app.plan_warnings
                    .iter()
                    .any(|warning| warning.object_id == original_id
                        && warning.message.contains("exceeds automatic Satin maximum"))
            );
        }
        assert_eq!(app.project.objects.len(), 2);
        assert!(app.project.objects.iter().any(
            |object| object.settings.kind == StitchKind::Satin && object.satin_column.is_some()
        ));
    }

    fn queued_result(app: &mut Editor, result: JobOutput) {
        let (sender, receiver) = mpsc::channel();
        sender.send(Ok(result)).unwrap();
        app.digitize_job = Some(DigitizeJob {
            id: app.job_revision,
            document_fingerprint: project_fingerprint(&app.project),
            import_fingerprint: None,
            cancelled: Arc::new(AtomicBool::new(false)),
            stage: Arc::new(AtomicU8::new(3)),
            receiver,
        });
    }

    #[test]
    fn generation_modal_uses_an_opaque_surface_on_its_first_visible_frame() {
        fn has_surface(shape: &egui::Shape, surface: Color32) -> bool {
            match shape {
                egui::Shape::Rect(rect) => rect.fill == surface && rect.rect.height() > 80.,
                egui::Shape::Vec(shapes) => shapes.iter().any(|shape| has_surface(shape, surface)),
                _ => false,
            }
        }
        for theme in [ui_theme::Theme::Dark, ui_theme::Theme::Light] {
            let mut app = Editor::workspace_with_language(Language::English);
            app.workspace.theme = theme;
            let ctx = egui::Context::default();
            theme.apply(&ctx);
            let (_sender, receiver) = mpsc::channel();
            app.digitize_job = Some(DigitizeJob {
                id: app.job_revision,
                document_fingerprint: project_fingerprint(&app.project),
                import_fingerprint: None,
                cancelled: Arc::new(AtomicBool::new(false)),
                stage: Arc::new(AtomicU8::new(3)),
                receiver,
            });
            let mut painted = false;
            // The first pass can measure the Area. Keep time fixed so an
            // implicit fade cannot make the next visible pass appear opaque.
            for _ in 0..2 {
                let output = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1360., 900.))),
                        time: Some(0.),
                        ..Default::default()
                    },
                    |ctx| app.digitize_progress(ctx),
                );
                painted |= output
                    .shapes
                    .iter()
                    .any(|shape| has_surface(&shape.shape, theme.tokens().surface));
            }
            assert!(
                painted,
                "{theme:?}: the generation panel must paint its opaque theme surface"
            );
        }
    }

    #[test]
    fn late_worker_cannot_replace_a_newer_revision_or_a_direct_document_edit() {
        let mut app = Editor::workspace();
        let original = serde_json::to_value(&app.stitches).unwrap();
        let mut replacement = app.project.clone();
        replacement.objects[0].contours[0][0].x += 1.;
        let plan = engine::generate_plan(&replacement).unwrap();
        queued_result(
            &mut app,
            JobOutput::Generated(PreparedPlan::new(&replacement, plan)),
        );
        app.job_revision += 1;
        app.poll_digitize(&egui::Context::default());
        assert_eq!(serde_json::to_value(&app.stitches).unwrap(), original);
        let plan = engine::generate_plan(&app.project).unwrap();
        let prepared = PreparedPlan::new(&app.project, plan);
        queued_result(&mut app, JobOutput::Generated(prepared));
        app.project.objects[0].settings.angle_deg = 31.;
        app.poll_digitize(&egui::Context::default());
        assert_eq!(serde_json::to_value(&app.stitches).unwrap(), original);
        assert!(app.message.starts_with("Outdated"));
    }

    #[test]
    fn cancelling_completed_import_keeps_document_and_last_machine_plan() {
        let mut app = Editor::workspace();
        let original = serde_json::to_value(&app.project).unwrap();
        let commands = serde_json::to_value(&app.stitches).unwrap();
        let mut replacement = Project::demo();
        replacement.name = "cancelled document".into();
        let plan = engine::generate_plan(&replacement).unwrap();
        let prepared = PreparedPlan::new(&replacement, plan);
        queued_result(&mut app, JobOutput::Imported(replacement, prepared));
        app.cancel_digitize();
        app.poll_digitize(&egui::Context::default());
        assert_eq!(serde_json::to_value(&app.project).unwrap(), original);
        assert_eq!(serde_json::to_value(&app.stitches).unwrap(), commands);
    }

    #[test]
    fn changed_physical_size_or_preparation_discards_completed_import() {
        for change_size in [true, false] {
            let mut app = Editor::workspace();
            let ctx = egui::Context::default();
            let source = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../tests/fixtures/synthetic_two_colors.jpg");
            app.request_open(source, &ctx);
            assert!(app.import_preview.is_some(), "{}", app.message);
            let original = serde_json::to_value(&app.project).unwrap();
            let frozen = app.import_fingerprint();
            let replacement = Project::demo();
            let plan = engine::generate_plan(&replacement).unwrap();
            queued_result(
                &mut app,
                JobOutput::Imported(replacement.clone(), PreparedPlan::new(&replacement, plan)),
            );
            app.digitize_job.as_mut().unwrap().import_fingerprint = Some(frozen);
            if change_size {
                app.import_width_mm *= 0.5;
            } else {
                app.raster_options.preparation = import::PreparationMode::Gentle;
            }
            app.poll_digitize(&ctx);
            assert_eq!(serde_json::to_value(&app.project).unwrap(), original);
            assert!(app.import_preview.is_some());
            assert!(app.message.starts_with("Outdated"));
        }
    }

    #[test]
    fn regeneration_uses_edited_objects_and_never_traces_source_again() {
        let mut app = Editor::workspace();
        let image = image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            32,
            32,
            image::Rgba([220, 20, 20, 255]),
        ));
        app.project.source_image = Some(
            embroidery_lab::persistence::SourceImage::from_image(
                "independent.png".into(),
                &image,
                100.,
            )
            .unwrap(),
        );
        app.project.objects[0].settings.angle_deg = 37.;
        let geometry = serde_json::to_value(&app.project.objects).unwrap();
        app.changed();
        app.generate();
        app.wait_for_digitize();
        assert!(!app.stale, "{}", app.message);
        assert_eq!(
            serde_json::to_value(&app.project.objects).unwrap(),
            geometry
        );
        assert_eq!(
            app.project.objects[0].contours.len(),
            2,
            "source would have traced one solid rectangle"
        );
    }
}
