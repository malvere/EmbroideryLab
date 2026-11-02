use super::*;

impl Editor {
    /// Select list rows in sewing order. The range anchor is an object ID so
    /// reordering rows does not silently move the start of a Shift selection.
    pub(super) fn select_layer(&mut self, index: usize, modifiers: egui::Modifiers) {
        if self.centerline_job.is_some() {
            return;
        }
        let Some(object) = self.project.objects.get(index) else {
            return;
        };
        let clicked_id = object.id;
        self.merge_selection
            .retain(|id| self.project.objects.iter().any(|object| object.id == *id));
        self.context_popup = None;

        if modifiers.shift {
            let anchor = self
                .selection_anchor
                .and_then(|id| {
                    self.project
                        .objects
                        .iter()
                        .position(|object| object.id == id)
                })
                .or((self.selected < self.project.objects.len()).then_some(self.selected))
                .unwrap_or(index);
            let anchor_id = self.project.objects[anchor].id;
            if modifiers.ctrl {
                self.seed_active_layer();
            } else {
                self.merge_selection.clear();
            }
            for object in &self.project.objects[anchor.min(index)..=anchor.max(index)] {
                if !self.merge_selection.contains(&object.id) {
                    self.merge_selection.push(object.id);
                }
            }
            self.select_object(index);
            // select_object also serves single/context clicks and resets the
            // anchor; repeated Shift clicks must retain this range's start.
            self.selection_anchor = Some(anchor_id);
        } else if modifiers.ctrl {
            self.seed_active_layer();
            if self.merge_selection.contains(&clicked_id) {
                self.merge_selection.retain(|id| *id != clicked_id);
                let active = self
                    .project
                    .objects
                    .get(self.selected)
                    .filter(|object| self.merge_selection.contains(&object.id))
                    .map(|_| self.selected)
                    .or_else(|| {
                        self.project
                            .objects
                            .iter()
                            .position(|object| self.merge_selection.contains(&object.id))
                    });
                if let Some(active) = active {
                    self.select_object(active);
                } else {
                    self.finish_field_edit();
                    self.clear_selection();
                    self.outline_preview = None;
                    self.preview_branch = None;
                    self.selected = self.project.objects.len();
                }
            } else {
                self.merge_selection.push(clicked_id);
                self.select_object(index);
            }
            self.selection_anchor = Some(clicked_id);
        } else {
            self.merge_selection.clear();
            self.select_object(index);
            self.selection_anchor = Some(clicked_id);
        }
    }

    /// All rows includes hidden layers; visibility controls stitching, not
    /// membership in the layer list.
    pub(super) fn select_all_layers(&mut self) {
        let anchor = self
            .selection_anchor
            .filter(|id| self.project.objects.iter().any(|object| object.id == *id));
        self.merge_selection = self
            .project
            .objects
            .iter()
            .map(|object| object.id)
            .collect();
        self.context_popup = None;
        if self.project.objects.is_empty() {
            self.finish_field_edit();
            self.clear_selection();
            self.outline_preview = None;
            self.preview_branch = None;
            self.selected = 0;
            self.selection_anchor = None;
        } else {
            let active = if self.selected < self.project.objects.len() {
                self.selected
            } else {
                0
            };
            self.select_object(active);
            self.selection_anchor = anchor.or(Some(self.project.objects[active].id));
        }
    }

    fn seed_active_layer(&mut self) {
        if self.merge_selection.is_empty()
            && let Some(object) = self.project.objects.get(self.selected)
        {
            self.merge_selection.push(object.id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn editor() -> Editor {
        let mut editor = Editor::workspace_with_language(Language::English);
        editor.project.objects = (1..=5)
            .map(|id| Object {
                id,
                ..Default::default()
            })
            .collect();
        editor
    }

    fn modifiers(ctrl: bool, shift: bool) -> egui::Modifiers {
        egui::Modifiers {
            ctrl,
            shift,
            ..Default::default()
        }
    }

    #[test]
    fn shift_selects_inclusive_ranges_in_both_directions_with_a_stable_anchor() {
        let mut app = editor();
        app.select_layer(1, modifiers(false, false));
        app.select_layer(4, modifiers(false, true));
        assert_eq!(app.merge_selection, vec![2, 3, 4, 5]);
        assert_eq!(app.selection_anchor, Some(2));
        app.select_layer(2, modifiers(false, true));
        assert_eq!(app.merge_selection, vec![2, 3]);
        app.select_layer(0, modifiers(false, true));
        assert_eq!(app.merge_selection, vec![1, 2]);
        assert_eq!(app.selected, 0);
        assert_eq!(app.selection_anchor, Some(2));
    }

    #[test]
    fn ctrl_toggles_individual_layers_including_the_only_active_layer() {
        let mut app = editor();
        app.select_layer(1, modifiers(false, false));
        app.select_layer(3, modifiers(true, false));
        assert_eq!(app.merge_selection, vec![2, 4]);
        app.select_layer(3, modifiers(true, false));
        assert_eq!(app.merge_selection, vec![2]);
        assert_eq!(app.selected, 1);
        app.select_layer(1, modifiers(true, false));
        assert!(app.merge_selection.is_empty());
        assert_eq!(app.selected, app.project.objects.len());
        app.select_layer(4, modifiers(true, false));
        assert_eq!(app.merge_selection, vec![5]);
        assert_eq!(app.selected, 4);
    }

    #[test]
    fn ctrl_shift_adds_a_range_and_plain_click_restores_single_selection() {
        let mut app = editor();
        app.select_layer(0, modifiers(false, false));
        app.select_layer(2, modifiers(true, false));
        app.select_layer(4, modifiers(true, true));
        assert_eq!(app.merge_selection, vec![1, 3, 4, 5]);
        assert_eq!(app.selection_anchor, Some(3));
        app.select_layer(1, modifiers(false, false));
        assert!(app.merge_selection.is_empty());
        assert_eq!(app.selected, 1);
        assert_eq!(app.selection_anchor, Some(2));
    }

    #[test]
    fn all_layers_includes_hidden_rows_and_keeps_valid_active_layer_and_anchor() {
        let mut app = editor();
        app.project.objects[2].visible = false;
        app.select_layer(1, modifiers(false, false));
        app.select_layer(3, modifiers(false, true));
        app.select_all_layers();
        assert_eq!(app.merge_selection, vec![1, 2, 3, 4, 5]);
        assert_eq!(app.selected, 3);
        assert_eq!(app.selection_anchor, Some(2));
        app.project.objects.clear();
        app.select_all_layers();
        assert!(app.merge_selection.is_empty());
        assert_eq!(app.selected, 0);
        assert!(app.selection_anchor.is_none());
    }

    #[test]
    fn range_anchor_follows_object_identity_and_deleted_anchor_falls_back_safely() {
        let mut app = editor();
        app.select_layer(1, modifiers(false, false));
        app.project.objects.swap(1, 3);
        app.select_layer(4, modifiers(false, true));
        assert_eq!(app.merge_selection, vec![2, 5]);
        app.project.objects.remove(3);
        app.selected = app.project.objects.len();
        app.select_layer(1, modifiers(false, true));
        assert_eq!(app.merge_selection, vec![4]);
        assert_eq!(app.selection_anchor, Some(4));
    }

    #[test]
    fn selection_changes_clear_stale_nodes_without_editing_the_document() {
        let mut app = editor();
        let before = serde_json::to_value(&app.project).unwrap();
        app.selected_node = Some((0, 2));
        app.selected_rail = Some((true, 1));
        app.select_layer(3, modifiers(false, true));
        app.select_all_layers();
        assert!(app.selected_node.is_none() && app.selected_rail.is_none());
        assert_eq!(serde_json::to_value(&app.project).unwrap(), before);
        assert!(!app.dirty && app.undo.is_empty() && app.redo.is_empty());
        let selection = app.merge_selection.clone();
        app.select_layer(500, modifiers(false, true));
        assert_eq!(app.merge_selection, selection);
    }
}
