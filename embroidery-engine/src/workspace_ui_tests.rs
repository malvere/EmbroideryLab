use super::*;

fn editor() -> Editor {
    let mut app = Editor::workspace_with_language(Language::English);
    app.project = Project {
        version: 4,
        objects: vec![Object {
            id: 71,
            name: "Editable square".into(),
            contours: vec![vec![
                Point::new(0., 0.),
                Point::new(10., 0.),
                Point::new(10., 10.),
                Point::new(0., 10.),
            ]],
            ..Default::default()
        }],
        ..Default::default()
    };
    app.generate();
    app.wait_for_digitize();
    assert!(!app.error, "{}", app.message);
    app.fit = false;
    app.pan = Vec2::ZERO;
    app.scale = 8.;
    app.selected = 0;
    app.canvas_scene = None;
    app
}

fn document_and_plan(app: &Editor) -> serde_json::Value {
    serde_json::to_value((
        &app.project,
        &app.stitches,
        &app.plan_metadata,
        &app.plan_warnings,
    ))
    .unwrap()
}

fn input(events: Vec<egui::Event>, size: Vec2) -> egui::RawInput {
    egui::RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
        focused: true,
        events,
        ..Default::default()
    }
}

fn button(pos: Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    }
}

fn key(key: egui::Key, pressed: bool) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: Some(key),
        pressed,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }
}

// These harnesses intentionally bypass update(), autosave and preference
// persistence. The actual canvas, shortcuts, menus and controls still run.
fn canvas_frame(app: &mut Editor, ctx: &egui::Context, events: Vec<egui::Event>) -> Rect {
    let mut canvas = Rect::NOTHING;
    let _ = ctx.run(input(events, Vec2::new(900., 680.)), |ctx| {
        app.shortcuts(ctx);
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ctx, |ui| {
                canvas = ui.available_rect_before_wrap();
                app.canvas(ui);
            });
    });
    canvas
}

fn chrome_frame(
    app: &mut Editor,
    ctx: &egui::Context,
    events: Vec<egui::Event>,
) -> egui::FullOutput {
    ctx.run(input(events, Vec2::new(1360., 900.)), |ctx| {
        app.workspace_chrome(ctx);
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.label("Workspace behavior test");
        });
    })
}

fn label_position(output: &egui::FullOutput, label: &str) -> Option<Pos2> {
    fn find(shape: &egui::epaint::Shape, label: &str) -> Option<Pos2> {
        match shape {
            egui::epaint::Shape::Text(text) if text.galley.job.text == label => {
                Some(text.pos + text.galley.size() / 2.)
            }
            egui::epaint::Shape::Vec(shapes) => {
                shapes.iter().rev().find_map(|shape| find(shape, label))
            }
            _ => None,
        }
    }
    // Popup content is painted after the matching panel heading beneath it.
    output
        .shapes
        .iter()
        .rev()
        .find_map(|shape| find(&shape.shape, label))
}

fn click_chrome(app: &mut Editor, ctx: &egui::Context, pos: Pos2) -> egui::FullOutput {
    let _ = chrome_frame(
        app,
        ctx,
        vec![egui::Event::PointerMoved(pos), button(pos, true)],
    );
    let _ = chrome_frame(app, ctx, vec![button(pos, false)]);
    chrome_frame(app, ctx, vec![])
}

#[test]
fn primary_pan_and_held_space_move_the_camera_in_logical_points_without_editing_nodes() {
    for pixels_per_point in [1., 2.] {
        for held_space in [false, true] {
            let mut app = editor();
            let ctx = egui::Context::default();
            ctx.set_pixels_per_point(pixels_per_point);
            app.pan_tool = !held_space;
            let rect = canvas_frame(&mut app, &ctx, vec![]);
            // The press starts on the actual editable origin node. Panning
            // must win over node-drag acquisition at both display scales.
            let start = rect.center();
            let delta = Vec2::new(54., -31.);
            let before = document_and_plan(&app);
            let progress = app.progress;
            let mut press = vec![egui::Event::PointerMoved(start), button(start, true)];
            if held_space {
                press.insert(0, key(egui::Key::Space, true));
            }
            canvas_frame(&mut app, &ctx, press);
            canvas_frame(
                &mut app,
                &ctx,
                vec![egui::Event::PointerMoved(start + delta)],
            );
            canvas_frame(&mut app, &ctx, vec![button(start + delta, false)]);
            if held_space {
                canvas_frame(&mut app, &ctx, vec![key(egui::Key::Space, false)]);
            }

            assert!(
                (app.pan - delta).length() < 0.01,
                "logical pan at {pixels_per_point} ppp: {:?}",
                app.pan
            );
            assert_eq!(document_and_plan(&app), before);
            assert_eq!(app.progress, progress);
            assert!(app.handle.is_none() && app.drag_before.is_none());
            assert!(
                app.undo.is_empty()
                    && app.redo.is_empty()
                    && app.field_before.is_none()
                    && !app.dirty
            );
            assert_eq!(app.pan_tool, !held_space);
        }
    }
}

#[test]
fn wheel_zoom_keeps_the_point_under_the_pointer_fixed_at_both_display_scales() {
    for pixels_per_point in [1., 2.] {
        let mut app = editor();
        let ctx = egui::Context::default();
        ctx.set_pixels_per_point(pixels_per_point);
        app.pan = Vec2::new(37., -21.);
        let _ = canvas_frame(&mut app, &ctx, vec![]);
        let rect = canvas_frame(&mut app, &ctx, vec![]);
        let pointer = rect.center() + Vec2::new(121., 57.);
        canvas_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(pointer)]);
        let world_before = (pointer - rect.center() - app.pan) / app.scale;
        let old_scale = app.scale;
        let before = document_and_plan(&app);
        canvas_frame(
            &mut app,
            &ctx,
            vec![egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: Vec2::new(0., 120.),
                modifiers: egui::Modifiers::NONE,
            }],
        );
        let world_after = (pointer - rect.center() - app.pan) / app.scale;
        assert!(app.scale > old_scale, "The real wheel handler must zoom");
        assert!((world_after - world_before).length() < 0.0001);
        assert_eq!(document_and_plan(&app), before);
        assert!(app.undo.is_empty() && !app.dirty);
    }
}

#[test]
fn v_h_and_f_shortcuts_do_not_steal_keys_from_a_focused_text_field() {
    let mut app = editor();
    let ctx = egui::Context::default();
    let mut text = "Layer".to_owned();
    let mut field_frame = |app: &mut Editor, events, focus| {
        let _ = ctx.run(input(events, Vec2::new(900., 680.)), |ctx| {
            app.shortcuts(ctx);
            egui::CentralPanel::default().show(ctx, |ui| {
                let response = ui.add(
                    egui::TextEdit::singleline(&mut text)
                        .id(egui::Id::new("workspace-shortcut-text")),
                );
                if focus {
                    response.request_focus();
                }
            });
        });
    };
    field_frame(&mut app, vec![], true);
    field_frame(&mut app, vec![], false);
    assert!(ctx.wants_keyboard_input());
    let before = document_and_plan(&app);
    for (pressed, character, pan_tool) in [
        (egui::Key::V, "v", true),
        (egui::Key::H, "h", false),
        (egui::Key::F, "f", false),
    ] {
        app.pan_tool = pan_tool;
        app.tool = Tool::Polygon;
        app.draft = vec![Point::new(1., 1.), Point::new(2., 2.)];
        app.fit = false;
        field_frame(
            &mut app,
            vec![key(pressed, true), egui::Event::Text(character.into())],
            false,
        );
        assert_eq!(app.pan_tool, pan_tool);
        assert!(app.tool == Tool::Polygon && !app.fit);
        assert_eq!(app.draft, vec![Point::new(1., 1.), Point::new(2., 2.)]);
        field_frame(&mut app, vec![key(pressed, false)], false);
    }
    assert!(text.contains('v') && text.contains('h') && text.contains('f'));
    assert_eq!(document_and_plan(&app), before);
    assert!(app.undo.is_empty() && !app.dirty);
}

#[test]
fn new_project_guard_and_its_cancel_keep_the_unsaved_document_and_plan() {
    let mut app = editor();
    app.snapshot();
    app.project.objects[0].name = "Unsaved embroidery".into();
    app.changed();
    let before = document_and_plan(&app);
    let history = app.undo.len();
    app.request_new_project();
    assert!(app.confirm_new);
    assert_eq!(document_and_plan(&app), before);
    let ctx = egui::Context::default();
    let guard_frame = |app: &mut Editor, events| {
        ctx.run(input(events, Vec2::new(900., 680.)), |ctx| {
            app.new_project_confirmation(ctx)
        })
    };
    let _ = guard_frame(&mut app, vec![]);
    let output = guard_frame(&mut app, vec![]);
    let cancel =
        label_position(&output, "Cancel").expect("The dirty-document guard exposes Cancel");
    let _ = guard_frame(
        &mut app,
        vec![egui::Event::PointerMoved(cancel), button(cancel, true)],
    );
    let _ = guard_frame(&mut app, vec![button(cancel, false)]);
    assert!(!app.confirm_new && app.dirty);
    assert_eq!(document_and_plan(&app), before);
    assert_eq!(app.undo.len(), history);
}

#[test]
fn document_history_respects_text_ownership_and_resumes_after_button_focus() {
    let mut app = editor();
    let original = app.project.objects[0].name.clone();
    app.snapshot();
    app.project.objects[0].name = "Changed in document".into();
    app.changed();
    let before = document_and_plan(&app);
    let ctx = egui::Context::default();
    let mut text = "Editable text".to_owned();
    let mut frame = |app: &mut Editor, events, text_focus: bool, button_focus: bool| {
        let _ = ctx.run(input(events, Vec2::new(900., 680.)), |ctx| {
            app.shortcuts(ctx);
            egui::CentralPanel::default().show(ctx, |ui| {
                let field = ui.text_edit_singleline(&mut text);
                if text_focus {
                    field.request_focus();
                }
                let button = ui.button("Ordinary toolbar button");
                if button_focus {
                    button.request_focus();
                }
            });
        });
    };
    frame(&mut app, vec![], true, false);
    frame(&mut app, vec![], false, false);
    let undo = || egui::Event::Key {
        key: egui::Key::Z,
        physical_key: Some(egui::Key::Z),
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::CTRL,
    };
    frame(&mut app, vec![undo()], false, false);
    assert_eq!(document_and_plan(&app), before);
    assert_eq!(app.undo.len(), 1);
    frame(&mut app, vec![key(egui::Key::Z, false)], false, true);
    frame(&mut app, vec![undo()], false, false);
    assert_eq!(app.project.objects[0].name, original);
    assert!(app.undo.is_empty() && app.redo.len() == 1);
    let mut redo = undo();
    if let egui::Event::Key { modifiers, .. } = &mut redo {
        *modifiers = egui::Modifiers::CTRL | egui::Modifiers::SHIFT;
    }
    frame(&mut app, vec![redo], false, false);
    assert_eq!(app.project.objects[0].name, "Changed in document");
    assert_eq!(app.undo.len(), 1);
    assert!(app.redo.is_empty());
}

#[test]
fn empty_localized_layers_reserve_the_complete_footer_above_the_sequence_panel() {
    for language in [Language::English, Language::Russian] {
        for theme in [ui_theme::Theme::Dark, ui_theme::Theme::Light] {
            for size in [Vec2::new(912., 512.), Vec2::new(850., 480.)] {
                let mut app = Editor::workspace_with_language(language);
                app.start_empty_document();
                app.workspace.theme = theme;
                let ctx = egui::Context::default();
                ctx.enable_accesskit();
                theme.apply(&ctx);
                let mut boundary = 0.;
                let mut output = None;
                for _ in 0..3 {
                    output = Some(ctx.run(input(vec![], size), |ctx| {
                        app.workspace_chrome(ctx);
                        boundary = ctx.available_rect().bottom();
                        egui::CentralPanel::default().show(ctx, |_| {});
                    }));
                }
                let output = output.unwrap();
                let nodes = &output
                    .platform_output
                    .accesskit_update
                    .as_ref()
                    .unwrap()
                    .nodes;
                for label in ["Earlier", "Later", "Object actions", "Remove object"] {
                    let node = nodes
                        .iter()
                        .find(|(_, node)| node.label() == Some(language.text(label)))
                        .unwrap_or_else(|| panic!("Missing {label}"));
                    let bounds = node.1.bounds().unwrap();
                    assert!(
                        bounds.y1 <= f64::from(boundary),
                        "{theme:?} {} {size:?}: {label} bottom {} exceeds sequence top {boundary}",
                        language.name(),
                        bounds.y1
                    );
                    assert!(
                        bounds.y1 - bounds.y0 >= 28.,
                        "Complete control height is retained"
                    );
                }
            }
        }
    }
}

#[test]
fn workspace_menu_theme_and_panel_controls_leave_the_embroidery_plan_unchanged() {
    let mut app = editor();
    let ctx = egui::Context::default();
    app.workspace.theme.apply(&ctx);
    let before = document_and_plan(&app);
    let output = chrome_frame(&mut app, &ctx, vec![]);
    let workspace = label_position(&output, "Workspace").expect("Workspace menu is visible");
    let output = click_chrome(&mut app, &ctx, workspace);
    let light = label_position(&output, "Light").expect("Theme choices are visible");
    let _ = click_chrome(&mut app, &ctx, light);
    assert_eq!(app.workspace.theme, ui_theme::Theme::Light);
    for label in ["Objects", "Inspector", "Stitch inspection"] {
        let mut output = chrome_frame(&mut app, &ctx, vec![]);
        if label_position(&output, "Dark").is_none() {
            let workspace = label_position(&output, "Workspace").unwrap();
            output = click_chrome(&mut app, &ctx, workspace);
        }
        let toggle =
            label_position(&output, label).expect("The actual workspace preference is visible");
        let _ = click_chrome(&mut app, &ctx, toggle);
    }
    assert!(
        !app.workspace.layers_open && !app.workspace.inspector_open && !app.workspace.timeline_open
    );
    assert_eq!(document_and_plan(&app), before);
    assert!(app.undo.is_empty() && app.redo.is_empty() && !app.dirty);
}

#[test]
fn many_long_layer_names_render_within_a_bounded_panel_without_changing_the_document() {
    let mut app = editor();
    app.project.objects = (0..2048)
        .map(|index| Object {
            id: index + 1,
            name: format!(
                "Layer {index}: {}",
                "Очень длинное имя объекта / long embroidery layer ".repeat(20)
            ),
            ..app.project.objects[0].clone()
        })
        .collect();
    let before = document_and_plan(&app);
    let ctx = egui::Context::default();
    let output = ctx.run(input(vec![], Vec2::new(1024., 680.)), |ctx| {
        app.workspace_layers(ctx)
    });
    assert!((180. ..=380.).contains(&app.workspace.layers_width));
    assert!(!output.shapes.is_empty());
    assert!(
        output
            .shapes
            .iter()
            .all(|shape| shape.clip_rect.is_finite())
    );
    let primitives = ctx.tessellate(output.shapes, output.pixels_per_point);
    for primitive in primitives {
        assert!(primitive.clip_rect.is_finite());
        if let egui::epaint::Primitive::Mesh(mesh) = primitive.primitive {
            assert!(
                mesh.vertices
                    .iter()
                    .all(|v| v.pos.x.is_finite() && v.pos.y.is_finite())
            );
        }
    }
    assert_eq!(document_and_plan(&app), before);
    assert!(app.undo.is_empty() && !app.dirty);
}
