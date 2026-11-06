//! Nondestructive colour selection. Preview is bounded; final masks use source pixels.
#[cfg(test)]
#[path = "cleanup_ui_tests.rs"]
mod tests;
use super::*;
use embroidery_lab::color_selection::{
    self as color, CleanupMask, SelectionCache, SelectionMode, SelectionOp,
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc,
};
use ui_theme::{Icon, action_button, icon_button};

#[derive(Clone, Copy, PartialEq, Default)]
enum Brush {
    #[default]
    Select,
    Protect,
    Restore,
}

struct Gesture {
    origin: Pos2,
    initial_tolerance: f32,
    seed: [u32; 2],
    base: Vec<bool>,
    preview_base: Vec<bool>,
    op: SelectionOp,
}

/// View-only transform in logical points. Source masks never use preview pixels.
struct PreviewViewport {
    zoom: f32,
    pan: Vec2,
    middle_position: Option<Pos2>,
    frozen_image: Option<Rect>,
}

impl Default for PreviewViewport {
    fn default() -> Self {
        Self {
            zoom: 1.,
            pan: Vec2::ZERO,
            middle_position: None,
            frozen_image: None,
        }
    }
}

impl PreviewViewport {
    fn image_rect(&self, canvas: Rect, fit_size: Vec2) -> Rect {
        self.frozen_image.unwrap_or_else(|| {
            Rect::from_center_size(canvas.center() + self.pan, fit_size * self.zoom)
        })
    }

    fn zoom_at(&mut self, canvas: Rect, fit_size: Vec2, pointer: Pos2, factor: f32) {
        let old = self.image_rect(canvas, fit_size);
        let uv = (pointer - old.min) / old.size();
        self.zoom = (self.zoom * factor).clamp(0.25, 16.);
        let center = pointer - (uv - Vec2::splat(0.5)) * fit_size * self.zoom;
        self.pan = center - canvas.center();
    }
}

pub(super) struct PreviewLayers<'a> {
    regions: Option<&'a egui::TextureHandle>,
    changes: Option<&'a egui::TextureHandle>,
    plan: Option<&'a MachinePlan>,
    width_mm: f64,
}

type SelectionReply = (u64, Result<Vec<bool>, String>);

/// Pixel work runs in the digitizing worker; the UI only uploads its texture.
pub(super) fn preparation_change_overlay(
    report: &import::RasterTraceReport,
    source: &image::RgbaImage,
) -> Option<image::RgbaImage> {
    let [w, h] = report.trace_dimensions;
    let count = w as usize * h as usize;
    if count == 0
        || source.width() == 0
        || source.height() == 0
        || report.labels_before_preparation.len() != count
        || report.labels_after_preparation.len() != count
    {
        return None;
    }
    Some(image::RgbaImage::from_fn(w, h, |x, y| {
        let sx = ((u64::from(x) * 2 + 1) * u64::from(source.width()) / (u64::from(w) * 2)) as u32;
        let sy = ((u64::from(y) * 2 + 1) * u64::from(source.height()) / (u64::from(h) * 2)) as u32;
        let mut pixel = *source.get_pixel(sx.min(source.width() - 1), sy.min(source.height() - 1));
        let i = (y * w + x) as usize;
        if report.labels_before_preparation[i] == 255 {
            pixel[3] = 0;
        } else if report.labels_before_preparation[i] != report.labels_after_preparation[i] {
            pixel = image::Rgba([255, 130, 25, 255]);
        }
        pixel
    }))
}

pub(super) struct CleanupState {
    preview: image::RgbaImage,
    pub excluded: Vec<bool>,
    protected: Vec<bool>,
    selection: Vec<bool>,
    preview_selection: Vec<bool>,
    cache: Option<SelectionCache>,
    gesture: Option<Gesture>,
    last_base: Vec<bool>,
    last_preview_base: Vec<bool>,
    last_op: SelectionOp,
    seed: Option<[u32; 2]>,
    tolerance: f32,
    mode: SelectionMode,
    op: SelectionOp,
    brush: Brush,
    brush_radius: f32,
    brush_before: Option<CleanupMask>,
    undo: Vec<CleanupMask>,
    redo: Vec<CleanupMask>,
    overlay: Option<egui::TextureHandle>,
    revision: u64,
    refining: Option<mpsc::Receiver<SelectionReply>>,
    refine_cancel: Arc<AtomicBool>,
    halo: Option<Vec<bool>>,
    pub view: u8,
    pub reimport: bool,
    pub reset_manual_decisions: bool,
    pub changed: bool,
    error: Option<String>,
    viewport: PreviewViewport,
}

impl Default for CleanupState {
    fn default() -> Self {
        Self {
            preview: image::RgbaImage::new(1, 1),
            excluded: vec![],
            protected: vec![],
            selection: vec![],
            preview_selection: vec![],
            cache: None,
            gesture: None,
            last_base: vec![],
            last_preview_base: vec![],
            last_op: SelectionOp::Replace,
            seed: None,
            tolerance: 8.,
            mode: SelectionMode::Connected,
            op: SelectionOp::Replace,
            brush: Brush::Select,
            brush_radius: 8.,
            brush_before: None,
            undo: vec![],
            redo: vec![],
            overlay: None,
            revision: 0,
            refining: None,
            refine_cancel: Arc::new(AtomicBool::new(false)),
            halo: None,
            view: 0,
            reimport: false,
            reset_manual_decisions: false,
            changed: false,
            error: None,
            viewport: PreviewViewport::default(),
        }
    }
}

impl Drop for CleanupState {
    fn drop(&mut self) {
        self.refine_cancel.store(true, Ordering::Relaxed);
    }
}

impl CleanupState {
    pub fn reset(&mut self, source: &image::RgbaImage, mask: Option<&CleanupMask>) {
        self.refine_cancel.store(true, Ordering::Relaxed);
        *self = Self::default();
        self.preview = color::thumbnail_rgba(source, 512);
        let count = source.len() / 4;
        (self.excluded, self.protected) = mask
            .and_then(|m| m.to_masks().ok())
            .filter(|(m, _)| m.len() == count)
            .unwrap_or((vec![false; count], vec![false; count]));
        self.selection = vec![false; count];
        self.preview_selection = vec![false; self.preview.len() / 4];
    }

    pub fn mask(&self, source: &image::RgbaImage) -> Result<CleanupMask, String> {
        CleanupMask::from_masks(
            source.width(),
            source.height(),
            &self.excluded,
            &self.protected,
        )
    }

    fn remember(&mut self, before: CleanupMask) {
        self.undo.push(before);
        if self.undo.len() > 30 {
            self.undo.remove(0);
        }
        self.redo.clear();
        self.changed = true;
        self.halo = None;
    }

    fn reduce_mask(&self, mask: &[bool], source: &image::RgbaImage) -> Vec<bool> {
        (0..self.preview.height())
            .flat_map(|y| {
                (0..self.preview.width()).map(move |x| {
                    let sx = x as usize * source.width() as usize / self.preview.width() as usize;
                    let sy = y as usize * source.height() as usize / self.preview.height() as usize;
                    mask[sy * source.width() as usize + sx]
                })
            })
            .collect()
    }

    fn update_preview(&mut self, source: &image::RgbaImage) {
        if let Some(cache) = &self.cache {
            let protected = self.reduce_mask(&self.protected, source);
            let candidates =
                cache.candidates_with_protection(self.tolerance, self.mode, &protected);
            let base = self
                .gesture
                .as_ref()
                .map(|g| g.preview_base.as_slice())
                .unwrap_or(&self.last_preview_base);
            let op = self.gesture.as_ref().map_or(self.last_op, |g| g.op);
            let fallback = vec![false; candidates.len()];
            self.preview_selection = color::combine_selection(
                if base.is_empty() { &fallback } else { base },
                &candidates,
                op,
            )
            .unwrap_or(candidates);
        }
        self.overlay = None;
    }

    fn refine(
        &mut self,
        source: Arc<image::RgbaImage>,
        base: Vec<bool>,
        seed: [u32; 2],
        op: SelectionOp,
    ) {
        self.refine_cancel.store(true, Ordering::Relaxed);
        self.refine_cancel = Arc::new(AtomicBool::new(false));
        let cancel = self.refine_cancel.clone();
        self.revision = self.revision.wrapping_add(1);
        let revision = self.revision;
        let tolerance = self.tolerance;
        let mode = self.mode;
        let protected = self.protected.clone();
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let result =
                SelectionCache::new_cancellable(&source, seed, &cancel).and_then(|cache| {
                    let candidates = cache.candidates_with_protection_cancellable(
                        tolerance, mode, &protected, &cancel,
                    )?;
                    color::combine_selection(&base, &candidates, op)
                });
            let _ = sender.send((revision, result));
        });
        self.refining = Some(receiver);
    }

    fn poll(&mut self, ctx: &egui::Context, source: &image::RgbaImage) {
        let result = self.refining.as_ref().and_then(|rx| match rx.try_recv() {
            Ok(result) => Some(result),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => Some((
                self.revision,
                Err("Selection worker stopped. The cleanup mask is unchanged.".into()),
            )),
        });
        if let Some((revision, result)) = result {
            self.refining = None;
            if revision == self.revision {
                match result {
                    Ok(selection) => {
                        self.preview_selection = self.reduce_mask(&selection, source);
                        self.selection = selection;
                        self.overlay = None;
                        self.error = None;
                    }
                    Err(error) => {
                        self.error = Some(error);
                    }
                }
            }
        }
        if self.refining.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
        }
    }

    fn can_exclude(&self) -> bool {
        self.halo
            .as_ref()
            .unwrap_or(&self.selection)
            .iter()
            .zip(&self.excluded)
            .zip(&self.protected)
            .any(|((&selected, &excluded), &protected)| selected && !excluded && !protected)
    }

    fn apply_exclusion(&mut self, source: &image::RgbaImage) {
        if self.refining.is_some() || self.gesture.is_some() || !self.can_exclude() {
            return;
        }
        if let Ok(before) = self.mask(source) {
            let chosen = self.halo.as_ref().unwrap_or(&self.selection);
            for ((excluded, protected), selected) in
                self.excluded.iter_mut().zip(&self.protected).zip(chosen)
            {
                if *selected && !*protected {
                    *excluded = true;
                }
            }
            self.remember(before);
            self.selection.fill(false);
            self.preview_selection.fill(false);
            self.overlay = None;
        }
    }

    fn undo(&mut self, source: &image::RgbaImage, redo: bool) {
        let from = if redo { &mut self.redo } else { &mut self.undo };
        let Some(previous) = from.pop() else {
            return;
        };
        if let Ok(current) = self.mask(source) {
            if redo {
                self.undo.push(current);
            } else {
                self.redo.push(current);
            }
            if let Ok((excluded, protected)) = previous.to_masks() {
                self.excluded = excluded;
                self.protected = protected;
            }
            self.changed = true;
            self.overlay = None;
            self.halo = None;
        }
    }

    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        source: Arc<image::RgbaImage>,
        texture: &egui::TextureHandle,
        layers: PreviewLayers<'_>,
        language: Language,
    ) -> Option<[f32; 2]> {
        let mut clicked = None;
        let width = ui.available_width();
        if width >= 840. {
            let tools_width = 292.;
            let preview_width = (width - tools_width - 24.).max(240.);
            ui.horizontal_top(|ui| {
                ui.allocate_ui_with_layout(
                    Vec2::new(preview_width, 0.),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.set_width(preview_width);
                        clicked = self.show_preview(ui, source.clone(), texture, layers, language);
                    },
                );
                ui.separator();
                ui.allocate_ui_with_layout(
                    Vec2::new(tools_width, 0.),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.set_width(tools_width);
                        self.selection_controls(ui, source.clone(), language);
                    },
                );
            });
        } else {
            clicked = self.show_preview(ui, source.clone(), texture, layers, language);
            ui.add_space(10.);
            ui.separator();
            self.selection_controls(ui, source, language);
        }
        clicked
    }

    fn show_preview(
        &mut self,
        ui: &mut egui::Ui,
        source: Arc<image::RgbaImage>,
        texture: &egui::TextureHandle,
        layers: PreviewLayers<'_>,
        language: Language,
    ) -> Option<[f32; 2]> {
        let PreviewLayers {
            regions,
            changes,
            plan,
            width_mm,
        } = layers;
        self.poll(ui.ctx(), &source);
        let old_view = self.view;
        let mut region_click = None;
        let viewport_locked = self.gesture.is_some() || self.brush_before.is_some();
        ui.horizontal_wrapped(|ui| {
            for (i, label) in ["Source", "Mask", "Regions", "Stitches", "Changes"]
                .iter()
                .enumerate()
            {
                if ui
                    .add(egui::Button::selectable(
                        self.view == i as u8,
                        language.text(label),
                    ))
                    .clicked()
                {
                    self.view = i as u8;
                }
            }
            ui.separator();
            if icon_button(ui, Icon::Fit, false, !viewport_locked, language.text("Fit")).clicked() {
                self.viewport = PreviewViewport::default();
            }
            ui.small(format!("{:.0}%", self.viewport.zoom * 100.))
                .on_hover_text(
                    language.text("Zoom relative to Fit. Wheel: zoom · Middle drag: pan."),
                );
        });
        if old_view != self.view {
            self.overlay = None;
        }
        let preview_size = self.preview.dimensions();
        if self.overlay.is_none() {
            let excluded = self.reduce_mask(&self.excluded, &source);
            let protected = self.reduce_mask(&self.protected, &source);
            let mut pixels = self.preview.clone();
            for (i, pixel) in pixels.pixels_mut().enumerate() {
                if self.view == 1 {
                    let shade = if excluded[i] { 40 } else { 220 };
                    *pixel = image::Rgba([shade, shade, shade, 255]);
                } else if excluded[i] {
                    pixel[0] = 235;
                    pixel[1] /= 3;
                    pixel[2] /= 3;
                    pixel[3] = 180;
                }
                if self.preview_selection[i] {
                    pixel[0] /= 2;
                    pixel[1] = 160;
                    pixel[2] = 245;
                    pixel[3] = 220;
                }
                if protected[i] {
                    pixel[0] /= 2;
                    pixel[1] = 200;
                    pixel[2] /= 2;
                    pixel[3] = 235;
                }
            }
            self.overlay = Some(ui.ctx().load_texture(
                "cleanup mask",
                egui::ColorImage::from_rgba_unmultiplied(
                    [preview_size.0 as usize, preview_size.1 as usize],
                    pixels.as_raw(),
                ),
                egui::TextureOptions::LINEAR,
            ));
        }
        let texture = if self.view == 2 {
            regions.unwrap_or(texture)
        } else if self.view == 4 {
            changes.unwrap_or(texture)
        } else if self.view == 3 {
            texture
        } else {
            self.overlay.as_ref().unwrap_or(texture)
        };
        // All comparison layers share the source's fit scale and viewport,
        // regardless of the resolution used to trace the region texture.
        let size = source.dimensions();
        let size = Vec2::new(size.0 as f32, size.1 as f32);
        let canvas_height = (ui.ctx().content_rect().height() - 360.).clamp(160., 420.);
        let canvas_width = ui.available_width().max(100.);
        let size = size
            * ((canvas_width - 16.) / size.x)
                .min((canvas_height - 16.) / size.y)
                .min(1.);
        let (canvas, _) =
            ui.allocate_exact_size(Vec2::new(canvas_width, canvas_height), Sense::hover());
        let theme = if ui.visuals().dark_mode {
            ui_theme::Theme::Dark
        } else {
            ui_theme::Theme::Light
        };
        let colors = theme.tokens();
        ui.painter().rect_filled(canvas, 0., colors.canvas);
        let checker = 12.;
        for y in 0..(canvas.height() / checker).ceil() as usize {
            for x in 0..(canvas.width() / checker).ceil() as usize {
                if (x + y) % 2 == 0 {
                    let rect = Rect::from_min_size(
                        canvas.min + Vec2::new(x as f32 * checker, y as f32 * checker),
                        Vec2::splat(checker),
                    )
                    .intersect(canvas);
                    ui.painter().rect_filled(rect, 0., colors.grid);
                }
            }
        }
        let visible_canvas = canvas.intersect(ui.clip_rect());
        let pointer = ui.input(|i| i.pointer.interact_pos());
        let primary_pressed = ui.input(|i| i.pointer.button_pressed(egui::PointerButton::Primary));
        let middle_press = ui.input(|i| {
            i.events.iter().find_map(|event| match event {
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Middle,
                    pressed: true,
                    ..
                } => Some(*pos),
                _ => None,
            })
        });
        let over_canvas = pointer.is_some_and(|p| visible_canvas.contains(p));
        if over_canvas || viewport_locked {
            let scroll = ui.input_mut(|i| {
                let scroll = i.smooth_scroll_delta.y;
                i.smooth_scroll_delta = Vec2::ZERO;
                i.raw_scroll_delta = Vec2::ZERO;
                scroll
            });
            if !viewport_locked
                && !primary_pressed
                && self.viewport.middle_position.is_none()
                && let Some(pointer) = pointer
                && scroll != 0.
            {
                self.viewport
                    .zoom_at(canvas, size, pointer, (scroll * 0.003).exp());
            }
        }
        if !viewport_locked && !primary_pressed {
            if let Some(pos) = middle_press.filter(|p| visible_canvas.contains(*p)) {
                self.viewport.middle_position = Some(pos);
            }
            if let (Some(previous), Some(pos)) = (self.viewport.middle_position, pointer) {
                self.viewport.pan += pos - previous;
                self.viewport.middle_position = Some(pos);
            }
        }
        let middle_down = ui.input(|i| i.pointer.button_down(egui::PointerButton::Middle));
        let panning = self.viewport.middle_position.is_some();
        if !middle_down {
            self.viewport.middle_position = None;
        }
        let image_rect = self.viewport.image_rect(canvas, size);
        let old_clip = ui.clip_rect();
        ui.set_clip_rect(visible_canvas);
        let response = ui.place(
            image_rect,
            egui::Image::new((texture.id(), image_rect.size())).sense(Sense::click_and_drag()),
        );
        ui.set_clip_rect(old_clip);
        ui.painter().rect_stroke(
            canvas,
            0.,
            Stroke::new(1.0_f32, colors.border),
            egui::StrokeKind::Inside,
        );
        if self.view == 2
            && response.clicked()
            && let Some(pos) = response.interact_pointer_pos()
            && visible_canvas.contains(pos)
        {
            let uv = (pos - response.rect.min) / response.rect.size();
            region_click = Some([uv.x, uv.y]);
        }
        if self.view == 3 {
            let painter = ui.painter().with_clip_rect(visible_canvas);
            painter.rect_filled(response.rect, 0., colors.canvas);
            if let Some(plan) = plan {
                let mut previous = Point::default();
                let width = source.width() as f64;
                let project_width = width_mm;
                let project_height = project_width * source.height() as f64 / width;
                let screen = |p: Point| {
                    response.rect.min
                        + Vec2::new(
                            (p.x / project_width) as f32 * response.rect.width(),
                            (p.y / project_height) as f32 * response.rect.height(),
                        )
                };
                for stitch in &plan.stitches {
                    if matches!(stitch.command, Command::Stitch | Command::Jump) {
                        let color = if stitch.command == Command::Jump {
                            colors.warning
                        } else {
                            Color32::from_rgb(stitch.color[0], stitch.color[1], stitch.color[2])
                        };
                        painter.line_segment(
                            [screen(previous), screen(stitch.point())],
                            Stroke::new(0.6_f32, color),
                        );
                    }
                    previous = stitch.point();
                }
            } else {
                ui.label(language.text("Preview regions to inspect actual stitches."));
            }
        }
        let pointer = ui.input(|i| i.pointer.interact_pos());
        let press_position = ui.input(|i| {
            i.events.iter().find_map(|event| match event {
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    ..
                } => Some(*pos),
                _ => None,
            })
        });
        let pressed = ui.input(|i| i.pointer.button_pressed(egui::PointerButton::Primary));
        let down = ui.input(|i| i.pointer.primary_down());
        let released = ui.input(|i| i.pointer.button_released(egui::PointerButton::Primary));
        if self.view < 2
            && pressed
            && !panning
            && let Some(pos) =
                press_position.filter(|p| response.rect.contains(*p) && visible_canvas.contains(*p))
        {
            self.viewport.frozen_image = Some(image_rect);
            let uv = (pos - response.rect.min) / response.rect.size();
            let seed = [
                (uv.x * source.width() as f32) as u32,
                (uv.y * source.height() as f32) as u32,
            ];
            let seed = [
                seed[0].min(source.width() - 1),
                seed[1].min(source.height() - 1),
            ];
            self.seed = Some(seed);
            self.revision = self.revision.wrapping_add(1);
            self.refining = None;
            self.refine_cancel.store(true, Ordering::Relaxed);
            if self.brush == Brush::Select {
                let preview_seed = [
                    (uv.x * preview_size.0 as f32) as u32,
                    (uv.y * preview_size.1 as f32) as u32,
                ];
                // Exact original reference colour, even when the preview is downsampled.
                let mut preview = self.preview.clone();
                let ps = [
                    preview_seed[0].min(preview.width() - 1),
                    preview_seed[1].min(preview.height() - 1),
                ];
                preview.put_pixel(ps[0], ps[1], *source.get_pixel(seed[0], seed[1]));
                match SelectionCache::new(&preview, ps) {
                    Ok(cache) => {
                        self.cache = Some(cache);
                        self.error = None;
                    }
                    Err(error) => {
                        self.cache = None;
                        self.error = Some(error);
                    }
                }
                let modifiers = ui.input(|i| i.modifiers);
                let op = if modifiers.alt {
                    SelectionOp::Subtract
                } else if modifiers.shift {
                    SelectionOp::Add
                } else {
                    self.op
                };
                self.last_base = self.selection.clone();
                self.last_preview_base = self.preview_selection.clone();
                self.last_op = op;
                self.gesture = Some(Gesture {
                    origin: pos,
                    initial_tolerance: self.tolerance,
                    seed,
                    base: self.selection.clone(),
                    preview_base: self.preview_selection.clone(),
                    op,
                });
                self.update_preview(&source);
            } else {
                self.brush_before = self.mask(&source).ok();
            }
        }
        if down || released {
            if let (Some(gesture), Some(pos)) = (&self.gesture, pointer) {
                self.tolerance = color::gesture_tolerance(
                    gesture.initial_tolerance,
                    gesture.origin.distance(pos),
                );
                self.update_preview(&source);
            } else if self.brush_before.is_some()
                && let Some(pos) =
                    pointer.filter(|p| response.rect.contains(*p) && visible_canvas.contains(*p))
            {
                let uv = (pos - response.rect.min) / response.rect.size();
                let center = [uv.x * source.width() as f32, uv.y * source.height() as f32];
                let radius = self.brush_radius * source.width() as f32 / response.rect.width();
                for y in ((center[1] - radius).max(0.) as u32)
                    ..=((center[1] + radius).min((source.height() - 1) as f32) as u32)
                {
                    for x in ((center[0] - radius).max(0.) as u32)
                        ..=((center[0] + radius).min((source.width() - 1) as f32) as u32)
                    {
                        if (x as f32 - center[0]).hypot(y as f32 - center[1]) <= radius {
                            let i = (y * source.width() + x) as usize;
                            self.excluded[i] = false;
                            self.protected[i] = self.brush == Brush::Protect;
                        }
                    }
                }
                self.overlay = None;
            }
        }
        let escape = ui.input(|i| i.key_pressed(egui::Key::Escape));
        if !down && !escape {
            if let Some(gesture) = self.gesture.take() {
                self.refine(source.clone(), gesture.base, gesture.seed, gesture.op);
            }
            if let Some(before) = self.brush_before.take() {
                self.remember(before);
            }
            self.viewport.frozen_image = None;
        }
        if escape {
            if let Some(gesture) = self.gesture.take() {
                self.selection = gesture.base;
                self.preview_selection = gesture.preview_base;
                self.tolerance = gesture.initial_tolerance;
            }
            if let Some(before) = self.brush_before.take()
                && let Ok((ex, pr)) = before.to_masks()
            {
                self.excluded = ex;
                self.protected = pr;
            }
            self.revision = self.revision.wrapping_add(1);
            self.refining = None;
            self.halo = None;
            self.overlay = None;
            self.refine_cancel.store(true, Ordering::Relaxed);
            self.cache = None;
            self.seed = None;
            self.viewport.frozen_image = None;
        }
        let selected = if self.refining.is_some() || self.gesture.is_some() {
            self.preview_selection.iter().filter(|&&v| v).count() as f64 * source.len() as f64
                / self.preview.len() as f64
        } else {
            self.selection.iter().filter(|&&v| v).count() as f64
        };
        ui.label(format!(
            "{}: {:.0} px · {}: {:.1}",
            language.text("Selected"),
            selected,
            language.text("Tolerance"),
            self.tolerance
        ));
        if self.refining.is_some() {
            ui.colored_label(
                colors.warning,
                language.text("Refining selection at source resolution…"),
            );
        }
        ui.horizontal_wrapped(|ui| {
            for (color, label) in [
                (Color32::from_rgb(235, 70, 70), "Excluded"),
                (Color32::from_rgb(40, 160, 245), "Selected"),
                (Color32::from_rgb(40, 200, 70), "Protected"),
            ] {
                let swatch = ui.allocate_exact_size(Vec2::splat(8.), Sense::hover()).0;
                ui.painter().rect_filled(swatch, 0., color);
                ui.small(language.text(label));
            }
        });
        if old_view != self.view {
            self.overlay = None;
        }
        region_click
    }

    fn selection_controls(
        &mut self,
        ui: &mut egui::Ui,
        source: Arc<image::RgbaImage>,
        language: Language,
    ) {
        ui.label(egui::RichText::new(language.text("Colour cleanup")).strong());
        ui.add(egui::Label::new(language.text("Press a colour and drag to widen tolerance. Return to narrow. Shift: add · Alt: subtract · Esc: cancel.")).wrap());
        ui.add_space(6.);
        ui.horizontal_wrapped(|ui| {
            ui.selectable_value(
                &mut self.brush,
                Brush::Select,
                language.text("Select by colour"),
            );
            ui.selectable_value(
                &mut self.brush,
                Brush::Protect,
                language.text("Protect brush"),
            );
            ui.selectable_value(
                &mut self.brush,
                Brush::Restore,
                language.text("Restore brush"),
            );
        });
        if self.brush != Brush::Select {
            ui.label(language.text("Brush radius (screen points)"));
            ui.add(egui::Slider::new(&mut self.brush_radius, 2. ..=30.));
        }
        let old_mode = self.mode;
        ui.add_space(6.);
        for (mode, label) in [
            (SelectionMode::Connected, "Connected region"),
            (SelectionMode::Exterior, "Exterior background"),
            (SelectionMode::Global, "All similar colours"),
        ] {
            ui.radio_value(&mut self.mode, mode, language.text(label));
        }
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.op, SelectionOp::Replace, language.text("Replace"));
            ui.selectable_value(&mut self.op, SelectionOp::Add, language.text("Add"));
            ui.selectable_value(
                &mut self.op,
                SelectionOp::Subtract,
                language.text("Subtract"),
            );
        });
        ui.add_space(6.);
        ui.label(language.text("Tolerance (OKLab ×100)"));
        let slider = ui.add(egui::Slider::new(&mut self.tolerance, 0. ..=100.));
        if slider.changed() {
            self.update_preview(&source);
        }
        if old_mode != self.mode {
            self.update_preview(&source);
        }
        if (old_mode != self.mode
            || slider.drag_stopped()
            || (slider.changed() && !slider.dragged()))
            && let Some(seed) = self.seed
        {
            self.refine(source.clone(), self.last_base.clone(), seed, self.last_op);
        }
        let no_gesture = self.gesture.is_none() && self.refining.is_none();
        let can_exclude = no_gesture && self.can_exclude();
        let text_input = keyboard_focus::text_input_active(ui.ctx());
        let delete = !text_input
            && can_exclude
            && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Delete));
        ui.horizontal_wrapped(|ui| {
            if action_button(
                ui,
                Icon::Trash,
                can_exclude,
                language.text("Exclude from digitizing"),
            )
            .on_disabled_hover_text(
                language.text("Select unprotected pixels that are not already excluded."),
            )
            .clicked()
                || delete
            {
                self.apply_exclusion(&source);
            }
            if icon_button(
                ui,
                Icon::Undo,
                false,
                !self.undo.is_empty(),
                language.text("Undo"),
            )
            .clicked()
            {
                self.undo(&source, false);
            }
            if icon_button(
                ui,
                Icon::Redo,
                false,
                !self.redo.is_empty(),
                language.text("Redo"),
            )
            .clicked()
            {
                self.undo(&source, true);
            }
        });
        if !text_input && no_gesture {
            let (redo, undo) = ui.input_mut(|i| {
                let redo = i.consume_key(
                    egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
                    egui::Key::Z,
                ) || i.consume_key(egui::Modifiers::COMMAND, egui::Key::Y);
                let undo = !redo && i.consume_key(egui::Modifiers::COMMAND, egui::Key::Z);
                (redo, undo)
            });
            if redo || undo {
                self.undo(&source, redo);
            }
        }
        ui.add_space(6.);
        ui.collapsing(language.text("Light fringe"), |ui| {
            if ui
                .button(language.text("Preview light fringe removal"))
                .clicked()
            {
                self.halo = color::light_halo_preview(
                    &source,
                    &self.excluded,
                    &self.protected,
                    2,
                    self.tolerance.min(25.),
                )
                .ok();
                if let Some(halo) = &self.halo {
                    self.preview_selection = self.reduce_mask(halo, &source);
                    self.overlay = None;
                }
            }
            if self.halo.is_some() && ui.button(language.text("Apply fringe removal")).clicked() {
                self.apply_exclusion(&source);
            }
        });
        if let Some(error) = &self.error {
            ui.colored_label(
                ui.visuals().error_fg_color,
                language.message(error).as_ref(),
            );
        }
    }
}

impl Editor {
    pub(super) fn clear_import_diagnostics(&mut self) {
        self.import_trace_report = None;
        self.import_changes_texture = None;
        self.import_plan = None;
        self.import_preview_fingerprint = None;
    }

    pub(super) fn set_import_trace_report(
        &mut self,
        ctx: &egui::Context,
        report: import::RasterTraceReport,
        overlay: Option<image::RgbaImage>,
    ) {
        self.import_changes_texture = overlay.map(|pixels| {
            ctx.load_texture(
                "preparation changes",
                egui::ColorImage::from_rgba_unmultiplied(
                    [pixels.width() as usize, pixels.height() as usize],
                    pixels.as_raw(),
                ),
                egui::TextureOptions::NEAREST,
            )
        });
        self.import_trace_report = Some(report);
    }

    pub(super) fn autodigitize_import_window(&mut self, ctx: &egui::Context) {
        let Some(preview) = &self.import_preview else {
            return;
        };
        let pixels = preview.pixels.clone();
        let texture = preview.texture.clone();
        let file_name = preview
            .path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let file_path = preview.path.display().to_string();
        let language = self.language;
        let colors = self.workspace.theme.tokens();
        let mut action = 0;
        egui::Modal::new(egui::Id::new("trace-artwork")).show(ctx, |ui| {
            ui.set_width((ctx.content_rect().width() - 48.).clamp(240., 1120.));
            ui.horizontal(|ui| {
                ui.heading(language.text("Auto Digitize"));
                ui.add_space(10.);
                ui.add(egui::Label::new(egui::RichText::new(&file_name).color(colors.muted)).truncate())
                    .on_hover_text(&file_path);
            });
            if let Some(pdf) = self.import_preview.as_ref().and_then(|p| p.pdf.as_ref()) {
                ui.horizontal(|ui| {
                    ui.label(format!("{} {} / {}", language.text("PDF page"), pdf.page_index + 1, pdf.page_count));
                    let mut page = pdf.page_index + 1;
                    if ui.add(egui::DragValue::new(&mut page).range(1..=pdf.page_count)).changed() {
                        action = 10 + page;
                    }
                });
            }
            ui.separator();
            let pdf_height = if self.import_preview.as_ref().is_some_and(|p| p.pdf.is_some()) { 32. } else { 0. };
            let error_height = if self.error { 58. } else { 0. };
            let body_height = (ctx.content_rect().height() - 220. - pdf_height - error_height).max(100.);
            egui::ScrollArea::vertical().id_salt("digitize-workspace").max_height(body_height)
                .auto_shrink([false, true]).show(ui, |ui| {
                    ui.columns(3, |columns| {
                        columns[0].label(language.text("Physical width (mm)"));
                        columns[0].add(egui::DragValue::new(&mut self.import_width_mm).range(5. ..=400.).speed(1.));
                        columns[1].label(language.text("Thread colours"));
                        columns[1].add(egui::Slider::new(&mut self.raster_options.palette_size, 2..=16).text(language.text("colors")));
                        columns[2].checkbox(&mut self.auto_classify, language.text("Choose Running / Satin / Tatami automatically"));
                    });
                    self.import_preparation_controls(ui);
                    ui.add(egui::Label::new(egui::RichText::new(language.text("Profile: flat artwork · experimental fabric settings")).small().color(colors.muted)).wrap())
                        .on_hover_text(language.text("Tatami 0.4 mm · 3.5 mm stitches · underlay 2.0 mm / inset 0.3 mm. Review before sewing."));
                    if self.cleanup.reimport {
                        let mut preserve = !self.cleanup.reset_manual_decisions;
                        if ui.checkbox(&mut preserve, language.text("Keep manual edits when regions match")).changed() {
                            self.cleanup.reset_manual_decisions = !preserve;
                        }
                        if !preserve {
                            ui.colored_label(colors.warning, language.text("Resetting import decisions replaces manual edits. Undo restores the previous project."));
                        }
                    }
                    if self.region_preview.is_some() && self.import_preview_fingerprint.is_some_and(|f| f != self.import_fingerprint()) {
                        ui.colored_label(colors.warning, language.text("Region/stitch preview is stale. Rebuild preview."));
                    }
                    ui.add_space(10.);
                    let region_click = self.cleanup.show(ui, pixels.clone(), &texture, PreviewLayers {
                        regions: self.mask_texture.as_ref(), changes: self.import_changes_texture.as_ref(),
                        plan: self.import_plan.as_ref(), width_mm: self.import_width_mm,
                    }, language);
                    if self.cleanup.view == 4 {
                        ui.small(language.text("Orange: reassigned pixels. Transparent: excluded background. Protect a detail or select Legacy to disable preparation."));
                        if self.import_changes_texture.is_none() {
                            ui.small(language.text("Preview regions to compare preparation changes."));
                        }
                    }
                    if let Some(report) = &self.import_trace_report {
                        ui.label(format!("{}: {} → {} · {}: {} · {}: {}",
                            language.text("Components"), report.components_before_preparation, report.components_after_preparation,
                            language.text("Contours"), report.contour_rings,
                            language.text("Objects"), report.object_count));
                        ui.small(format!("{}: {} px / {:.3} mm² · {}: {} · {}: {}",
                            language.text("Reassigned"), report.preparation_changed_pixels, report.preparation_changed_area_mm2,
                            language.text("Uncertain regions retained"), report.preparation_uncertain_regions,
                            language.text("Omitted regions"), report.omitted_region_count));
                        ui.small(format!("{}: {}", language.text("Residual regions retained"), report.preparation_preserved_residual_regions));
                        ui.collapsing(language.text("Preparation decisions"), |ui| {
                            ui.small(language.text("First reasons partition each pass; additional conditions overlap. Size bins: 1 / 2 / 3–4 / 5+ pixels."));
                            for (index, pass) in report.preparation_diagnostics.passes.iter().enumerate() {
                                ui.label(format!("RAG {}", index+1));
                                for (reason,counts) in &pass.first_reasons { ui.monospace(format!("{reason}: {counts:?}")); }
                            }
                            if let Some(pass)=&report.preparation_diagnostics.context_pass {
                                ui.label(language.text("Contextual sweep"));
                                for (reason,counts) in &pass.first_reasons {ui.monospace(format!("{reason}: {counts:?}"));}
                            }
                            ui.small(format!("{}: {} px", language.text("Next RAG pass (diagnostic only)"), report.preparation_diagnostics.next_pass_changed_pixels));
                        });
                    }
                    if let Some(uv) = region_click && let Some(regions) = &mut self.region_preview {
                        regions.toggle_at(Point::new(uv[0] as f64 * regions.project.width_mm,
                            uv[1] as f64 * regions.project.width_mm * regions.height as f64 / regions.width as f64));
                        self.refresh_mask_texture(ctx);
                    }
                    ui.add_space(10.);
                    ui.separator();
                    self.import_palette_controls(ui);
                    self.import_tracing_controls(ui);
                    if let Some(regions) = self.region_preview.as_ref().map(|r| r.regions.clone()) {
                        egui::CollapsingHeader::new(language.text("Regions and objects"))
                            .default_open(self.cleanup.view == 2).show(ui, |ui| {
                                egui::ScrollArea::vertical().id_salt("import-regions").max_height(160.).show(ui, |ui| {
                                    for region in regions {
                                        ui.horizontal(|ui| {
                                            let swatch = ui.allocate_exact_size(Vec2::splat(12.), Sense::hover()).0;
                                            ui.painter().rect_filled(swatch, 0., Color32::from_rgb(region.color[0], region.color[1], region.color[2]));
                                            let mut included = region.included;
                                            if ui.checkbox(&mut included, format!("#{} · {} px", region.object_id, region.pixel_count))
                                                .on_hover_text(format!("RGB {}, {}, {}", region.color[0], region.color[1], region.color[2])).changed() {
                                                let _ = self.region_preview.as_mut().unwrap().set_included(region.id, included);
                                            }
                                        });
                                    }
                                });
                            });
                    }
                });
            let mask_changed=self.cleanup.changed;
            if mask_changed {
                self.region_preview = None;
                self.mask_texture = None;
                self.clear_import_diagnostics();
                self.cleanup.changed = false;
            }
            if (mask_changed || self.raster_options.cleanup.is_none()) && let Ok(mask) = self.cleanup.mask(&pixels) {
                self.raster_options.cleanup = Some(mask);
            }
            if self.error {
                egui::ScrollArea::vertical().id_salt("import-error").max_height(52.).show(ui, |ui| {
                    ui.colored_label(colors.error, language.message(&self.message).as_ref());
                });
            }
            ui.separator();
            let ready = self.cleanup.refining.is_none() && self.cleanup.gesture.is_none()
                && self.cleanup.brush_before.is_none() && self.digitize_job.is_none();
            if !ready {
                let reason = if self.digitize_job.is_some() { "A digitizing job is running." }
                    else if self.cleanup.refining.is_some() { "Refining selection at source resolution…" }
                    else { "Release the mouse to finish the selection." };
                ui.colored_label(colors.warning, language.text(reason));
            }
            ui.horizontal_wrapped(|ui| {
                let generate = ui.scope(|ui| {
                    let foreground = if ui.visuals().dark_mode { colors.canvas } else { colors.elevated };
                    let widgets = &mut ui.visuals_mut().widgets;
                    for widget in [&mut widgets.inactive, &mut widgets.hovered] {
                        widget.weak_bg_fill = colors.accent;
                        widget.bg_fill = colors.accent;
                        widget.fg_stroke = Stroke::new(1.5_f32, foreground);
                        widget.bg_stroke = Stroke::new(1.0_f32, colors.accent);
                    }
                    action_button(ui, Icon::Wand, ready, language.text("Generate embroidery"))
                }).inner;
                if generate.clicked() { action = 1; }
                if action_button(ui, Icon::Layers, ready, language.text("Preview regions")).clicked() { action = 3; }
                if ui.button(language.text("Cancel")).clicked() { action = 2; }
            });
        });
        match action {
            1 => self.commit_import(ctx),
            2 => self.cancel_import(),
            3 => self.preview_regions(ctx),
            page if page > 10 => {
                let path = self.import_preview.as_ref().unwrap().path.clone();
                self.start_pdf_load(path, page - 11, ctx);
            }
            _ => {}
        }
    }

    fn import_palette_controls(&mut self, ui: &mut egui::Ui) {
        let language = self.language;
        ui.collapsing(language.text("Thread palette"), |ui| {
            if ui
                .add_enabled(
                    self.cleanup.cache.is_some()
                        && self.raster_options.pinned_colors.len()
                            < self.raster_options.palette_size,
                    egui::Button::new(language.text("Pin selected colour")),
                )
                .clicked()
                && let Some(cache) = &self.cleanup.cache
                && !self.raster_options.pinned_colors.contains(&cache.seed_rgb)
            {
                self.raster_options.pinned_colors.push(cache.seed_rgb);
            }
            let mut remove = None;
            egui::ScrollArea::vertical()
                .id_salt("pinned-palette")
                .max_height(100.)
                .show(ui, |ui| {
                    for (i, color) in self.raster_options.pinned_colors.iter_mut().enumerate() {
                        ui.horizontal(|ui| {
                            ui.color_edit_button_srgb(color);
                            ui.label(format!("RGB {}, {}, {}", color[0], color[1], color[2]));
                            if icon_button(ui, Icon::Trash, false, true, language.text("Remove"))
                                .clicked()
                            {
                                remove = Some(i);
                            }
                        });
                    }
                });
            if let Some(i) = remove {
                self.raster_options.pinned_colors.remove(i);
            }
        });
    }

    fn import_preparation_controls(&mut self, ui: &mut egui::Ui) {
        let language = self.language;
        use import::PreparationMode;
        let label = |mode| match mode {
            PreparationMode::Legacy => "Legacy",
            PreparationMode::Gentle => "Gentle",
            PreparationMode::Standard => "Standard",
            PreparationMode::Small => "Small design",
            PreparationMode::Contextual => "Contextual (experimental)",
        };
        let previous = self.raster_options.preparation;
        ui.horizontal_wrapped(|ui| {
            ui.label(language.text("Artwork preparation"));
            egui::ComboBox::from_id_salt("raster-preparation")
                .selected_text(language.text(label(previous)))
                .show_ui(ui, |ui| {
                    for mode in [
                        PreparationMode::Legacy,
                        PreparationMode::Gentle,
                        PreparationMode::Standard,
                        PreparationMode::Small,
                        PreparationMode::Contextual,
                    ] {
                        ui.selectable_value(
                            &mut self.raster_options.preparation,
                            mode,
                            language.text(label(mode)),
                        );
                    }
                });
        });
        if previous != self.raster_options.preparation
            && self.raster_options.preparation != PreparationMode::Legacy
        {
            // A preset must never obtain its improvement from dropping small regions.
            // The operator can still set an explicit threshold under Advanced tracing.
            self.raster_options.min_region_pixels = 1;
            self.raster_options.min_region_area_mm2 = 0.;
            self.raster_options.denoise_passes = 0;
        }
        if self.raster_options.preparation != PreparationMode::Legacy {
            ui.small(language.text("Experimental — requires sew-out. Rebuild the preview after changing size or preparation. Protected pixels and pinned colours are retained."));
        }
    }

    fn import_tracing_controls(&mut self, ui: &mut egui::Ui) {
        let language = self.language;
        ui.collapsing(language.text("Advanced tracing"), |ui| {
            if !matches!(self.raster_options.background, import::BackgroundMode::TransparentOnly) {
                ui.small(language.text("The saved background rule is active in addition to the cleanup mask."));
                if ui.button(language.text("Keep all opaque colours")).clicked() {
                    self.raster_options.background = import::BackgroundMode::TransparentOnly;
                }
            }
            ui.add(egui::Slider::new(&mut self.raster_options.denoise_passes, 0..=2).text(language.text("Isolated noise cleanup")));
            ui.add(egui::Slider::new(&mut self.raster_options.max_dimension, 128..=1024).text(language.text("Trace resolution (longest side)")));
            ui.add(egui::DragValue::new(&mut self.raster_options.min_region_area_mm2).speed(0.01).range(0. ..=5.).prefix(language.text("Minimum area (mm²): ")));
            ui.add(egui::DragValue::new(&mut self.raster_options.simplify_tolerance_mm).speed(0.01).range(0. ..=1.).prefix(language.text("Contour tolerance (mm): ")));
            ui.small(language.text("Shared colour boundaries retain the common pixel boundary. No dithering or automatic removal of white."));
        });
    }
}
