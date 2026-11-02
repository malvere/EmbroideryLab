use super::*;

fn rectangle(id: u64, x: f64) -> Object {
    Object {
        id,
        contours: vec![vec![
            Point::new(x, 0.),
            Point::new(x + 10., 0.),
            Point::new(x + 10., 10.),
            Point::new(x, 10.),
        ]],
        ..Default::default()
    }
}

fn document(app: &Editor) -> serde_json::Value {
    serde_json::to_value(&app.project).unwrap()
}

fn editor(objects: Vec<Object>) -> Editor {
    let mut app = Editor::workspace();
    app.project = Project {
        version: 4,
        objects,
        ..Default::default()
    };
    app.scale = 24.;
    app
}

#[test]
fn selecting_another_context_object_clears_old_nodes_and_previews_without_editing() {
    let mut app = editor(vec![rectangle(1, 0.), rectangle(2, 100.)]);
    app.selected_node = Some((7, 99));
    app.selected_rail = Some((true, 99));
    app.selected_path = Some((7, 99));
    app.selected_branch = Some(99);
    app.selected_rung = Some(99);
    app.handle = Some(Handle::Node(7, 99));
    app.drag_before = Some(app.project.clone());
    app.smoothing_preview = Some(Object::default());
    app.outline_preview = Some(SatinColumn::default());
    app.preview_branch = Some(99);
    let before = document(&app);

    app.select_context_at(Point::new(105., 5.));

    assert_eq!(app.selected, 1);
    assert!(app.selected_node.is_none());
    assert!(app.selected_rail.is_none());
    assert!(app.selected_path.is_none());
    assert!(app.selected_branch.is_none());
    assert!(app.selected_rung.is_none());
    assert!(app.handle.is_none() && app.drag_before.is_none());
    assert!(app.smoothing_preview.is_none() && app.outline_preview.is_none());
    assert!(app.preview_branch.is_none());
    assert_eq!(document(&app), before);
    assert!(app.undo.is_empty() && !app.dirty);
}

#[test]
fn context_picking_targets_polygon_nodes_satin_rails_and_running_branches() {
    let mut app = editor(vec![
        rectangle(1, 0.),
        Object {
            id: 2,
            satin_column: Some(SatinColumn {
                left: vec![Point::new(30., 0.), Point::new(30., 10.)],
                right: vec![Point::new(34., 0.), Point::new(34., 10.)],
                ..Default::default()
            }),
            settings: Settings {
                kind: StitchKind::Satin,
                ..Default::default()
            },
            ..Default::default()
        },
        Object {
            id: 3,
            running_paths: vec![vec![Point::new(50., 0.), Point::new(60., 0.)]],
            settings: Settings {
                kind: StitchKind::Running,
                ..Default::default()
            },
            ..Default::default()
        },
    ]);

    app.select_context_at(Point::new(0.1, 0.1));
    assert_eq!(app.selected, 0);
    assert_eq!(app.selected_node, Some((0, 0)));

    app.select_context_at(Point::new(34., 10.));
    assert_eq!(app.selected, 1);
    assert_eq!(app.selected_rail, Some((true, 1)));
    assert!(app.selected_node.is_none() && app.selected_path.is_none());

    app.select_context_at(Point::new(60., 0.));
    assert_eq!(app.selected, 2);
    assert_eq!(app.selected_path, Some((0, 1)));
    assert_eq!(app.selected_branch, Some(0));
    assert!(app.selected_node.is_none() && app.selected_rail.is_none());

    app.select_context_at(Point::new(55., 0.));
    assert!(app.selected_path.is_none());
    assert_eq!(app.selected_branch, Some(0));
}

#[test]
fn reversing_each_geometry_kind_is_an_undoable_direction_change() {
    let objects = vec![
        rectangle(1, 0.),
        Object {
            bezier_segments: vec![CubicBezier {
                start: Point::new(0., 0.),
                control1: Point::new(0., 10.),
                control2: Point::new(10., 10.),
                end: Point::new(10., 0.),
            }],
            ..Default::default()
        },
        Object {
            running_paths: vec![vec![Point::new(0., 0.), Point::new(10., 0.)]],
            settings: Settings {
                kind: StitchKind::Running,
                ..Default::default()
            },
            ..Default::default()
        },
        Object {
            satin_column: Some(SatinColumn {
                left: vec![Point::new(0., 0.), Point::new(0., 10.)],
                right: vec![Point::new(4., 0.), Point::new(4., 10.)],
                ..Default::default()
            }),
            settings: Settings {
                kind: StitchKind::Satin,
                ..Default::default()
            },
            ..Default::default()
        },
    ];
    for object in objects {
        let mut app = editor(vec![object]);
        let before = document(&app);
        app.reverse_selected();
        assert!(!app.error, "{}", app.message);
        assert_ne!(document(&app), before);
        assert!(app.dirty && app.stale);
        assert_eq!(app.undo.len(), 1);
        app.project.validate_limits().unwrap();
        let reversed = document(&app);

        app.undo_edit();
        assert_eq!(document(&app), before);
        app.redo_edit();
        assert_eq!(document(&app), reversed);
    }
}

#[test]
fn reordering_and_removing_objects_preserve_identity_and_undo_history() {
    let mut app = editor(vec![rectangle(1, 0.), rectangle(2, 20.), rectangle(3, 40.)]);
    app.select_object(1);
    let before = document(&app);
    app.move_selected(true);
    assert_eq!(app.selected, 2);
    assert_eq!(app.project.objects[app.selected].id, 2);
    assert_eq!(app.undo.len(), 1);
    app.undo_edit();
    assert_eq!(document(&app), before);
    assert_eq!(app.project.objects[app.selected].id, 2);
    app.redo_edit();

    let reordered = document(&app);
    app.merge_selection = vec![1, 2, 3];
    app.selected_node = Some((0, 3));
    app.remove_selected();
    assert_eq!(app.project.objects.len(), 2);
    assert!(!app.merge_selection.contains(&2));
    assert!(app.selected < app.project.objects.len());
    assert!(app.selected_node.is_none());
    assert_eq!(app.undo.len(), 2);
    app.undo_edit();
    assert_eq!(document(&app), reordered);

    let mut last = editor(vec![rectangle(4, 0.)]);
    let original = document(&last);
    last.remove_selected();
    assert!(last.project.objects.is_empty());
    last.undo_edit();
    assert_eq!(document(&last), original);
}

#[test]
fn machine_preview_and_active_drawing_guard_context_actions() {
    let mut app = editor(vec![rectangle(1, 0.), rectangle(2, 20.)]);
    let before = document(&app);
    app.stitch_only = true;
    app.select_context_at(Point::new(25., 5.));
    app.reverse_selected();
    app.move_selected(true);
    app.remove_selected();
    assert_eq!(document(&app), before);
    assert_eq!(app.selected, 0);
    assert!(app.undo.is_empty() && !app.dirty);

    app.stitch_only = false;
    app.tool = Tool::Polygon;
    app.draft = vec![Point::new(1., 1.)];
    app.select_context_at(Point::new(25., 5.));
    assert_eq!(app.selected, 0);
    assert_eq!(app.draft, vec![Point::new(1., 1.)]);
    assert_eq!(document(&app), before);
    assert!(app.undo.is_empty());
}

fn frame(app: &mut Editor, ctx: &egui::Context, events: Vec<egui::Event>) -> egui::FullOutput {
    ctx.run(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(900., 680.))),
            events,
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| app.canvas(ui));
        },
    )
}

fn label_position(output: &egui::FullOutput, label: &str) -> Option<Pos2> {
    fn text_position(shape: &egui::epaint::Shape, label: &str) -> Option<Pos2> {
        match shape {
            egui::epaint::Shape::Text(text) if text.galley.job.text == label => {
                Some(text.pos + text.galley.size() / 2.)
            }
            egui::epaint::Shape::Vec(shapes) => {
                shapes.iter().find_map(|shape| text_position(shape, label))
            }
            _ => None,
        }
    }
    output
        .shapes
        .iter()
        .find_map(|clipped| text_position(&clipped.shape, label))
}

fn click(app: &mut Editor, ctx: &egui::Context, pos: Pos2, button: egui::PointerButton) {
    let pointer = |pos, button, pressed| egui::Event::PointerButton {
        pos,
        button,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    let _ = frame(
        app,
        ctx,
        vec![egui::Event::PointerMoved(pos), pointer(pos, button, true)],
    );
    let _ = frame(app, ctx, vec![pointer(pos, button, false)]);
}

#[test]
fn canvas_right_click_opens_parameters_and_editing_and_runs_fit_action() {
    let context = egui::Context::default();
    let mut app = editor(vec![rectangle(1, 0.)]);
    app.language = Language::English;
    app.fit = false;
    app.show_stitches = false;
    let _ = frame(&mut app, &context, vec![]);
    click(
        &mut app,
        &context,
        Pos2::new(250., 200.),
        egui::PointerButton::Secondary,
    );
    let output = frame(&mut app, &context, vec![]);
    assert!(app.context_popup.is_some());
    assert!(label_position(&output, "Parameters").is_some());
    assert!(label_position(&output, "Editing").is_some());
    let fit =
        label_position(&output, "Fit design").expect("Fit design is rendered in the context menu");
    click(&mut app, &context, fit, egui::PointerButton::Primary);
    assert!(
        app.fit,
        "The context action must request fitting the canvas"
    );
    assert!(app.context_popup.is_none());
}

#[test]
fn nested_stitch_type_popup_changes_parameters_without_closing_context_menu() {
    let context = egui::Context::default();
    let mut app = editor(vec![rectangle(1, 0.)]);
    app.language = Language::English;
    app.fit = false;
    app.show_stitches = false;
    let _ = frame(&mut app, &context, vec![]);
    click(
        &mut app,
        &context,
        Pos2::new(250., 200.),
        egui::PointerButton::Secondary,
    );
    let output = frame(&mut app, &context, vec![]);
    let parameters = label_position(&output, "Parameters").expect("Parameters menu is visible");
    click(&mut app, &context, parameters, egui::PointerButton::Primary);
    let output = frame(&mut app, &context, vec![]);
    let kind = label_position(&output, "Tatami").expect("Type combo is visible in Parameters");
    click(&mut app, &context, kind, egui::PointerButton::Primary);
    let output = frame(&mut app, &context, vec![]);
    assert!(app.context_popup.is_some());
    assert!(egui::Popup::is_any_open(&context));
    let running = label_position(&output, "Running").expect("Type choices remain open");
    click(&mut app, &context, running, egui::PointerButton::Primary);
    let output = frame(&mut app, &context, vec![]);
    assert_eq!(app.project.objects[0].settings.kind, StitchKind::Running);
    assert!(app.context_popup.is_some());
    assert!(label_position(&output, "Parameters").is_some());
    assert!(label_position(&output, "Max stitch length (mm)").is_some());
    assert!(app.dirty && app.stale);
}
