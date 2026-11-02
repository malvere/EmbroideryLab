#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod artwork_import;
mod context_hit;
mod context_menu;
mod editor_tools;
mod i18n;
mod layer_selection;
#[cfg(test)]
#[path = "layer_selection_ui_tests.rs"]
mod layer_selection_ui_tests;
mod marquee;
mod marquee_hit;
mod stitch_3d;
mod stitch_metrics;
mod stitch_view;

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
    pixels: image::RgbaImage,
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
    capture_requested: bool,
    render_passes: u8,
    raster_options: import::RasterOptions,
    import_width_mm: f64,
    import_preview: Option<ImportPreview>,
    centerline_pixel_mm: f64,
    centerline_job: Option<CenterlineJob>,
    pdf_job: Option<artwork_import::PdfJob>,
}

impl Editor {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let mut style = (*cc.egui_ctx.style()).clone();
        style.visuals = egui::Visuals::light();
        style.visuals.panel_fill = Color32::from_rgb(237, 240, 244);
        style.visuals.selection.bg_fill = Color32::from_rgb(34, 104, 162);
        style.spacing.item_spacing = Vec2::new(8., 8.);
        style.spacing.interact_size.y = 28.;
        cc.egui_ctx.set_style(style);
        let language = std::env::var("EMBROIDERYLAB_LANGUAGE")
            .ok()
            .and_then(|code| Language::from_code(&code))
            .unwrap_or_else(Language::load);
        let mut app = Self::workspace_with_language(language);
        app.preview_3d = stitch_3d::Preview3d::new(cc.gl.clone());
        app.preview_3d
            .set_stitches(&app.stitches, app.stitch_revision);
        if let Some(path) = std::env::args_os().nth(1) {
            app.request_open(PathBuf::from(path), &cc.egui_ctx);
        } else if app.capture_path.is_none() {
            match embroidery_lab::persistence::load_recovery(None) {
                Ok(copy) => app.recovery = copy,
                Err(e) => app.result(Err(e)),
            }
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
            render_passes: 0,
            raster_options: import::RasterOptions {
                max_dimension: 512,
                palette_size: 8,
                ..Default::default()
            },
            import_width_mm: 100.,
            import_preview: None,
            centerline_pixel_mm: 0.2,
            centerline_job: None,
            pdf_job: None,
        };
        app.generate();
        app
    }
    fn guard_close(&mut self, ctx: &egui::Context) {
        if ctx.input(|i| i.viewport().close_requested()) {
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
        let Some(path) = self.capture_path.clone() else {
            return;
        };
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
    fn generate(&mut self) {
        if self.stitch_only {
            return;
        }
        self.stale = true;
        self.playing = false;
        match engine::generate(&self.project) {
            Ok(stitches) => {
                self.quality_issues =
                    embroidery_lab::quality::analyze(&stitches, Default::default());
                self.stitches = stitches;
                self.update_stitch_view();
                self.progress = self.stitches.len();
                self.stale = false;
                self.result(Ok(format!(
                    "Generated {} commands. Preview ready.",
                    self.stitches.len()
                )));
            }
            Err(e) => self.result(Err(e)),
        }
    }
    fn open_dialog(&mut self, ctx: &egui::Context) {
        let language = self.language;
        if let Some(path) = rfd::FileDialog::new()
            .add_filter(
                language.text("Artwork / project / DST / DSB"),
                &[
                    "jpg", "jpeg", "png", "pdf", "svg", "elab", "json", "dst", "dsb",
                ],
            )
            .add_filter(
                language.text("Images (JPEG, PNG, PDF, SVG)"),
                &["jpg", "jpeg", "png", "pdf", "svg"],
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
        if !["png", "jpg", "jpeg"].contains(&ext.as_str()) {
            self.open(path, ctx);
            return;
        }
        let preview = image::ImageReader::open(&path)
            .map_err(|e| e.to_string())
            .and_then(|mut reader| {
                let mut limits = image::Limits::default();
                limits.max_image_width = Some(16_384);
                limits.max_image_height = Some(16_384);
                limits.max_alloc = Some(128 * 1024 * 1024);
                reader.limits(limits);
                reader.decode().map_err(|e| e.to_string())
            });
        match preview {
            Ok(image) => {
                self.region_preview = None;
                self.mask_texture = None;
                let pixels = image.thumbnail(480, 480).to_rgba8();
                let data = egui::ColorImage::from_rgba_unmultiplied(
                    [pixels.width() as usize, pixels.height() as usize],
                    pixels.as_raw(),
                );
                self.import_preview = Some(ImportPreview {
                    path,
                    pixels,
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
                    self.project.name = path
                        .file_stem()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into();
                    self.stitches = s;
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
        } else if ["png", "jpg", "jpeg"].contains(&ext.as_str()) {
            import::load_image_with_options(&path, self.import_width_mm, &self.raster_options)
        } else {
            import::load(&path, self.import_width_mm)
        };
        let loaded = loaded.and_then(|p| {
            if ["png", "jpg", "jpeg", "pdf"].contains(&ext.as_str())
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
                if ["png", "jpg", "jpeg", "pdf"].contains(&ext.as_str()) {
                    for object in &mut p.objects {
                        object.name = self.language.message(&object.name).into_owned();
                    }
                }
                self.project = p;
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
            self.generate();
        }
        if self.stale {
            return;
        }
        if let Some(path) = rfd::FileDialog::new()
            .add_filter(language.text("Tajima DST"), &["dst"])
            .set_file_name(format!("{}.dst", self.project.name))
            .save_file()
        {
            let result = dst::encode(&self.stitches, &self.project.name).and_then(|data| {
                dst::decode(&data)?;
                std::fs::write(&path, data).map_err(|e| e.to_string())?;
                Ok(format!("Exported and decoded {}", path.display()))
            });
            self.result(result);
        }
    }
    fn dsb_export_bytes(&mut self) -> Result<Vec<u8>, String> {
        if self.stale {
            self.generate();
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
            self.result(
                std::fs::write(&path, bytes)
                    .map_err(|e| e.to_string())
                    .map(|_| format!("Exported experimental DSB: {}", path.display())),
            );
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
        let language = self.language;
        if self.import_preview.is_none() {
            return;
        }
        let mut action = 0_u8;
        let mut pdf_page = None;
        let header_height = if self
            .import_preview
            .as_ref()
            .is_some_and(|preview| preview.pdf.is_some())
        {
            300.
        } else {
            220.
        };
        egui::Modal::new(egui::Id::new("trace-artwork")).show(ctx, |ui| {
                ui.set_width(760.);
                ui.heading(language.text("Trace artwork"));
                if let Some(preview) = &self.import_preview {
                    ui.label(preview.path.file_name().unwrap_or_default().to_string_lossy());
                    if let Some(pdf) = &preview.pdf {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(language.text("PDF page"));
                            let mut number = pdf.page_index + 1;
                            if ui.add(egui::DragValue::new(&mut number).range(1..=pdf.page_count).speed(1)).changed() {pdf_page=Some(number-1);}
                            ui.label(format!("/ {}", pdf.page_count));
                            if ui.add_enabled(pdf.page_index > 0, egui::Button::new(language.text("Previous page"))).clicked() {pdf_page=Some(pdf.page_index-1);}
                            if ui.add_enabled(pdf.page_index+1 < pdf.page_count, egui::Button::new(language.text("Next page"))).clicked() {pdf_page=Some(pdf.page_index+1);}
                        });
                        ui.small(language.text("Import one page as an image. Choose its background and tracing settings below."));
                    }
                }
                ui.label(language.text("Choose what becomes thread. Click to exclude a background color; Shift-click adds another."));
                egui::ScrollArea::vertical().max_height((ctx.content_rect().height()-header_height).max(160.)).show(ui,|ui| {
                ui.horizontal_top(|ui| {
                    let preview = self.import_preview.as_ref().unwrap();
                    let texture=self.mask_texture.as_ref().unwrap_or(&preview.texture);
                    let image_size = texture.size_vec2();
                    let size = image_size * (430. / image_size.x).min(430. / image_size.y).min(1.);
                    let response = ui.add(egui::Image::new((texture.id(), size)).sense(Sense::click()));
                    if response.clicked() && let Some(pos) = response.interact_pointer_pos() {
                        let uv = (pos - response.rect.min) / response.rect.size();
                        let x = ((uv.x * preview.pixels.width() as f32) as u32).min(preview.pixels.width()-1);
                        let y = ((uv.y * preview.pixels.height() as f32) as u32).min(preview.pixels.height()-1);
                        if let Some(mask)=&mut self.region_preview {
                            mask.toggle_at(Point::new(uv.x as f64*mask.project.width_mm,uv.y as f64*mask.project.width_mm*mask.height as f64/mask.width as f64));
                            self.refresh_mask_texture(ctx);
                        }else{
                            let pixel = preview.pixels.get_pixel(x,y);
                            let rgb = [pixel[0],pixel[1],pixel[2]];
                            self.pick_background_color(rgb, ui.input(|i|i.modifiers.shift));
                        }
                    }
                    ui.vertical(|ui| {
                        ui.set_width(300.);
                        ui.label(language.text("Width (mm)"));
                        ui.add(egui::DragValue::new(&mut self.import_width_mm).range(5. ..=400.).speed(1.));
                        ui.label(language.text("Trace resolution (longest side)"));
                        egui::ComboBox::from_id_salt("trace-resolution").selected_text(format!("{} {}",self.raster_options.max_dimension,language.text("px"))).show_ui(ui,|ui| {
                            for n in [256,512,768,1024] { ui.selectable_value(&mut self.raster_options.max_dimension,n,format!("{n} {}",language.text("px"))); }
                        });
                        ui.add(egui::Slider::new(&mut self.raster_options.palette_size,2..=16).text(language.text("colors")));
                        ui.add(egui::Slider::new(&mut self.raster_options.min_region_pixels,1..=40).text(language.text("min. region px")));
                        ui.label(language.text("Contour tolerance (mm)"));
                        ui.add(egui::DragValue::new(&mut self.raster_options.simplify_tolerance_mm).speed(0.01).range(0. ..=1.).fixed_decimals(2));
                        egui::CollapsingHeader::new(language.text("Contour smoothing")).id_salt("contour-smoothing").show(ui,|ui| {
                            ui.add(egui::Slider::new(&mut self.raster_options.smoothing_iterations,0..=3).text(language.text("passes")));
                            ui.small(language.text("Contour intersections and changed hole nesting are rejected. Zero keeps the traced boundary."));
                        });
                        ui.separator();
                        let mut mode = match &self.raster_options.background {
                            import::BackgroundMode::NearWhite => 0,
                            import::BackgroundMode::TransparentOnly => 1,
                            import::BackgroundMode::Border { .. } | import::BackgroundMode::ConnectedBorder { .. } => 2,
                            import::BackgroundMode::Color { .. } | import::BackgroundMode::Colors { .. } | import::BackgroundMode::ConnectedColors { .. } => 3,
                        };
                        let old_mode = mode;
                        egui::ComboBox::from_label(language.text("Background")).selected_text([language.text("Near white"),language.text("Keep opaque colors"),language.text("Estimate from border"),language.text("Picked colors")][mode]).show_ui(ui,|ui| {
                            for (i,label) in [language.text("Near white"),language.text("Keep opaque colors"),language.text("Estimate from border"),language.text("Picked colors")].iter().enumerate() {ui.selectable_value(&mut mode,i,*label);}
                        });
                        if mode != old_mode {
                            self.raster_options.background = match mode {
                                0 => import::BackgroundMode::NearWhite,
                                1 => import::BackgroundMode::TransparentOnly,
                                2 => import::BackgroundMode::ConnectedBorder { tolerance: 35. },
                                _ => import::BackgroundMode::ConnectedColors { rgb:vec![[255;3]], tolerance:35. },
                            };
                        }
                        match &mut self.raster_options.background {
                            import::BackgroundMode::Color { rgb, tolerance } => {
                                ui.color_edit_button_srgb(rgb);
                                ui.add(egui::Slider::new(tolerance,0. ..=150.).text(language.text("RGB tolerance")));
                            },
                            import::BackgroundMode::Border { tolerance } | import::BackgroundMode::ConnectedBorder { tolerance } => {ui.add(egui::Slider::new(tolerance,0. ..=150.).text(language.text("RGB tolerance")));},
                            import::BackgroundMode::Colors { rgb, tolerance } | import::BackgroundMode::ConnectedColors { rgb, tolerance } => {
                                let mut remove=None;
                                egui::ScrollArea::vertical().max_height(100.).show(ui,|ui| {
                                    for (i,color) in rgb.iter_mut().enumerate() {ui.horizontal(|ui| {
                                        ui.color_edit_button_srgb(color);
                                        if ui.small_button(language.text("Remove")).clicked() {remove=Some(i);}
                                    });}
                                });
                                if let Some(i)=remove&&rgb.len()>1{rgb.remove(i);}
                                ui.add(egui::Slider::new(tolerance,0. ..=150.).text(language.text("RGB tolerance")));
                            },
                            _ => {},
                        }
                        if !matches!(self.raster_options.background, import::BackgroundMode::NearWhite | import::BackgroundMode::TransparentOnly) {
                            let mut connected = matches!(self.raster_options.background, import::BackgroundMode::ConnectedColors { .. } | import::BackgroundMode::ConnectedBorder { .. });
                            if ui.checkbox(&mut connected,language.text("Only remove areas connected to image edge")).changed() {
                                self.set_background_connectivity(connected);
                            }
                            ui.small(if connected {language.text("Enclosed areas remain thread, including details of the same color. Open gaps can still connect a detail to the edge.")} else {language.text("Matching colors are removed everywhere, including enclosed details.")});
                        }
                        ui.small(language.text("Tracing creates editable contours. Small lettering may need manual correction. No smoothing is applied by default."));
                    });
                });
                });
                if self.error {ui.colored_label(Color32::from_rgb(161,43,50),language.message(&self.message).as_ref());}
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button(language.text("Preview mask")).clicked(){action=3;}
                    if self.region_preview.is_some() && ui.button(language.text("Reset mask")).clicked(){self.region_preview=None;self.mask_texture=None;}
                    if ui.button(language.text("Trace and open")).clicked() {action=1;}
                    if ui.button(language.text("Cancel")).clicked() {action=2;}
                });
            });
        if let Some(page) = pdf_page {
            if let Some(preview) = &self.import_preview {
                self.start_pdf_load(preview.path.clone(), page, ctx);
            }
        } else if action == 1 {
            self.commit_import(ctx);
        } else if action == 2 {
            self.cancel_import();
        } else if action == 3 {
            self.preview_regions(ctx);
        }
    }
    fn cancel_import(&mut self) {
        self.import_preview = None;
        self.region_preview = None;
        self.mask_texture = None;
    }
    fn commit_import(&mut self, ctx: &egui::Context) {
        if let Some(preview) = &self.import_preview {
            let path = preview.path.clone();
            if self.open(path, ctx) {
                self.import_preview = None;
                self.region_preview = None;
                self.mask_texture = None;
            }
        }
    }
    fn shortcuts(&mut self, ctx: &egui::Context) {
        if self.import_preview.is_some()
            || self.pdf_job.as_ref().is_some_and(|job| !job.cancelled)
            || self.recovery.is_some()
            || self.confirm_close
            || self.discard.is_some()
            || self
                .centerline_job
                .as_ref()
                .is_some_and(|job| !job.cancelled)
        {
            return;
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::O)) {
            self.open_dialog(ctx);
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::S)) {
            self.save();
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::E)) {
            self.export();
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::Z)) {
            self.undo_edit();
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::Y))
            || ctx.input_mut(|i| {
                i.consume_key(egui::Modifiers::CTRL | egui::Modifiers::SHIFT, egui::Key::Z)
            })
        {
            self.redo_edit();
        }
        let layer_list_focused = self
            .layer_focus_id
            .is_some_and(|id| ctx.memory(|memory| memory.has_focus(id)));
        if (!ctx.wants_keyboard_input() || layer_list_focused)
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
        if !ctx.wants_keyboard_input() {
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
    fn sidebars(&mut self, ctx: &egui::Context) {
        let language = self.language;
        egui::SidePanel::left("objects")
            .default_width(220.)
            .width_range(180. ..=340.)
            .show(ctx, |ui| {
                ui.heading(language.text("Objects"));
                ui.label(language.text("Sewing order · top to bottom")).on_hover_text(language.text(
                    "Click: one layer · Shift-click: range · Ctrl-click: toggle · Ctrl+A: all layers",
                ));
                if !self.merge_selection.is_empty() {
                    ui.label(
                        language
                            .message(&format!("Selected areas: {}", self.merge_selection.len())),
                    );
                }
                ui.separator();
                let mut changed = false;
                let before_visibility = self.project.clone();
                let mut target = None;
                let mut context_responses = Vec::new();
                egui::ScrollArea::vertical()
                    .max_height((ui.available_height() - 205.).max(80.))
                    .show(ui, |ui| {
                        for (i, o) in self.project.objects.iter_mut().enumerate() {
                            ui.horizontal(|ui| {
                                changed |= ui
                                    .checkbox(&mut o.visible, "")
                                    .on_hover_text(
                                        language.text("Include object in generated stitches"),
                                    )
                                    .changed();
                                let (r, _) =
                                    ui.allocate_exact_size(Vec2::splat(12.), Sense::hover());
                                ui.painter().rect_filled(
                                    r,
                                    2.,
                                    Color32::from_rgb(o.color[0], o.color[1], o.color[2]),
                                );
                                let row = ui.selectable_label(
                                    i == self.selected || self.merge_selection.contains(&o.id),
                                    &o.name,
                                );
                                if row.has_focus() || row.clicked() {
                                    self.layer_focus_id = Some(row.id);
                                }
                                if row.clicked() {
                                    target = Some((i, ui.input(|input| input.modifiers)));
                                }
                                context_responses.push((i, row));
                                if self.merge_selection.contains(&o.id) {
                                    ui.small(language.text("Selected"));
                                }
                            });
                        }
                    });
                if let Some((i, modifiers)) = target {
                    self.select_layer(i, modifiers);
                }
                for (i, response) in context_responses {
                    if response.secondary_clicked() {
                        self.select_object(i);
                    }
                    self.object_context_menu(&response);
                }
                if changed {
                    self.finish_field_edit();
                    self.push_history(before_visibility);
                    self.changed();
                }
                ui.separator();
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(
                            self.selected > 0 && self.selected < self.project.objects.len(),
                            egui::Button::new(language.text("Earlier")),
                        )
                        .clicked()
                    {
                        self.move_selected(false);
                    }
                    if ui
                        .add_enabled(
                            self.selected + 1 < self.project.objects.len(),
                            egui::Button::new(language.text("Later")),
                        )
                        .clicked()
                    {
                        self.move_selected(true);
                    }
                });
                ui.horizontal_wrapped(|ui| {
                    if ui
                        .add_enabled(
                            !self.stitch_only,
                            egui::Button::new(language.text("Rectangle")),
                        )
                        .clicked()
                    {
                        self.add_object(false);
                    }
                    if ui
                        .add_enabled(
                            !self.stitch_only,
                            egui::Button::new(language.text("Bezier")),
                        )
                        .clicked()
                    {
                        self.add_object(true);
                    }
                });
                if ui
                    .add_enabled(
                        !self.stitch_only,
                        egui::Button::new(language.text("Satin column")),
                    )
                    .clicked()
                {
                    self.cancel_draft();
                    self.tool = Tool::Satin;
                }
                if ui
                    .add_enabled(
                        self.project.objects.len() > 1,
                        egui::Button::new(language.text("Optimize same-color order")),
                    )
                    .on_hover_text(language.text(
                        "Reorders nearby objects within color blocks. Inspect overlaps afterward.",
                    ))
                    .clicked()
                {
                    self.snapshot();
                    engine::optimize_order(&mut self.project);
                    self.selected = 0;
                    self.selected_node = None;
                    self.changed();
                }
                if ui
                    .add_enabled(
                        !self.stitch_only && self.selected < self.project.objects.len(),
                        egui::Button::new(language.text("Remove object")),
                    )
                    .clicked()
                {
                    self.remove_selected();
                }
            });
        egui::SidePanel::right("parameters").default_width(265.).width_range(240. ..=380.).show(ctx,|ui|{
            egui::ScrollArea::vertical().show(ui,|ui|{
                ui.heading(language.text("Stitch settings"));
                if !self.stitch_only {
                    egui::CollapsingHeader::new(language.text("Centerlines")).id_salt("centerline-options").show(ui,|ui| {
                        ui.label(language.text("Pixel size (mm)"));ui.add(egui::DragValue::new(&mut self.centerline_pixel_mm).speed(0.01).range(0.05..=2.).fixed_decimals(2));
                        ui.small(language.text("For narrow areas and lettering. Creates Running paths and keeps the original hidden."));
                        let available=self.project.objects.get(self.selected).is_some_and(|o|o.running_paths.is_empty()&&(!o.contours.is_empty()||!o.bezier_segments.is_empty()||o.satin_column.is_some()));
                        if ui.add_enabled(available && self.centerline_job.is_none(),egui::Button::new(language.text("Create running copy"))).clicked(){self.create_centerlines();}
                        if self.centerline_job.as_ref().is_some_and(|job|job.cancelled) {ui.small(language.text("Finishing cancelled extraction…"));}
                        ui.small(language.text("Pixel size controls detail. Review branches before export; Satin width is not inferred."));
                    });
                }
                self.object_parameters(ui);
                self.geometry_tools(ui);
                ui.separator();ui.heading(language.text("Artwork"));ui.label(language.text("Next import width (mm)"));ui.add(egui::DragValue::new(&mut self.import_width_mm).speed(1.).range(5. ..=400.));
                ui.small(language.text("Open a raster to choose its background, palette and tracing resolution. DST preview colors are illustrative."));
            });
        });
    }
    fn canvas(&mut self, ui: &mut egui::Ui) {
        let language = self.language;
        let (response, p) = ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
        let rect = response.rect;
        p.rect_filled(rect, 0., Color32::from_rgb(248, 249, 251));
        if self.fit {
            let points: Vec<Point> = self
                .project
                .objects
                .iter()
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
        if response.dragged_by(egui::PointerButton::Middle) {
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
        let selecting_areas = self.update_marquee(ui, &response, center, scale);
        if response.secondary_clicked()
            && !selecting_areas
            && let Some(pos) = response.interact_pointer_pos()
        {
            self.select_context_at(world(pos));
        }
        let step = 10. * scale;
        if step >= 12. {
            let x0 = rect.left() + ((center.x - rect.left()) % step + step) % step;
            let y0 = rect.top() + ((center.y - rect.top()) % step + step) % step;
            let mut x = x0;
            while x < rect.right() {
                p.line_segment(
                    [Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())],
                    Stroke::new(0.5_f32, Color32::from_gray(222)),
                );
                x += step;
            }
            let mut y = y0;
            while y < rect.bottom() {
                p.line_segment(
                    [Pos2::new(rect.left(), y), Pos2::new(rect.right(), y)],
                    Stroke::new(0.5_f32, Color32::from_gray(222)),
                );
                y += step;
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
        for (i, o) in self.project.objects.iter().enumerate() {
            if !o.visible {
                continue;
            }
            let col = if i == self.selected {
                Color32::from_rgb(34, 104, 162)
            } else {
                Color32::from_gray(135)
            };
            for ring in o.flattened_contours() {
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
                            Stroke::new(0.7_f32, Color32::from_rgb(195, 90, 64)),
                        );
                    }
                    _ => {}
                }
                prev = s.point();
            }
            if self.progress > 0
                && let Some(s) = self.stitches.get(self.progress - 1)
            {
                p.circle_filled(screen(s.point()), 4., Color32::from_rgb(24, 35, 49));
            }
        }
        if self.show_quality && !self.stale {
            for issue in self.quality_issues.iter().take(3000) {
                p.circle_stroke(
                    screen(issue.point),
                    5.,
                    Stroke::new(1.5_f32, Color32::from_rgb(161, 43, 50)),
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
                            Color32::from_rgb(220, 110, 40)
                        } else {
                            Color32::WHITE
                        },
                    );
                    p.circle_stroke(
                        pos,
                        3.0,
                        Stroke::new(1_f32, Color32::from_rgb(34, 104, 162)),
                    );
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
                            Color32::from_rgb(220, 110, 40)
                        } else {
                            Color32::WHITE
                        },
                    );
                    p.rect_stroke(
                        Rect::from_center_size(pos, Vec2::splat(6.)),
                        0.,
                        Stroke::new(1_f32, Color32::from_rgb(34, 104, 162)),
                        egui::StrokeKind::Middle,
                    );
                    handles.push((pos, Handle::Node(r, n)));
                }
            }
            for (b, c) in o.bezier_segments.iter().enumerate() {
                for (a, h) in [(c.start, c.control1), (c.end, c.control2)] {
                    p.line_segment(
                        [screen(a), screen(h)],
                        Stroke::new(0.7_f32, Color32::from_gray(130)),
                    );
                }
                for (n, q) in [c.start, c.control1, c.control2, c.end].iter().enumerate() {
                    p.circle_filled(screen(*q), 3.5, Color32::from_rgb(34, 104, 162));
                    handles.push((screen(*q), Handle::Curve(b, n)));
                }
            }
            if let Some(column) = &o.satin_column {
                for (right, rail) in [(false, &column.left), (true, &column.right)] {
                    let color = if right {
                        Color32::from_rgb(195, 90, 64)
                    } else {
                        Color32::from_rgb(34, 104, 162)
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
                                Color32::from_rgb(220, 110, 40)
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
                            Color32::from_rgb(56, 120, 94),
                        ),
                    );
                    p.text(
                        screen(a.lerp(b, 0.5)),
                        egui::Align2::CENTER_CENTER,
                        (i + 1).to_string(),
                        egui::FontId::proportional(11.),
                        Color32::from_rgb(30, 75, 55),
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
                    Stroke::new(2_f32, Color32::from_rgb(30, 100, 60)),
                );
                p.text(
                    screen(a.lerp(b, 0.5)) + Vec2::new(4., -8.),
                    egui::Align2::LEFT_BOTTOM,
                    language.text("Start"),
                    egui::FontId::proportional(12.),
                    Color32::from_rgb(30, 100, 60),
                );
            }
        }
        if let Some(column) = &self.outline_preview {
            for ring in column.boundary_rings().unwrap_or_default() {
                for pair in ring.windows(2) {
                    p.line_segment(
                        [screen(pair[0]), screen(pair[1])],
                        Stroke::new(2_f32, Color32::from_rgb(56, 120, 94)),
                    );
                }
            }
        }
        if let Some(object) = &self.smoothing_preview {
            for path in &object.running_paths {
                for pair in path.windows(2) {
                    p.line_segment(
                        [screen(pair[0]), screen(pair[1])],
                        Stroke::new(2_f32, Color32::from_rgb(56, 120, 94)),
                    );
                }
            }
        }
        if response.drag_started_by(egui::PointerButton::Primary)
            && self.tool == Tool::Nodes
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
                Stroke::new(2_f32, Color32::from_rgb(34, 104, 162)),
            );
        }
        for win in self.satin_first.windows(2) {
            p.line_segment(
                [screen(win[0]), screen(win[1])],
                Stroke::new(2_f32, Color32::from_rgb(34, 104, 162)),
            );
        }
        for q in &self.draft {
            p.circle_filled(screen(*q), 3., Color32::from_rgb(34, 104, 162));
        }
        p.text(
            rect.left_bottom() + Vec2::new(14., -14.),
            egui::Align2::LEFT_BOTTOM,
            language.message(&format!(
                "10 mm grid  |  {:.1} px/mm  |  Wheel: zoom · Middle drag: pan",
                self.scale
            )),
            egui::FontId::proportional(12.),
            Color32::from_rgb(70, 80, 95),
        );
        let selection_hint = p.layout_no_wrap(
            language
                .text("Right-click: menu · Right drag: select areas\nCtrl/Shift: add to selection")
                .to_owned(),
            egui::FontId::proportional(12.),
            Color32::from_rgb(70, 80, 95),
        );
        let hint_position = rect.left_bottom() + Vec2::new(14., -50. - selection_hint.size().y);
        p.rect_filled(
            Rect::from_min_size(hint_position, selection_hint.size()).expand(4.),
            0.,
            Color32::from_rgb(248, 249, 251),
        );
        p.galley(hint_position, selection_hint, Color32::from_rgb(70, 80, 95));
        self.paint_marquee(&p);
    }
}

impl eframe::App for Editor {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let language = self.language;
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
        egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.heading("EmbroideryLab");
                ui.separator();
                if ui
                    .button(language.text("Open…"))
                    .on_hover_text(language.text("Ctrl+O · JPEG / PNG / PDF / SVG / project / DST / DSB"))
                    .clicked()
                {
                    self.open_dialog(ctx);
                }
                if ui.button(language.text("Import image…"))
                    .on_hover_text(language.text("JPEG, PNG, PDF or SVG. You can also drop a file onto the editor."))
                    .clicked() { self.image_dialog(ctx); }
                if ui
                    .add_enabled(!self.stitch_only, egui::Button::new(language.text("Save project")))
                    .on_hover_text(language.text("Ctrl+S · Editable geometry projects"))
                    .clicked()
                {
                    self.save();
                }
                if ui
                    .add_enabled(
                        self.dsb_records.is_none() && (!self.stitches.is_empty() || !self.project.objects.is_empty()),
                        egui::Button::new(language.text("Export DST…")),
                    )
                    .clicked()
                {
                    self.export();
                }
                if ui.add_enabled(!self.stitches.is_empty() || !self.project.objects.is_empty(),egui::Button::new(language.text("Export DSB…")))
                    .on_hover_text(language.text("Experimental format. Generated color changes become manual stops; map threads on the controller."))
                    .clicked() {self.export_dsb();}
                ui.separator();
                if ui
                    .add_enabled(!self.stitch_only, egui::Button::new(language.text("Generate stitches")))
                    .clicked()
                {
                    self.generate();
                }
                if ui
                    .add_enabled(!self.undo.is_empty()||self.field_before.is_some(), egui::Button::new(language.text("Undo")))
                    .clicked()
                {
                    self.undo_edit();
                }
                if ui.add_enabled(!self.redo.is_empty(),egui::Button::new(language.text("Redo"))).clicked(){self.redo_edit();}
                ui.label(format!(
                    "{}{}",
                    self.project.name,
                    if self.dirty { " *" } else { "" }
                ));
            });
            ui.horizontal_wrapped(|ui| {
                if ui.selectable_label(!self.show_3d, "2D").clicked() { self.set_3d_view(false); }
                if ui.selectable_label(self.show_3d, "3D").on_hover_text(language.text("3D stitch preview")).clicked() { self.set_3d_view(true); }
                ui.separator();
                if !self.show_3d {
                if ui.selectable_label(self.tool==Tool::Nodes,language.text("Edit nodes")).clicked(){self.cancel_draft();}
                ui.add_enabled_ui(!self.stitch_only, |ui| {
                    if ui.selectable_label(self.tool==Tool::Polygon,language.text("Draw contour")).clicked(){self.cancel_draft();self.tool=Tool::Polygon;}
                    if ui.selectable_label(self.tool==Tool::Satin,language.text("Draw satin")).clicked(){self.cancel_draft();self.tool=Tool::Satin;}
                });
                if self.tool == Tool::Polygon && ui.button(language.text("Complete · Enter")).clicked() {
                    self.complete_polygon();
                }
                if self.tool==Tool::Satin {
                    ui.label(language.text(if self.satin_first.is_empty(){"First rail"}else{"Second rail"}));
                    if ui.button(language.text("Complete · Enter")).clicked(){self.complete_satin();}
                }
                ui.separator();
                ui.checkbox(&mut self.show_nodes, language.text("Nodes"));
                ui.checkbox(&mut self.show_stitches, language.text("Stitches"));
                ui.checkbox(&mut self.show_jumps, language.text("Jumps"));
                ui.checkbox(&mut self.show_image, language.text("Source"));
                } else {
                    ui.label(language.text("Thread diameter (mm)"));
                    ui.add(egui::DragValue::new(&mut self.preview_3d.thread_diameter_mm).range(0.08..=0.8).speed(0.01).fixed_decimals(2));
                    ui.checkbox(&mut self.show_jumps, language.text("Jumps"));
                }
                if ui.button(language.text("Fit design")).clicked() {
                    if self.show_3d { self.preview_3d.reset_view(); } else { self.fit = true; }
                }
                ui.separator();
                ui.label(language.text("Language"));
                let previous_language = self.language;
                egui::ComboBox::from_id_salt("interface-language")
                    .selected_text(self.language.name())
                    .show_ui(ui, |ui| {
                        for value in [Language::Russian, Language::English] {
                            ui.selectable_value(&mut self.language, value, value.name());
                        }
                    });
                if self.language != previous_language {
                    if let Err(error) = self.language.save() {
                        self.result(Err(error));
                    }
                    ctx.request_repaint();
                }
            });
        });
        egui::TopBottomPanel::bottom("sequence").show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                if ui
                    .add_enabled(
                        !self.stale && !self.stitches.is_empty(),
                        egui::Button::new(if self.playing {
                            language.text("Pause")
                        } else {
                            language.text("Simulate")
                        }),
                    )
                    .clicked()
                {
                    if self.progress >= self.stitches.len() {
                        self.progress = 0;
                    }
                    self.playing = !self.playing;
                }
                ui.add_enabled_ui(!self.stale, |ui| {
                    ui.add(
                        egui::Slider::new(&mut self.progress, 0..=self.stitches.len())
                            .text(language.text("command")),
                    );
                });
                ui.add(
                    egui::DragValue::new(&mut self.speed)
                        .speed(50.)
                        .range(10. ..=20000.)
                        .suffix(language.text(" / s")),
                );
                ui.label(self.stitch_summary_text()).on_hover_text(
                    language.text("Width × height of sewn stitches. Travel jumps are excluded."),
                );
                if self.stale {
                    ui.colored_label(
                        Color32::from_rgb(161, 43, 50),
                        language.text("Preview needs regeneration"),
                    );
                }
            });
            ui.colored_label(
                if self.error {
                    Color32::from_rgb(161, 43, 50)
                } else {
                    Color32::from_rgb(45, 65, 82)
                },
                language.message(&self.message).as_ref(),
            );
        });
        self.sidebars(ctx);
        if self.show_3d && self.tool != Tool::Nodes {
            self.show_3d = false;
        }
        egui::CentralPanel::default().show(ctx, |ui| {
            if self.show_3d {
                self.three_d_canvas(ui);
            } else {
                self.canvas(ui);
            }
        });
        if self.confirm_close {
            egui::Window::new(language.text("Save before closing?"))
                .collapsible(false)
                .resizable(false)
                .show(ctx, |ui| {
                    ui.label(language.text("Your editable project has unsaved changes."));
                    ui.horizontal(|ui| {
                        if ui.button(language.text("Save project")).clicked() {
                            self.save();
                            if !self.dirty {
                                self.confirm_close = false;
                                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                            }
                        }
                        if ui.button(language.text("Discard and close")).clicked() {
                            let _ =
                                embroidery_lab::persistence::clear_recovery(self.path.as_deref());
                            self.dirty = false;
                            self.confirm_close = false;
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                        if ui.button(language.text("Cancel")).clicked() {
                            self.confirm_close = false;
                        }
                    });
                });
        } else if let Some(path) = self.discard.clone() {
            egui::Window::new(language.text("Unsaved project"))
                .collapsible(false)
                .resizable(false)
                .show(ctx, |ui| {
                    ui.label(
                        language.text("Save your editable project before opening another file."),
                    );
                    ui.horizontal(|ui| {
                        if ui.button(language.text("Save project")).clicked() {
                            self.save();
                            if !self.dirty {
                                self.discard = None;
                                self.request_open(path.clone(), ctx);
                            }
                        }
                        if ui.button(language.text("Discard and open")).clicked() {
                            self.discard = None;
                            self.request_open(path.clone(), ctx);
                        }
                        if ui.button(language.text("Cancel")).clicked() {
                            self.discard = None;
                        }
                    });
                });
        }
        self.import_window(ctx);
        self.pdf_progress(ctx);
        self.centerline_progress(ctx);
        if self.field_before.is_some()
            && ctx.input(|i| i.pointer.any_released() || i.key_pressed(egui::Key::Enter))
        {
            self.finish_field_edit();
        }
        self.recovery_and_autosave(ctx);
        self.capture_frame(ctx);
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
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
                ((1024..=3840).contains(&w) && (680..=2160).contains(&h))
                    .then_some([w as f32, h as f32])
            })
    } else {
        None
    };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size(capture_size.unwrap_or([1360., 900.]))
            .with_min_inner_size([1024., 680.]),
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
    fn traced_geometry_commits_even_when_generation_hits_command_budget() {
        let path = std::env::temp_dir().join(format!("embroidery-tall-{}.png", std::process::id()));
        let mut raster = image::RgbImage::from_pixel(16, 160, image::Rgb([255, 255, 255]));
        for y in 1..159 {
            for x in 1..15 {
                raster.put_pixel(x, y, image::Rgb([35, 85, 190]));
            }
        }
        raster.save(&path).unwrap();
        let mut app = Editor::workspace();
        let context = egui::Context::default();
        app.request_open(path, &context);
        app.import_width_mm = 400.;
        app.commit_import(&context);
        assert!(
            app.import_preview.is_none(),
            "Valid traced geometry must leave the import modal: {}",
            app.message
        );
        assert!(app.error && app.stale && app.dirty, "{}", app.message);
        assert_eq!(app.project.width_mm, 400.);
        assert!(!app.project.objects.is_empty());
        app.project.objects[0].settings.spacing_mm = 5.;
        app.generate();
        assert!(
            !app.stale,
            "Generation must recover after editing density: {}",
            app.message
        );
    }

    #[test]
    fn failed_tracing_keeps_existing_document_and_import_choices() {
        let mut app = Editor::workspace();
        let before = serde_json::to_value(&app.project).unwrap();
        let context = egui::Context::default();
        app.request_open(raster_fixture(), &context);
        app.raster_options.palette_size = 0;
        app.commit_import(&context);
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
        assert!(app.stale && app.error);
        app.export(); // Error path must return before opening a save dialog.
        assert!(app.stale && app.error);
    }
}
