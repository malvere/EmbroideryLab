use super::*;

#[derive(Default)]
pub(super) struct Marquee {
    drag: Option<Drag>,
    frame: Option<u64>,
    suppress_menu: bool,
    cancelled: bool,
}

struct Drag {
    start: Pos2,
    end: Pos2,
    center: Pos2,
    scale: f32,
    additive: bool,
    moved: bool,
}

impl Drag {
    fn update(&mut self, position: Pos2, bounds: Rect) {
        self.moved |= position.distance(self.start) >= 6.;
        self.end = bounds.clamp(position);
    }

    fn world(&self, position: Pos2) -> Point {
        Point::new(
            f64::from((position.x - self.center.x) / self.scale),
            f64::from((position.y - self.center.y) / self.scale),
        )
    }
}

impl Editor {
    /// Ordered events also handle a complete press/drag/release in one frame.
    pub(super) fn update_marquee(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        center: Pos2,
        scale: f32,
    ) -> bool {
        let frame = ui.ctx().cumulative_frame_nr();
        if self.marquee.frame == Some(frame) {
            return self.marquee.suppress_menu;
        }
        self.marquee.frame = Some(frame);
        self.marquee.suppress_menu = false;
        let (events, focused, escape) = ui.input(|input| {
            (
                input.events.clone(),
                input.raw.focused,
                input.key_pressed(egui::Key::Escape),
            )
        });
        let allowed = !self.stitch_only
            && !self.show_3d
            && self.tool == Tool::Nodes
            && self.draft.is_empty()
            && self.satin_first.is_empty()
            && self.rung_start.is_none()
            && self.outline_preview.is_none()
            && self.smoothing_preview.is_none()
            && self.centerline_job.is_none()
            && ui.is_enabled();
        if !allowed || !focused || escape {
            self.marquee.cancelled |= self.marquee.drag.take().is_some();
            self.marquee.suppress_menu = self.marquee.cancelled;
            return self.marquee.suppress_menu;
        }
        for event in events {
            match event {
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Secondary,
                    pressed: true,
                    modifiers,
                } => {
                    self.marquee.cancelled = false;
                    let unobstructed = ui
                        .ctx()
                        .layer_id_at(pos)
                        .is_none_or(|layer| layer == response.layer_id);
                    if response.rect.contains(pos) && unobstructed {
                        self.marquee.drag = Some(Drag {
                            start: pos,
                            end: pos,
                            center,
                            scale,
                            additive: modifiers.ctrl || modifiers.shift,
                            moved: false,
                        });
                    }
                }
                egui::Event::PointerMoved(pos) => {
                    if let Some(drag) = &mut self.marquee.drag {
                        drag.update(pos, response.rect);
                        self.marquee.suppress_menu |= drag.moved;
                    }
                }
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Secondary,
                    pressed: false,
                    ..
                } => {
                    if self.marquee.cancelled {
                        self.marquee.cancelled = false;
                        self.marquee.suppress_menu = true;
                        continue;
                    }
                    if let Some(mut drag) = self.marquee.drag.take() {
                        drag.update(pos, response.rect);
                        if drag.moved {
                            self.marquee.suppress_menu = true;
                            self.select_marquee(
                                drag.world(drag.start),
                                drag.world(drag.end),
                                drag.additive,
                            );
                            self.context_popup = None;
                            egui::Popup::close_all(ui.ctx());
                            ui.ctx().request_repaint();
                        }
                    }
                }
                egui::Event::PointerGone => {
                    self.marquee.cancelled |= self.marquee.drag.take().is_some();
                    self.marquee.suppress_menu |= self.marquee.cancelled;
                }
                _ => {}
            }
        }
        if self.marquee.drag.as_ref().is_some_and(|drag| drag.moved) {
            self.context_popup = None;
            egui::Popup::close_all(ui.ctx());
            self.marquee.suppress_menu = true;
        }
        self.marquee.suppress_menu
    }

    fn select_marquee(&mut self, a: Point, b: Point, additive: bool) {
        let indices = marquee_hit::objects_in_rect(&self.project, a, b);
        self.finish_field_edit();
        self.clear_selection();
        self.outline_preview = None;
        self.preview_branch = None;
        if !additive {
            self.merge_selection.clear();
        } else if self.merge_selection.is_empty()
            && let Some(object) = self.project.objects.get(self.selected)
            && object.visible
        {
            self.merge_selection.push(object.id);
        }
        for index in &indices {
            let id = self.project.objects[*index].id;
            if !self.merge_selection.contains(&id) {
                self.merge_selection.push(id);
            }
        }
        if !indices.contains(&self.selected) {
            if let Some(index) = indices.last() {
                self.selected = *index;
            } else if !additive {
                // The existing inspector uses get(selected) for its empty state.
                self.selected = self.project.objects.len();
            }
        }
        self.selection_anchor = self
            .project
            .objects
            .get(self.selected)
            .map(|object| object.id);
    }

    pub(super) fn paint_marquee(&self, painter: &egui::Painter) {
        if let Some(drag) = &self.marquee.drag
            && drag.moved
        {
            let rect = Rect::from_two_pos(drag.start, drag.end);
            let color = Color32::from_rgb(34, 104, 162);
            painter.rect_filled(rect, 0., Color32::from_rgba_unmultiplied(34, 104, 162, 28));
            painter.rect_stroke(
                rect,
                0.,
                Stroke::new(1.5_f32, color),
                egui::StrokeKind::Inside,
            );
        }
    }

    pub(super) fn paint_selected_areas(&self, painter: &egui::Painter, center: Pos2, scale: f32) {
        let screen = |q: Point| center + Vec2::new(q.x as f32 * scale, q.y as f32 * scale);
        let stroke = Stroke::new(1.8_f32, Color32::from_rgb(34, 104, 162));
        for object in &self.project.objects {
            if !object.visible || !self.merge_selection.contains(&object.id) {
                continue;
            }
            for ring in object.flattened_contours() {
                for (&a, &b) in ring
                    .iter()
                    .zip(ring.iter().cycle().skip(1))
                    .take(ring.len())
                {
                    painter.line_segment([screen(a), screen(b)], stroke);
                }
            }
            for path in &object.running_paths {
                for pair in path.windows(2) {
                    painter.line_segment([screen(pair[0]), screen(pair[1])], stroke);
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "marquee_tests.rs"]
mod tests;
