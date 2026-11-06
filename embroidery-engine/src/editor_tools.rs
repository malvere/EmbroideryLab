use super::*;
use embroidery_lab::{axis_edit, persistence, satin_edit};

#[cfg(test)]
#[path = "protected_ui_tests.rs"]
mod protected_ui_tests;

pub(super) fn project_equal(a: &Project, b: &Project) -> bool {
    serde_json::to_value(a).ok() == serde_json::to_value(b).ok()
}

pub(super) fn nearest_segment(path: &[Point], p: Point) -> Option<(usize, f64, f64)> {
    path.windows(2)
        .enumerate()
        .map(|(i, w)| {
            let dx = w[1].x - w[0].x;
            let dy = w[1].y - w[0].y;
            let t = if dx * dx + dy * dy > 1e-15 {
                (((p.x - w[0].x) * dx + (p.y - w[0].y) * dy) / (dx * dx + dy * dy)).clamp(0., 1.)
            } else {
                0.
            };
            (i, t, p.distance(w[0].lerp(w[1], t)))
        })
        .min_by(|a, b| a.2.total_cmp(&b.2))
}

pub(super) fn rail_fraction(path: &[Point], p: Point) -> Option<(f64, f64)> {
    let (i, t, d) = nearest_segment(path, p)?;
    let lengths: Vec<f64> = path.windows(2).map(|w| w[0].distance(w[1])).collect();
    let total: f64 = lengths.iter().sum();
    (total > 1e-9).then(|| {
        (
            (lengths[..i].iter().sum::<f64>() + lengths[i] * t) / total,
            d,
        )
    })
}

pub(super) fn rail_sample(path: &[Point], fraction: f64) -> Option<Point> {
    let total: f64 = path.windows(2).map(|w| w[0].distance(w[1])).sum();
    let mut remaining = fraction.clamp(0., 1.) * total;
    for pair in path.windows(2) {
        let len = pair[0].distance(pair[1]);
        if remaining <= len && len > 1e-9 {
            return Some(pair[0].lerp(pair[1], remaining / len));
        }
        remaining -= len;
    }
    path.last().copied()
}

pub(super) fn nearest_rail(column: &SatinColumn, point: Point) -> Option<(bool, usize, f64, f64)> {
    let mut best: Option<(bool, usize, f64, f64)> = None;
    for (right, nodes) in [(false, &column.left), (true, &column.right)] {
        let segments = nodes.len().saturating_sub(usize::from(!column.closed));
        for segment in 0..segments {
            let Ok(curve) = satin_edit::rail_curve(column, right, segment) else {
                continue;
            };
            let sample = |t: f64| {
                let a = curve.start.lerp(curve.control1, t);
                let b = curve.control1.lerp(curve.control2, t);
                let c = curve.control2.lerp(curve.end, t);
                a.lerp(b, t).lerp(b.lerp(c, t), t)
            };
            let mut prior = curve.start;
            for step in 1..=64 {
                let next = sample(step as f64 / 64.);
                if let Some((_, t, d)) = nearest_segment(&[prior, next], point)
                    && best.is_none_or(|b| d < b.3)
                {
                    best = Some((right, segment, (step as f64 - 1. + t) / 64., d));
                }
                prior = next;
            }
        }
    }
    best
}

impl Editor {
    /// These choices can replace or close the document. Guard the editor before
    /// drawing their modal, including the first frame before egui knows its layer.
    pub(super) fn decision_pending(&self) -> bool {
        self.recovery.is_some() || self.confirm_close || self.confirm_new || self.discard.is_some()
    }

    pub(super) fn document_confirmations(&mut self, ctx: &egui::Context) {
        let language = self.language;
        if self.confirm_close {
            egui::Modal::new(egui::Id::new("close-project-confirmation")).show(ctx, |ui| {
                ui.heading(language.text("Save before closing?"));
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
                        let _ = persistence::clear_recovery(self.path.as_deref());
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
            egui::Modal::new(egui::Id::new("open-project-confirmation")).show(ctx, |ui| {
                ui.heading(language.text("Unsaved project"));
                ui.label(language.text("Save your editable project before opening another file."));
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
    }
    pub(super) fn refresh_mask_texture(&mut self, ctx: &egui::Context) {
        if let Some(preview) = &self.region_preview {
            let pixels = preview.overlay();
            let data = egui::ColorImage::from_rgba_unmultiplied(
                [pixels.width() as usize, pixels.height() as usize],
                pixels.as_raw(),
            );
            self.mask_texture =
                Some(ctx.load_texture("region mask", data, egui::TextureOptions::NEAREST));
        }
    }
    pub(super) fn preview_regions(&mut self, _ctx: &egui::Context) {
        self.start_trace_job(true);
    }
    pub(super) fn complete_region_split(&mut self) {
        if self.stitch_only || self.selected_locked() {
            return;
        }
        match embroidery_lab::region_edit::split_object(&self.project, self.selected, &self.draft) {
            Ok(candidate) => {
                self.snapshot();
                self.project = candidate;
                self.clear_selection();
                self.cancel_draft();
                self.changed();
            }
            Err(e) => self.result(Err(e)),
        }
    }
    pub(super) fn restore_source(&mut self, ctx: &egui::Context) {
        self.image = None;
        if let Some(source) = &self.project.source_image {
            match source.decode() {
                Ok(image) => {
                    let rgb = image.thumbnail(1800, 1800).to_rgba8();
                    let data = egui::ColorImage::from_rgba_unmultiplied(
                        [rgb.width() as usize, rgb.height() as usize],
                        rgb.as_raw(),
                    );
                    self.image = Some((
                        ctx.load_texture("source artwork", data, egui::TextureOptions::LINEAR),
                        source.width_mm,
                        source.height_mm,
                    ));
                }
                Err(error) => self.result(Err(error)),
            }
        }
    }
    pub(super) fn recovery_and_autosave(&mut self, ctx: &egui::Context) {
        if self.recovery.is_some() {
            let language = self.language;
            egui::Modal::new(egui::Id::new("recover-project")).show(ctx, |ui| {
                ui.heading(language.text("Recover unsaved work?"));
                ui.label(language.text(
                    "A recovery copy is available. Your saved project has not been replaced.",
                ));
                ui.horizontal(|ui| {
                    if ui.button(language.text("Restore recovery")).clicked()
                        && let Some(project) = self.recovery.take()
                    {
                        self.snapshot();
                        self.project = project;
                        self.selected = 0;
                        self.selection_anchor = None;
                        self.merge_selection.clear();
                        self.clear_selection();
                        self.cancel_draft();
                        self.restore_source(ctx);
                        self.changed();
                        self.generate();
                        self.fit = true;
                        self.last_autosave = Instant::now();
                    }
                    if ui.button(language.text("Discard recovery")).clicked() {
                        self.recovery = None;
                        let result = persistence::clear_recovery(self.path.as_deref());
                        if let Err(e) = result {
                            self.result(Err(e));
                        }
                    }
                });
            });
        } else if self.dirty
            && !self.stitch_only
            && self.import_preview.is_none()
            && self.centerline_job.is_none()
            && self.last_autosave.elapsed().as_secs() >= 30
        {
            self.last_autosave = Instant::now();
            if let Err(e) = persistence::write_recovery(&self.project, self.path.as_deref()) {
                self.result(Err(e));
            }
        }
        if self.dirty && !self.stitch_only {
            ctx.request_repaint_after(std::time::Duration::from_secs(1));
        }
    }
    pub(super) fn push_history(&mut self, before: Project) {
        if self.undo.len() >= 30 {
            self.undo.remove(0);
        }
        self.undo.push(before);
        self.redo.clear();
    }
    pub(super) fn finish_field_edit(&mut self) {
        if let Some(before) = self.field_before.take()
            && !project_equal(&before, &self.project)
        {
            self.push_history(before);
        }
    }
    pub(super) fn record_field_edit(&mut self, before: Project) {
        if self.field_before.is_none() {
            self.field_before = Some(before);
        }
        self.redo.clear();
        self.changed();
    }
    /// Inspecting an object should not clone embedded artwork every frame.
    pub(super) fn record_object_field_edit(&mut self, object: Object, version: u32) {
        if self.field_before.is_none() {
            let mut before = self.project.clone();
            if let Some(index) = before.objects.iter().position(|o| o.id == object.id) {
                before.objects[index] = object;
                before.version = version;
                self.field_before = Some(before);
            }
        }
        self.redo.clear();
        self.changed();
    }
    pub(super) fn clear_selection(&mut self) {
        self.selected_node = None;
        self.selected_rail = None;
        self.selected_path = None;
        self.selected_rung = None;
        self.selected_branch = None;
        self.handle = None;
        self.drag_before = None;
        self.smoothing_preview = None;
    }
    pub(super) fn cancel_draft(&mut self) {
        self.draft.clear();
        self.satin_first.clear();
        self.rung_start = None;
        self.outline_preview = None;
        self.smoothing_preview = None;
        self.tool = Tool::Nodes;
        self.preview_branch = None;
    }
    pub(super) fn undo_edit(&mut self) {
        if self.stitch_only {
            return;
        }
        self.finish_field_edit();
        if let Some(previous) = self.undo.pop() {
            let id = self.project.objects.get(self.selected).map(|o| o.id);
            self.redo
                .push(std::mem::replace(&mut self.project, previous));
            self.image = None;
            self.selected = id
                .and_then(|id| self.project.objects.iter().position(|o| o.id == id))
                .unwrap_or(
                    self.selected
                        .min(self.project.objects.len().saturating_sub(1)),
                );
            self.clear_selection();
            self.merge_selection.clear();
            self.selection_anchor = None;
            self.cancel_draft();
            self.changed();
        }
    }
    pub(super) fn redo_edit(&mut self) {
        if self.stitch_only {
            return;
        }
        self.finish_field_edit();
        if let Some(next) = self.redo.pop() {
            let id = self.project.objects.get(self.selected).map(|o| o.id);
            if self.undo.len() >= 30 {
                self.undo.remove(0);
            }
            self.undo.push(std::mem::replace(&mut self.project, next));
            self.image = None;
            self.selected = id
                .and_then(|id| self.project.objects.iter().position(|o| o.id == id))
                .unwrap_or(
                    self.selected
                        .min(self.project.objects.len().saturating_sub(1)),
                );
            self.clear_selection();
            self.merge_selection.clear();
            self.selection_anchor = None;
            self.cancel_draft();
            self.changed();
        }
    }
    fn next_id(&self) -> Result<u64, String> {
        self.project
            .objects
            .iter()
            .map(|o| o.id)
            .max()
            .unwrap_or(0)
            .checked_add(1)
            .ok_or_else(|| "Object ID limit reached".into())
    }
    pub(super) fn commit_column(
        &mut self,
        column: SatinColumn,
        hide_source: bool,
    ) -> Result<(), String> {
        if hide_source && self.selected_locked() {
            return Err("Object is locked. Unlock it in Layers to edit.".into());
        }
        engine::satin_stations(&column, 0.4)?;
        let id = self.next_id()?;
        let source = self.project.objects.get(self.selected);
        let object = Object {
            id,
            name: format!("{} {id}", self.language.text("Satin column")),
            color: source.map_or([34, 104, 162], |o| o.color),
            settings: Settings {
                kind: StitchKind::Satin,
                length_mm: 8.,
                ..Settings::default()
            },
            satin_column: Some(column),
            ..Object::default()
        };
        self.snapshot();
        if hide_source && let Some(source) = self.project.objects.get_mut(self.selected) {
            source.visible = false;
        }
        self.project.version = 4;
        self.project.objects.push(object);
        self.selected = self.project.objects.len() - 1;
        self.clear_selection();
        self.cancel_draft();
        self.changed();
        self.generate();
        Ok(())
    }
    pub(super) fn complete_satin(&mut self) {
        if self.draft.len() < 2 {
            self.result(Err("Each satin rail requires at least two points".into()));
            return;
        }
        if self.satin_first.is_empty() {
            self.satin_first = std::mem::take(&mut self.draft);
            self.message = "Draw the second rail in the same direction, then press Enter.".into();
        } else {
            let column = SatinColumn {
                left: self.satin_first.clone(),
                right: self.draft.clone(),
                ..Default::default()
            };
            if let Err(e) = self.commit_column(column, false) {
                self.result(Err(e));
            }
        }
    }
    pub(super) fn preview_outline(&mut self) {
        let path = if self.draft.len() >= 2 {
            self.draft.clone()
        } else {
            self.project
                .objects
                .get(self.selected)
                .and_then(|o| {
                    o.contours
                        .get(self.selected_node.map_or(0, |selected| selected.0))
                        .cloned()
                        .or_else(|| {
                            o.running_paths
                                .get(
                                    self.selected_branch
                                        .or(self.selected_path.map(|p| p.0))
                                        .unwrap_or(0),
                                )
                                .cloned()
                        })
                        .or_else(|| o.flattened_contours().into_iter().next())
                })
                .unwrap_or_default()
        };
        match satin_edit::outline(&path, self.outline_closed, self.outline_width) {
            Ok(column) => {
                self.outline_preview = Some(column);
                self.preview_branch = None;
                self.error = false;
            }
            Err(e) => {
                self.outline_preview = None;
                self.result(Err(e));
            }
        }
    }
    pub(super) fn edit_column(
        &mut self,
        action: impl FnOnce(&mut SatinColumn) -> Result<(), String>,
    ) {
        if self.stitch_only || self.selected_locked() {
            return;
        }
        let Some(mut column) = self
            .project
            .objects
            .get(self.selected)
            .and_then(|o| o.satin_column.clone())
        else {
            return;
        };
        match action(&mut column) {
            Ok(()) => {
                self.snapshot();
                self.project.objects[self.selected].satin_column = Some(column);
                self.project.version = 4;
                self.changed();
            }
            Err(e) => self.result(Err(e)),
        }
    }
    pub(super) fn delete_selection(&mut self) {
        if self.stitch_only || self.selected_locked() {
            return;
        }
        if let Some(index) = self.selected_rung {
            self.edit_column(|c| satin_edit::remove_rung(c, index));
            self.selected_rung = None;
        } else if let Some((right, n)) = self.selected_rail {
            self.edit_column(|c| satin_edit::remove_node(c, right, n));
            self.selected_rail = None;
        } else if let Some(index) = self.selected_branch {
            self.edit_branch(|o| axis_edit::delete_branch(o, index));
        } else if let Some((r, n)) = self.selected_node
            && self
                .project
                .objects
                .get(self.selected)
                .and_then(|o| o.contours.get(r))
                .is_some_and(|ring| ring.len() > 3)
        {
            self.snapshot();
            self.project.objects[self.selected].contours[r].remove(n);
            self.selected_node = None;
            self.changed();
        }
    }
    fn edit_branch(&mut self, action: impl FnOnce(&mut Object) -> Result<(), String>) {
        if self.stitch_only || self.selected_locked() {
            return;
        }
        let Some(mut candidate) = self.project.objects.get(self.selected).cloned() else {
            return;
        };
        match action(&mut candidate) {
            Ok(()) => {
                self.snapshot();
                self.project.objects[self.selected] = candidate;
                self.selected_branch = None;
                self.selected_path = None;
                self.project.version = 4;
                self.changed();
            }
            Err(e) => self.result(Err(e)),
        }
    }
    fn split_column(&mut self) {
        if self.stitch_only || self.selected_locked() {
            return;
        }
        let Some(column) = self
            .project
            .objects
            .get(self.selected)
            .and_then(|o| o.satin_column.as_ref())
        else {
            return;
        };
        let (mut left, mut right) = self
            .selected_rung
            .and_then(|i| column.rungs.get(i))
            .map_or((0.5, 0.5), |r| (r.left, r.right));
        if column.closed && self.selected_rung.is_some() {
            match satin_edit::right_fraction(column, column.seam) {
                Ok(seam_right) => {
                    left = (left - column.seam).rem_euclid(1.);
                    right = (right - seam_right).rem_euclid(1.);
                }
                Err(e) => {
                    self.result(Err(e));
                    return;
                }
            }
        }
        match satin_edit::split(column, left, right).and_then(|(a, b)| Ok((a, b, self.next_id()?)))
        {
            Ok((a, b, id)) => {
                self.snapshot();
                let mut second = self.project.objects[self.selected].clone();
                second.id = id;
                second.name.push_str(" · 2");
                second.satin_column = Some(b);
                self.project.objects[self.selected].satin_column = Some(a);
                self.project.objects.insert(self.selected + 1, second);
                self.project.version = 4;
                self.clear_selection();
                self.changed();
            }
            Err(e) => self.result(Err(e)),
        }
    }
    fn convert_branch(&mut self, index: usize) {
        if self.stitch_only || self.selected_locked() {
            return;
        }
        let result = (|| -> Result<(Project, usize), String> {
            let source = self
                .project
                .objects
                .get(self.selected)
                .ok_or("Select a branch first")?;
            let columns = axis_edit::semi_auto_satin(source, index)?;
            let mut candidate = self.project.clone();
            candidate.version = 4;
            candidate.objects[self.selected].visible = false;
            let mut id = self.next_id()?;
            if source.running_paths.len() > 1 {
                let mut remaining = source.clone();
                axis_edit::delete_branch(&mut remaining, index)?;
                remaining.id = id;
                id = id.checked_add(1).ok_or("Object ID limit reached")?;
                remaining.name.push_str(" · remaining");
                remaining.visible = true;
                candidate.objects.push(remaining);
            }
            let selected = candidate.objects.len();
            for column in columns {
                candidate.objects.push(Object {
                    id,
                    name: format!("{} {id}", self.language.text("Satin column")),
                    color: source.color,
                    satin_column: Some(column),
                    settings: Settings {
                        kind: StitchKind::Satin,
                        length_mm: 8.,
                        ..source.settings.clone()
                    },
                    ..Object::default()
                });
                id = id.checked_add(1).ok_or("Object ID limit reached")?;
            }
            candidate.validate_limits()?;
            engine::generate(&candidate)?;
            Ok((candidate, selected))
        })();
        match result {
            Ok((candidate, selected)) => {
                self.snapshot();
                self.project = candidate;
                self.selected = selected;
                self.clear_selection();
                self.cancel_draft();
                self.changed();
                self.generate();
            }
            Err(e) => self.result(Err(e)),
        }
    }
    pub(super) fn geometry_tools(&mut self, ui: &mut egui::Ui) {
        if self.stitch_only || self.selected_locked() {
            return;
        }
        let language = self.language;
        ui.separator();
        ui.collapsing(language.text("Regions and objects"), |ui| {
            ui.small(
                language.text("Shift-click: select a range. Ctrl-click: toggle a layer. Ctrl+A: select all layers."),
            );
            if ui.button(language.text("Split region")).clicked() {
                self.cancel_draft();
                self.tool = Tool::SplitRegion;
            }
            if self.tool == Tool::SplitRegion {
                ui.small(language.text("Place two points outside the region, then press Enter."));
                if ui.button(language.text("Apply split")).clicked() {
                    self.complete_region_split();
                }
            }
            if ui
                .add_enabled(
                    self.merge_selection.len() >= 2,
                    egui::Button::new(language.text("Merge selected regions")),
                )
                .clicked()
            {
                let indices: Vec<usize> = self
                    .project
                    .objects
                    .iter()
                    .enumerate()
                    .filter_map(|(i, o)| self.merge_selection.contains(&o.id).then_some(i))
                    .collect();
                match embroidery_lab::region_edit::merge_objects(&self.project, &indices) {
                    Ok(candidate) => {
                        self.snapshot();
                        self.project = candidate;
                        self.merge_selection.clear();
                        self.selected = 0;
                        self.clear_selection();
                        self.changed();
                    }
                    Err(e) => self.result(Err(e)),
                }
            }
        });
        egui::CollapsingHeader::new(language.text("Satin editor"))
            .id_salt("satin-editor")
            .default_open(
                self.project
                    .objects
                    .get(self.selected)
                    .is_some_and(|o| o.satin_column.is_some()),
            )
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    if ui
                        .selectable_label(self.tool == Tool::Satin, language.text("Draw satin"))
                        .clicked()
                    {
                        self.cancel_draft();
                        self.tool = Tool::Satin;
                    }
                    if ui
                        .selectable_label(self.tool == Tool::Outline, language.text("Draw outline"))
                        .clicked()
                    {
                        self.cancel_draft();
                        self.tool = Tool::Outline;
                    }
                });
                if self.tool == Tool::Satin {
                    ui.small(language.text(if self.satin_first.is_empty() {
                        "Draw the first rail, then press Enter."
                    } else {
                        "Draw the second rail in the same direction, then press Enter."
                    }));
                    if ui.button(language.text("Complete · Enter")).clicked() {
                        self.complete_satin();
                    }
                }
                ui.label(language.text("Outline width (mm)"));
                if ui
                    .add(
                        egui::DragValue::new(&mut self.outline_width)
                            .speed(0.1)
                            .range(0.2..=30.),
                    )
                    .changed()
                {
                    self.outline_preview = None;
                }
                if ui
                    .checkbox(&mut self.outline_closed, language.text("Closed outline"))
                    .changed()
                {
                    self.outline_preview = None;
                }
                if ui.button(language.text("Preview outline")).clicked() {
                    self.preview_outline();
                }
                if self.outline_preview.is_some()
                    && self.preview_branch.is_none()
                    && ui.button(language.text("Apply outline")).clicked()
                    && let Some(column) = self.outline_preview.clone()
                    && let Err(e) = self.commit_column(column, false)
                {
                    self.result(Err(e));
                }
                if self
                    .project
                    .objects
                    .get(self.selected)
                    .is_some_and(|o| o.satin_column.is_some())
                {
                    ui.separator();
                    if ui
                        .selectable_label(
                            self.tool == Tool::Rung,
                            language.text("Add direction rung"),
                        )
                        .clicked()
                    {
                        self.rung_start = None;
                        self.tool = Tool::Rung;
                    }
                    if self.tool == Tool::Rung {
                        ui.small(
                            language
                                .text("Click one rail, then the opposite rail. Escape cancels."),
                        );
                    }
                    if let Some((right, n)) = self.selected_rail {
                        ui.horizontal_wrapped(|ui| {
                            if ui.button(language.text("Insert node after")).clicked() {
                                self.edit_column(|c| {
                                    satin_edit::insert_node(c, right, n, 0.5).map(|_| ())
                                });
                            }
                            if ui.button(language.text("Delete node")).clicked() {
                                self.delete_selection();
                            }
                        });
                        let mut smooth = self.project.objects[self.selected]
                            .satin_column
                            .as_ref()
                            .and_then(|c| {
                                if right {
                                    c.right_handles.get(n)
                                } else {
                                    c.left_handles.get(n)
                                }
                            })
                            .is_some_and(|h| h.smooth);
                        if ui
                            .checkbox(&mut smooth, language.text("Smooth node"))
                            .changed()
                        {
                            self.edit_column(|c| satin_edit::set_smooth(c, right, n, smooth));
                        }
                    }
                    let rungs = self.project.objects[self.selected]
                        .satin_column
                        .as_ref()
                        .map(|c| c.rungs.clone())
                        .unwrap_or_default();
                    egui::ScrollArea::vertical()
                        .id_salt("satin-rungs")
                        .max_height(120.)
                        .show(ui, |ui| {
                            for (i, rung) in rungs.iter().enumerate() {
                                if ui
                                    .selectable_label(
                                        self.selected_rung == Some(i),
                                        format!(
                                            "{} {} · {:.2} / {:.2}",
                                            language.text("Rung"),
                                            i + 1,
                                            rung.left,
                                            rung.right
                                        ),
                                    )
                                    .clicked()
                                {
                                    self.selected_rung = Some(i);
                                    self.selected_rail = None;
                                }
                            }
                        });
                    if let Some(index) = self.selected_rung
                        && let Some(rung) = rungs.get(index)
                    {
                        let mut edited = *rung;
                        ui.small(language.text("Rung positions along rails (0–1)"));
                        ui.label(language.text("First rail"));
                        let mut changed = ui
                            .add(
                                egui::DragValue::new(&mut edited.left)
                                    .speed(0.005)
                                    .range(0.001..=0.999),
                            )
                            .changed();
                        ui.label(language.text("Second rail"));
                        changed |= ui
                            .add(
                                egui::DragValue::new(&mut edited.right)
                                    .speed(0.005)
                                    .range(0.001..=0.999),
                            )
                            .changed();
                        if changed {
                            let before = self.project.clone();
                            let c = self.project.objects[self.selected]
                                .satin_column
                                .as_mut()
                                .unwrap();
                            c.rungs[index] = edited;
                            match engine::satin_stations(c, 0.4) {
                                Ok(_) => self.record_field_edit(before),
                                Err(error) => {
                                    self.project = before;
                                    self.result(Err(error));
                                }
                            }
                        }
                    }
                    if self.selected_rung.is_some()
                        && ui.button(language.text("Delete rung")).clicked()
                    {
                        self.delete_selection();
                    }
                    if ui.button(language.text("Split at rung / middle")).clicked() {
                        self.split_column();
                    }
                    let before = self.project.clone();
                    let mut changed = false;
                    if let Some(c) = &mut self.project.objects[self.selected].satin_column {
                        changed |= ui
                            .checkbox(
                                &mut c.first_right,
                                language.text("First puncture on right rail"),
                            )
                            .changed();
                        if c.closed {
                            ui.label(language.text("Start position around loop"));
                            changed |= ui
                                .add(egui::Slider::new(&mut c.seam, 0. ..=0.999))
                                .changed();
                        }
                    }
                    if changed {
                        self.project.version = 4;
                        self.record_field_edit(before);
                    }
                    if let Some(c) = self.project.objects[self.selected].satin_column.as_ref()
                        && let Err(error) = engine::satin_stations(c, 0.4)
                    {
                        ui.colored_label(
                            ui.visuals().error_fg_color,
                            language.message(&error).as_ref(),
                        );
                    }
                }
                if self.tool != Tool::Nodes && ui.button(language.text("Cancel · Escape")).clicked()
                {
                    self.cancel_draft();
                }
            });
        let paths = self
            .project
            .objects
            .get(self.selected)
            .map(|o| o.running_paths.clone())
            .unwrap_or_default();
        if !paths.is_empty() {
            ui.collapsing(language.text("Branch cleanup"),|ui|{
                ui.checkbox(&mut self.show_width,language.text("Show local width"));
                ui.small(language.text("Width is twice the nearest-boundary distance; it is an estimate, not a satin cross-section."));
                for (i,path) in paths.iter().enumerate() {
                    let len:f64=path.windows(2).map(|w|w[0].distance(w[1])).sum();
                    if ui.selectable_label(self.selected_branch==Some(i),format!("{} {} · {:.2} mm",language.text("Branch"),i+1,len)).clicked(){self.selected_branch=Some(i);self.selected_path=None;self.smoothing_preview=None;self.outline_preview=None;}
                }
                if let Some(index)=self.selected_branch {
                    if ui.button(language.text("Delete branch")).clicked(){self.edit_branch(|o|axis_edit::delete_branch(o,index));}
                    ui.label(language.text("Maximum smoothing error (mm)"));
                    if ui.add(egui::DragValue::new(&mut self.smooth_tolerance).speed(0.01).range(0.001..=2.)).changed(){self.smoothing_preview=None;}
                    if ui.button(language.text("Preview smoothing")).clicked(){
                        let mut candidate=self.project.objects[self.selected].clone();
                        match axis_edit::smooth_branch(&mut candidate,index,self.smooth_tolerance) {
                            Ok(())=>{self.smoothing_preview=Some(candidate);self.error=false;},Err(e)=>self.result(Err(e))
                        }
                    }
                    if self.smoothing_preview.is_some() && ui.button(language.text("Apply smoothing")).clicked()
                        && let Some(candidate)=self.smoothing_preview.take(){self.snapshot();self.project.objects[self.selected]=candidate;self.changed();}
                    if ui.button(language.text("Preview branch as satin")).clicked(){
                        match axis_edit::semi_auto_satin(&self.project.objects[self.selected],index){
                            Ok(columns)=>{self.outline_preview=columns.into_iter().next();self.preview_branch=Some(index);self.error=false;},Err(e)=>self.result(Err(e))
                        }
                    }
                    if self.outline_preview.is_some() && self.preview_branch==Some(index) && ui.button(language.text("Create satin copy")).clicked(){self.convert_branch(index);}
                }
            });
        }
    }

    pub(super) fn underlay_parameters(&mut self, ui: &mut egui::Ui) {
        if self.stitch_only || self.selected_locked() {
            return;
        }
        let language = self.language;
        let rail_satin = self
            .project
            .objects
            .get(self.selected)
            .is_some_and(|o| o.satin_column.is_some());
        let tatami = self
            .project
            .objects
            .get(self.selected)
            .is_some_and(|o| o.settings.kind == StitchKind::Tatami);
        if rail_satin || tatami {
            ui.collapsing(language.text("Additional underlay"), |ui| {
                let before = self.project.objects[self.selected].clone();
                let before_version = self.project.version;
                let mut changed = false;
                let s = &mut self.project.objects[self.selected].settings;
                changed |= ui
                    .checkbox(&mut s.edge_underlay, language.text("Edgewalk underlay"))
                    .changed();
                if rail_satin {
                    changed |= ui
                        .checkbox(&mut s.zigzag_underlay, language.text("Zigzag underlay"))
                        .changed();
                }
                if tatami {
                    let mut automatic = s.underlay_angle_deg.is_none();
                    if ui
                        .checkbox(
                            &mut automatic,
                            language.text("Underlay follows top angle + 90°"),
                        )
                        .changed()
                    {
                        s.underlay_angle_deg =
                            (!automatic).then_some((s.angle_deg + 90.).rem_euclid(180.));
                        changed = true;
                    }
                    if let Some(angle) = &mut s.underlay_angle_deg {
                        ui.label(language.text("Underlay angle (°)"));
                        changed |= ui
                            .add(egui::DragValue::new(angle).speed(1.).range(-180. ..=180.))
                            .changed();
                    }
                }
                ui.label(language.text("Underlay inset (mm)"));
                changed |= ui
                    .add(
                        egui::DragValue::new(&mut s.underlay_inset_mm)
                            .speed(0.05)
                            .range(0. ..=5.),
                    )
                    .changed();
                ui.label(language.text("Underlay spacing (mm)"));
                changed |= ui
                    .add(
                        egui::DragValue::new(&mut s.underlay_spacing_mm)
                            .speed(0.1)
                            .range(0.3..=20.),
                    )
                    .changed();
                ui.label(language.text("Underlay stitch length (mm)"));
                changed |= ui
                    .add(
                        egui::DragValue::new(&mut s.underlay_length_mm)
                            .speed(0.1)
                            .range(0.3..=12.),
                    )
                    .changed();
                if changed {
                    self.project.version = self
                        .project
                        .version
                        .max(if s.underlay_angle_deg.is_some() { 5 } else { 4 });
                    self.record_object_field_edit(before, before_version);
                }
            });
        }
    }

    pub(super) fn quality_controls(&mut self, ui: &mut egui::Ui) {
        let language = self.language;
        ui.checkbox(
            &mut self.show_quality,
            language.text("Highlight stitch warnings"),
        );
        ui.checkbox(
            &mut self.show_segment_density,
            language.text("Thread segment density"),
        );
        if !self.stale
            && let Some(report) = &self.segment_density
        {
            let max = report
                .cells
                .iter()
                .map(|c| c.total_mm / (report.grid_mm * report.grid_mm))
                .fold(0., f64::max);
            ui.small(format!(
                "{}: {:.2} mm/mm²",
                language.text("Maximum segment length density"),
                max
            ));
            if !report.complete {
                ui.small(language.text("Segment density is partial: work limit reached."));
            }
        }
        if !self.stale {
            if let Some(summary) = &self.plan_summary {
                ui.small(format!(
                    "{}: {:.1} mm · {}: {:.1} mm",
                    language.text("Top fill"),
                    summary.top_mm,
                    language.text("Underlay"),
                    summary.underlay_mm
                ));
                ui.small(format!(
                    "{}: {:.1} mm · Jump: {:.1} mm · Trim: {}",
                    language.text("Sewn travel"),
                    summary.sewn_travel_mm,
                    summary.jump_mm,
                    summary.trim_count
                ));
            }
            for estimate in self.coverage.iter().take(30) {
                if estimate.samples > 0 {
                    ui.small(format!(
                        "#{} · {}: {:.1}% ({} {:.2} mm)",
                        estimate.object_id,
                        language.text("Top thread coverage estimate"),
                        100. * (estimate.samples - estimate.uncovered_samples) as f64
                            / estimate.samples as f64,
                        language.text("thread width"),
                        estimate.effective_thread_width_mm
                    ));
                }
            }
            let warnings: Vec<_> = self
                .plan_warnings
                .iter()
                .filter(|w| w.point.is_none())
                .chain(self.plan_warnings.iter().filter(|w| w.point.is_some()))
                .take(100)
                .cloned()
                .collect();
            if !warnings.is_empty() {
                ui.small(format!(
                    "{}: {} ({}: {})",
                    language.text("Plan warnings"),
                    self.plan_warnings.len(),
                    language.text("shown"),
                    warnings.len()
                ));
            }
            egui::ScrollArea::vertical()
                .id_salt("plan-warnings")
                .max_height(160.)
                .show(ui, |ui| {
                    for warning in &warnings {
                        if ui
                            .add(
                                egui::Button::new(
                                    egui::RichText::new(format!(
                                        "#{} · {}",
                                        warning.object_id,
                                        language.message(&warning.message)
                                    ))
                                    .small(),
                                )
                                .wrap(),
                            )
                            .clicked()
                        {
                            if let Some(index) = self
                                .project
                                .objects
                                .iter()
                                .position(|o| o.id == warning.object_id)
                            {
                                self.selected = index;
                                self.clear_selection();
                            }
                            self.show_quality = true;
                            if let Some(point) = warning.point {
                                self.pan = Vec2::new(
                                    -point.x as f32 * self.scale,
                                    -point.y as f32 * self.scale,
                                );
                                self.fit = false;
                            }
                        }
                    }
                });
        }

        ui.small(language.text("Heuristic checks: stitches below 0.3 mm, jumps above 10 mm, more than 20 punctures per 1 mm² cell."));
        if self.stale {
            ui.label(language.text("Generate stitches to refresh quality checks."));
        } else {
            let total = self.quality_issues.len();
            let issues: Vec<_> = self.quality_issues.iter().take(100).cloned().collect();
            ui.label(format!("{}: {}", language.text("Warnings"), total));
            egui::ScrollArea::vertical()
                .max_height(130.)
                .show(ui, |ui| {
                    for issue in issues.iter().take(100) {
                        let label = match issue.kind {
                            embroidery_lab::quality::QualityKind::ShortStitch => "Short stitch",
                            embroidery_lab::quality::QualityKind::LongJump => "Long jump",
                            embroidery_lab::quality::QualityKind::DenseArea => "Dense area",
                            embroidery_lab::quality::QualityKind::LongStitch => "Long stitch",
                            embroidery_lab::quality::QualityKind::EmptyObject => {
                                "Object has no top stitches"
                            }
                            embroidery_lab::quality::QualityKind::OutsideGeometry => {
                                "Stitch crosses protected geometry"
                            }
                            embroidery_lab::quality::QualityKind::UncoveredArea => {
                                "Area has insufficient top coverage"
                            }
                            embroidery_lab::quality::QualityKind::UnsupportedSatin => {
                                "Satin requires valid rails"
                            }
                            embroidery_lab::quality::QualityKind::SkippedUnderlay => {
                                "Underlay omitted"
                            }
                            embroidery_lab::quality::QualityKind::AnalysisLimit => {
                                "Diagnostic work limit reached"
                            }
                            embroidery_lab::quality::QualityKind::MachineLimit => {
                                "Machine limit exceeded"
                            }
                            embroidery_lab::quality::QualityKind::SmallFeature => {
                                "Detail below machine resolution"
                            }
                            embroidery_lab::quality::QualityKind::RepeatedTopFill => {
                                "Repeated top-fill segment"
                            }
                            embroidery_lab::quality::QualityKind::LayerOcclusion => {
                                "Hidden lower fill removed"
                            }
                            embroidery_lab::quality::QualityKind::TravelRouting => {
                                "Travel redirected to jump"
                            }
                        };
                        if ui
                            .small_button(format!("{} · {}", language.text(label), issue.index + 1))
                            .clicked()
                        {
                            if let Some(index) = self
                                .project
                                .objects
                                .iter()
                                .position(|o| o.id == issue.object_id)
                            {
                                self.selected = index;
                                self.clear_selection();
                            }
                            self.progress = issue.index + 1;
                            self.show_quality = true;
                            self.pan = Vec2::new(
                                -issue.point.x as f32 * self.scale,
                                -issue.point.y as f32 * self.scale,
                            );
                            self.fit = false;
                        }
                    }
                });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn locked_geometry_handlers_cannot_replace_split_or_hide_the_source() {
        let column = SatinColumn {
            left: vec![Point::new(0., 0.), Point::new(0., 20.)],
            right: vec![Point::new(4., 0.), Point::new(4., 20.)],
            ..Default::default()
        };
        let mut app = Editor::workspace();
        app.project = Project {
            version: 4,
            objects: vec![Object {
                id: 42,
                satin_column: Some(column.clone()),
                settings: Settings {
                    kind: StitchKind::Satin,
                    ..Default::default()
                },
                ..Default::default()
            }],
            ..Default::default()
        };
        app.selected = 0;
        app.locked_objects.insert(42);
        app.draft = vec![Point::new(-1., 10.), Point::new(5., 10.)];
        let before = app.project.clone();
        let mut branch_action_ran = false;
        app.edit_branch(|object| {
            branch_action_ran = true;
            object.color = [255, 0, 0];
            Ok(())
        });
        app.split_column();
        app.convert_branch(0);
        app.complete_region_split();
        assert!(app.commit_column(column, true).is_err());
        assert!(!branch_action_ran);
        assert!(project_equal(&app.project, &before));
        assert!(app.undo.is_empty() && !app.dirty && !app.error);
        assert_eq!(app.draft.len(), 2, "A blocked edit preserves its draft");

        app.locked_objects.remove(&42);
        app.split_column();
        assert!(!app.error, "{}", app.message);
        assert_eq!(app.project.objects.len(), 2);
        assert_eq!(app.undo.len(), 1);
        app.undo_edit();
        assert!(project_equal(&app.project, &before));
    }
    #[test]
    fn converting_a_branch_with_full_history_is_one_atomic_edit() {
        let mut app = Editor::workspace();
        let area = Object {
            id: 10,
            visible: false,
            contours: vec![vec![
                Point::new(0., 0.),
                Point::new(30., 0.),
                Point::new(30., 4.),
                Point::new(0., 4.),
            ]],
            ..Default::default()
        };
        let paths = vec![vec![Point::new(3., 2.), Point::new(27., 2.)]];
        let graph = axis_edit::attach(&paths, &area, 0.2).unwrap();
        app.project = Project {
            version: 4,
            objects: vec![
                area,
                Object {
                    id: 11,
                    running_paths: paths,
                    centerline_graph: Some(graph),
                    settings: Settings {
                        kind: StitchKind::Running,
                        underlay: false,
                        tie_in: false,
                        tie_off: false,
                        ..Default::default()
                    },
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        app.selected = 1;
        for _ in 0..30 {
            app.snapshot();
        }
        let before = app.project.clone();
        app.convert_branch(0);
        assert!(!app.error, "{}", app.message);
        assert_eq!(app.undo.len(), 30);
        assert!(!app.project.objects[1].visible);
        assert_eq!(
            app.project
                .objects
                .iter()
                .filter(|o| o.visible && o.satin_column.is_some())
                .count(),
            1
        );
        assert!(
            app.project
                .objects
                .iter()
                .filter(|o| o.visible)
                .all(|o| o.running_paths.is_empty())
        );
        app.undo_edit();
        assert!(project_equal(&app.project, &before));
        app.redo_edit();
        assert!(app.project.objects.last().unwrap().satin_column.is_some());
    }
    #[test]
    fn committed_edits_invalidate_geometry_previews() {
        let mut app = Editor::workspace();
        app.outline_preview = Some(SatinColumn::default());
        app.smoothing_preview = Some(Object::default());
        app.changed();
        assert!(app.outline_preview.is_none());
        assert!(app.smoothing_preview.is_none());
    }
    #[test]
    fn history_coalesces_field_changes_and_clears_redo() {
        let mut app = Editor::workspace();
        let original = app.project.clone();
        for n in 0..5 {
            let before = app.project.clone();
            app.project.width_mm = 101. + n as f64;
            app.record_field_edit(before);
        }
        app.finish_field_edit();
        assert_eq!(app.undo.len(), 1);
        app.undo_edit();
        assert!(project_equal(&app.project, &original));
        assert!(app.stale);
        app.redo_edit();
        assert_eq!(app.project.width_mm, 105.);
        app.undo_edit();
        app.snapshot();
        app.project.width_mm = 200.;
        app.changed();
        assert!(app.redo.is_empty());
    }
    #[test]
    fn invalid_satin_draft_never_modifies_document_or_history() {
        let mut app = Editor::workspace();
        let before = app.project.clone();
        app.tool = Tool::Satin;
        app.draft = vec![Point::new(0., 0.), Point::new(10., 0.)];
        app.complete_satin();
        app.draft = vec![Point::new(10., 0.), Point::new(0., 0.)];
        app.complete_satin();
        assert!(app.error);
        assert!(app.undo.is_empty());
        assert!(project_equal(&before, &app.project));
        app.cancel_draft();
        assert!(app.draft.is_empty());
        assert!(app.satin_first.is_empty());
    }
    #[test]
    fn satin_draft_commits_once_and_roundtrips_history() {
        let mut app = Editor::workspace();
        let before = app.project.clone();
        app.tool = Tool::Satin;
        app.draft = vec![Point::new(0., 0.), Point::new(20., 0.)];
        app.complete_satin();
        app.draft = vec![Point::new(0., 3.), Point::new(20., 3.)];
        app.complete_satin();
        assert!(!app.error, "{}", app.message);
        assert_eq!(app.undo.len(), 1);
        assert_eq!(app.project.version, 4);
        app.undo_edit();
        assert!(project_equal(&app.project, &before));
        app.redo_edit();
        assert!(app.project.objects.last().unwrap().satin_column.is_some());
    }
}
