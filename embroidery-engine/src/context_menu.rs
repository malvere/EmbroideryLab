use super::*;

impl Editor {
    pub(super) fn select_object(&mut self, index: usize) {
        if index >= self.project.objects.len() {
            return;
        }
        self.finish_field_edit();
        self.selected = index;
        self.selection_anchor = Some(self.project.objects[index].id);
        self.clear_selection();
        self.outline_preview = None;
        self.preview_branch = None;
    }

    pub(super) fn select_context_at(&mut self, point: Point) {
        if self.stitch_only || self.tool != Tool::Nodes {
            return;
        }
        let tolerance = 12. / f64::from(self.scale);
        let Some(index) = context_hit::object_at(&self.project, point, tolerance) else {
            return;
        };
        if !self
            .merge_selection
            .contains(&self.project.objects[index].id)
        {
            self.merge_selection.clear();
        }
        self.select_object(index);
        let object = &self.project.objects[index];
        self.selected_node = object
            .contours
            .iter()
            .enumerate()
            .flat_map(|(r, ring)| {
                ring.iter()
                    .enumerate()
                    .map(move |(n, p)| (r, n, p.distance(point)))
            })
            .filter(|(_, _, d)| *d <= tolerance)
            .min_by(|a, b| a.2.total_cmp(&b.2))
            .map(|(r, n, _)| (r, n));
        self.selected_rail = object.satin_column.as_ref().and_then(|column| {
            [(false, &column.left), (true, &column.right)]
                .into_iter()
                .flat_map(|(right, rail)| {
                    rail.iter()
                        .enumerate()
                        .map(move |(n, p)| (right, n, p.distance(point)))
                })
                .filter(|(_, _, d)| *d <= tolerance)
                .min_by(|a, b| a.2.total_cmp(&b.2))
                .map(|(right, n, _)| (right, n))
        });
        self.selected_path = object
            .running_paths
            .iter()
            .enumerate()
            .flat_map(|(r, path)| {
                path.iter()
                    .enumerate()
                    .map(move |(n, p)| (r, n, p.distance(point)))
            })
            .filter(|(_, _, d)| *d <= tolerance)
            .min_by(|a, b| a.2.total_cmp(&b.2))
            .map(|(r, n, _)| (r, n));
        self.selected_branch = object
            .running_paths
            .iter()
            .enumerate()
            .filter_map(|(i, path)| {
                editor_tools::nearest_segment(path, point).map(|(_, _, d)| (i, d))
            })
            .filter(|(_, d)| *d <= tolerance)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| i);
    }

    fn reverse_selected(&mut self) {
        if self.stitch_only || self.selected >= self.project.objects.len() {
            return;
        }
        let mut object = self.project.objects[self.selected].clone();
        for ring in &mut object.contours {
            ring.reverse();
        }
        object.bezier_segments.reverse();
        for segment in &mut object.bezier_segments {
            std::mem::swap(&mut segment.start, &mut segment.end);
            std::mem::swap(&mut segment.control1, &mut segment.control2);
        }
        if let Some(graph) = &mut object.centerline_graph {
            for branch in &mut graph.branches {
                branch.points.reverse();
                std::mem::swap(&mut branch.start_vertex, &mut branch.end_vertex);
            }
            embroidery_lab::axis_edit::sync_paths(&mut object);
        } else {
            for path in &mut object.running_paths {
                path.reverse();
            }
        }
        if let Some(column) = &mut object.satin_column
            && let Err(error) = embroidery_lab::satin_edit::reverse(column)
        {
            self.result(Err(error));
            return;
        }
        self.snapshot();
        self.project.objects[self.selected] = object;
        self.clear_selection();
        self.changed();
    }

    pub(super) fn move_selected(&mut self, later: bool) {
        if self.stitch_only || self.selected >= self.project.objects.len() {
            return;
        }
        let target = if later {
            self.selected.checked_add(1)
        } else {
            self.selected.checked_sub(1)
        };
        let Some(target) = target.filter(|i| *i < self.project.objects.len()) else {
            return;
        };
        self.snapshot();
        self.project.objects.swap(self.selected, target);
        self.selected = target;
        self.clear_selection();
        self.changed();
    }

    pub(super) fn remove_selected(&mut self) {
        if self.stitch_only || self.selected >= self.project.objects.len() {
            return;
        }
        self.snapshot();
        let object = self.project.objects.remove(self.selected);
        self.merge_selection.retain(|id| *id != object.id);
        self.selected = self
            .selected
            .min(self.project.objects.len().saturating_sub(1));
        self.clear_selection();
        self.cancel_draft();
        self.changed();
    }

    pub(super) fn object_context_menu(&mut self, response: &egui::Response) {
        if response.secondary_clicked() {
            self.context_popup = response
                .interact_pointer_pos()
                .map(|pos| (response.id, pos));
            egui::Popup::close_all(&response.ctx);
        }
        let Some((id, anchor)) = self.context_popup else {
            return;
        };
        if id != response.id {
            return;
        }
        let mut open = true;
        // Color and type choosers use egui's memory popup slot. Keep the
        // surrounding context menu independent of that slot while they open.
        let close_behavior = if egui::Popup::is_any_open(&response.ctx) {
            egui::PopupCloseBehavior::IgnoreClicks
        } else {
            egui::PopupCloseBehavior::CloseOnClickOutside
        };
        let language = self.language;
        egui::Popup::context_menu(response)
            .open_bool(&mut open)
            .at_position(anchor)
            .close_behavior(close_behavior)
            .show(|ui| {
                ui.set_min_width(195.);
                if let Some(object) = self.project.objects.get(self.selected) {
                    ui.label(egui::RichText::new(&object.name).strong());
                    ui.separator();
                }
                let editable = !self.stitch_only && self.selected < self.project.objects.len();
                let config =
                    egui::containers::menu::MenuConfig::new().close_behavior(close_behavior);
                ui.add_enabled_ui(editable, |ui| {
                    egui::containers::menu::SubMenuButton::new(language.text("Parameters"))
                        .config(config.clone())
                        .ui(ui, |ui| {
                            ui.set_style(ui.ctx().style());
                            ui.set_width(275.);
                            egui::ScrollArea::vertical()
                                .id_salt("context-parameters")
                                .max_height(
                                    (ui.ctx().content_rect().height() - 100.).clamp(160., 480.),
                                )
                                .show(ui, |ui| self.object_parameters(ui));
                        });
                    egui::containers::menu::SubMenuButton::new(language.text("Editing"))
                        .config(config)
                        .ui(ui, |ui| {
                            ui.set_width(275.);
                            egui::ScrollArea::vertical()
                                .id_salt("context-editing")
                                .max_height(
                                    (ui.ctx().content_rect().height() - 100.).clamp(160., 480.),
                                )
                                .show(ui, |ui| self.context_editing(ui));
                        });
                });
                if self.stitch_only {
                    ui.small(language.text("Machine preview has no editable objects."));
                }
                ui.separator();
                if ui
                    .add_enabled(
                        !self.stitch_only && (!self.undo.is_empty() || self.field_before.is_some()),
                        egui::Button::new(language.text("Undo")).shortcut_text("Ctrl+Z"),
                    )
                    .clicked()
                {
                    self.undo_edit();
                    ui.close();
                }
                if ui
                    .add_enabled(
                        !self.stitch_only && !self.redo.is_empty(),
                        egui::Button::new(language.text("Redo")).shortcut_text("Ctrl+Y"),
                    )
                    .clicked()
                {
                    self.redo_edit();
                    ui.close();
                }
                if ui.button(language.text("Fit design")).clicked() {
                    self.fit = true;
                    ui.close();
                }
                if self.tool != Tool::Nodes && ui.button(language.text("Cancel · Escape")).clicked()
                {
                    self.cancel_draft();
                    ui.close();
                }
            });
        if !open {
            self.context_popup = None;
            egui::Popup::close_all(&response.ctx);
            self.finish_field_edit();
        }
    }

    fn context_editing(&mut self, ui: &mut egui::Ui) {
        let language = self.language;
        if ui.button(language.text("Edit nodes")).clicked() {
            self.cancel_draft();
            self.show_nodes = true;
            ui.close();
        }
        let deletion = if self.selected_rung.is_some() {
            Some("Delete rung")
        } else if self.selected_rail.is_some() {
            Some("Delete node")
        } else if self.selected_branch.is_some()
            && self.project.objects[self.selected]
                .centerline_graph
                .is_some()
        {
            Some("Delete branch")
        } else if self.selected_node.is_some_and(|(r, _)| {
            self.project.objects[self.selected]
                .contours
                .get(r)
                .is_some_and(|ring| ring.len() > 3)
        }) {
            Some("Delete node")
        } else {
            None
        };
        if let Some(label) = deletion
            && ui.button(language.text(label)).clicked()
        {
            self.delete_selection();
            ui.close();
        }
        if let Some((right, n)) = self.selected_rail
            && ui.button(language.text("Insert node after")).clicked()
        {
            self.edit_column(|c| {
                embroidery_lab::satin_edit::insert_node(c, right, n, 0.5).map(|_| ())
            });
            ui.close();
        }
        if ui.button(language.text("Reverse direction")).clicked() {
            self.reverse_selected();
            ui.close();
        }
        ui.separator();
        if ui
            .add_enabled(
                self.selected > 0,
                egui::Button::new(language.text("Earlier")),
            )
            .clicked()
        {
            self.move_selected(false);
            ui.close();
        }
        if ui
            .add_enabled(
                self.selected + 1 < self.project.objects.len(),
                egui::Button::new(language.text("Later")),
            )
            .clicked()
        {
            self.move_selected(true);
            ui.close();
        }
        if ui.button(language.text("Remove object")).clicked() {
            self.remove_selected();
            ui.close();
            return;
        }
        self.geometry_tools(ui);
    }

    pub(super) fn object_parameters(&mut self, ui: &mut egui::Ui) {
        if self.stitch_only {
            ui.label(
                self.language
                    .text("Machine preview has no editable objects."),
            );
            return;
        }
        let language = self.language;
        let before = self.project.clone();
        let mut changed = false;
        let mut reverse = false;
        if let Some(o) = self.project.objects.get_mut(self.selected) {
            changed |= ui.text_edit_singleline(&mut o.name).changed();
            ui.horizontal(|ui| {
                ui.label(language.text("Thread"));
                changed |= ui.color_edit_button_srgb(&mut o.color).changed();
            });
            ui.separator();
            let open_paths = !o.running_paths.is_empty();
            ui.add_enabled_ui(!open_paths, |ui| {
                egui::ComboBox::from_label(language.text("Type"))
                    .selected_text(stitch_kind_label(language, o.settings.kind))
                    .show_ui(ui, |ui| {
                        for k in [StitchKind::Running, StitchKind::Satin, StitchKind::Tatami] {
                            changed |= ui
                                .selectable_value(
                                    &mut o.settings.kind,
                                    k,
                                    stitch_kind_label(language, k),
                                )
                                .changed();
                        }
                    });
            });
            if open_paths {
                ui.small(language.text(
                    "Running paths. Branches are sewn separately; closed loops stay closed.",
                ));
            }
            let rail_satin = o.satin_column.is_some() && o.settings.kind == StitchKind::Satin;
            if !open_paths {
                ui.label(if rail_satin {
                    language.text("Rail puncture spacing (mm)")
                } else {
                    language.text("Row spacing (mm)")
                });
                changed |= ui
                    .add(
                        egui::DragValue::new(&mut o.settings.spacing_mm)
                            .speed(0.01)
                            .range(0.15..=5.)
                            .fixed_decimals(2),
                    )
                    .changed();
                ui.small(format!(
                    "{:.2} {} / {}",
                    1. / o.settings.spacing_mm,
                    if rail_satin {
                        language.text("rail punctures")
                    } else {
                        language.text("rows")
                    },
                    language.text("mm")
                ));
            }
            ui.label(language.text("Max stitch length (mm)"));
            changed |= ui
                .add(
                    egui::DragValue::new(&mut o.settings.length_mm)
                        .speed(0.1)
                        .range(0.5..=12.)
                        .fixed_decimals(1),
                )
                .changed();
            if rail_satin {
                ui.small(
                    language.text(
                        "Direction follows the two rails. Drag rail nodes to shape the column.",
                    ),
                );
            } else if !open_paths {
                ui.label(language.text("Angle (degrees)"));
                changed |= ui
                    .add(egui::Slider::new(&mut o.settings.angle_deg, 0. ..=180.))
                    .changed();
                changed |= ui
                    .checkbox(
                        &mut o.settings.auto_angle,
                        language.text("Automatic direction"),
                    )
                    .changed();
            }
            ui.separator();
            if !open_paths {
                changed |= ui
                    .checkbox(
                        &mut o.settings.underlay,
                        if rail_satin {
                            language.text("Centerwalk underlay")
                        } else {
                            language.text("Sparse underlay")
                        },
                    )
                    .changed();
                ui.label(language.text("Pull compensation (mm)"));
                changed |= ui
                    .add(
                        egui::DragValue::new(&mut o.settings.pull_comp_mm)
                            .speed(0.01)
                            .range(0. ..=1.)
                            .fixed_decimals(2),
                    )
                    .changed();
            }
            changed |= ui
                .checkbox(&mut o.settings.tie_in, language.text("Tie-in"))
                .changed();
            changed |= ui
                .checkbox(&mut o.settings.tie_off, language.text("Tie-off"))
                .changed();
            ui.separator();
            ui.label(if open_paths {
                language.text("Running path direction")
            } else if rail_satin {
                language.text("Satin direction")
            } else {
                language.text("Start / end contour")
            });
            if !open_paths
                && !rail_satin
                && ui
                    .button(language.text("Move start to selected node"))
                    .clicked()
                && let Some((r, n)) = self.selected_node
                && let Some(ring) = o.contours.get_mut(r)
            {
                ring.rotate_left(n);
                self.selected_node = Some((r, 0));
                changed = true;
            }
            if ui.button(language.text("Reverse direction")).clicked() {
                reverse = true;
            }
            if let Some((r, n)) = self.selected_node
                && let Some(p) = o.contours.get_mut(r).and_then(|ring| ring.get_mut(n))
            {
                ui.label(language.text("Selected node (mm)"));
                ui.horizontal(|ui| {
                    changed |= ui
                        .add(egui::DragValue::new(&mut p.x).prefix("X ").speed(0.1))
                        .changed();
                    changed |= ui
                        .add(egui::DragValue::new(&mut p.y).prefix("Y ").speed(0.1))
                        .changed();
                });
            }
            ui.small(if open_paths {language.text("Drag path nodes on the canvas. Changes remain editable in ELAB.")}else if rail_satin{language.text("Drag rail nodes or handles. Double-click a rail to insert a node; Delete removes the selected node.")}else{language.text("Drag square nodes or Bezier handles on the canvas. Delete removes a selected polygon node.")});
            if let Some((r, n)) = self.selected_path
                && let Some(path) = o.running_paths.get(r)
                && let Some(point) = path.get(n)
            {
                let mut point = *point;
                ui.label(language.text("Selected path node (mm)"));
                let mut edited = false;
                ui.horizontal(|ui| {
                    edited |= ui
                        .add(egui::DragValue::new(&mut point.x).prefix("X ").speed(0.1))
                        .changed();
                    edited |= ui
                        .add(egui::DragValue::new(&mut point.y).prefix("Y ").speed(0.1))
                        .changed();
                });
                if edited {
                    match set_path_node(o, r, n, point) {
                        Ok(()) => changed = true,
                        Err(e) => {
                            self.message = e;
                            self.error = true;
                        }
                    }
                }
            }
            if let Some((right, n)) = self.selected_rail
                && let Some(column) = &mut o.satin_column
            {
                let rail = if right { &column.right } else { &column.left };
                if let Some(point) = rail.get(n) {
                    let mut point = *point;
                    let mut edited = false;
                    ui.label(language.text("Selected rail node (mm)"));
                    ui.horizontal(|ui| {
                        edited |= ui
                            .add(egui::DragValue::new(&mut point.x).prefix("X ").speed(0.1))
                            .changed();
                        edited |= ui
                            .add(egui::DragValue::new(&mut point.y).prefix("Y ").speed(0.1))
                            .changed();
                    });
                    if edited {
                        match embroidery_lab::satin_edit::set_node(column, right, n, point) {
                            Ok(()) => {
                                changed = true;
                                self.project.version = 4;
                            }
                            Err(e) => {
                                self.message = e;
                                self.error = true;
                            }
                        }
                    }
                }
            }
        } else {
            ui.label(language.text("Select or create an object."));
        }
        if changed {
            self.record_field_edit(before);
        }
        if reverse {
            self.reverse_selected();
        }
    }
}

#[cfg(test)]
#[path = "context_menu_tests.rs"]
mod tests;
