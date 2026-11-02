use super::*;

const NAMES: [&str; 4] = ["Layer alpha", "Layer beta", "Layer gamma", "Layer delta"];

fn editor() -> Editor {
    let mut app = Editor::workspace_with_language(Language::English);
    app.project.objects = NAMES
        .iter()
        .enumerate()
        .map(|(index, name)| Object {
            id: index as u64 + 1,
            name: (*name).to_owned(),
            contours: vec![vec![
                Point::new(0., 0.),
                Point::new(10., 0.),
                Point::new(10., 10.),
                Point::new(0., 10.),
            ]],
            ..Default::default()
        })
        .collect();
    app.fit = false;
    app.show_stitches = false;
    app
}

fn frame(
    app: &mut Editor,
    context: &egui::Context,
    events: Vec<egui::Event>,
    modifiers: egui::Modifiers,
    mut text: Option<&mut String>,
    focus_text: bool,
) -> egui::FullOutput {
    context.run(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1360., 900.))),
            focused: true,
            modifiers,
            events,
            ..Default::default()
        },
        |context| {
            app.shortcuts(context);
            app.sidebars(context);
            egui::CentralPanel::default().show(context, |ui| {
                if let Some(text) = text.as_deref_mut() {
                    let response = ui.add(
                        egui::TextEdit::singleline(text)
                            .id(egui::Id::new("layer-shortcut-test-text")),
                    );
                    if focus_text {
                        response.request_focus();
                    }
                } else {
                    app.canvas(ui);
                }
            });
        },
    )
}

fn label_position(output: &egui::FullOutput, label: &str) -> Pos2 {
    fn find(shape: &egui::epaint::Shape, label: &str) -> Option<Pos2> {
        match shape {
            egui::epaint::Shape::Text(text) if text.galley.job.text == label => {
                Some(text.pos + text.galley.size() / 2.)
            }
            egui::epaint::Shape::Vec(shapes) => shapes.iter().find_map(|shape| find(shape, label)),
            _ => None,
        }
    }
    output
        .shapes
        .iter()
        .find_map(|shape| find(&shape.shape, label))
        .expect("layer row is rendered")
}

fn click(app: &mut Editor, context: &egui::Context, position: Pos2, modifiers: egui::Modifiers) {
    let pointer = |pressed| egui::Event::PointerButton {
        pos: position,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers,
    };
    frame(
        app,
        context,
        vec![egui::Event::PointerMoved(position), pointer(true)],
        modifiers,
        None,
        false,
    );
    frame(app, context, vec![pointer(false)], modifiers, None, false);
}

fn select_all_event(modifiers: egui::Modifiers) -> egui::Event {
    egui::Event::Key {
        key: egui::Key::A,
        physical_key: Some(egui::Key::A),
        pressed: true,
        repeat: false,
        modifiers,
    }
}

#[test]
fn sidebar_shift_clicks_keep_the_original_anchor_and_ctrl_a_works_after_row_focus() {
    let mut app = editor();
    let context = egui::Context::default();
    frame(
        &mut app,
        &context,
        vec![],
        egui::Modifiers::NONE,
        None,
        false,
    );
    let output = frame(
        &mut app,
        &context,
        vec![],
        egui::Modifiers::NONE,
        None,
        false,
    );
    click(
        &mut app,
        &context,
        label_position(&output, NAMES[1]),
        egui::Modifiers::NONE,
    );
    let output = frame(
        &mut app,
        &context,
        vec![],
        egui::Modifiers::NONE,
        None,
        false,
    );
    click(
        &mut app,
        &context,
        label_position(&output, NAMES[3]),
        egui::Modifiers::SHIFT,
    );
    assert_eq!(app.merge_selection, vec![2, 3, 4]);
    let output = frame(
        &mut app,
        &context,
        vec![],
        egui::Modifiers::NONE,
        None,
        false,
    );
    click(
        &mut app,
        &context,
        label_position(&output, NAMES[2]),
        egui::Modifiers::SHIFT,
    );
    assert_eq!(app.merge_selection, vec![2, 3]);
    assert_eq!(app.selection_anchor, Some(2));
    let output = frame(
        &mut app,
        &context,
        vec![],
        egui::Modifiers::NONE,
        None,
        false,
    );
    click(
        &mut app,
        &context,
        label_position(&output, NAMES[0]),
        egui::Modifiers::SHIFT,
    );
    assert_eq!(app.merge_selection, vec![1, 2]);
    assert_eq!(app.selection_anchor, Some(2));
    let control = egui::Modifiers {
        ctrl: true,
        command: true,
        ..Default::default()
    };
    frame(
        &mut app,
        &context,
        vec![select_all_event(control)],
        control,
        None,
        false,
    );
    assert_eq!(app.merge_selection, vec![1, 2, 3, 4]);
    assert!(!app.dirty && app.undo.is_empty());
}

#[test]
fn ctrl_a_selects_layers_on_canvas_but_selects_only_text_in_a_focused_editor() {
    let mut app = editor();
    let context = egui::Context::default();
    frame(
        &mut app,
        &context,
        vec![],
        egui::Modifiers::NONE,
        None,
        false,
    );
    click(
        &mut app,
        &context,
        Pos2::new(700., 200.),
        egui::Modifiers::NONE,
    );
    let control = egui::Modifiers {
        ctrl: true,
        command: true,
        ..Default::default()
    };
    frame(
        &mut app,
        &context,
        vec![select_all_event(control)],
        control,
        None,
        false,
    );
    assert_eq!(app.merge_selection, vec![1, 2, 3, 4]);
    app.select_layer(0, egui::Modifiers::NONE);
    let previous = app.merge_selection.clone();
    let mut text = "Embroidery layer name".to_owned();
    frame(
        &mut app,
        &context,
        vec![],
        egui::Modifiers::NONE,
        Some(&mut text),
        true,
    );
    frame(
        &mut app,
        &context,
        vec![],
        egui::Modifiers::NONE,
        Some(&mut text),
        false,
    );
    assert!(context.wants_keyboard_input());
    frame(
        &mut app,
        &context,
        vec![select_all_event(control)],
        control,
        Some(&mut text),
        false,
    );
    assert_eq!(app.merge_selection, previous);
    let state =
        egui::TextEdit::load_state(&context, egui::Id::new("layer-shortcut-test-text")).unwrap();
    let selected = state.cursor.char_range().unwrap().as_sorted_char_range();
    assert_eq!(selected, 0..text.chars().count());
    assert_eq!(text, "Embroidery layer name");
    assert!(!app.dirty && app.undo.is_empty());
}
