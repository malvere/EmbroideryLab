use super::*;

impl Editor {
    pub(super) fn update_stitch_view(&mut self) {
        self.stitch_count = self
            .stitches
            .iter()
            .filter(|s| s.command == Command::Stitch)
            .count();
        self.jump_count = self
            .stitches
            .iter()
            .filter(|s| s.command == Command::Jump)
            .count();
        self.stitch_bounds = stitch_metrics::stitched_bounds(&self.stitches);
        self.stitch_revision = self.stitch_revision.wrapping_add(1);
        self.preview_3d
            .set_stitches(&self.stitches, self.stitch_revision);
    }

    pub(super) fn set_3d_view(&mut self, enabled: bool) {
        if self.show_3d != enabled {
            if enabled {
                if !self.draft.is_empty()
                    || !self.satin_first.is_empty()
                    || self.rung_start.is_some()
                    || self.outline_preview.is_some()
                    || self.smoothing_preview.is_some()
                {
                    self.result(Err(
                        "Finish or cancel the current drawing before opening 3D.".into(),
                    ));
                    return;
                }
                self.tool = Tool::Nodes;
                self.clear_selection();
            }
            self.context_popup = None;
            self.show_3d = enabled;
        }
    }

    pub(super) fn stitch_summary_text(&self) -> String {
        let counts = self
            .language
            .message(&format!(
                "{} needle stitches · {} jumps",
                self.stitch_count, self.jump_count
            ))
            .into_owned();
        let size = if !self.stale
            && let Some(bounds) = self.stitch_bounds
        {
            self.language
                .message(&format!(
                    "Size: {:.1} × {:.1} mm",
                    bounds.width_mm(),
                    bounds.height_mm()
                ))
                .into_owned()
        } else {
            self.language.text("Size: —").to_owned()
        };
        format!("{counts} · {size}")
    }

    pub(super) fn three_d_canvas(&mut self, ui: &mut egui::Ui) {
        let language = self.language;
        ui.horizontal_wrapped(|ui| {
            ui.strong(language.text("3D stitch preview"));
            ui.small(language.text("Geometric thread preview on fabric."));
        });
        if self.stale
            && !self.stitch_only
            && ui.button(language.text("Generate stitches")).clicked()
        {
            self.generate();
        }
        if let Some(error) = self.preview_3d.error() {
            ui.colored_label(
                Color32::from_rgb(161, 43, 50),
                language.message(&error).as_ref(),
            );
        }
        self.preview_3d.show(
            ui,
            self.progress,
            self.stale,
            self.show_jumps,
            stitch_3d::PreviewLabels {
                empty: language.text("No stitches to preview. Open a design or generate stitches."),
                stale: language.text("Generate stitches to refresh the 3D preview."),
                unavailable: language.text("3D preview requires the OpenGL renderer."),
                navigation: language.text("Drag: rotate · Middle drag: pan · Wheel: zoom"),
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_shows_sewn_size_next_to_counts_and_hides_stale_measurements() {
        let mut editor = Editor::workspace_with_language(Language::English);
        editor.stitches = vec![
            Stitch {
                x: 80.,
                y: 90.,
                command: Command::Jump,
                color: [0; 3],
                object_id: 1,
            },
            Stitch {
                x: 90.,
                y: 110.,
                command: Command::Stitch,
                color: [0; 3],
                object_id: 1,
            },
            Stitch {
                x: 999.,
                y: 999.,
                command: Command::Jump,
                color: [0; 3],
                object_id: 1,
            },
        ];
        editor.update_stitch_view();
        editor.stale = false;
        assert_eq!(
            editor.stitch_summary_text(),
            "1 needle stitches · 2 jumps · Size: 10.0 × 20.0 mm"
        );
        editor.progress = 0;
        assert!(editor.stitch_summary_text().contains("10.0 × 20.0"));
        editor.language = Language::Russian;
        assert!(
            editor
                .stitch_summary_text()
                .contains("Размер: 10.0 × 20.0 мм")
        );
        editor.changed();
        assert!(!editor.stitch_summary_text().contains("10.0 × 20.0"));
        assert!(editor.stitch_summary_text().contains("Размер: —"));
    }

    #[test]
    fn preview_mode_and_visual_thread_width_do_not_edit_the_document() {
        let mut editor = Editor::workspace();
        let document = serde_json::to_value(&editor.project).unwrap();
        editor.selected_node = Some((0, 0));
        editor.set_3d_view(true);
        editor.preview_3d.thread_diameter_mm = 0.45;
        assert!(editor.show_3d);
        assert!(editor.draft.is_empty() && editor.selected_node.is_none());
        editor.set_3d_view(false);
        assert_eq!(serde_json::to_value(&editor.project).unwrap(), document);
        assert!(!editor.dirty && editor.undo.is_empty());
    }

    #[test]
    fn opening_3d_keeps_an_unfinished_drawing_available_for_completion() {
        let mut editor = Editor::workspace();
        editor.tool = Tool::Polygon;
        editor.draft = vec![Point::new(1., 1.), Point::new(2., 3.)];
        let draft = editor.draft.clone();
        editor.set_3d_view(true);
        assert!(!editor.show_3d);
        assert_eq!(editor.draft, draft);
        assert!(editor.tool == Tool::Polygon && editor.error);
        assert!(editor.undo.is_empty() && !editor.dirty);
    }
}
