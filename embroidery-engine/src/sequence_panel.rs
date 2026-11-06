//! Native workstation panels; document and stitch behavior remain shared.
use super::*;
use ui_theme::{Icon, icon_button};

impl Editor {
    pub(super) fn workspace_sequence(&mut self, ctx: &egui::Context) {
        let language = self.language;
        let colors = self.workspace.theme.tokens();
        let panel = egui::TopBottomPanel::bottom(if self.workspace.timeline_open {
            "sequence"
        } else {
            "sequence-collapsed"
        })
        .resizable(self.workspace.timeline_open)
        .default_height(if self.workspace.timeline_open {
            self.workspace.timeline_height
        } else {
            58.
        })
        .height_range(if self.workspace.timeline_open {
            88. ..=240.
        } else {
            58. ..=58.
        })
        .frame(
            egui::Frame::new()
                .fill(colors.surface)
                .inner_margin(egui::Margin::symmetric(12, 6)),
        )
        .show(ctx, |ui| {
            if self.decision_pending() {
                ui.disable();
            }
            ui.horizontal(|ui| {
                if icon_button(
                    ui,
                    Icon::Chevron,
                    self.workspace.timeline_open,
                    true,
                    language.text("Stitch inspection"),
                )
                .clicked()
                {
                    self.workspace.timeline_open = !self.workspace.timeline_open;
                }
                if icon_button(
                    ui,
                    if self.playing {
                        Icon::Pause
                    } else {
                        Icon::Play
                    },
                    self.playing,
                    !self.stale && !self.stitches.is_empty(),
                    language.text(if self.playing { "Pause" } else { "Simulate" }),
                )
                .clicked()
                {
                    if self.progress >= self.stitches.len() {
                        self.progress = 0;
                    }
                    self.playing = !self.playing;
                }
                if icon_button(
                    ui,
                    Icon::Stop,
                    false,
                    !self.stale && !self.stitches.is_empty(),
                    language.text("Reset simulation"),
                )
                .clicked()
                {
                    self.playing = false;
                    self.progress = 0;
                }
                ui.label(self.stitch_summary_text()).on_hover_text(
                    language.text("Width × height of sewn stitches. Travel jumps are excluded."),
                );
                if self.stale {
                    ui.label(
                        egui::RichText::new(language.text("Preview needs regeneration"))
                            .color(colors.warning),
                    );
                }
                let warning_count = self.quality_issues.len() + self.plan_warnings.len();
                if !self.stale
                    && warning_count > 0
                    && ui
                        .button(format!("{} · {}", language.text("Warnings"), warning_count))
                        .clicked()
                {
                    self.quality_window = true;
                }
                if !self.workspace.timeline_open && self.error {
                    ui.label(
                        egui::RichText::new(language.text("Error — expand for details"))
                            .color(colors.error),
                    );
                }
            });
            if self.workspace.timeline_open {
                ui.horizontal(|ui| {
                    ui.add_enabled_ui(!self.stale, |ui| {
                        if !self.stitches.is_empty() {
                            self.paint_sequence(ui, (ui.available_width() - 330.).max(70.));
                        }
                        ui.label(language.text("command"));
                        if ui
                            .add(
                                egui::DragValue::new(&mut self.progress)
                                    .range(0..=self.stitches.len())
                                    .speed(1.),
                            )
                            .changed()
                        {
                            self.playing = false;
                        }
                        ui.label(format!("/ {}", self.stitches.len()));
                    });
                    ui.add(
                        egui::DragValue::new(&mut self.speed)
                            .speed(50.)
                            .range(10. ..=20000.)
                            .suffix(language.text(" / s")),
                    );
                });
            }
            if self.error && self.workspace.timeline_open {
                egui::ScrollArea::vertical()
                    .id_salt("workspace-error-details")
                    .max_height(54.)
                    .show(ui, |ui| {
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(language.message(&self.message))
                                    .size(12.)
                                    .color(colors.error),
                            )
                            .wrap(),
                        );
                    });
            } else {
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(language.message(&self.message))
                            .size(11.)
                            .color(if self.error {
                                colors.error
                            } else {
                                colors.muted
                            }),
                    )
                    .truncate(),
                )
                .on_hover_text(language.message(&self.message).as_ref());
            }
        });
        if self.workspace.timeline_open {
            self.workspace.timeline_height = panel.response.rect.height();
        }
    }

    fn paint_sequence(&mut self, ui: &mut egui::Ui, width: f32) {
        let colors = self.workspace.theme.tokens();
        let (rect, response) =
            ui.allocate_exact_size(Vec2::new(width, 22.), Sense::click_and_drag());
        ui.painter().rect_filled(rect, 2., colors.elevated);
        let len = self.stitches.len();
        let stride = (len / (rect.width().max(1.) as usize)).max(1);
        for i in (0..len).step_by(stride) {
            let stitch = &self.stitches[i];
            let left = rect.left() + rect.width() * i as f32 / len as f32;
            let right = rect.left() + rect.width() * (i + stride).min(len) as f32 / len as f32;
            ui.painter().rect_filled(
                Rect::from_min_max(
                    Pos2::new(left, rect.top()),
                    Pos2::new(right.max(left + 1.), rect.bottom()),
                ),
                0.,
                Color32::from_rgb(stitch.color[0], stitch.color[1], stitch.color[2]),
            );
        }
        let x = rect.left() + rect.width() * self.progress as f32 / len as f32;
        ui.painter().line_segment(
            [
                Pos2::new(x, rect.top() - 2.),
                Pos2::new(x, rect.bottom() + 2.),
            ],
            Stroke::new(4.0_f32, colors.surface),
        );
        ui.painter().line_segment(
            [
                Pos2::new(x, rect.top() - 2.),
                Pos2::new(x, rect.bottom() + 2.),
            ],
            Stroke::new(2.0_f32, colors.accent),
        );
        if (response.clicked() || response.dragged())
            && let Some(pos) = response.interact_pointer_pos()
        {
            response.request_focus();
            self.playing = false;
            self.progress = (((pos.x - rect.left()) / rect.width()).clamp(0., 1.) * len as f32)
                .round() as usize;
        }
        if response.has_focus() {
            let step = ui.input(|i| {
                i32::from(i.key_pressed(egui::Key::ArrowRight))
                    - i32::from(i.key_pressed(egui::Key::ArrowLeft))
            });
            if step != 0 {
                self.playing = false;
                self.progress = self.progress.saturating_add_signed(step as isize).min(len);
            }
        }
        response.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::WidgetType::Slider,
                response.enabled(),
                format!(
                    "{} {} / {}",
                    self.language.text("command"),
                    self.progress,
                    len
                ),
            )
        });
        response.on_hover_text(
            self.language
                .text("Click or drag to inspect the actual command sequence."),
        );
    }
}
