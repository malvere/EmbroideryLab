use super::*;

fn editor() -> Editor {
    let mut app = Editor::workspace_with_language(Language::English);
    app.project.objects.truncate(1);
    app.project.objects[0].name = "Editable object".into();
    app.fit = false;
    app.tool = Tool::Polygon;
    app.snapshot();
    app.project.name = "Unsaved design".into();
    app.dirty = true;
    app
}

fn input(events: Vec<egui::Event>) -> egui::RawInput {
    egui::RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1360., 900.))),
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

fn frame(
    app: &mut Editor,
    ctx: &egui::Context,
    events: Vec<egui::Event>,
) -> (egui::FullOutput, Rect) {
    let mut canvas = Rect::NOTHING;
    let output = ctx.run(input(events), |ctx| {
        app.shortcuts(ctx);
        app.workspace_chrome(ctx);
        egui::CentralPanel::default().show(ctx, |ui| {
            canvas = ui.available_rect_before_wrap();
            app.canvas(ui);
        });
        app.workspace_overlays(ctx);
        app.document_confirmations(ctx);
        app.recovery_and_autosave(ctx);
    });
    (output, canvas)
}

fn labelled_control(output: &egui::FullOutput, label: &str) -> (Pos2, bool) {
    let (_, node) = output
        .platform_output
        .accesskit_update
        .as_ref()
        .expect("AccessKit output is enabled")
        .nodes
        .iter()
        .find(|(_, node)| node.label() == Some(label))
        .unwrap_or_else(|| panic!("Expected the {label} control"));
    let rect = node.bounds().expect("A visible control has bounds");
    (
        Pos2::new(
            ((rect.x0 + rect.x1) / 2.) as f32,
            ((rect.y0 + rect.y1) / 2.) as f32,
        ),
        node.is_disabled(),
    )
}

#[test]
fn document_decisions_protect_background_clicks_and_undo_on_the_first_modal_frame() {
    for decision in ["close", "open", "new", "recovery"] {
        for target in ["canvas", "visibility"] {
            let mut app = editor();
            let ctx = egui::Context::default();
            ctx.enable_accesskit();
            app.workspace.theme.apply(&ctx);
            let (output, canvas) = frame(&mut app, &ctx, vec![]);
            let point = if target == "canvas" {
                canvas.left_top() + Vec2::new(70., 70.)
            } else {
                labelled_control(&output, "Exclude object from generated stitches").0
            };
            // Press before the dialog appears, then release on its first frame.
            // Relying only on egui's previous modal layer lets this click through.
            let _ = frame(
                &mut app,
                &ctx,
                vec![egui::Event::PointerMoved(point), button(point, true)],
            );
            match decision {
                "close" => app.confirm_close = true,
                "open" => app.discard = Some(PathBuf::from("unopened.elab")),
                "new" => app.confirm_new = true,
                "recovery" => app.recovery = Some(Project::default()),
                _ => unreachable!(),
            }
            let before = serde_json::to_value((&app.project, &app.stitches)).unwrap();
            let history = app.undo.len();
            let _ = frame(
                &mut app,
                &ctx,
                vec![
                    button(point, false),
                    egui::Event::Key {
                        key: egui::Key::Z,
                        physical_key: Some(egui::Key::Z),
                        pressed: true,
                        repeat: false,
                        modifiers: egui::Modifiers::CTRL,
                    },
                ],
            );
            assert!(
                app.decision_pending(),
                "{decision}: an outside click must leave the choice pending"
            );
            assert_eq!(
                serde_json::to_value((&app.project, &app.stitches)).unwrap(),
                before,
                "{decision}/{target}"
            );
            assert_eq!(
                app.undo.len(),
                history,
                "{decision}: Ctrl+Z must not edit the pending document"
            );
            assert!(
                app.draft.is_empty(),
                "{decision}: the canvas must not accept a contour point"
            );
        }
    }
}

#[test]
fn stitch_only_machine_limits_are_disabled_and_cannot_stale_the_plan() {
    let mut app = editor();
    app.project.objects.clear();
    app.project.machine_profile.max_stitch_mm = 7.3;
    app.dirty = false;
    app.stale = false;
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    app.workspace.theme.apply(&ctx);
    ctx.style_mut(|style| style.animation_time = 0.);
    let inspector = |app: &mut Editor, events| {
        ctx.run(input(events), |ctx| {
            app.workspace_inspector(ctx);
            egui::CentralPanel::default().show(ctx, |_| {});
        })
    };
    let output = inspector(&mut app, vec![]);
    let (header, _) = labelled_control(&output, "Machine limits and thread cuts");
    let _ = inspector(
        &mut app,
        vec![egui::Event::PointerMoved(header), button(header, true)],
    );
    let _ = inspector(&mut app, vec![button(header, false)]);
    app.stitch_only = true;
    let before = serde_json::to_value((&app.project, &app.stitches)).unwrap();
    let history = app.undo.len();
    let output = inspector(&mut app, vec![]);
    let (_, node) = output
        .platform_output
        .accesskit_update
        .as_ref()
        .unwrap()
        .nodes
        .iter()
        .find(|(_, node)| node.numeric_value() == Some(7.3))
        .expect("The expanded machine limits remain visible");
    assert!(
        node.is_disabled(),
        "Imported stitches have no regeneration path"
    );
    let bounds = node.bounds().unwrap();
    let point = Pos2::new(
        ((bounds.x0 + bounds.x1) / 2.) as f32,
        ((bounds.y0 + bounds.y1) / 2.) as f32,
    );
    let _ = inspector(
        &mut app,
        vec![egui::Event::PointerMoved(point), button(point, true)],
    );
    let _ = inspector(
        &mut app,
        vec![egui::Event::PointerMoved(point - Vec2::new(40., 0.))],
    );
    let _ = inspector(&mut app, vec![button(point - Vec2::new(40., 0.), false)]);
    assert_eq!(
        serde_json::to_value((&app.project, &app.stitches)).unwrap(),
        before
    );
    assert_eq!(app.undo.len(), history);
    assert!(!app.dirty && !app.stale && app.field_before.is_none());
}

#[test]
fn sewing_order_buttons_reflect_each_locked_neighbor() {
    let mut app = editor();
    app.project.objects = (1..=3)
        .map(|id| Object {
            id,
            name: format!("Object {id}"),
            ..Default::default()
        })
        .collect();
    app.selected = 1;
    app.locked_objects.extend([1, 3]);
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let layers = |app: &mut Editor| {
        ctx.run(input(vec![]), |ctx| {
            app.workspace_layers(ctx);
            egui::CentralPanel::default().show(ctx, |_| {});
        })
    };
    let output = layers(&mut app);
    assert!(labelled_control(&output, "Earlier").1);
    assert!(labelled_control(&output, "Later").1);
    app.locked_objects.remove(&1);
    let output = layers(&mut app);
    assert!(!labelled_control(&output, "Earlier").1);
    assert!(labelled_control(&output, "Later").1);
}
