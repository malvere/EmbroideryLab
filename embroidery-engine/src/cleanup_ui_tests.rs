//! Gesture replay uses logical egui input coordinates at two pixel scales.
use super::*;

#[test]
fn preparation_change_overlay_preserves_alpha_and_excluded_background() {
    let source = image::RgbaImage::from_pixel(4, 1, image::Rgba([20, 80, 160, 150]));
    let report = import::RasterTraceReport {
        trace_dimensions: [4, 1],
        labels_before_preparation: vec![0, 1, 255, 2],
        labels_after_preparation: vec![0, 0, 255, 2],
        ..Default::default()
    };
    let overlay = preparation_change_overlay(&report, &source).unwrap();
    assert_eq!(overlay.get_pixel(0, 0).0, [20, 80, 160, 150]);
    assert_eq!(overlay.get_pixel(1, 0).0, [255, 130, 25, 255]);
    assert_eq!(overlay.get_pixel(2, 0)[3], 0);
    assert_eq!(overlay.get_pixel(3, 0).0, [20, 80, 160, 150]);
}

#[test]
fn comparison_layers_keep_the_same_physical_fit_at_different_resolutions() {
    let ctx = egui::Context::default();
    let source = source();
    let mut state = CleanupState::default();
    state.reset(&source, None);
    let texture = |name, w, h| {
        ctx.load_texture(
            name,
            egui::ColorImage::filled([w, h], Color32::RED),
            egui::TextureOptions::NEAREST,
        )
    };
    let original = texture("original", 64, 32);
    let regions = texture("regions", 16, 8);
    let changes = texture("changes", 128, 64);
    let mut rectangles = Vec::new();
    for (view, target) in [(0, None), (2, Some(&regions)), (4, Some(&changes))] {
        state.view = view;
        let mut last = None;
        for _ in 0..2 {
            last = Some(ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(800., 1000.))),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        state.show(
                            ui,
                            source.clone(),
                            &original,
                            PreviewLayers {
                                regions: Some(&regions),
                                changes: Some(&changes),
                                plan: None,
                                width_mm: 64.,
                            },
                            Language::English,
                        );
                    });
                },
            ));
        }
        let id = target
            .map(|t| t.id())
            .unwrap_or_else(|| state.overlay.as_ref().unwrap().id());
        rectangles.push(image_rect(&last.unwrap(), id));
    }
    assert_eq!(rectangles[0], rectangles[1]);
    assert_eq!(rectangles[0], rectangles[2]);
}

fn source() -> Arc<image::RgbaImage> {
    Arc::new(image::RgbaImage::from_fn(64, 32, |x, _| {
        let v = 80 + x as u8 * 2;
        image::Rgba([v, v, v, 255])
    }))
}

fn frame(
    ctx: &egui::Context,
    state: &mut CleanupState,
    source: &Arc<image::RgbaImage>,
    events: Vec<egui::Event>,
) -> egui::FullOutput {
    let texture = ctx.load_texture(
        "test source",
        egui::ColorImage::from_rgba_unmultiplied(
            [source.width() as usize, source.height() as usize],
            source.as_raw(),
        ),
        egui::TextureOptions::LINEAR,
    );
    ctx.run(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(800., 1000.))),
            modifiers: events
                .iter()
                .rev()
                .find_map(|event| match event {
                    egui::Event::Key { modifiers, .. }
                    | egui::Event::PointerButton { modifiers, .. } => Some(*modifiers),
                    _ => None,
                })
                .unwrap_or_default(),
            events,
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let _ = state.show(
                    ui,
                    source.clone(),
                    &texture,
                    PreviewLayers {
                        regions: None,
                        changes: None,
                        plan: None,
                        width_mm: 64.,
                    },
                    Language::English,
                );
            });
        },
    )
}

// Native scale/zoom changes become active at the next egui pass. Discover the
// image rectangle after that transition, rather than clicking the first pass's
// temporary layout. All source-pixel and tolerance expectations remain exact.
fn settled_frame(
    ctx: &egui::Context,
    state: &mut CleanupState,
    source: &Arc<image::RgbaImage>,
) -> egui::FullOutput {
    let _ = frame(ctx, state, source, vec![]);
    frame(ctx, state, source, vec![])
}

fn image_rect(output: &egui::FullOutput, id: egui::TextureId) -> Rect {
    fn find(shape: &egui::epaint::Shape, id: egui::TextureId) -> Option<Rect> {
        match shape {
            egui::epaint::Shape::Rect(rect)
                if rect
                    .brush
                    .as_ref()
                    .is_some_and(|brush| brush.fill_texture_id == id) =>
            {
                Some(rect.rect)
            }
            egui::epaint::Shape::Mesh(mesh) if mesh.texture_id == id => {
                let mut min = Pos2::new(f32::INFINITY, f32::INFINITY);
                let mut max = Pos2::new(f32::NEG_INFINITY, f32::NEG_INFINITY);
                for vertex in &mesh.vertices {
                    min.x = min.x.min(vertex.pos.x);
                    min.y = min.y.min(vertex.pos.y);
                    max.x = max.x.max(vertex.pos.x);
                    max.y = max.y.max(vertex.pos.y);
                }
                Some(Rect::from_min_max(min, max))
            }
            egui::epaint::Shape::Vec(shapes) => shapes.iter().find_map(|s| find(s, id)),
            _ => None,
        }
    }
    output
        .shapes
        .iter()
        .find_map(|s| find(&s.shape, id))
        .expect("cleanup image with the expected texture")
}

fn button(pos: Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: Default::default(),
    }
}

fn escape() -> egui::Event {
    egui::Event::Key {
        key: egui::Key::Escape,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Default::default(),
    }
}

fn wait_refine(state: &mut CleanupState, ctx: &egui::Context, source: &image::RgbaImage) {
    let start = std::time::Instant::now();
    while state.refining.is_some() {
        state.poll(ctx, source);
        assert!(
            start.elapsed().as_secs() < 10,
            "source-mask refinement timed out"
        );
        std::thread::yield_now();
    }
}

#[test]
fn final_release_motion_changes_tolerance_before_source_refinement() {
    let ctx = egui::Context::default();
    let source = source();
    let mut state = CleanupState::default();
    state.reset(&source, None);
    state.tolerance = 2.;
    let first = settled_frame(&ctx, &mut state, &source);
    let rect = image_rect(&first, state.overlay.as_ref().unwrap().id());
    let start = rect.min + Vec2::new(2.5, 4.5);
    frame(
        &ctx,
        &mut state,
        &source,
        vec![egui::Event::PointerMoved(start), button(start, true)],
    );
    let finish = start + Vec2::new(60., 0.);
    frame(
        &ctx,
        &mut state,
        &source,
        vec![egui::Event::PointerMoved(finish), button(finish, false)],
    );
    assert!((state.tolerance - color::gesture_tolerance(2., 60.)).abs() < 1e-5);
    assert!(state.gesture.is_none());
    wait_refine(&mut state, &ctx, &source);
    let expected = SelectionCache::new(&source, state.seed.unwrap())
        .unwrap()
        .candidates(state.tolerance, SelectionMode::Connected);
    assert_eq!(state.selection, expected);
}

#[test]
fn batched_press_move_release_keeps_the_original_source_seed() {
    let ctx = egui::Context::default();
    let source = source();
    let mut state = CleanupState::default();
    state.reset(&source, None);
    state.tolerance = 2.;
    let first = settled_frame(&ctx, &mut state, &source);
    let rect = image_rect(&first, state.overlay.as_ref().unwrap().id());
    let start = rect.min + Vec2::new(2.5, 4.5);
    let finish = start + Vec2::new(60., 0.);
    frame(
        &ctx,
        &mut state,
        &source,
        vec![
            egui::Event::PointerMoved(start),
            button(start, true),
            egui::Event::PointerMoved(finish),
            button(finish, false),
        ],
    );
    assert_eq!(state.seed, Some([2, 4]));
    assert_eq!(state.cache.as_ref().unwrap().seed_rgb, [84; 3]);
    assert!((state.tolerance - color::gesture_tolerance(2., 60.)).abs() < 1e-5);
    wait_refine(&mut state, &ctx, &source);
}

#[test]
fn held_colour_gesture_has_fixed_seed_and_reversible_logical_tolerance_at_hidpi() {
    let mut final_masks = vec![];
    for pixel_scale in [1., 2.] {
        let ctx = egui::Context::default();
        ctx.set_pixels_per_point(pixel_scale);
        let source = source();
        let mut state = CleanupState::default();
        state.reset(&source, None);
        state.tolerance = 2.;
        let first = settled_frame(&ctx, &mut state, &source);
        let rect = image_rect(&first, state.overlay.as_ref().unwrap().id());
        let point = rect.min + Vec2::new(2.5, 4.5);
        frame(
            &ctx,
            &mut state,
            &source,
            vec![egui::Event::PointerMoved(point), button(point, true)],
        );
        let seed = state.gesture.as_ref().unwrap().seed;
        let reference = state.cache.as_ref().unwrap().seed_rgb;
        let narrow = state.preview_selection.clone();
        frame(
            &ctx,
            &mut state,
            &source,
            vec![egui::Event::PointerMoved(point + Vec2::new(60., 0.))],
        );
        assert!(
            state.preview_selection.iter().filter(|&&x| x).count()
                > narrow.iter().filter(|&&x| x).count()
        );
        assert_eq!(state.gesture.as_ref().unwrap().seed, seed);
        assert_eq!(state.cache.as_ref().unwrap().seed_rgb, reference);
        frame(
            &ctx,
            &mut state,
            &source,
            vec![egui::Event::PointerMoved(point)],
        );
        assert_eq!(state.tolerance, 2.);
        assert_eq!(state.preview_selection, narrow);
        final_masks.push(state.preview_selection.clone());
        frame(&ctx, &mut state, &source, vec![button(point, false)]);
        wait_refine(&mut state, &ctx, &source);
    }
    assert_eq!(final_masks[0], final_masks[1]);
}

#[test]
fn releasing_outside_image_finishes_and_full_mask_refreshes_preview() {
    let ctx = egui::Context::default();
    let source = source();
    let mut state = CleanupState::default();
    state.reset(&source, None);
    let first = settled_frame(&ctx, &mut state, &source);
    let rect = image_rect(&first, state.overlay.as_ref().unwrap().id());
    let point = rect.min + Vec2::new(2.5, 4.5);
    frame(
        &ctx,
        &mut state,
        &source,
        vec![egui::Event::PointerMoved(point), button(point, true)],
    );
    let outside = rect.max + Vec2::new(100., 100.);
    frame(
        &ctx,
        &mut state,
        &source,
        vec![egui::Event::PointerMoved(outside), button(outside, false)],
    );
    assert!(state.gesture.is_none());
    wait_refine(&mut state, &ctx, &source);
    assert!(state.selection.iter().any(|&v| v));
    assert_eq!(
        state.preview_selection,
        state.reduce_mask(&state.selection, &source)
    );
    assert!(state.undo.is_empty());
    let selection = state.selection.clone();
    state.apply_exclusion(&source);
    assert_eq!(state.excluded, selection);
    assert_eq!(state.undo.len(), 1);
    state.undo(&source, false);
    assert!(state.excluded.iter().all(|&v| !v));
    state.undo(&source, true);
    assert_eq!(state.excluded, selection);
}

#[test]
fn escape_in_release_frame_restores_frozen_selection_and_does_not_start_worker() {
    let ctx = egui::Context::default();
    let source = source();
    let mut state = CleanupState::default();
    state.reset(&source, None);
    state.selection[1000] = true;
    state.preview_selection[1000] = true;
    let base = state.selection.clone();
    let first = settled_frame(&ctx, &mut state, &source);
    let rect = image_rect(&first, state.overlay.as_ref().unwrap().id());
    let point = rect.min + Vec2::new(2.5, 4.5);
    frame(
        &ctx,
        &mut state,
        &source,
        vec![egui::Event::PointerMoved(point), button(point, true)],
    );
    frame(
        &ctx,
        &mut state,
        &source,
        vec![egui::Event::PointerMoved(point + Vec2::new(80., 0.))],
    );
    frame(
        &ctx,
        &mut state,
        &source,
        vec![button(point, false), escape()],
    );
    assert!(state.gesture.is_none());
    assert!(state.refining.is_none());
    assert_eq!(state.selection, base);
    assert_eq!(state.preview_selection, base);
    assert!(state.undo.is_empty());
}

#[test]
fn subtract_preview_and_slider_recalculation_always_use_mouse_down_base() {
    let source = source();
    let mut state = CleanupState::default();
    state.reset(&source, None);
    state.selection.fill(true);
    state.preview_selection.fill(true);
    state.cache = Some(SelectionCache::new(&state.preview, [0, 0]).unwrap());
    state.last_base = state.selection.clone();
    state.last_preview_base = state.preview_selection.clone();
    state.last_op = SelectionOp::Subtract;
    state.tolerance = 2.;
    state.update_preview(&source);
    let small = state.preview_selection.clone();
    state.tolerance = 10.;
    state.update_preview(&source);
    let large = state.preview_selection.clone();
    assert!(large.iter().zip(&small).all(|(&a, &b)| !a || b));
    state.tolerance = 2.;
    state.update_preview(&source);
    assert_eq!(state.preview_selection, small);
}

#[test]
fn stale_refinement_is_discarded_and_transparent_seed_error_is_visible() {
    let ctx = egui::Context::default();
    let source = source();
    let mut state = CleanupState::default();
    state.reset(&source, None);
    let before = state.selection.clone();
    let (sender, receiver) = mpsc::channel();
    state.revision = 5;
    state.refining = Some(receiver);
    sender.send((4, Ok(vec![true; source.len() / 4]))).unwrap();
    state.poll(&ctx, &source);
    assert_eq!(state.selection, before);
    let (sender, receiver) = mpsc::channel();
    state.refining = Some(receiver);
    sender
        .send((5, Err("Choose an opaque source pixel".into())))
        .unwrap();
    state.poll(&ctx, &source);
    assert_eq!(
        state.error.as_deref(),
        Some("Choose an opaque source pixel")
    );
}

fn key(key: egui::Key, shift: bool) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers {
            ctrl: true,
            command: true,
            shift,
            ..Default::default()
        },
    }
}

fn text_center(output: &egui::FullOutput, label: &str) -> Pos2 {
    fn find(shape: &egui::Shape, label: &str) -> Option<Pos2> {
        match shape {
            egui::Shape::Text(text) if text.galley.text() == label => {
                Some(text.pos + text.galley.size() * 0.5)
            }
            egui::Shape::Vec(shapes) => shapes.iter().find_map(|shape| find(shape, label)),
            _ => None,
        }
    }
    output
        .shapes
        .iter()
        .find_map(|shape| find(&shape.shape, label))
        .expect(label)
}

#[test]
fn exclusion_button_focus_allows_consumed_undo_and_both_redo_shortcuts() {
    fn focused_button_frame(
        ctx: &egui::Context,
        state: &mut CleanupState,
        source: &Arc<image::RgbaImage>,
        events: Vec<egui::Event>,
        request_focus: bool,
    ) {
        let mut button_id = None;
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(800., 1000.))),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let button = ui
                        .push_id("cleanup enabled focus fixture", |ui| ui.button("Preview"))
                        .inner;
                    if request_focus {
                        button.request_focus();
                    }
                    button_id = Some(button.id);
                    assert!(button.enabled());
                    assert!(button.has_focus());
                    state.selection_controls(ui, source.clone(), Language::English);
                });
            },
        );
        assert_eq!(ctx.memory(|memory| memory.focused()), button_id);
        assert!(ctx.wants_keyboard_input());
        assert!(!keyboard_focus::text_input_active(ctx));
    }
    let ctx = egui::Context::default();
    let source = source();
    let mut state = CleanupState::default();
    state.reset(&source, None);
    state.selection[100] = true;
    state.preview_selection[100] = true;
    let first = settled_frame(&ctx, &mut state, &source);
    let point = text_center(&first, "Exclude from digitizing");
    frame(
        &ctx,
        &mut state,
        &source,
        vec![egui::Event::PointerMoved(point), button(point, true)],
    );
    frame(&ctx, &mut state, &source, vec![button(point, false)]);
    assert!(state.excluded[100]);
    // Exclude now becomes disabled when its selection is cleared. Give an
    // ordinary enabled native button focus, as keyboard navigation does, and
    // retain that exact focused ID throughout the mask shortcuts below.
    focused_button_frame(&ctx, &mut state, &source, vec![], true);
    for redo_key in [key(egui::Key::Y, false), key(egui::Key::Z, true)] {
        focused_button_frame(
            &ctx,
            &mut state,
            &source,
            vec![key(egui::Key::Z, false)],
            false,
        );
        assert!(!state.excluded[100]);
        assert!(
            !ctx.input(|i| i.key_pressed(egui::Key::Z)),
            "cleanup consumes its shortcut"
        );
        focused_button_frame(&ctx, &mut state, &source, vec![redo_key], false);
        assert!(state.excluded[100]);
        assert_eq!(state.undo.len(), 1);
    }
    state.selection[101] = true;
    let mut delete = key(egui::Key::Delete, false);
    if let egui::Event::Key { modifiers, .. } = &mut delete {
        *modifiers = egui::Modifiers::NONE;
    }
    focused_button_frame(&ctx, &mut state, &source, vec![delete], false);
    assert!(state.excluded[101]);
}

#[test]
fn empty_protected_and_already_excluded_selections_do_not_create_history() {
    let source = source();
    let mut state = CleanupState::default();
    state.reset(&source, None);
    for case in 0..3 {
        state.selection.fill(false);
        if case != 0 {
            state.selection[100] = true;
        }
        state.protected[100] = case == 1;
        state.excluded[100] = case == 2;
        let before = state.mask(&source).unwrap();
        assert!(!state.can_exclude());
        state.apply_exclusion(&source);
        assert_eq!(state.mask(&source).unwrap(), before);
        assert!(state.undo.is_empty());
        assert!(!state.changed);
    }
}

#[test]
fn focused_native_text_editor_keeps_its_undo_and_leaves_mask_history_untouched() {
    fn edit_frame(
        ctx: &egui::Context,
        state: &mut CleanupState,
        source: &Arc<image::RgbaImage>,
        text: &mut String,
        time: f64,
        events: Vec<egui::Event>,
    ) {
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(800., 1000.))),
                time: Some(time),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let response = ui.add(
                        egui::TextEdit::singleline(text).id(egui::Id::new("cleanup test text")),
                    );
                    if time == 0. {
                        response.request_focus();
                    }
                    assert!(keyboard_focus::text_input_active(ctx));
                    state.selection_controls(ui, source.clone(), Language::English);
                });
            },
        );
    }
    let ctx = egui::Context::default();
    let source = source();
    let mut state = CleanupState::default();
    state.reset(&source, None);
    state.selection[100] = true;
    state.apply_exclusion(&source);
    let before = state.mask(&source).unwrap();
    let mut text = String::new();
    edit_frame(&ctx, &mut state, &source, &mut text, 0., vec![]);
    edit_frame(
        &ctx,
        &mut state,
        &source,
        &mut text,
        1.,
        vec![egui::Event::Text("source notes".into())],
    );
    assert_eq!(text, "source notes");
    edit_frame(&ctx, &mut state, &source, &mut text, 2., vec![]);
    edit_frame(
        &ctx,
        &mut state,
        &source,
        &mut text,
        3.,
        vec![key(egui::Key::Z, false)],
    );
    assert!(text.is_empty(), "native TextEdit handles its own Undo");
    assert_eq!(state.mask(&source).unwrap(), before);
    assert_eq!(state.undo.len(), 1);
    assert!(state.redo.is_empty());
    edit_frame(
        &ctx,
        &mut state,
        &source,
        &mut text,
        4.,
        vec![key(egui::Key::Z, true)],
    );
    assert_eq!(text, "source notes");
    assert_eq!(state.mask(&source).unwrap(), before);
}

fn middle(pos: Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Middle,
        pressed,
        modifiers: egui::Modifiers::NONE,
    }
}

fn wheel(delta: f32) -> egui::Event {
    egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: Vec2::new(0., delta),
        modifiers: egui::Modifiers::NONE,
    }
}

#[test]
fn wheel_anchors_source_position_and_batched_middle_pan_never_edits_masks() {
    for pixel_scale in [1., 2.] {
        let ctx = egui::Context::default();
        ctx.set_pixels_per_point(pixel_scale);
        let source = source();
        let mut state = CleanupState::default();
        state.reset(&source, None);
        let first = settled_frame(&ctx, &mut state, &source);
        let old = image_rect(&first, state.overlay.as_ref().unwrap().id());
        let uv = Vec2::new(0.35, 0.4);
        let anchor = old.min + old.size() * uv;
        let output = frame(
            &ctx,
            &mut state,
            &source,
            vec![egui::Event::PointerMoved(anchor), wheel(200.)],
        );
        let zoomed = image_rect(&output, state.overlay.as_ref().unwrap().id());
        assert!(state.viewport.zoom > 1.);
        assert!((zoomed.min + zoomed.size() * uv - anchor).length() <= 1. / pixel_scale);
        // Drain egui's wheel smoothing before checking an exact middle-drag delta.
        for _ in 0..100 {
            frame(&ctx, &mut state, &source, vec![]);
        }
        let pan_before = state.viewport.pan;
        let before = state.mask(&source).unwrap();
        let end = anchor + Vec2::new(30., 12.);
        frame(
            &ctx,
            &mut state,
            &source,
            vec![
                egui::Event::PointerMoved(anchor),
                middle(anchor, true),
                egui::Event::PointerMoved(end),
                middle(end, false),
            ],
        );
        assert!((state.viewport.pan - pan_before - Vec2::new(30., 12.)).length() < 1e-4);
        assert_eq!(state.mask(&source).unwrap(), before);
        assert!(state.seed.is_none());
        assert!(state.undo.is_empty());
    }
}

#[test]
fn zoomed_gesture_freezes_mapping_keeps_exact_seed_and_finishes_outside_at_two_dpi_scales() {
    let mut masks = vec![];
    for pixel_scale in [1., 2.] {
        let ctx = egui::Context::default();
        ctx.set_pixels_per_point(pixel_scale);
        let source = source();
        let mut state = CleanupState::default();
        state.reset(&source, None);
        state.viewport.zoom = 2.;
        state.tolerance = 2.;
        let first = settled_frame(&ctx, &mut state, &source);
        let rect = image_rect(&first, state.overlay.as_ref().unwrap().id());
        let start = rect.min + rect.size() * Vec2::new(2.5 / 64., 4.5 / 32.);
        frame(
            &ctx,
            &mut state,
            &source,
            vec![egui::Event::PointerMoved(start), button(start, true)],
        );
        assert_eq!(state.seed, Some([2, 4]));
        assert_eq!(state.cache.as_ref().unwrap().seed_rgb, [84; 3]);
        let narrow = state.preview_selection.clone();
        let frozen = state.viewport.frozen_image.unwrap();
        frame(
            &ctx,
            &mut state,
            &source,
            vec![
                wheel(200.),
                middle(start, true),
                egui::Event::PointerMoved(start + Vec2::new(60., 0.)),
            ],
        );
        assert_eq!(state.viewport.zoom, 2.);
        assert_eq!(state.viewport.pan, Vec2::ZERO);
        assert_eq!(state.viewport.frozen_image, Some(frozen));
        assert_eq!(state.gesture.as_ref().unwrap().seed, [2, 4]);
        frame(
            &ctx,
            &mut state,
            &source,
            vec![middle(start, false), egui::Event::PointerMoved(start)],
        );
        assert_eq!(state.tolerance, 2.);
        assert_eq!(state.preview_selection, narrow);
        let outside = rect.max + Vec2::new(100., 100.);
        frame(
            &ctx,
            &mut state,
            &source,
            vec![egui::Event::PointerMoved(outside), button(outside, false)],
        );
        assert!(state.viewport.frozen_image.is_none());
        assert!(state.gesture.is_none());
        wait_refine(&mut state, &ctx, &source);
        let expected = SelectionCache::new(&source, [2, 4])
            .unwrap()
            .candidates(state.tolerance, SelectionMode::Connected);
        assert_eq!(state.selection, expected);
        masks.push(state.selection.clone());
    }
    assert_eq!(masks[0], masks[1]);
}

#[test]
fn zoomed_image_cannot_start_selection_outside_canvas_clip() {
    let ctx = egui::Context::default();
    let source = source();
    let mut state = CleanupState::default();
    state.reset(&source, None);
    state.viewport.zoom = 16.;
    let first = settled_frame(&ctx, &mut state, &source);
    let rect = image_rect(&first, state.overlay.as_ref().unwrap().id());
    let outside_canvas = rect.min + Vec2::splat(1.);
    assert!(rect.contains(outside_canvas));
    frame(
        &ctx,
        &mut state,
        &source,
        vec![
            egui::Event::PointerMoved(outside_canvas),
            button(outside_canvas, true),
            button(outside_canvas, false),
        ],
    );
    assert!(state.seed.is_none());
    assert!(state.gesture.is_none());
    assert!(state.refining.is_none());
}
