use super::*;

fn editor() -> Editor {
    let mut editor = Editor::workspace_with_language(Language::English);
    editor.project.objects = [0., 20., 40.]
        .into_iter()
        .enumerate()
        .map(|(i, x)| Object {
            id: i as u64 + 1,
            contours: vec![vec![
                Point::new(x, 0.),
                Point::new(x + 10., 0.),
                Point::new(x + 10., 10.),
                Point::new(x, 10.),
            ]],
            ..Default::default()
        })
        .collect();
    editor.scale = 10.;
    editor.pan = Vec2::ZERO;
    editor.fit = false;
    editor.show_stitches = false;
    editor
}

fn frame(editor: &mut Editor, context: &egui::Context, events: Vec<egui::Event>) {
    let _ = context.run(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(900., 680.))),
            focused: true,
            events,
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| editor.canvas(ui));
        },
    );
}

fn button(pos: Pos2, pressed: bool, additive: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos,
        pressed,
        button: egui::PointerButton::Secondary,
        modifiers: egui::Modifiers {
            ctrl: additive,
            ..Default::default()
        },
    }
}

fn drag(editor: &mut Editor, context: &egui::Context, a: Pos2, b: Pos2, additive: bool) {
    frame(
        editor,
        context,
        vec![
            egui::Event::PointerMoved(a),
            button(a, true, additive),
            egui::Event::PointerMoved(b),
            button(b, false, additive),
        ],
    );
}

#[test]
fn batched_secondary_drag_selects_multiple_areas_without_menu_or_document_edit() {
    let mut app = editor();
    let context = egui::Context::default();
    frame(&mut app, &context, vec![]);
    let before = serde_json::to_value(&app.project).unwrap();
    drag(
        &mut app,
        &context,
        Pos2::new(445., 335.),
        Pos2::new(695., 455.),
        false,
    );
    assert_eq!(app.merge_selection, vec![1, 2]);
    assert!(app.context_popup.is_none() && app.marquee.drag.is_none());
    assert_eq!(serde_json::to_value(&app.project).unwrap(), before);
    assert!(!app.dirty && app.undo.is_empty());
    // Reverse direction replaces selection and excludes the previous objects.
    drag(
        &mut app,
        &context,
        Pos2::new(885., 455.),
        Pos2::new(845., 335.),
        false,
    );
    assert_eq!(app.merge_selection, vec![3]);
    assert_eq!(app.selected, 2);
}

#[test]
fn additive_drag_keeps_active_object_and_empty_drag_clears_inspector_safely() {
    let mut app = editor();
    let context = egui::Context::default();
    frame(&mut app, &context, vec![]);
    drag(
        &mut app,
        &context,
        Pos2::new(645., 335.),
        Pos2::new(755., 455.),
        true,
    );
    assert_eq!(app.merge_selection, vec![1, 2]);
    drag(
        &mut app,
        &context,
        Pos2::new(40., 40.),
        Pos2::new(90., 90.),
        true,
    );
    assert_eq!(app.merge_selection, vec![1, 2]);
    drag(
        &mut app,
        &context,
        Pos2::new(40., 40.),
        Pos2::new(90., 90.),
        false,
    );
    assert!(app.merge_selection.is_empty());
    assert_eq!(app.selected, app.project.objects.len());
    let _ = context.run(egui::RawInput::default(), |ctx| app.sidebars(ctx));
    app.create_centerlines();
    assert!(app.centerline_job.is_none());
}

#[test]
fn held_rectangle_and_final_release_motion_are_kept_and_escape_cancels() {
    let mut app = editor();
    let context = egui::Context::default();
    frame(&mut app, &context, vec![]);
    let a = Pos2::new(445., 335.);
    let b = Pos2::new(695., 455.);
    frame(
        &mut app,
        &context,
        vec![egui::Event::PointerMoved(a), button(a, true, false)],
    );
    frame(&mut app, &context, vec![egui::Event::PointerMoved(b)]);
    assert!(app.marquee.drag.as_ref().unwrap().moved);
    assert!(app.merge_selection.is_empty());
    // The final release position is included without an extra PointerMoved.
    frame(
        &mut app,
        &context,
        vec![button(Pos2::new(885., 455.), false, false)],
    );
    assert_eq!(app.merge_selection, vec![1, 2, 3]);
    frame(
        &mut app,
        &context,
        vec![
            egui::Event::PointerMoved(a),
            button(a, true, false),
            egui::Event::PointerMoved(b),
        ],
    );
    frame(
        &mut app,
        &context,
        vec![egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        }],
    );
    frame(&mut app, &context, vec![button(b, false, false)]);
    assert_eq!(app.merge_selection, vec![1, 2, 3]);
    assert!(app.marquee.drag.is_none() && app.context_popup.is_none());
}

#[test]
fn short_right_click_keeps_menu_and_outside_or_drawing_drag_cannot_select() {
    let mut app = editor();
    let context = egui::Context::default();
    frame(&mut app, &context, vec![]);
    let a = Pos2::new(465., 355.);
    let b = Pos2::new(467., 356.);
    frame(
        &mut app,
        &context,
        vec![egui::Event::PointerMoved(a), button(a, true, false)],
    );
    frame(
        &mut app,
        &context,
        vec![egui::Event::PointerMoved(b), button(b, false, false)],
    );
    assert!(app.context_popup.is_some());
    app.context_popup = None;
    egui::Popup::close_all(&context);
    drag(
        &mut app,
        &context,
        Pos2::new(-10., 335.),
        Pos2::new(695., 455.),
        false,
    );
    assert!(app.merge_selection.is_empty());
    app.tool = Tool::Polygon;
    app.draft = vec![Point::new(1., 2.)];
    drag(
        &mut app,
        &context,
        Pos2::new(445., 335.),
        Pos2::new(695., 455.),
        false,
    );
    assert!(app.merge_selection.is_empty());
    assert_eq!(app.draft, vec![Point::new(1., 2.)]);
}

#[test]
fn escape_before_drag_threshold_also_suppresses_menu_on_release() {
    let mut app = editor();
    let context = egui::Context::default();
    frame(&mut app, &context, vec![]);
    let a = Pos2::new(465., 355.);
    frame(
        &mut app,
        &context,
        vec![egui::Event::PointerMoved(a), button(a, true, false)],
    );
    frame(
        &mut app,
        &context,
        vec![egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        }],
    );
    frame(&mut app, &context, vec![button(a, false, false)]);
    assert!(app.context_popup.is_none() && app.marquee.drag.is_none());
    assert!(app.merge_selection.is_empty());
}
