//! Native workstation chrome. Geometry and stitch generation stay in their existing modules.
use super::*;
use ui_theme::{Icon, action_button, icon_button, icon_menu};

#[cfg(test)]
#[path = "workspace_ui_tests.rs"]
mod tests;

impl Editor {
    #[cfg(test)]
    pub(super) fn sidebars(&mut self, ctx: &egui::Context) {
        self.workspace_layers(ctx);
        self.workspace_inspector(ctx);
    }
    pub(super) fn selected_locked(&self) -> bool {
        self.project
            .objects
            .get(self.selected)
            .is_some_and(|o| self.locked_objects.contains(&o.id))
            || self
                .merge_selection
                .iter()
                .any(|id| self.locked_objects.contains(id))
    }

    pub(super) fn workspace_chrome(&mut self, ctx: &egui::Context) {
        self.workspace_header(ctx);
        self.workspace_sequence(ctx);
        self.workspace_tools(ctx);
        if self.workspace.layers_open {
            self.workspace_layers(ctx);
        }
        if self.workspace.inspector_open {
            self.workspace_inspector(ctx);
        }
    }

    pub(super) fn workspace_overlays(&mut self, ctx: &egui::Context) {
        self.new_project_confirmation(ctx);
        if self.quality_window {
            let mut open = true;
            egui::Window::new(self.language.text("Quality checks"))
                .open(&mut open)
                .default_width(460.)
                .show(ctx, |ui| {
                    if self.decision_pending() {
                        ui.disable();
                    }
                    egui::ScrollArea::vertical()
                        .max_height((ctx.content_rect().height() - 140.).max(180.))
                        .show(ui, |ui| self.quality_controls(ui));
                });
            self.quality_window = open;
        }
    }

    fn workspace_header(&mut self, ctx: &egui::Context) {
        let language = self.language;
        let colors = self.workspace.theme.tokens();
        egui::TopBottomPanel::top("workspace-header")
            .frame(egui::Frame::new().fill(colors.surface).inner_margin(egui::Margin::symmetric(12,8)))
            .show(ctx,|ui| {
                if self.decision_pending() { ui.disable(); }
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("EmbroideryLab").strong().size(16.));
                    ui.separator();
                    ui.menu_button(language.text("File"),|ui| {
                        if ui.button(language.text("New project")).clicked(){self.request_new_project();ui.close();}
                        if ui.button(language.text("Open…")).clicked(){self.open_dialog(ctx);ui.close();}
                        if ui.button(language.text("Import image…")).clicked(){self.image_dialog(ctx);ui.close();}
                        ui.separator();
                        if ui.add_enabled(!self.stitch_only,egui::Button::new(language.text("Save project")).shortcut_text("Ctrl+S")).clicked(){self.save();ui.close();}
                        if ui.add_enabled(self.project.source_image.is_some(),egui::Button::new(language.text("Redigitize saved source…"))).clicked(){self.redigitize_source(ctx);ui.close();}
                    });
                    if icon_button(ui,Icon::Open,false,true,language.text("Open…")).on_hover_text(language.text("Ctrl+O · JPEG / PNG / WebP / PDF / SVG / project / DST / DSB")).clicked(){self.open_dialog(ctx);}
                    if icon_button(ui,Icon::Save,false,!self.stitch_only,language.text("Save project")).on_hover_text("Ctrl+S").clicked(){self.save();}
                    ui.separator();
                    if icon_button(ui,Icon::Undo,false,!self.undo.is_empty()||self.field_before.is_some(),language.text("Undo")).on_hover_text("Ctrl+Z").clicked(){self.undo_edit();}
                    if icon_button(ui,Icon::Redo,false,!self.redo.is_empty(),language.text("Redo")).on_hover_text("Ctrl+Y").clicked(){self.redo_edit();}
                    ui.add_space(6.);
                    let project_width=(ui.available_width()-375.).clamp(55.,240.);
                    ui.add_sized([project_width,28.],egui::Label::new(egui::RichText::new(&self.project.name).strong()).truncate())
                        .on_hover_text(self.path.as_ref().map(|p|p.display().to_string()).unwrap_or_else(||language.text("Unsaved project").into()));
                    if self.dirty {ui.label(egui::RichText::new("*").strong().color(colors.warning)).on_hover_text(language.text("Unsaved changes"));}
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center),|ui| {
                        let workspace_menu=ui.menu_button(if ctx.content_rect().width()<1100.{""}else{language.text("Workspace")},|ui| {
                            let previous=self.workspace.theme;
                            ui.label(language.text("Theme"));
                            ui.selectable_value(&mut self.workspace.theme,ui_theme::Theme::Dark,language.text("Dark"));
                            ui.selectable_value(&mut self.workspace.theme,ui_theme::Theme::Light,language.text("Light"));
                            if previous!=self.workspace.theme{self.workspace.theme.apply(ctx);}
                            ui.separator();
                            ui.checkbox(&mut self.workspace.layers_open,language.text("Objects"));
                            ui.checkbox(&mut self.workspace.inspector_open,language.text("Inspector"));
                            ui.checkbox(&mut self.workspace.timeline_open,language.text("Stitch inspection"));
                            ui.checkbox(&mut self.workspace.rulers,language.text("Rulers"));
                            ui.checkbox(&mut self.workspace.hints,language.text("Canvas hints"));
                            ui.separator();
                            ui.label(language.text("Language"));
                            for value in [Language::Russian,Language::English] {
                                if ui.selectable_value(&mut self.language,value,value.name()).changed()
                                    && let Err(error)=self.language.save(){self.result(Err(error));}
                            }
                            if ui.button(language.text("Reset workspace layout")).clicked(){
                                let theme=self.workspace.theme;
                                self.workspace=Default::default();self.workspace.theme=theme;
                                ctx.data_mut(|data|{for name in ["objects","parameters","sequence","sequence-collapsed"]{data.remove::<egui::containers::panel::PanelState>(egui::Id::new(name));}});
                                ui.close();
                            }
                        });
                        if ctx.content_rect().width()<1100. { Icon::Settings.paint(ui.painter(),Rect::from_center_size(workspace_menu.response.rect.center(),Vec2::splat(18.)),colors.text); }
                        workspace_menu.response.widget_info(||egui::WidgetInfo::labeled(egui::WidgetType::Button,true,language.text("Workspace")));
                        workspace_menu.response.on_hover_text(language.text("Workspace"));
                        ui.menu_button(language.text("Export"),|ui| {
                            let available=!self.stale&&!self.stitches.is_empty();
                            if !available{ui.label(language.text("Generate stitches before exporting."));}
                            if ui.add_enabled(available&&self.dsb_records.is_none(),egui::Button::new(language.text("Export DST…")).shortcut_text("Ctrl+E")).clicked(){self.export();ui.close();}
                            if ui.add_enabled(available,egui::Button::new(language.text("Export DSB… (experimental)"))).on_hover_text(language.text("Experimental format. Generated color changes become manual stops; map threads on the controller.")).clicked(){self.export_dsb();ui.close();}
                        });
                        let primary=ui.scope(|ui| {
                            let ink=if self.workspace.theme==ui_theme::Theme::Dark{Color32::from_rgb(12,28,32)}else{Color32::WHITE};
                            ui.visuals_mut().widgets.inactive.weak_bg_fill=colors.accent;
                            ui.visuals_mut().widgets.inactive.bg_fill=colors.accent;
                            ui.visuals_mut().widgets.inactive.fg_stroke.color=ink;
                            ui.visuals_mut().widgets.hovered.weak_bg_fill=colors.accent.gamma_multiply(1.12);
                            ui.visuals_mut().widgets.hovered.fg_stroke.color=ink;
                            ui.visuals_mut().widgets.active.fg_stroke.color=ink;
                            action_button(ui,Icon::Wand,self.digitize_job.is_none(),language.text("Auto Digitize…"))
                        }).inner;
                        if primary.on_hover_text(language.text("Import artwork, clean colors and generate editable embroidery.")).clicked(){self.image_dialog(ctx);}
                    });
                });
            });
    }

    fn workspace_tools(&mut self, ctx: &egui::Context) {
        let language = self.language;
        let colors = self.workspace.theme.tokens();
        egui::SidePanel::left("tool-rail")
            .exact_width(44.)
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(colors.surface)
                    .inner_margin(egui::Margin::symmetric(6, 10)),
            )
            .show(ctx, |ui| {
                if self.decision_pending() {
                    ui.disable();
                }
                ui.spacing_mut().item_spacing.y = 5.;
                let edit = !self.stitch_only && !self.show_3d;
                if icon_button(
                    ui,
                    Icon::Select,
                    self.tool == Tool::Nodes && !self.pan_tool && !self.show_3d,
                    true,
                    language.text("Select / edit nodes · V"),
                )
                .clicked()
                {
                    self.pan_tool = false;
                    self.set_3d_view(false);
                    self.cancel_draft();
                }
                if icon_button(
                    ui,
                    Icon::Pan,
                    self.pan_tool && !self.show_3d,
                    !self.show_3d,
                    language.text("Pan · H / hold Space"),
                )
                .clicked()
                {
                    self.cancel_draft();
                    self.pan_tool = true;
                }
                ui.separator();
                if icon_button(
                    ui,
                    Icon::Nodes,
                    self.tool == Tool::Polygon,
                    edit,
                    language.text("Draw contour · P"),
                )
                .clicked()
                {
                    self.pan_tool = false;
                    self.cancel_draft();
                    self.tool = Tool::Polygon;
                }
                if icon_button(
                    ui,
                    Icon::Stitches,
                    self.tool == Tool::Satin,
                    edit,
                    language.text("Draw satin · S"),
                )
                .clicked()
                {
                    self.pan_tool = false;
                    self.cancel_draft();
                    self.tool = Tool::Satin;
                }
                if icon_button(
                    ui,
                    Icon::Wand,
                    false,
                    self.digitize_job.is_none(),
                    language.text("Color cleanup"),
                )
                .clicked()
                {
                    if self.project.source_image.is_some() {
                        self.redigitize_source(ctx);
                    } else {
                        self.image_dialog(ctx);
                    }
                }
                icon_menu(ui, Icon::Add, language.text("Create object"), |ui| {
                    if ui
                        .add_enabled(edit, egui::Button::new(language.text("Rectangle")))
                        .clicked()
                    {
                        self.add_object(false);
                        ui.close();
                    }
                    if ui
                        .add_enabled(edit, egui::Button::new(language.text("Bezier")))
                        .clicked()
                    {
                        self.add_object(true);
                        ui.close();
                    }
                    if ui
                        .add_enabled(
                            edit && self.selected < self.project.objects.len()
                                && !self.selected_locked(),
                            egui::Button::new(language.text("Create running copy")),
                        )
                        .clicked()
                    {
                        self.create_centerlines();
                        ui.close();
                    }
                });
                ui.separator();
                if icon_button(ui, Icon::Fit, false, true, language.text("Fit design · F"))
                    .clicked()
                {
                    self.fit_view();
                }
                if icon_button(
                    ui,
                    Icon::Grid,
                    self.workspace.grid,
                    !self.show_3d,
                    language.text("Grid · 10 mm"),
                )
                .clicked()
                {
                    self.workspace.grid = !self.workspace.grid;
                }
                ui.add_space(12.);
                if icon_button(
                    ui,
                    Icon::Layers,
                    self.workspace.layers_open,
                    true,
                    language.text("Objects"),
                )
                .clicked()
                {
                    self.workspace.layers_open = !self.workspace.layers_open;
                }
                if icon_button(
                    ui,
                    Icon::Inspector,
                    self.workspace.inspector_open,
                    true,
                    language.text("Inspector"),
                )
                .clicked()
                {
                    self.workspace.inspector_open = !self.workspace.inspector_open;
                }
            });
    }

    pub(super) fn canvas_view_bar(&mut self, ui: &mut egui::Ui) {
        let language = self.language;
        let colors = self.workspace.theme.tokens();
        let width = ui.available_width();
        egui::Frame::new()
            .fill(colors.surface)
            .inner_margin(egui::Margin::symmetric(10, 6))
            .show(ui, |ui| {
                ui.set_min_width((width - 20.).max(0.));
                ui.horizontal_wrapped(|ui| {
                    if ui
                        .selectable_label(
                            !self.show_3d && !self.show_stitches,
                            language.text("Vector"),
                        )
                        .clicked()
                    {
                        self.set_3d_view(false);
                        self.show_stitches = false;
                    }
                    if ui
                        .selectable_label(
                            !self.show_3d && self.show_stitches,
                            language.text("Stitches"),
                        )
                        .clicked()
                    {
                        self.set_3d_view(false);
                        self.show_stitches = true;
                    }
                    if ui
                        .selectable_label(self.show_3d, "3D")
                        .on_hover_text(language.text("3D stitch preview"))
                        .clicked()
                    {
                        self.set_3d_view(true);
                    }
                    ui.separator();
                    if !self.show_3d {
                        ui.checkbox(&mut self.show_nodes, language.text("Nodes"));
                        ui.checkbox(&mut self.show_image, language.text("Source"));
                    } else {
                        ui.label(language.text("Thread diameter (mm)"));
                        ui.add(
                            egui::DragValue::new(&mut self.preview_3d.thread_diameter_mm)
                                .range(0.08..=0.8)
                                .speed(0.01)
                                .fixed_decimals(2),
                        );
                    }
                    ui.checkbox(&mut self.show_jumps, language.text("Jumps"));
                    if icon_button(ui, Icon::Fit, false, true, language.text("Fit design · F"))
                        .clicked()
                    {
                        self.fit_view();
                    }
                    if !self.show_3d {
                        let mut zoom = self.scale / 5. * 100.;
                        if ui
                            .add(
                                egui::DragValue::new(&mut zoom)
                                    .range(4. ..=2000.)
                                    .speed(5.)
                                    .suffix("%"),
                            )
                            .on_hover_text(language.text("Canvas zoom"))
                            .changed()
                        {
                            let previous = self.scale;
                            self.scale = zoom * 0.05;
                            self.pan *= self.scale / previous;
                        }
                    }
                    if self.tool != Tool::Nodes {
                        ui.separator();
                        ui.label(language.text(match self.tool {
                            Tool::Satin => {
                                if self.satin_first.is_empty() {
                                    "First rail"
                                } else {
                                    "Second rail"
                                }
                            }
                            Tool::Polygon => "Draw contour",
                            Tool::Rung => "Add direction rung",
                            Tool::Outline => "Draw outline",
                            Tool::SplitRegion => "Split area",
                            Tool::Nodes => "Edit nodes",
                        }));
                        if matches!(
                            self.tool,
                            Tool::Polygon | Tool::Satin | Tool::Outline | Tool::SplitRegion
                        ) && ui.button(language.text("Complete · Enter")).clicked()
                        {
                            match self.tool {
                                Tool::Polygon => self.complete_polygon(),
                                Tool::Satin => self.complete_satin(),
                                Tool::Outline => self.preview_outline(),
                                Tool::SplitRegion => self.complete_region_split(),
                                _ => {}
                            }
                        }
                        if ui.button(language.text("Cancel · Escape")).clicked() {
                            self.cancel_draft();
                        }
                    }
                    if self.stale && !self.stitch_only {
                        ui.separator();
                        if action_button(
                            ui,
                            Icon::Stitches,
                            self.digitize_job.is_none(),
                            language.text("Generate stitches"),
                        )
                        .clicked()
                        {
                            self.generate();
                        }
                    }
                });
            });
    }

    pub(super) fn fit_view(&mut self) {
        if self.show_3d {
            self.preview_3d.reset_view();
        } else {
            self.fit = true;
        }
    }

    pub(super) fn persist_workspace(&mut self, ctx: &egui::Context) {
        if self.capture_path.is_some()
            || self.capture_requested
            || ctx.input(|i| i.pointer.any_down())
        {
            return;
        }
        let encoded = serde_json::to_string(&self.workspace).unwrap_or_default();
        if encoded != self.workspace_saved {
            if let Some(next) = self.workspace_retry_after
                && next > Instant::now()
            {
                ctx.request_repaint_after(next.saturating_duration_since(Instant::now()));
                return;
            }
            match self.workspace.save() {
                Ok(()) => {
                    self.workspace_saved = encoded;
                    self.workspace_retry_after = None;
                }
                Err(error) => {
                    self.workspace_retry_after =
                        Some(Instant::now() + std::time::Duration::from_secs(5));
                    self.result(Err(error));
                    ctx.request_repaint_after(std::time::Duration::from_secs(5));
                }
            }
        }
    }

    fn request_new_project(&mut self) {
        if self.dirty {
            self.confirm_new = true;
        } else {
            self.start_empty_document();
        }
    }

    pub(super) fn paint_rulers(&self, p: &egui::Painter, rect: Rect, center: Pos2, scale: f32) {
        let colors = self.workspace.theme.tokens();
        p.rect_filled(
            Rect::from_min_max(rect.min, Pos2::new(rect.right(), rect.top() + 20.)),
            0.,
            colors.surface,
        );
        p.rect_filled(
            Rect::from_min_max(rect.min, Pos2::new(rect.left() + 25., rect.bottom())),
            0.,
            colors.surface,
        );
        let step = if scale >= 3. { 10. } else { 50. };
        let start_x = ((rect.left() - center.x) / scale / step).floor() as i32;
        let end_x = ((rect.right() - center.x) / scale / step).ceil() as i32;
        for index in start_x..=end_x {
            let x = center.x + index as f32 * step * scale;
            if x > rect.left() + 28. {
                p.line_segment(
                    [
                        Pos2::new(x, rect.top() + 14.),
                        Pos2::new(x, rect.top() + 20.),
                    ],
                    Stroke::new(1.0_f32, colors.border),
                );
                p.text(
                    Pos2::new(x + 3., rect.top() + 3.),
                    egui::Align2::LEFT_TOP,
                    format!("{:.0}", index as f32 * step),
                    egui::FontId::proportional(10.),
                    colors.muted,
                );
            }
        }
        let start_y = ((rect.top() - center.y) / scale / step).floor() as i32;
        let end_y = ((rect.bottom() - center.y) / scale / step).ceil() as i32;
        for index in start_y..=end_y {
            let y = center.y + index as f32 * step * scale;
            if y > rect.top() + 22. {
                p.line_segment(
                    [
                        Pos2::new(rect.left() + 20., y),
                        Pos2::new(rect.left() + 25., y),
                    ],
                    Stroke::new(1.0_f32, colors.border),
                );
                p.text(
                    Pos2::new(rect.left() + 3., y + 2.),
                    egui::Align2::LEFT_TOP,
                    format!("{:.0}", index as f32 * step),
                    egui::FontId::proportional(10.),
                    colors.muted,
                );
            }
        }
    }

    pub(super) fn empty_canvas(&mut self, ui: &mut egui::Ui, rect: Rect) {
        let language = self.language;
        let colors = self.workspace.theme.tokens();
        let bounds =
            Rect::from_center_size(rect.center(), Vec2::new(rect.width().min(420.) - 32., 205.));
        let mut content = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(bounds)
                .layout(egui::Layout::top_down(egui::Align::Center)),
        );
        content.heading(language.text("Start with your artwork"));
        content.add_space(8.);
        content.label(
            egui::RichText::new(
                language
                    .text("Import a logo, flag or outline, then turn it into editable embroidery."),
            )
            .color(colors.muted),
        );
        content.add_space(16.);
        if action_button(
            &mut content,
            Icon::Import,
            true,
            language.text("Import image…"),
        )
        .clicked()
        {
            self.image_dialog(ui.ctx());
        }
        content.add_space(8.);
        if content.button(language.text("Draw a contour")).clicked() {
            self.pan_tool = false;
            self.tool = Tool::Polygon;
        }
        content.small(language.text("JPEG · PNG · WebP · PDF · SVG · ELAB · DST · DSB"));
    }

    pub(super) fn start_empty_document(&mut self) {
        self.cancel_digitize();
        self.cancel_draft();
        self.project = Project::default();
        self.project.name = self.language.text("Untitled design").into();
        self.stitches.clear();
        self.plan_metadata.clear();
        self.plan_warnings.clear();
        self.quality_issues.clear();
        self.plan_summary = None;
        self.coverage.clear();
        self.segment_density = None;
        self.dsb_records = None;
        self.image = None;
        self.path = None;
        self.selected = 0;
        self.merge_selection.clear();
        self.locked_objects.clear();
        self.undo.clear();
        self.redo.clear();
        self.field_before = None;
        self.dirty = false;
        self.stale = false;
        self.stitch_only = false;
        self.playing = false;
        self.progress = 0;
        self.fit = true;
        self.pan_tool = false;
        self.update_stitch_view();
        self.result(Ok("Import artwork or draw a contour to begin.".into()));
    }

    fn new_project_confirmation(&mut self, ctx: &egui::Context) {
        if !self.confirm_new {
            return;
        }
        let language = self.language;
        egui::Modal::new(egui::Id::new("new-project-confirmation")).show(ctx, |ui| {
            ui.heading(language.text("Unsaved project"));
            ui.label(language.text("Save your editable project before creating another design."));
            ui.horizontal(|ui| {
                if ui.button(language.text("Save project")).clicked() {
                    self.save();
                    if !self.dirty {
                        self.confirm_new = false;
                        self.start_empty_document();
                    }
                }
                if ui.button(language.text("Discard and create")).clicked() {
                    let _ = embroidery_lab::persistence::clear_recovery(self.path.as_deref());
                    self.confirm_new = false;
                    self.start_empty_document();
                }
                if ui.button(language.text("Cancel")).clicked() {
                    self.confirm_new = false;
                }
            });
        });
    }
}
