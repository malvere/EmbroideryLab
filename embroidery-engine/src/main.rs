#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod artwork_import;
mod canvas_scene;
#[cfg(feature = "capture")]
mod capture_probe;
mod cleanup_ui;
mod context_hit;
mod context_menu;
mod digitize_job;
mod editor_tools;
mod i18n;
mod keyboard_focus;
mod layer_panels;
mod layer_selection;
#[cfg(test)]
#[path = "layer_selection_ui_tests.rs"]
mod layer_selection_ui_tests;
mod marquee;
mod marquee_hit;
#[cfg(feature = "capture")]
mod quality_capture;
mod sequence_panel;
mod stitch_3d;
mod stitch_metrics;
mod stitch_view;
mod ui_theme;
mod workspace_preferences;
mod workspace_ui;

use eframe::egui::{self, Color32, Pos2, Rect, Sense, Stroke, Vec2};
use embroidery_lab::{centerline_workflow, dsb, dst, engine, import, model::*};
use i18n::Language;
use std::{path::PathBuf, time::Instant};

#[derive(Clone, Copy, PartialEq)]
enum Tool {
    Nodes,
    Polygon,
    Satin,
    Rung,
    Outline,
    SplitRegion,
}
#[derive(Clone, Copy)]
enum Handle {
    Node(usize, usize),
    Curve(usize, usize),
    Rail(bool, usize),
    Path(usize, usize),
    RailControl(bool, usize, bool),
}

fn set_path_node(
    object: &mut Object,
    path_index: usize,
    node: usize,
    point: Point,
) -> Result<(), String> {
    if object.centerline_graph.is_some() {
        return embroidery_lab::axis_edit::move_node(object, path_index, node, point);
    }
    let Some(path) = object.running_paths.get_mut(path_index) else {
        return Err("Select a path node".into());
    };
    if node >= path.len() {
        return Err("Select a path node".into());
    }
    let closed = path.len() > 2 && path.first() == path.last();
    let last = path.len() - 1;
    path[node] = point;
    if closed && (node == 0 || node == last) {
        path[0] = point;
        path[last] = point;
    }
    Ok(())
}

struct ImportPreview {
    path: PathBuf,
    texture: egui::TextureHandle,
    pixels: std::sync::Arc<image::RgbaImage>,
    pdf: Option<artwork_import::PdfPreview>,
}

struct CenterlineJob {
    receiver: std::sync::mpsc::Receiver<
        Result<(Project, embroidery_lab::centerline::Centerlines), String>,
    >,
    cancelled: bool,
}

struct Editor {
    language: Language,
    project: Project,
    stitches: Vec<Stitch>,
    dsb_records: Option<Vec<dsb::DsbRecord>>,
    selected: usize,
    selection_anchor: Option<u64>,
    layer_focus_id: Option<egui::Id>,
    context_popup: Option<(egui::Id, Pos2)>,
    marquee: marquee::Marquee,
    tool: Tool,
    draft: Vec<Point>,
    handle: Option<Handle>,
    drag_before: Option<Project>,
    selected_node: Option<(usize, usize)>,
    selected_rail: Option<(bool, usize)>,
    selected_path: Option<(usize, usize)>,
    dirty: bool,
    stitch_only: bool,
    confirm_close: bool,
    stale: bool,
    undo: Vec<Project>,
    redo: Vec<Project>,
    field_before: Option<Project>,
    satin_first: Vec<Point>,
    rung_start: Option<(bool, f64)>,
    selected_rung: Option<usize>,
    selected_branch: Option<usize>,
    outline_width: f64,
    outline_closed: bool,
    outline_preview: Option<SatinColumn>,
    preview_branch: Option<usize>,
    smooth_tolerance: f64,
    smoothing_preview: Option<Object>,
    show_width: bool,
    last_autosave: Instant,
    recovery: Option<Project>,
    region_preview: Option<embroidery_lab::region_edit::RegionPreview>,
    mask_texture: Option<egui::TextureHandle>,
    merge_selection: Vec<u64>,
    show_quality: bool,
    quality_issues: Vec<embroidery_lab::quality::QualityIssue>,
    path: Option<PathBuf>,
    message: String,
    error: bool,
    scale: f32,
    pan: Vec2,
    fit: bool,
    show_stitches: bool,
    show_jumps: bool,
    show_nodes: bool,
    show_image: bool,
    show_3d: bool,
    preview_3d: stitch_3d::Preview3d,
    stitch_revision: u64,
    stitch_bounds: Option<stitch_metrics::StitchBounds>,
    stitch_count: usize,
    jump_count: usize,
    image: Option<(egui::TextureHandle, f64, f64)>,
    progress: usize,
    playing: bool,
    speed: f64,
    clock: Instant,
    fractional: f64,
    discard: Option<PathBuf>,
    capture_path: Option<PathBuf>,
    #[cfg(feature = "capture")]
    capture_probe: Option<capture_probe::WorkerProbe>,
    capture_requested: bool,
    render_passes: u8,
    raster_options: import::RasterOptions,
    import_width_mm: f64,
    import_preview: Option<ImportPreview>,
    centerline_pixel_mm: f64,
    centerline_job: Option<CenterlineJob>,
    pdf_job: Option<artwork_import::PdfJob>,
    cleanup: cleanup_ui::CleanupState,
    digitize_job: Option<digitize_job::DigitizeJob>,
    job_revision: u64,
    plan_metadata: Vec<CommandMeta>,
    plan_warnings: Vec<PlanWarning>,
    plan_summary: Option<embroidery_lab::quality::PlanSummary>,
    coverage: Vec<embroidery_lab::quality::CoverageEstimate>,
    segment_density: Option<embroidery_lab::quality::SegmentDensityReport>,
    show_segment_density: bool,
    import_plan: Option<MachinePlan>,
    import_trace_report: Option<import::RasterTraceReport>,
    import_changes_texture: Option<egui::TextureHandle>,
    import_preview_fingerprint: Option<u64>,
    auto_classify: bool,
    workspace: workspace_preferences::WorkspacePreferences,
    workspace_saved: String,
    workspace_retry_after: Option<Instant>,
    pan_tool: bool,
    locked_objects: std::collections::BTreeSet<u64>,
    confirm_new: bool,
    quality_window: bool,
    canvas_scene: Option<canvas_scene::Scene>,
}

impl Editor {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let language = std::env::var("EMBROIDERYLAB_LANGUAGE")
            .ok()
            .and_then(|code| Language::from_code(&code))
            .unwrap_or_else(Language::load);
        let mut app = Self::workspace_with_language(language);
        app.workspace = workspace_preferences::WorkspacePreferences::load();
        if let Ok(theme) = std::env::var("EMBROIDERYLAB_THEME") {
            app.workspace.theme = if theme == "light" {
                ui_theme::Theme::Light
            } else {
                ui_theme::Theme::Dark
            };
        }
        app.workspace_saved = serde_json::to_string(&app.workspace).unwrap_or_default();
        app.workspace.theme.apply(&cc.egui_ctx);
        app.preview_3d = stitch_3d::Preview3d::new(cc.gl.clone());
        app.preview_3d
            .set_stitches(&app.stitches, app.stitch_revision);
        #[cfg(feature = "capture")]
        let worker_probe = capture_probe::WorkerProbe::arm(&mut app);
        if let Some(path) = std::env::args_os().nth(1) {
            app.request_open(PathBuf::from(path), &cc.egui_ctx);
        } else if app.capture_path.is_none() {
            app.start_empty_document();
            match embroidery_lab::persistence::load_recovery(None) {
                Ok(copy) => app.recovery = copy,
                Err(e) => app.result(Err(e)),
            }
        }
        if cfg!(feature = "capture") && std::env::var_os("EMBROIDERYLAB_CAPTURE_EMPTY").is_some() {
            app.start_empty_document();
        }
        if cfg!(feature = "capture") && std::env::var_os("EMBROIDERYLAB_CAPTURE_3D").is_some() {
            app.show_3d = true;
        }
        #[cfg(feature = "capture")]
        {
            app.capture_probe = worker_probe.filter(|probe| probe.matches_preview(&app));
        }
        app
    }
    #[cfg(test)]
    fn workspace() -> Self {
        Self::workspace_with_language(Language::default())
    }
    fn workspace_with_language(language: Language) -> Self {
        let mut project = Project::demo();
        project.name = language.text("Independent demo").into();
        project.objects[0].name = language.text("Holed fill").into();
        let mut app = Self {
            language,
            project,
            stitches: vec![],
            dsb_records: None,
            selected: 0,
            selection_anchor: None,
            layer_focus_id: None,
            context_popup: None,
            marquee: marquee::Marquee::default(),
            tool: Tool::Nodes,
            draft: vec![],
            handle: None,
            drag_before: None,
            selected_node: None,
            selected_rail: None,
            selected_path: None,
            dirty: false,
            stitch_only: false,
            confirm_close: false,
            stale: true,
            undo: vec![],
            redo: vec![],
            field_before: None,
            satin_first: vec![],
            rung_start: None,
            selected_rung: None,
            selected_branch: None,
            outline_width: 2.,
            outline_closed: false,
            outline_preview: None,
            preview_branch: None,
            smooth_tolerance: 0.05,
            smoothing_preview: None,
            show_width: false,
            last_autosave: Instant::now(),
            recovery: None,
            region_preview: None,
            mask_texture: None,
            merge_selection: vec![],
            show_quality: false,
            quality_issues: vec![],
            path: None,
            message: "Synthetic demo. Open your artwork to begin.".into(),
            error: false,
            scale: 5.,
            pan: Vec2::ZERO,
            fit: true,
            show_stitches: true,
            show_jumps: false,
            show_nodes: true,
            show_image: true,
            show_3d: false,
            preview_3d: stitch_3d::Preview3d::default(),
            stitch_revision: 0,
            stitch_bounds: None,
            stitch_count: 0,
            jump_count: 0,
            image: None,
            progress: 0,
            playing: false,
            speed: 1000.,
            clock: Instant::now(),
            fractional: 0.,
            discard: None,
            capture_path: if cfg!(feature = "capture") {
                std::env::var_os("EMBROIDERYLAB_CAPTURE_TO").map(PathBuf::from)
            } else {
                None
            },
            capture_requested: false,
            #[cfg(feature = "capture")]
            capture_probe: None,
            render_passes: 0,
            raster_options: import::RasterOptions::autodigitize(),
            import_width_mm: 100.,
            import_preview: None,
            centerline_pixel_mm: 0.2,
            centerline_job: None,
            pdf_job: None,
            cleanup: Default::default(),
            digitize_job: None,
            job_revision: 0,
            plan_metadata: vec![],
            plan_warnings: vec![],
            plan_summary: None,
            coverage: vec![],
            segment_density: None,
            show_segment_density: false,
            import_plan: None,
            import_trace_report: None,
            import_changes_texture: None,
            import_preview_fingerprint: None,
            auto_classify: true,
            workspace: Default::default(),
            workspace_saved: String::new(),
            workspace_retry_after: None,
            pan_tool: false,
            locked_objects: Default::default(),
            confirm_new: false,
            quality_window: false,
            canvas_scene: None,
        };
        app.generate();
        #[cfg(test)]
        app.wait_for_digitize();
        app
    }
    fn guard_close(&mut self, ctx: &egui::Context) {
        if ctx.input(|i| i.viewport().close_requested()) {
            self.cancel_digitize();
            self.import_preview = None;
            if let Some(job) = &mut self.centerline_job {
                job.cancelled = true;
            }
            if let Some(job) = &mut self.pdf_job {
                job.cancelled = true;
            }
        }
        if ctx.input(|i| i.viewport().close_requested()) && self.dirty {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.confirm_close = true;
        }
    }
    fn capture_frame(&mut self, ctx: &egui::Context) {
        #[cfg(feature = "capture")]
        if self.capture_quality_workflow(ctx) {
            return;
        }
        #[cfg(feature = "capture")]
        if self.capture_worker_probe(ctx) {
            return;
        }
        let Some(path) = self.capture_path.clone() else {
            return;
        };
        if self.digitize_job.is_some() || self.pdf_job.is_some() {
            ctx.request_repaint();
            return;
        }
        self.render_passes = self.render_passes.saturating_add(1);
        let frame_image = ctx.input(|i| {
            i.events.iter().find_map(|event| {
                if let egui::Event::Screenshot { image, .. } = event {
                    Some(image.clone())
                } else {
                    None
                }
            })
        });
        if let Some(frame_image) = frame_image {
            let rgba: Vec<u8> = frame_image
                .pixels
                .iter()
                .flat_map(|pixel| pixel.to_array())
                .collect();
            if let Err(error) = image::save_buffer(
                &path,
                &rgba,
                frame_image.width() as u32,
                frame_image.height() as u32,
                image::ColorType::Rgba8,
            ) {
                eprintln!("Capture failed: {error}");
            }
            self.capture_path = None;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        } else if self.render_passes >= 3 && !self.capture_requested {
            // Public screenshot API reads the rendered buffer before swap;
            // post-swap reads can contain discarded regions on VMware OpenGL.
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
            self.capture_requested = true;
        }
        ctx.request_repaint();
    }
    fn result(&mut self, result: Result<String, String>) {
        match result {
            Ok(msg) => {
                self.message = msg;
                self.error = false;
            }
            Err(msg) => {
                self.message = msg;
                self.error = true;
            }
        }
    }
    fn changed(&mut self) {
        self.canvas_scene = None;
        self.cancel_digitize();
        self.outline_preview = None;
        self.smoothing_preview = None;
        self.dirty = true;
        self.stale = true;
        self.playing = false;
        self.message =
            "Geometry or parameters changed. Generate stitches to update the preview.".into();
        self.error = false;
    }
    fn snapshot(&mut self) {
        self.finish_field_edit();
        self.push_history(self.project.clone());
    }
    fn create_centerlines(&mut self) {
        if self.stitch_only
            || self.centerline_job.is_some()
            || self.selected >= self.project.objects.len()
        {
            return;
        }
        let project = self.project.clone();
        let selected = self.selected;
        let pixel_mm = self.centerline_pixel_mm;
        let (sender, receiver) = std::sync::mpsc::channel();
        match std::thread::Builder::new()
            .name("centerline-extraction".into())
            .spawn(move || {
                let result = centerline_workflow::running_copy(&project, selected, pixel_mm);
                let _ = sender.send(result);
            }) {
            Ok(_) => {
                self.playing = false;
                self.centerline_job = Some(CenterlineJob {
                    receiver,
                    cancelled: false,
                });
            }
            Err(error) => self.result(Err(format!("Cannot start centerline extraction: {error}"))),
        }
    }
    fn apply_centerlines(
        &mut self,
        result: Result<(Project, embroidery_lab::centerline::Centerlines), String>,
    ) {
        match result {
            Ok((mut project, report)) => {
                project.objects[self.selected + 1].name = format!(
                    "{} · {}",
                    self.project.objects[self.selected].name,
                    self.language.text("centerlines")
                );
                self.snapshot();
                self.project = project;
                self.selected += 1;
                self.selected_node = None;
                self.selected_rail = None;
                self.selected_path = None;
                self.handle = None;
                self.changed();
                self.generate();
                if !self.error {
                    self.message = format!(
                        "Created {} running paths; {} isolated points skipped. Original area hidden. Undo restores it.",
                        report.paths.len(),
                        report.isolated_points.len()
                    );
                }
            }
            Err(error) => self.result(Err(error)),
        }
    }
    fn centerline_progress(&mut self, ctx: &egui::Context) {
        let language = self.language;
        let Some(job) = &self.centerline_job else {
            return;
        };
        let result = match job.receiver.try_recv() {
            Ok(result) => Some(result),
            Err(std::sync::mpsc::TryRecvError::Empty) => None,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => Some(Err(
                "Centerline extraction stopped. The document is unchanged".into(),
            )),
        };
        if let Some(result) = result {
            let cancelled = self.centerline_job.take().is_some_and(|job| job.cancelled);
            if !cancelled {
                self.apply_centerlines(result);
            }
            return;
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(50));
        if !job.cancelled {
            egui::Modal::new(egui::Id::new("centerline-progress")).show(ctx,|ui| {
                ui.heading(language.text("Extracting centerlines"));
                ui.label(language.text("Finding the axes of the selected area. The document is unchanged until this finishes."));
                ui.label(language.text("Large or thick areas take longer. Cancel keeps your current project."));
                if ui.button(language.text("Cancel")).clicked() {
                    if let Some(job)=&mut self.centerline_job {job.cancelled=true;}
                    self.result(Ok("Centerline extraction cancelled. Document unchanged.".into()));
                }
            });
        }
    }
    fn open_dialog(&mut self, ctx: &egui::Context) {
        let language = self.language;
        if let Some(path) = rfd::FileDialog::new()
            .add_filter(
                language.text("Artwork / project / DST / DSB"),
                &[
                    "jpg", "jpeg", "png", "webp", "pdf", "svg", "elab", "json", "dst", "dsb",
                ],
            )
            .add_filter(
                language.text("Images (JPEG, PNG, WebP, PDF, SVG)"),
                &["jpg", "jpeg", "png", "webp", "pdf", "svg"],
            )
            .pick_file()
        {
            if self.dirty {
                self.discard = Some(path);
            } else {
                self.request_open(path, ctx);
            }
        }
    }
    fn request_open(&mut self, path: PathBuf, ctx: &egui::Context) {
        self.cancel_digitize();
        self.context_popup = None;
        let ext = path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if ext == "pdf" {
            self.start_pdf_load(path, 0, ctx);
            return;
        }
        if !["png", "jpg", "jpeg", "webp"].contains(&ext.as_str()) {
            self.open(path, ctx);
            return;
        }
        let preview = import::decode_raster_file(&path);
        match preview {
            Ok(image) => {
                self.clear_import_diagnostics();
                self.region_preview = None;
                self.mask_texture = None;
                self.raster_options.cleanup = None;
                self.raster_options.background = import::BackgroundMode::TransparentOnly;
                self.cleanup.reset(&image.to_rgba8(), None);
                let pixels =
                    embroidery_lab::color_selection::thumbnail_rgba(&image.to_rgba8(), 480);
                let data = egui::ColorImage::from_rgba_unmultiplied(
                    [pixels.width() as usize, pixels.height() as usize],
                    pixels.as_raw(),
                );
                self.import_preview = Some(ImportPreview {
                    path,
                    pixels: std::sync::Arc::new(image.to_rgba8()),
                    pdf: None,
                    texture: ctx.load_texture("import preview", data, egui::TextureOptions::LINEAR),
                });
                self.error = false;
            }
            Err(error) => self.result(Err(error)),
        }
    }
    /// Returns whether a document was loaded. Generation failure is a separate,
    /// recoverable editor state after valid geometry has been committed.
    fn open(&mut self, path: PathBuf, ctx: &egui::Context) -> bool {
        self.cancel_digitize();
        let ext = path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if ext == "dst" || ext == "dsb" {
            let decoded = std::fs::read(&path)
                .map_err(|e| e.to_string())
                .and_then(|bytes| {
                    if ext == "dsb" {
                        let records = dsb::decode(&bytes)?;
                        let preview = dsb::preview(&records)?;
                        Ok((preview, Some(records)))
                    } else {
                        dst::decode(&bytes).map(|stitches| (stitches, None))
                    }
                });
            let loaded = decoded.is_ok();
            match decoded {
                Ok((s, records)) => {
                    self.project = Project::default();
                    self.locked_objects.clear();
                    self.project.name = path
                        .file_stem()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into();
                    self.stitches = s;
                    self.plan_metadata.clear();
                    self.plan_warnings.clear();
                    self.plan_summary = None;
                    self.coverage.clear();
                    self.segment_density = None;
                    self.update_stitch_view();
                    self.preview_3d.reset_view();
                    self.quality_issues =
                        embroidery_lab::quality::analyze(&self.stitches, Default::default());
                    self.dsb_records = records;
                    self.progress = self.stitches.len();
                    self.image = None;
                    self.stitch_only = true;
                    self.selection_anchor = None;
                    self.merge_selection.clear();
                    self.tool = Tool::Nodes;
                    self.draft.clear();
                    self.selected_node = None;
                    self.selected_rail = None;
                    self.path = None;
                    self.dirty = false;
                    self.stale = false;
                    self.fit = true;
                    self.undo.clear();
                    self.redo.clear();
                    self.field_before = None;
                    self.clear_selection();
                    self.cancel_draft();
                    self.result(Ok(if ext=="dsb" {
                        "DSB view: raw trim, stop and needle commands are retained for DSB re-export. Preview colors are illustrative."
                    } else {"Opened machine stitches. DST contains no editable vector objects or exact thread colors."}.into()));
                }
                Err(e) => self.result(Err(e)),
            }
            return loaded;
        }
        let loaded = if ext == "elab" || ext == "json" {
            Project::load(&path)
        } else if ext == "pdf"
            && self
                .import_preview
                .as_ref()
                .is_some_and(|p| p.path == path && p.pdf.is_some())
        {
            self.trace_import_preview()
        } else if ["png", "jpg", "jpeg", "webp"].contains(&ext.as_str()) {
            import::load_image_with_options(&path, self.import_width_mm, &self.raster_options)
        } else {
            import::load(&path, self.import_width_mm)
        };
        let loaded = loaded.and_then(|p| {
            if ["png", "jpg", "jpeg", "webp", "pdf"].contains(&ext.as_str())
                && let Some(preview) = &self.region_preview
            {
                preview.apply_overrides_to(p).map(|preview| preview.project)
            } else {
                Ok(p)
            }
        });
        match loaded {
            Ok(mut p) => {
                if self.dirty {
                    let _ = embroidery_lab::persistence::clear_recovery(self.path.as_deref());
                }
                if ["png", "jpg", "jpeg", "webp", "pdf"].contains(&ext.as_str()) {
                    for object in &mut p.objects {
                        object.name = self.language.message(&object.name).into_owned();
                    }
                }
                self.project = p;
                self.locked_objects.clear();
                self.dsb_records = None;
                self.stitch_only = false;
                self.selected = self
                    .project
                    .objects
                    .iter()
                    .position(|o| o.visible)
                    .unwrap_or(0);
                self.selected_node = None;
                self.selected_rail = None;
                self.draft.clear();
                self.image = None;
                self.restore_source(ctx);
                self.path = if ext == "elab" || ext == "json" {
                    Some(path)
                } else {
                    None
                };
                self.undo.clear();
                self.redo.clear();
                self.merge_selection.clear();
                self.selection_anchor = None;
                self.field_before = None;
                self.clear_selection();
                self.cancel_draft();
                self.dirty = self.path.is_none();
                self.last_autosave = Instant::now();
                self.recovery = None;
                if self.capture_path.is_none() && self.path.is_some() {
                    match embroidery_lab::persistence::load_recovery(self.path.as_deref()) {
                        Ok(copy) => self.recovery = copy,
                        Err(e) => self.result(Err(e)),
                    }
                }
                self.fit = true;
                self.generate();
                self.preview_3d.reset_view();
                true
            }
            Err(e) => {
                self.result(Err(e));
                false
            }
        }
    }
    fn save(&mut self) {
        self.finish_field_edit();
        let language = self.language;
        if self.stitch_only {
            self.result(Err("Machine files contain stitches only. Re-export the machine format to retain the sequence, or open artwork for an editable project.".into()));
            return;
        }
        let path = self.path.clone().or_else(|| {
            rfd::FileDialog::new()
                .add_filter(language.text("EmbroideryLab project"), &["elab"])
                .set_file_name(format!("{}.elab", self.project.name))
                .save_file()
        });
        if let Some(path) = path {
            let r = self
                .project
                .save(&path)
                .map(|_| format!("Saved {}", path.display()));
            if r.is_ok() {
                let _ = embroidery_lab::persistence::clear_recovery(self.path.as_deref());
                let _ = embroidery_lab::persistence::clear_recovery(Some(&path));
                self.dirty = false;
                self.path = Some(path);
                self.last_autosave = Instant::now();
            }
            self.result(r);
        }
    }
    fn export(&mut self) {
        let language = self.language;
        if self.dsb_records.is_some() {
            self.result(Err("DSB trim and needle controls are retained for DSB re-export. DST conversion is unavailable because it would discard these controls.".into()));
            return;
        }
        if self.stale {
            self.result(Err(
                "Generate embroidery and wait for completion before exporting.".into(),
            ));
        }
        if self.stale {
            return;
        }
        if let Some(path) = rfd::FileDialog::new()
            .add_filter(language.text("Tajima DST"), &["dst"])
            .set_file_name(format!("{}.dst", self.project.name))
            .save_file()
        {
            let result = dst::encode_with_trim(
                &self.stitches,
                &self.project.name,
                self.project.machine_profile.dst_trim_encoding,
            )
            .and_then(|data| {
                dst::decode(&data)?;
                std::fs::write(&path, data).map_err(|e| e.to_string())?;
                let sidecar =
                    embroidery_lab::commands::thread_sidecar(&self.project, &self.stitches);
                std::fs::write(
                    path.with_extension("threads.json"),
                    serde_json::to_vec_pretty(&sidecar).map_err(|e| e.to_string())?,
                )
                .map_err(|e| e.to_string())?;
                Ok(format!("Exported and decoded {}", path.display()))
            });
            self.result(result);
        }
    }
    fn dsb_export_bytes(&mut self) -> Result<Vec<u8>, String> {
        if self.stale {
            self.result(Err(
                "Generate embroidery and wait for completion before exporting.".into(),
            ));
        }
        if self.stale {
            return Err(self.message.clone());
        }
        let records = if let Some(records) = &self.dsb_records {
            records.clone()
        } else {
            dsb::from_stitches(&self.stitches)?
        };
        let bytes = dsb::encode(&records, &self.project.name)?;
        dsb::decode(&bytes)?;
        Ok(bytes)
    }
    fn export_dsb(&mut self) {
        let language = self.language;
        let bytes = match self.dsb_export_bytes() {
            Ok(bytes) => bytes,
            Err(error) => {
                self.result(Err(error));
                return;
            }
        };
        if let Some(path) = rfd::FileDialog::new()
            .set_title(
                language.text("Experimental DSB export — generated color changes use manual stops"),
            )
            .add_filter(language.text("Tajima binary DSB"), &["dsb"])
            .set_file_name(format!("{}.dsb", self.project.name))
            .save_file()
        {
            let result = (|| {
                std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
                let sidecar =
                    embroidery_lab::commands::thread_sidecar(&self.project, &self.stitches);
                std::fs::write(
                    path.with_extension("threads.json"),
                    serde_json::to_vec_pretty(&sidecar).map_err(|e| e.to_string())?,
                )
                .map_err(|e| e.to_string())?;
                Ok(format!("Exported experimental DSB: {}", path.display()))
            })();
            self.result(result);
        }
    }
    fn add_object(&mut self, curves: bool) {
        let language = self.language;
        self.snapshot();
        let id = self.project.objects.iter().map(|o| o.id).max().unwrap_or(0) + 1;
        let mut object = Object {
            id,
            name: language.message(&format!("Object {id}")).into_owned(),
            ..Object::default()
        };
        if curves {
            object.name = language.message(&format!("Bezier {id}")).into_owned();
            object.bezier_segments = vec![
                CubicBezier {
                    start: Point::new(15., 20.),
                    control1: Point::new(15., 0.),
                    control2: Point::new(65., 0.),
                    end: Point::new(65., 20.),
                },
                CubicBezier {
                    start: Point::new(65., 20.),
                    control1: Point::new(65., 45.),
                    control2: Point::new(15., 45.),
                    end: Point::new(15., 20.),
                },
            ];
        } else {
            object.contours = vec![vec![
                Point::new(10., 10.),
                Point::new(40., 10.),
                Point::new(40., 30.),
                Point::new(10., 30.),
            ]];
        }
        self.project.objects.push(object);
        self.selected = self.project.objects.len() - 1;
        self.changed();
    }
    fn complete_polygon(&mut self) {
        let language = self.language;
        if self.draft.len() < 3 {
            self.result(Err(
                "Place at least three points before completing a closed contour.".into(),
            ));
            return;
        }
        self.snapshot();
        let id = self.project.objects.iter().map(|o| o.id).max().unwrap_or(0) + 1;
        self.project.objects.push(Object {
            id,
            name: language.message(&format!("Contour {id}")).into_owned(),
            contours: vec![std::mem::take(&mut self.draft)],
            ..Object::default()
        });
        self.selected = self.project.objects.len() - 1;
        self.tool = Tool::Nodes;
        self.changed();
    }
    #[cfg(test)]
    fn add_satin_column(&mut self) {
        let language = self.language;
        self.snapshot();
        let id = self.project.objects.iter().map(|o| o.id).max().unwrap_or(0) + 1;
        self.project.objects.push(Object {
            id,
            name: language.message(&format!("Satin column {id}")).into_owned(),
            satin_column: Some(SatinColumn {
                left: vec![
                    Point::new(15., 10.),
                    Point::new(20., 25.),
                    Point::new(40., 40.),
                ],
                right: vec![
                    Point::new(22., 10.),
                    Point::new(27., 22.),
                    Point::new(44., 34.),
                ],
                ..SatinColumn::default()
            }),
            settings: Settings {
                kind: StitchKind::Satin,
                length_mm: 8.,
                ..Settings::default()
            },
            ..Object::default()
        });
        self.selected = self.project.objects.len() - 1;
        self.selected_node = None;
        self.selected_rail = None;
        self.changed();
        self.fit = true;
    }
    #[cfg(test)]
    fn pick_background_color(&mut self, color: [u8; 3], add: bool) {
        use import::BackgroundMode as B;
        let old = &self.raster_options.background;
        let connected = !matches!(old, B::Color { .. } | B::Colors { .. } | B::Border { .. });
        let tolerance = match old {
            B::Color { tolerance, .. }
            | B::Colors { tolerance, .. }
            | B::ConnectedColors { tolerance, .. }
            | B::Border { tolerance }
            | B::ConnectedBorder { tolerance } => *tolerance,
            _ => 35.,
        };
        let mut rgb = if add {
            match old {
                B::Color { rgb, .. } => vec![*rgb],
                B::Colors { rgb, .. } | B::ConnectedColors { rgb, .. } => rgb.clone(),
                _ => vec![],
            }
        } else {
            vec![]
        };
        if rgb.len() < 16 && !rgb.contains(&color) {
            rgb.push(color);
        }
        self.raster_options.background = if connected {
            B::ConnectedColors { rgb, tolerance }
        } else {
            B::Colors { rgb, tolerance }
        };
    }
    #[cfg(test)]
    fn set_background_connectivity(&mut self, connected: bool) {
        use import::BackgroundMode as B;
        self.raster_options.background = match self.raster_options.background.clone() {
            B::Color { rgb, tolerance } => {
                if connected {
                    B::ConnectedColors {
                        rgb: vec![rgb],
                        tolerance,
                    }
                } else {
                    B::Color { rgb, tolerance }
                }
            }
            B::Colors { rgb, tolerance } | B::ConnectedColors { rgb, tolerance } => {
                if connected {
                    B::ConnectedColors { rgb, tolerance }
                } else {
                    B::Colors { rgb, tolerance }
                }
            }
            B::Border { tolerance } | B::ConnectedBorder { tolerance } => {
                if connected {
                    B::ConnectedBorder { tolerance }
                } else {
                    B::Border { tolerance }
                }
            }
            other => other,
        };
    }
    fn import_window(&mut self, ctx: &egui::Context) {
        self.autodigitize_import_window(ctx);
    }
    fn cancel_import(&mut self) {
        self.cancel_digitize();
        self.clear_import_diagnostics();
        self.cleanup = Default::default();
        self.import_preview = None;
        self.region_preview = None;
        self.mask_texture = None;
    }
    fn commit_import(&mut self, _ctx: &egui::Context) {
        self.start_trace_job(false);
    }
    fn shortcuts(&mut self, ctx: &egui::Context) {
        if self.import_preview.is_some()
            || self.pdf_job.as_ref().is_some_and(|job| !job.cancelled)
            || self.decision_pending()
            || self
                .centerline_job
                .as_ref()
                .is_some_and(|job| !job.cancelled)
        {
            return;
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::O)) {
            self.open_dialog(ctx);
            if self.decision_pending() {
                return;
            }
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::S)) {
            self.save();
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::E)) {
            self.export();
        }
        let text_input = keyboard_focus::text_input_active(ctx);
        // egui's logical modifier matching ignores extra Shift: redo must
        // consume Ctrl+Shift+Z before the less-specific Ctrl+Z binding.
        if !text_input
            && (ctx.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::Y))
                || ctx.input_mut(|i| {
                    i.consume_key(egui::Modifiers::CTRL | egui::Modifiers::SHIFT, egui::Key::Z)
                }))
        {
            self.redo_edit();
        }
        if !text_input && ctx.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::Z)) {
            self.undo_edit();
        }
        let layer_list_focused = self
            .layer_focus_id
            .is_some_and(|id| ctx.memory(|memory| memory.has_focus(id)));
        if (!text_input || layer_list_focused)
            && !self.stitch_only
            && self.tool == Tool::Nodes
            && self.draft.is_empty()
            && self.satin_first.is_empty()
            && self.rung_start.is_none()
            && self.outline_preview.is_none()
            && self.smoothing_preview.is_none()
            && self.context_popup.is_none()
            && !egui::Popup::is_any_open(ctx)
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::A))
        {
            self.select_all_layers();
        }
        if !text_input {
            if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::V)) {
                self.pan_tool = false;
                self.set_3d_view(false);
                self.cancel_draft();
            }
            if !self.show_3d
                && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::H))
            {
                self.cancel_draft();
                self.pan_tool = true;
            }
            if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::F)) {
                self.fit_view();
            }
            if !self.stitch_only && !self.show_3d {
                if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::P)) {
                    self.pan_tool = false;
                    self.cancel_draft();
                    self.tool = Tool::Polygon;
                }
                if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::S)) {
                    self.pan_tool = false;
                    self.cancel_draft();
                    self.tool = Tool::Satin;
                }
            }
            if ctx.input(|i| i.key_pressed(egui::Key::Enter)) && self.tool == Tool::Polygon {
                self.complete_polygon();
            }
            if ctx.input(|i| i.key_pressed(egui::Key::Enter)) && self.tool == Tool::Satin {
                self.complete_satin();
            }
            if ctx.input(|i| i.key_pressed(egui::Key::Enter)) && self.tool == Tool::Outline {
                self.preview_outline();
            }
            if ctx.input(|i| i.key_pressed(egui::Key::Enter)) && self.tool == Tool::SplitRegion {
                self.complete_region_split();
            }
            if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                self.cancel_draft();
            }
            if ctx.input(|i| i.key_pressed(egui::Key::Delete)) {
                self.delete_selection();
            }
        }
    }
    fn canvas(&mut self, ui: &mut egui::Ui) {
        if self.decision_pending() {
            ui.disable();
        }
        let language = self.language;
        let colors = self.workspace.theme.tokens();
        if self.canvas_scene.is_none() && !ui.input(|i| i.pointer.any_down()) {
            self.canvas_scene = Some(canvas_scene::Scene::build(&self.project));
        }
        let panning = self.pan_tool
            || (!keyboard_focus::text_input_active(ui.ctx())
                && ui.input(|i| i.key_down(egui::Key::Space)));
        let (response, p) = ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
        let rect = response.rect;
        p.rect_filled(rect, 0., colors.canvas);
        if self.fit {
            let points: Vec<Point> = self
                .project
                .objects
                .iter()
                .filter(|o| o.visible)
                .flat_map(|o| o.flattened_contours())
                .flatten()
                .chain(
                    self.project
                        .objects
                        .iter()
                        .filter(|o| o.visible)
                        .flat_map(|o| o.running_paths.iter().flatten())
                        .copied(),
                )
                .chain(self.stitches.iter().map(|s| s.point()))
                .collect();
            if !points.is_empty() {
                let minx = points.iter().map(|p| p.x).fold(f64::INFINITY, f64::min);
                let maxx = points.iter().map(|p| p.x).fold(f64::NEG_INFINITY, f64::max);
                let miny = points.iter().map(|p| p.y).fold(f64::INFINITY, f64::min);
                let maxy = points.iter().map(|p| p.y).fold(f64::NEG_INFINITY, f64::max);
                self.scale = ((rect.width() - 90.) / ((maxx - minx) as f32).max(10.))
                    .min((rect.height() - 90.) / ((maxy - miny) as f32).max(10.))
                    .clamp(0.2, 60.);
                self.pan = Vec2::new(
                    -((minx + maxx) * 0.5) as f32 * self.scale,
                    -((miny + maxy) * 0.5) as f32 * self.scale,
                );
            }
            self.fit = false;
        }
        if response.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll != 0. {
                let old = self.scale;
                self.scale = (self.scale * (scroll * 0.002).exp()).clamp(0.2, 100.);
                if let Some(pos) = response.hover_pos() {
                    let delta = pos - rect.center();
                    self.pan = delta - (delta - self.pan) * (self.scale / old);
                }
            }
        }
        if response.dragged_by(egui::PointerButton::Middle)
            || response.drag_stopped_by(egui::PointerButton::Middle)
            || (panning
                && (response.dragged_by(egui::PointerButton::Primary)
                    || response.drag_stopped_by(egui::PointerButton::Primary)))
        {
            self.pan += ui.input(|i| i.pointer.delta());
        }
        let center = rect.center() + self.pan;
        let scale = self.scale;
        let screen = |q: Point| center + Vec2::new(q.x as f32 * scale, q.y as f32 * scale);
        let world = |q: Pos2| {
            Point::new(
                ((q.x - center.x) / scale) as f64,
                ((q.y - center.y) / scale) as f64,
            )
        };
        let selecting_areas = if panning {
            self.marquee = Default::default();
            self.handle = None;
            self.drag_before = None;
            false
        } else {
            self.update_marquee(ui, &response, center, scale)
        };
        if response.hovered() {
            ui.ctx().set_cursor_icon(if panning {
                if response.dragged() {
                    egui::CursorIcon::Grabbing
                } else {
                    egui::CursorIcon::Grab
                }
            } else if self.tool != Tool::Nodes {
                egui::CursorIcon::Crosshair
            } else {
                egui::CursorIcon::Default
            });
        }
        if !panning
            && self.tool == Tool::Nodes
            && response.clicked()
            && let Some(pos) = response.interact_pointer_pos()
        {
            self.select_context_at(world(pos));
        }
        if response.secondary_clicked()
            && !selecting_areas
            && let Some(pos) = response.interact_pointer_pos()
        {
            self.select_context_at(world(pos));
        }
        let step = 10. * scale;
        if self.workspace.grid && step >= 12. {
            let x0 = rect.left() + ((center.x - rect.left()) % step + step) % step;
            let y0 = rect.top() + ((center.y - rect.top()) % step + step) % step;
            let mut x = x0;
            while x < rect.right() {
                p.line_segment(
                    [Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())],
                    Stroke::new(0.5_f32, colors.grid),
                );
                x += step;
            }
            let mut y = y0;
            while y < rect.bottom() {
                p.line_segment(
                    [Pos2::new(rect.left(), y), Pos2::new(rect.right(), y)],
                    Stroke::new(0.5_f32, colors.grid),
                );
                y += step;
            }
        }
        if let Some(scene) = &self.canvas_scene
            && let Some((a, b)) = scene.bounds
        {
            let page = Rect::from_two_pos(screen(a), screen(b)).expand(18.);
            p.rect_filled(page, 0., colors.paper);
            if self.workspace.grid && step >= 12. {
                let paper = p.with_clip_rect(page.intersect(rect));
                let mut x = rect.left() + ((center.x - rect.left()) % step + step) % step;
                while x < rect.right() {
                    paper.line_segment(
                        [Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())],
                        Stroke::new(0.5_f32, colors.paper_grid),
                    );
                    x += step;
                }
                let mut y = rect.top() + ((center.y - rect.top()) % step + step) % step;
                while y < rect.bottom() {
                    paper.line_segment(
                        [Pos2::new(rect.left(), y), Pos2::new(rect.right(), y)],
                        Stroke::new(0.5_f32, colors.paper_grid),
                    );
                    y += step;
                }
            }
        }

        if self.show_image
            && let Some((texture, w, h)) = &self.image
        {
            p.image(
                texture.id(),
                Rect::from_two_pos(screen(Point::default()), screen(Point::new(*w, *h))),
                Rect::from_min_max(Pos2::ZERO, Pos2::new(1., 1.)),
                Color32::from_white_alpha(90),
            );
        }
        if !self.show_stitches
            && let Some(scene) = &self.canvas_scene
        {
            scene.paint_fills(&p, center, scale, rect);
        }
        let artwork_ink = ui_theme::artwork_ink();
        for (i, o) in self.project.objects.iter().enumerate() {
            if !o.visible {
                continue;
            }
            let col = if i == self.selected {
                artwork_ink.accent
            } else {
                artwork_ink.muted
            };
            let cached = self
                .canvas_scene
                .as_ref()
                .and_then(|scene| scene.objects.iter().find(|object| object.id == o.id));
            let fallback;
            let rings = if let Some(cached) = cached {
                &cached.rings
            } else {
                fallback = o.flattened_contours();
                &fallback
            };
            for ring in rings {
                for n in 0..ring.len() {
                    p.line_segment(
                        [screen(ring[n]), screen(ring[(n + 1) % ring.len()])],
                        Stroke::new(if i == self.selected { 1.7_f32 } else { 0.8_f32 }, col),
                    );
                }
            }
            for (branch, path) in o.running_paths.iter().enumerate() {
                for pair in path.windows(2) {
                    p.line_segment(
                        [screen(pair[0]), screen(pair[1])],
                        Stroke::new(
                            if i == self.selected && self.selected_branch == Some(branch) {
                                3.
                            } else if i == self.selected {
                                1.7_f32
                            } else {
                                0.8_f32
                            },
                            col,
                        ),
                    );
                }
                if self.show_width && i == self.selected && self.selected_branch == Some(branch) {
                    for point in path.iter().step_by((path.len() / 8).max(1)) {
                        if let Some(width) = embroidery_lab::axis_edit::width_at(o, *point) {
                            p.circle_stroke(
                                screen(*point),
                                (width as f32 * self.scale * 0.5).min(200.),
                                Stroke::new(0.5_f32, col),
                            );
                            p.text(
                                screen(*point) + Vec2::new(5., -5.),
                                egui::Align2::LEFT_BOTTOM,
                                format!("{width:.2} mm"),
                                egui::FontId::proportional(11.),
                                col,
                            );
                        }
                    }
                }
            }
        }
        if self.show_stitches && !self.stale {
            let mut prev = Point::default();
            for s in self.stitches.iter().take(self.progress) {
                match s.command {
                    Command::Stitch => {
                        p.line_segment(
                            [screen(prev), screen(s.point())],
                            Stroke::new(
                                0.8_f32,
                                Color32::from_rgb(s.color[0], s.color[1], s.color[2]),
                            ),
                        );
                    }
                    Command::Jump if self.show_jumps => {
                        p.line_segment(
                            [screen(prev), screen(s.point())],
                            Stroke::new(0.7_f32, artwork_ink.warning),
                        );
                    }
                    _ => {}
                }
                prev = s.point();
            }
            if self.progress > 0
                && let Some(s) = self.stitches.get(self.progress - 1)
            {
                p.circle_filled(screen(s.point()), 4., artwork_ink.accent);
            }
        }
        if self.show_segment_density
            && !self.stale
            && let Some(report) = &self.segment_density
        {
            let maximum = report
                .cells
                .iter()
                .map(|cell| cell.total_mm)
                .fold(0., f64::max)
                .max(1e-9);
            for cell in &report.cells {
                let strength = (cell.total_mm / maximum).sqrt();
                p.rect_filled(
                    Rect::from_two_pos(screen(cell.min), screen(cell.max)),
                    0.,
                    Color32::from_rgba_unmultiplied(200, 60, 30, (strength * 90.) as u8),
                );
            }
        }
        if self.show_quality && !self.stale {
            for warning in self.plan_warnings.iter().take(3000) {
                if let Some(point) = warning.point {
                    p.circle_stroke(screen(point), 5., Stroke::new(1.5_f32, artwork_ink.error));
                }
            }
            for issue in self.quality_issues.iter().take(3000) {
                p.circle_stroke(
                    screen(issue.point),
                    5.,
                    Stroke::new(1.5_f32, artwork_ink.error),
                );
            }
        }
        self.paint_selected_areas(&p, center, scale);
        let mut handles = vec![];
        if self.show_nodes
            && self.tool == Tool::Nodes
            && let Some(o) = self.project.objects.get(self.selected)
        {
            for (r, path) in o.running_paths.iter().enumerate() {
                for (n, point) in path.iter().enumerate() {
                    // A closed path's duplicated endpoint shares one editable handle.
                    if n > 0 && n + 1 == path.len() && path.first() == path.last() {
                        continue;
                    }
                    let pos = screen(*point);
                    p.circle_filled(
                        pos,
                        3.0,
                        if self.selected_path == Some((r, n)) {
                            artwork_ink.warning
                        } else {
                            artwork_ink.handle
                        },
                    );
                    p.circle_stroke(pos, 3.0, Stroke::new(1_f32, artwork_ink.accent));
                    handles.push((pos, Handle::Path(r, n)));
                }
            }
            for (r, ring) in o.contours.iter().enumerate() {
                for (n, q) in ring.iter().enumerate() {
                    let pos = screen(*q);
                    p.rect_filled(
                        Rect::from_center_size(pos, Vec2::splat(6.)),
                        0.,
                        if self.selected_node == Some((r, n)) {
                            artwork_ink.warning
                        } else {
                            artwork_ink.handle
                        },
                    );
                    p.rect_stroke(
                        Rect::from_center_size(pos, Vec2::splat(6.)),
                        0.,
                        Stroke::new(1_f32, artwork_ink.accent),
                        egui::StrokeKind::Middle,
                    );
                    handles.push((pos, Handle::Node(r, n)));
                }
            }
            for (b, c) in o.bezier_segments.iter().enumerate() {
                for (a, h) in [(c.start, c.control1), (c.end, c.control2)] {
                    p.line_segment(
                        [screen(a), screen(h)],
                        Stroke::new(0.7_f32, artwork_ink.muted),
                    );
                }
                for (n, q) in [c.start, c.control1, c.control2, c.end].iter().enumerate() {
                    p.circle_filled(screen(*q), 3.5, artwork_ink.accent);
                    handles.push((screen(*q), Handle::Curve(b, n)));
                }
            }
            if let Some(column) = &o.satin_column {
                for (right, rail) in [(false, &column.left), (true, &column.right)] {
                    let color = if right {
                        artwork_ink.warning
                    } else {
                        artwork_ink.accent
                    };
                    let flattened = column.flatten_rail(right, 0.02).unwrap_or_default();
                    for pair in flattened.windows(2) {
                        p.line_segment(
                            [screen(pair[0]), screen(pair[1])],
                            Stroke::new(2_f32, color),
                        );
                    }
                    for (n, q) in rail.iter().enumerate() {
                        if column.closed && n + 1 == rail.len() && rail.first() == rail.last() {
                            continue;
                        }
                        p.circle_filled(
                            screen(*q),
                            4.5,
                            if self.selected_rail == Some((right, n)) {
                                artwork_ink.warning
                            } else {
                                color
                            },
                        );
                        handles.push((screen(*q), Handle::Rail(right, n)));
                        let controls = if right {
                            &column.right_handles
                        } else {
                            &column.left_handles
                        };
                        if self.selected_rail == Some((right, n))
                            && let Some(control) = controls.get(n)
                        {
                            for (outgoing, point) in
                                [(false, control.incoming), (true, control.outgoing)]
                            {
                                if let Some(point) = point {
                                    p.line_segment(
                                        [screen(*q), screen(point)],
                                        Stroke::new(0.7_f32, color),
                                    );
                                    p.circle_stroke(screen(point), 4., Stroke::new(1_f32, color));
                                    handles.push((
                                        screen(point),
                                        Handle::RailControl(right, n, outgoing),
                                    ));
                                }
                            }
                        }
                    }
                }
            }
        }
        if let Some(column) = self
            .project
            .objects
            .get(self.selected)
            .and_then(|o| o.satin_column.as_ref())
        {
            let left = column.flatten_rail(false, 0.02).unwrap_or_default();
            let right = column.flatten_rail(true, 0.02).unwrap_or_default();
            for (i, rung) in column.rungs.iter().enumerate() {
                if let (Some(a), Some(b)) = (
                    editor_tools::rail_sample(&left, rung.left),
                    editor_tools::rail_sample(&right, rung.right),
                ) {
                    p.line_segment(
                        [screen(a), screen(b)],
                        Stroke::new(
                            if self.selected_rung == Some(i) {
                                2.5_f32
                            } else {
                                1_f32
                            },
                            artwork_ink.success,
                        ),
                    );
                    ui_theme::canvas_label(
                        &p,
                        screen(a.lerp(b, 0.5)),
                        egui::Align2::CENTER_CENTER,
                        (i + 1).to_string(),
                        egui::FontId::proportional(11.),
                        artwork_ink.success,
                        colors.paper,
                    );
                }
            }
            if let (Some(a), Some(b)) = (
                editor_tools::rail_sample(&left, if column.closed { column.seam } else { 0. }),
                editor_tools::rail_sample(
                    &right,
                    if column.closed {
                        embroidery_lab::satin_edit::right_fraction(column, column.seam)
                            .unwrap_or(column.seam)
                    } else {
                        0.
                    },
                ),
            ) {
                p.circle_stroke(
                    screen(if column.first_right { b } else { a }),
                    7.,
                    Stroke::new(2_f32, artwork_ink.success),
                );
                ui_theme::canvas_label(
                    &p,
                    screen(a.lerp(b, 0.5)) + Vec2::new(4., -8.),
                    egui::Align2::LEFT_BOTTOM,
                    language.text("Start"),
                    egui::FontId::proportional(12.),
                    artwork_ink.success,
                    colors.paper,
                );
            }
        }
        if let Some(column) = &self.outline_preview {
            for ring in column.boundary_rings().unwrap_or_default() {
                for pair in ring.windows(2) {
                    p.line_segment(
                        [screen(pair[0]), screen(pair[1])],
                        Stroke::new(2_f32, artwork_ink.success),
                    );
                }
            }
        }
        if let Some(object) = &self.smoothing_preview {
            for path in &object.running_paths {
                for pair in path.windows(2) {
                    p.line_segment(
                        [screen(pair[0]), screen(pair[1])],
                        Stroke::new(2_f32, artwork_ink.success),
                    );
                }
            }
        }
        if response.drag_started_by(egui::PointerButton::Primary)
            && self.tool == Tool::Nodes
            && !panning
            && !self.selected_locked()
            && let Some(pos) = response.interact_pointer_pos()
        {
            self.handle = handles
                .iter()
                .filter(|(p, _)| p.distance(pos) < 12.)
                .min_by(|a, b| a.0.distance(pos).total_cmp(&b.0.distance(pos)))
                .map(|(_, h)| *h);
            if let Some(h) = self.handle {
                self.finish_field_edit();
                self.drag_before = Some(self.project.clone());
                if let Handle::Node(r, n) = h {
                    self.selected_node = Some((r, n));
                    self.selected_rail = None;
                    self.selected_path = None;
                } else if let Handle::Rail(right, n) = h {
                    self.selected_rail = Some((right, n));
                    self.selected_node = None;
                    self.selected_path = None;
                } else if let Handle::Path(r, n) = h {
                    self.selected_path = Some((r, n));
                    self.selected_node = None;
                    self.selected_rail = None;
                }
            }
        }
        if response.dragged_by(egui::PointerButton::Primary)
            && !panning
            && !self.selected_locked()
            && let (Some(h), Some(pos)) = (self.handle, response.interact_pointer_pos())
        {
            let q = world(pos);
            if let Some(o) = self.project.objects.get_mut(self.selected) {
                let mut accepted = true;
                match h {
                    Handle::Node(r, n) => o.contours[r][n] = q,
                    Handle::Path(r, n) => {
                        if let Err(e) = set_path_node(o, r, n, q) {
                            self.message = e;
                            self.error = true;
                            accepted = false;
                        }
                    }
                    Handle::Curve(b, n) => {
                        let len = o.bezier_segments.len();
                        match n {
                            0 => {
                                o.bezier_segments[b].start = q;
                                o.bezier_segments[(b + len - 1) % len].end = q;
                            }
                            1 => o.bezier_segments[b].control1 = q,
                            2 => o.bezier_segments[b].control2 = q,
                            _ => {
                                o.bezier_segments[b].end = q;
                                o.bezier_segments[(b + 1) % len].start = q;
                            }
                        }
                    }
                    Handle::Rail(right, n) => {
                        if let Some(column) = &mut o.satin_column
                            && let Err(e) =
                                embroidery_lab::satin_edit::set_node(column, right, n, q)
                        {
                            self.message = e;
                            self.error = true;
                            accepted = false;
                        }
                    }
                    Handle::RailControl(right, n, outgoing) => {
                        if let Some(column) = &mut o.satin_column {
                            let controls = if right {
                                &mut column.right_handles
                            } else {
                                &mut column.left_handles
                            };
                            let rail = if right { &column.right } else { &column.left };
                            if let (Some(h), Some(node)) = (controls.get_mut(n), rail.get(n)) {
                                if outgoing {
                                    h.outgoing = Some(q);
                                    if h.smooth {
                                        h.incoming =
                                            Some(Point::new(node.x * 2. - q.x, node.y * 2. - q.y));
                                    }
                                } else {
                                    h.incoming = Some(q);
                                    if h.smooth {
                                        h.outgoing =
                                            Some(Point::new(node.x * 2. - q.x, node.y * 2. - q.y));
                                    }
                                }
                            }
                        }
                    }
                }
                if accepted {
                    if matches!(h, Handle::Rail(..) | Handle::RailControl(..)) {
                        self.project.version = 4;
                    }
                    if let Some(before) = self.drag_before.take() {
                        self.push_history(before);
                    }
                    self.changed();
                }
            }
        }
        if response.drag_stopped() {
            self.handle = None;
            self.drag_before = None;
        }
        if response.clicked()
            && !panning
            && !self.selected_locked()
            && let Some(pos) = response.interact_pointer_pos()
        {
            if matches!(
                self.tool,
                Tool::Polygon | Tool::Satin | Tool::Outline | Tool::SplitRegion
            ) {
                self.draft.push(world(pos));
            } else if self.tool == Tool::Rung {
                let nearest = self
                    .project
                    .objects
                    .get(self.selected)
                    .and_then(|o| o.satin_column.as_ref())
                    .and_then(|c| {
                        [
                            (false, c.flatten_rail(false, 0.02).unwrap_or_default()),
                            (true, c.flatten_rail(true, 0.02).unwrap_or_default()),
                        ]
                        .iter()
                        .filter_map(|(right, path)| {
                            editor_tools::rail_fraction(path, world(pos))
                                .map(|(t, d)| (*right, t, d))
                        })
                        .filter(|(_, _, d)| *d * (self.scale as f64) < 12.)
                        .min_by(|a, b| a.2.total_cmp(&b.2))
                    });
                if let Some((right, t, _)) = nearest {
                    if let Some((first_right, first_t)) = self.rung_start {
                        if right != first_right {
                            let (a, b) = if first_right {
                                (t, first_t)
                            } else {
                                (first_t, t)
                            };
                            self.edit_column(|c| {
                                embroidery_lab::satin_edit::add_rung(c, a, b).map(|_| ())
                            });
                            self.rung_start = None;
                        }
                    } else {
                        self.rung_start = Some((right, t));
                    }
                }
            } else {
                self.outline_preview = None;
                self.smoothing_preview = None;
                self.preview_branch = None;
                self.selected_rung = None;
                self.selected_path = handles.iter().find_map(|(p, h)| {
                    if p.distance(pos) < 12.
                        && let Handle::Path(r, n) = h
                    {
                        Some((*r, *n))
                    } else {
                        None
                    }
                });
                self.selected_rail = handles.iter().find_map(|(p, h)| {
                    if p.distance(pos) < 12.
                        && let Handle::Rail(right, n) = h
                    {
                        Some((*right, *n))
                    } else {
                        None
                    }
                });
                self.selected_node = handles.iter().find_map(|(p, h)| {
                    if p.distance(pos) < 12. {
                        if let Handle::Node(r, n) = h {
                            Some((*r, *n))
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                });
                self.selected_branch = self.project.objects.get(self.selected).and_then(|o| {
                    o.running_paths
                        .iter()
                        .enumerate()
                        .filter_map(|(i, path)| {
                            editor_tools::nearest_segment(path, world(pos)).map(|(_, _, d)| (i, d))
                        })
                        .filter(|(_, d)| *d * (self.scale as f64) < 10.)
                        .min_by(|a, b| a.1.total_cmp(&b.1))
                        .map(|(i, _)| i)
                });
                if response.double_clicked()
                    && let Some(column) = self
                        .project
                        .objects
                        .get(self.selected)
                        .and_then(|o| o.satin_column.as_ref())
                {
                    let nearest = editor_tools::nearest_rail(column, world(pos))
                        .filter(|(_, _, _, d)| *d * (self.scale as f64) < 10.);
                    if let Some((right, n, t, _)) = nearest {
                        self.edit_column(|c| {
                            embroidery_lab::satin_edit::insert_node(
                                c,
                                right,
                                n,
                                t.clamp(0.01, 0.99),
                            )
                            .map(|_| ())
                        });
                    }
                }
            }
        }
        if !selecting_areas {
            self.object_context_menu(&response);
        }
        for win in self.draft.windows(2) {
            p.line_segment(
                [screen(win[0]), screen(win[1])],
                Stroke::new(2_f32, colors.accent),
            );
        }
        for win in self.satin_first.windows(2) {
            p.line_segment(
                [screen(win[0]), screen(win[1])],
                Stroke::new(2_f32, colors.accent),
            );
        }
        for q in &self.draft {
            p.circle_filled(screen(*q), 3., colors.accent);
        }
        if self.workspace.hints {
            p.text(
                rect.left_bottom() + Vec2::new(14., -14.),
                egui::Align2::LEFT_BOTTOM,
                language.message(&format!(
                    "10 mm grid  |  {:.1} px/mm  |  Wheel: zoom · Middle drag: pan",
                    self.scale
                )),
                egui::FontId::proportional(12.),
                colors.muted,
            );
            let selection_hint = p.layout_no_wrap(
            language
                .text("Right-click: menu · Right drag: select areas\nCtrl/Shift: add to selection")
                .to_owned(),
            egui::FontId::proportional(12.),
            colors.muted,
        );
            let hint_position = rect.left_bottom() + Vec2::new(14., -50. - selection_hint.size().y);
            p.rect_filled(
                Rect::from_min_size(hint_position, selection_hint.size()).expand(4.),
                0.,
                colors.canvas,
            );
            p.galley(hint_position, selection_hint, colors.muted);
        }
        if self.workspace.rulers {
            self.paint_rulers(&p, rect, center, scale);
        }
        if self.project.objects.is_empty() && self.stitches.is_empty() {
            self.empty_canvas(ui, rect);
        }
        if response.hovered()
            && let Some(pos) = response.hover_pos()
        {
            let point = world(pos);
            ui_theme::canvas_label(
                &p,
                rect.right_bottom() + Vec2::new(-12., -12.),
                egui::Align2::RIGHT_BOTTOM,
                format!("X {:.2} · Y {:.2} mm", point.x, point.y),
                egui::FontId::proportional(11.),
                colors.muted,
                colors.surface,
            );
        }
        if self
            .canvas_scene
            .as_ref()
            .is_some_and(|scene| scene.issue_count > 0)
        {
            p.text(
                rect.left_bottom() + Vec2::new(12., -12.),
                egui::Align2::LEFT_BOTTOM,
                language.text("Some vector fills are unavailable; inspect retained outlines."),
                egui::FontId::proportional(11.),
                colors.warning,
            );
        }
        self.paint_marquee(&p);
    }
}

impl eframe::App for Editor {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.image.is_none() && self.project.source_image.is_some() {
            self.restore_source(ctx);
        }
        self.guard_close(ctx);
        self.shortcuts(ctx);
        self.dropped_artwork(ctx);
        let dt = self.clock.elapsed().as_secs_f64().min(0.2);
        self.clock = Instant::now();
        if self.playing && !self.stale {
            self.fractional += dt * self.speed;
            let n = self.fractional.floor() as usize;
            self.fractional -= n as f64;
            self.progress = (self.progress + n).min(self.stitches.len());
            if self.progress == self.stitches.len() {
                self.playing = false;
            }
            ctx.request_repaint();
        }
        self.workspace_chrome(ctx);
        if self.show_3d && self.tool != Tool::Nodes {
            self.show_3d = false;
        }
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(self.workspace.theme.tokens().canvas))
            .show(ctx, |ui| {
                if self.decision_pending() {
                    ui.disable();
                }
                self.canvas_view_bar(ui);
                if self.show_3d {
                    self.three_d_canvas(ui);
                } else {
                    self.canvas(ui);
                }
            });
        self.workspace_overlays(ctx);
        self.document_confirmations(ctx);
        self.import_window(ctx);
        self.digitize_progress(ctx);
        self.pdf_progress(ctx);
        self.centerline_progress(ctx);
        if !self.decision_pending()
            && self.field_before.is_some()
            && ctx.input(|i| i.pointer.any_released() || i.key_pressed(egui::Key::Enter))
        {
            self.finish_field_edit();
        }
        self.recovery_and_autosave(ctx);
        self.persist_workspace(ctx);
        self.capture_frame(ctx);
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        if !self.capture_requested
            && self.capture_path.is_none()
            && serde_json::to_string(&self.workspace).unwrap_or_default() != self.workspace_saved
            && let Err(error) = self.workspace.save()
        {
            eprintln!("Workspace preferences not saved: {error}");
        }
        self.preview_3d.destroy();
    }
}

fn stitch_kind_label(language: Language, kind: StitchKind) -> &'static str {
    language.text(match kind {
        StitchKind::Running => "Running",
        StitchKind::Satin => "Satin",
        StitchKind::Tatami => "Tatami",
    })
}

fn main() -> eframe::Result {
    let capture_size = if cfg!(feature = "capture") {
        std::env::var("EMBROIDERYLAB_CAPTURE_SIZE")
            .ok()
            .and_then(|value| {
                let (w, h) = value.split_once('x')?;
                let (w, h) = (w.parse::<u32>().ok()?, h.parse::<u32>().ok()?);
                ((850..=3840).contains(&w) && (480..=2160).contains(&h))
                    .then_some([w as f32, h as f32])
            })
    } else {
        None
    };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size(capture_size.unwrap_or([1360., 900.]))
            .with_min_inner_size([850., 480.]),
        ..Default::default()
    };
    eframe::run_native(
        "EmbroideryLab",
        options,
        Box::new(|cc| Ok(Box::new(Editor::new(cc)))),
    )
}

#[cfg(test)]
mod state_tests {
    use super::*;

    #[test]
    fn centerline_copy_is_undoable_and_has_current_exportable_stitches() {
        let mut app = Editor::workspace();
        let before = serde_json::to_value(&app.project).unwrap();
        let result = centerline_workflow::running_copy(&app.project, 0, 0.25);
        app.apply_centerlines(result);
        app.wait_for_digitize();
        assert!(!app.error && !app.stale && app.dirty, "{}", app.message);
        assert_eq!(app.selected, 1);
        assert!(!app.project.objects[0].visible);
        assert!(!app.project.objects[1].running_paths.is_empty());
        assert_eq!(
            serde_json::to_value(app.undo.last().unwrap()).unwrap(),
            before
        );
        assert!(dst::encode(&app.stitches, "Centerline GUI").is_ok());
    }

    #[test]
    fn failed_and_cancelled_centerline_jobs_cannot_replace_document() {
        let mut app = Editor::workspace();
        let before = serde_json::to_value(&app.project).unwrap();
        let undo = app.undo.len();
        app.apply_centerlines(Err("Too coarse; choose a finer pixel size".into()));
        assert!(app.error);
        assert_eq!(serde_json::to_value(&app.project).unwrap(), before);
        assert_eq!(app.undo.len(), undo);
        let result = centerline_workflow::running_copy(&app.project, 0, 0.25);
        let (sender, receiver) = std::sync::mpsc::channel();
        sender.send(result).unwrap();
        app.centerline_job = Some(CenterlineJob {
            receiver,
            cancelled: true,
        });
        app.centerline_progress(&egui::Context::default());
        assert!(app.centerline_job.is_none());
        assert_eq!(serde_json::to_value(&app.project).unwrap(), before);
        assert_eq!(app.undo.len(), undo);
    }

    #[test]
    fn machine_only_view_cannot_start_centerline_worker() {
        let mut app = Editor::workspace();
        app.stitch_only = true;
        app.create_centerlines();
        assert!(app.centerline_job.is_none());
    }

    #[test]
    fn editing_closed_path_endpoint_keeps_explicit_cycle_closed() {
        let a = Point::new(1., 1.);
        let b = Point::new(3., 1.);
        let c = Point::new(2., 3.);
        let mut object = Object {
            running_paths: vec![vec![a, b, c, a], vec![a, b]],
            ..Object::default()
        };
        let moved = Point::new(0., 0.);
        set_path_node(&mut object, 0, 0, moved).unwrap();
        assert_eq!(object.running_paths[0], vec![moved, b, c, moved]);
        set_path_node(&mut object, 1, 0, moved).unwrap();
        assert_eq!(object.running_paths[1], vec![moved, b]);
    }

    #[test]
    fn picking_background_preserves_connectivity_and_all_selected_colors() {
        use import::BackgroundMode as B;
        let mut app = Editor::workspace();
        app.pick_background_color([230, 170, 40], false);
        assert!(matches!(
            app.raster_options.background,
            B::ConnectedColors { .. }
        ));
        app.pick_background_color([255; 3], true);
        app.pick_background_color([255; 3], true);
        app.set_background_connectivity(false);
        assert_eq!(
            app.raster_options.background,
            B::Colors {
                rgb: vec![[230, 170, 40], [255; 3]],
                tolerance: 35.,
            }
        );
        app.pick_background_color([245, 245, 245], false);
        assert_eq!(
            app.raster_options.background,
            B::Colors {
                rgb: vec![[245, 245, 245]],
                tolerance: 35.,
            }
        );
        app.set_background_connectivity(true);
        assert_eq!(
            app.raster_options.background,
            B::ConnectedColors {
                rgb: vec![[245, 245, 245]],
                tolerance: 35.,
            }
        );
        app.raster_options.background = B::Border { tolerance: 12.5 };
        app.set_background_connectivity(true);
        assert_eq!(
            app.raster_options.background,
            B::ConnectedBorder { tolerance: 12.5 }
        );
    }

    fn raster_fixture() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../tests/fixtures/synthetic_holes.png")
    }

    #[test]
    fn dsb_preview_preserves_trim_stop_and_needle_for_reexport() {
        use dsb::{DsbCommand as C, DsbRecord as R};
        let records = vec![
            R {
                x: 1.,
                y: 2.,
                command: C::Jump,
            },
            R {
                x: 2.,
                y: 2.,
                command: C::Stitch,
            },
            R {
                x: 2.,
                y: 2.,
                command: C::Trim,
            },
            R {
                x: 2.,
                y: 2.,
                command: C::NeedleChange(3),
            },
            R {
                x: 2.,
                y: 2.,
                command: C::Stop,
            },
            R {
                x: 2.,
                y: 2.,
                command: C::End,
            },
        ];
        let path = std::env::temp_dir().join(format!("embroidery-gui-{}.dsb", std::process::id()));
        std::fs::write(&path, dsb::encode(&records, "Controls").unwrap()).unwrap();
        let mut app = Editor::workspace();
        app.open(path, &egui::Context::default());
        assert!(app.stitch_only);
        app.generate();
        app.wait_for_digitize();
        app.save();
        app.export(); // These paths must not show a save dialog or discard raw commands.
        assert_eq!(app.dsb_records.as_ref().unwrap(), &records);
        assert_eq!(
            dsb::decode(&app.dsb_export_bytes().unwrap()).unwrap(),
            records
        );
    }

    #[test]
    fn cancelling_import_preserves_existing_geometry_and_document_width() {
        let mut app = Editor::workspace();
        let before = serde_json::to_value(&app.project).unwrap();
        app.request_open(raster_fixture(), &egui::Context::default());
        assert!(app.import_preview.is_some());
        app.import_width_mm = 85.;
        app.raster_options.palette_size = 2;
        app.cancel_import();
        assert_eq!(before, serde_json::to_value(&app.project).unwrap());
        assert!(app.import_preview.is_none());
    }

    #[test]
    fn failed_import_generation_preserves_document_and_last_valid_stitches() {
        let path = std::env::temp_dir().join(format!("embroidery-tall-{}.png", std::process::id()));
        let raster = image::RgbImage::from_pixel(16, 160, image::Rgb([35, 85, 190]));
        raster.save(&path).unwrap();
        let mut app = Editor::workspace();
        let context = egui::Context::default();
        let before = serde_json::to_value(&app.project).unwrap();
        let stitches = serde_json::to_value(&app.stitches).unwrap();
        app.request_open(path, &context);
        app.import_width_mm = 400.;
        app.auto_classify = false;
        app.commit_import(&context);
        app.wait_for_digitize();
        assert!(app.error && app.import_preview.is_some(), "{}", app.message);
        assert_eq!(serde_json::to_value(&app.project).unwrap(), before);
        assert_eq!(serde_json::to_value(&app.stitches).unwrap(), stitches);
        assert!(!app.stale);
        app.import_width_mm = 20.;
        app.commit_import(&context);
        app.wait_for_digitize();
        assert!(
            app.import_preview.is_none() && !app.stale,
            "{}",
            app.message
        );
        assert_eq!(app.project.width_mm, 20.);
        app.undo_edit();
        assert!(
            app.image.is_none(),
            "Undo must discard the imported source texture"
        );
        assert_eq!(serde_json::to_value(&app.project).unwrap(), before);
    }

    #[test]
    fn failed_tracing_keeps_existing_document_and_import_choices() {
        let mut app = Editor::workspace();
        let before = serde_json::to_value(&app.project).unwrap();
        let context = egui::Context::default();
        app.request_open(raster_fixture(), &context);
        app.raster_options.palette_size = 0;
        app.commit_import(&context);
        app.wait_for_digitize();
        assert!(app.error && app.import_preview.is_some());
        assert_eq!(before, serde_json::to_value(&app.project).unwrap());
    }

    #[test]
    fn new_satin_column_is_editable_and_generates_independent_stitches() {
        let mut app = Editor::workspace();
        let original_objects = app.project.objects.len();
        app.add_satin_column();
        assert!(app.project.objects[app.selected].satin_column.is_some());
        app.project.objects[app.selected]
            .satin_column
            .as_mut()
            .unwrap()
            .right[1]
            .x += 0.5;
        app.generate();
        app.wait_for_digitize();
        assert!(!app.stale, "{}", app.message);
        assert!(
            app.stitches
                .iter()
                .filter(|s| s.command == Command::Stitch)
                .count()
                > 100
        );
        app.project = app.undo.pop().unwrap();
        assert_eq!(app.project.objects.len(), original_objects);
    }

    #[test]
    fn reopening_dst_cannot_erase_sequence_via_vector_actions() {
        let mut app = Editor::workspace();
        let path = std::env::temp_dir().join(format!("embroidery-gui-{}.dst", std::process::id()));
        std::fs::write(&path, dst::encode(&app.stitches, "GUI test").unwrap()).unwrap();
        app.open(path, &egui::Context::default());
        let before: Vec<_> = app
            .stitches
            .iter()
            .map(|s| (s.point(), s.command))
            .collect();
        app.generate();
        app.wait_for_digitize();
        app.save(); // Must reject before any OS file dialog is opened.
        assert!(app.stitch_only && app.error);
        let after: Vec<_> = app
            .stitches
            .iter()
            .map(|s| (s.point(), s.command))
            .collect();
        assert_eq!(before, after);
        assert!(!app.stale);
    }

    #[test]
    fn native_close_event_retains_dirty_document() {
        let mut app = Editor::workspace();
        app.project.objects[0].contours[0][0].x = 12.;
        app.changed();
        let context = egui::Context::default();
        let mut input = egui::RawInput::default();
        input
            .viewports
            .get_mut(&egui::ViewportId::ROOT)
            .unwrap()
            .events
            .push(egui::ViewportEvent::Close);
        let output = context.run(input, |ctx| app.guard_close(ctx));
        assert!(app.dirty && app.confirm_close);
        assert_eq!(app.project.objects[0].contours[0][0].x, 12.);
        assert!(
            output.viewport_output[&egui::ViewportId::ROOT]
                .commands
                .iter()
                .any(|c| matches!(c, egui::ViewportCommand::CancelClose))
        );
    }

    #[test]
    fn failed_generation_cannot_export_old_stitches_as_current() {
        let mut app = Editor::workspace();
        app.project.objects[0].settings.spacing_mm = 0.;
        app.generate();
        app.wait_for_digitize();
        assert!(app.stale && app.error);
        app.export(); // Error path must return before opening a save dialog.
        assert!(app.stale && app.error);
    }
}
