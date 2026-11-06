//! Native workbench chrome. Tokens never transform artwork or thread colours.
use eframe::egui::{
    self, Color32, CornerRadius, FontFamily, FontId, Painter, Pos2, Rect, Response, Shape, Stroke,
    StrokeKind, TextStyle, Ui, Vec2, WidgetInfo, WidgetType, pos2, vec2,
};
use serde::{Deserialize, Serialize};

pub(super) const CONTROL_HEIGHT: f32 = 28.;
pub(super) const SPACE_COMPACT: f32 = 6.;
pub(super) const SPACE_ITEM: f32 = 10.;
pub(super) const SPACE_SECTION: f32 = 16.;
pub(super) const ICON_SIZE: f32 = 18.;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(super) enum Theme {
    #[default]
    Dark,
    Light,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Tokens {
    pub canvas: Color32,
    /// A theme-independent neutral artwork plate keeps thread RGB comparable.
    pub paper: Color32,
    pub paper_grid: Color32,
    pub grid: Color32,
    pub surface: Color32,
    pub panel: Color32,
    pub elevated: Color32,
    pub border: Color32,
    pub text: Color32,
    pub muted: Color32,
    pub accent: Color32,
    pub selection: Color32,
    pub warning: Color32,
    pub error: Color32,
    pub success: Color32,
    pub handle: Color32,
}

impl Theme {
    pub(super) fn tokens(self) -> Tokens {
        let rgb = Color32::from_rgb;
        match self {
            Self::Dark => Tokens {
                canvas: rgb(22, 25, 29),
                paper: rgb(247, 247, 247),
                paper_grid: rgb(224, 224, 224),
                grid: rgb(42, 47, 53),
                surface: rgb(32, 36, 41),
                panel: rgb(28, 32, 37),
                elevated: rgb(43, 49, 56),
                border: rgb(66, 74, 84),
                text: rgb(227, 232, 238),
                muted: rgb(163, 173, 185),
                accent: rgb(85, 197, 216),
                selection: rgb(25, 63, 73),
                warning: rgb(228, 161, 102),
                error: rgb(237, 129, 123),
                success: rgb(131, 188, 155),
                handle: rgb(215, 226, 236),
            },
            Self::Light => Tokens {
                canvas: rgb(237, 240, 243),
                paper: rgb(247, 247, 247),
                paper_grid: rgb(224, 224, 224),
                grid: rgb(210, 216, 222),
                surface: rgb(248, 250, 251),
                panel: rgb(230, 234, 238),
                elevated: rgb(255, 255, 255),
                border: rgb(176, 187, 198),
                text: rgb(32, 42, 52),
                muted: rgb(83, 98, 114),
                accent: rgb(0, 107, 130),
                selection: rgb(211, 234, 240),
                warning: rgb(151, 85, 29),
                error: rgb(174, 55, 57),
                success: rgb(40, 100, 65),
                handle: rgb(51, 78, 104),
            },
        }
    }

    pub(super) fn apply(self, ctx: &egui::Context) {
        let t = self.tokens();
        ctx.set_theme(match self {
            Self::Dark => egui::Theme::Dark,
            Self::Light => egui::Theme::Light,
        });
        let mut style = (*ctx.style()).clone();
        style.text_styles = [
            (TextStyle::Body, FontId::proportional(13.)),
            (TextStyle::Button, FontId::proportional(13.)),
            (TextStyle::Small, FontId::proportional(11.)),
            (TextStyle::Heading, FontId::proportional(16.)),
            (
                TextStyle::Monospace,
                FontId::new(13., FontFamily::Monospace),
            ),
        ]
        .into();
        style.drag_value_text_style = TextStyle::Body;
        style.spacing.item_spacing = vec2(SPACE_ITEM, SPACE_COMPACT);
        style.spacing.button_padding = vec2(SPACE_ITEM, SPACE_COMPACT);
        style.spacing.interact_size = vec2(CONTROL_HEIGHT, CONTROL_HEIGHT);
        style.spacing.window_margin = egui::Margin::same(SPACE_ITEM as i8);
        style.spacing.menu_margin = egui::Margin::same(SPACE_COMPACT as i8);
        style.spacing.indent = SPACE_SECTION;
        style.spacing.slider_width = 140.;
        style.spacing.combo_width = 140.;
        style.animation_time = 0.15;
        let mut visuals = match self {
            Self::Dark => egui::Visuals::dark(),
            Self::Light => egui::Visuals::light(),
        };
        visuals.override_text_color = None;
        visuals.weak_text_color = Some(t.muted);
        visuals.panel_fill = t.panel;
        visuals.window_fill = t.surface;
        visuals.window_stroke = Stroke::new(1.0_f32, t.border);
        visuals.window_corner_radius = CornerRadius::same(4);
        visuals.menu_corner_radius = CornerRadius::same(4);
        visuals.window_shadow = egui::epaint::Shadow::NONE;
        visuals.popup_shadow = egui::epaint::Shadow::NONE;
        visuals.extreme_bg_color = t.canvas;
        visuals.text_edit_bg_color = Some(t.canvas);
        visuals.faint_bg_color = t.panel;
        visuals.code_bg_color = t.panel;
        visuals.hyperlink_color = t.accent;
        visuals.warn_fg_color = t.warning;
        visuals.error_fg_color = t.error;
        visuals.selection.bg_fill = t.selection;
        visuals.selection.stroke = Stroke::new(1.0_f32, t.accent);
        visuals.text_cursor.stroke = Stroke::new(1.5_f32, t.accent);
        visuals.disabled_alpha = 0.5;
        visuals.slider_trailing_fill = false;
        visuals.collapsing_header_frame = false;
        visuals.indent_has_left_vline = true;
        for widget in [
            &mut visuals.widgets.noninteractive,
            &mut visuals.widgets.inactive,
            &mut visuals.widgets.hovered,
            &mut visuals.widgets.active,
            &mut visuals.widgets.open,
        ] {
            widget.corner_radius = CornerRadius::same(4);
            widget.expansion = 0.;
            widget.fg_stroke = Stroke::new(1.5_f32, t.text);
            widget.bg_stroke = Stroke::new(1.0_f32, t.border);
            widget.bg_fill = t.surface;
            widget.weak_bg_fill = t.surface;
        }
        visuals.widgets.noninteractive.bg_fill = t.panel;
        visuals.widgets.noninteractive.weak_bg_fill = t.panel;
        // Native Slider paints its rail with inactive.bg_fill; buttons use
        // weak_bg_fill. Keep the rail visible on both panel and popup surfaces.
        visuals.widgets.inactive.bg_fill = t.muted;
        style.spacing.slider_rail_height = 3.;
        visuals.widgets.hovered.bg_fill = t.elevated;
        visuals.widgets.hovered.weak_bg_fill = t.elevated;
        visuals.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, t.muted);
        visuals.widgets.active.bg_fill = t.selection;
        visuals.widgets.active.weak_bg_fill = t.selection;
        visuals.widgets.active.bg_stroke = Stroke::new(1.5_f32, t.accent);
        visuals.widgets.active.fg_stroke = Stroke::new(1.5_f32, t.accent);
        visuals.widgets.open.bg_fill = t.elevated;
        visuals.widgets.open.weak_bg_fill = t.elevated;
        visuals.widgets.open.bg_stroke = Stroke::new(1.0_f32, t.accent);
        style.visuals = visuals;
        ctx.set_style(style);
    }
}

/// Authored 24-unit line icons, sharing optical size and stroke weight.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Icon {
    Open,
    Import,
    Save,
    Undo,
    Redo,
    Wand,
    Export,
    Select,
    Nodes,
    Pan,
    Fit,
    Grid,
    Image,
    Stitches,
    Cube,
    Layers,
    Inspector,
    Play,
    Pause,
    Stop,
    Eye,
    Hidden,
    Lock,
    Unlock,
    Add,
    Trash,
    Up,
    Down,
    Settings,
    Warning,
    Chevron,
}

impl Icon {
    /// A stable inventory for native icon review and future command pickers.
    pub(super) const ALL: [Self; 31] = [
        Self::Open,
        Self::Import,
        Self::Save,
        Self::Undo,
        Self::Redo,
        Self::Wand,
        Self::Export,
        Self::Select,
        Self::Nodes,
        Self::Pan,
        Self::Fit,
        Self::Grid,
        Self::Image,
        Self::Stitches,
        Self::Cube,
        Self::Layers,
        Self::Inspector,
        Self::Play,
        Self::Pause,
        Self::Stop,
        Self::Eye,
        Self::Hidden,
        Self::Lock,
        Self::Unlock,
        Self::Add,
        Self::Trash,
        Self::Up,
        Self::Down,
        Self::Settings,
        Self::Warning,
        Self::Chevron,
    ];

    pub(super) fn paint(self, painter: &Painter, rect: Rect, color: Color32) {
        debug_assert!(Self::ALL.contains(&self));
        let ink = IconInk::new(painter, rect, color);
        match self {
            Self::Open => {
                ink.path(&[
                    (3., 9.),
                    (3., 5.),
                    (10., 5.),
                    (12., 7.),
                    (21., 7.),
                    (21., 9.),
                ]);
                ink.path(&[(3., 9.), (22., 9.), (19., 20.), (2., 20.), (3., 9.)]);
            }
            Self::Import | Self::Export => {
                ink.path(&[(4., 14.), (4., 20.), (20., 20.), (20., 14.)]);
                ink.line((12., 3.), (12., 15.));
                if self == Self::Import {
                    ink.path(&[(8., 11.), (12., 15.), (16., 11.)]);
                } else {
                    ink.path(&[(8., 7.), (12., 3.), (16., 7.)]);
                }
            }
            Self::Save => {
                ink.path(&[
                    (4., 3.),
                    (17., 3.),
                    (21., 7.),
                    (21., 21.),
                    (3., 21.),
                    (3., 3.),
                    (4., 3.),
                ]);
                ink.path(&[(7., 3.), (7., 9.), (16., 9.), (16., 3.)]);
                ink.rect((7., 14.), (17., 21.));
                ink.line((13., 5.), (13., 7.));
            }
            Self::Undo | Self::Redo => {
                let flip = |x: f32| if self == Self::Undo { x } else { 24. - x };
                ink.path(&[
                    (flip(5.), 10.),
                    (flip(8.), 6.),
                    (flip(14.), 6.),
                    (flip(19.), 10.),
                    (flip(19.), 17.),
                ]);
                ink.path(&[(flip(5.), 4.), (flip(5.), 10.), (flip(11.), 10.)]);
            }
            Self::Wand => {
                ink.path(&[(5., 19.), (16., 8.), (19., 11.), (8., 22.), (5., 19.)]);
                ink.line((13., 11.), (16., 14.));
                ink.line((6., 3.), (6., 9.));
                ink.line((3., 6.), (9., 6.));
                ink.line((18., 2.), (18., 5.));
                ink.line((20., 4.), (22., 4.));
            }
            Self::Select => ink.path(&[
                (5., 3.),
                (5., 19.),
                (10., 15.),
                (14., 22.),
                (17., 20.),
                (13., 13.),
                (20., 12.),
                (5., 3.),
            ]),
            Self::Nodes => {
                ink.path(&[(5., 17.), (12., 6.), (19., 17.)]);
                for (x, y) in [(5., 17.), (12., 6.), (19., 17.)] {
                    ink.rect((x - 2., y - 2.), (x + 2., y + 2.));
                }
            }
            Self::Pan => ink.path(&[
                (5., 13.),
                (5., 8.),
                (8., 8.),
                (8., 12.),
                (8., 4.),
                (11., 4.),
                (11., 11.),
                (11., 3.),
                (14., 3.),
                (14., 11.),
                (14., 5.),
                (17., 5.),
                (17., 13.),
                (19., 11.),
                (22., 12.),
                (18., 20.),
                (15., 22.),
                (9., 22.),
                (4., 16.),
                (3., 13.),
                (5., 13.),
            ]),
            Self::Fit => {
                for (x, y, sx, sy) in [
                    (3., 3., 1., 1.),
                    (21., 3., -1., 1.),
                    (3., 21., 1., -1.),
                    (21., 21., -1., -1.),
                ] {
                    ink.path(&[(x + 5. * sx, y), (x, y), (x, y + 5. * sy)]);
                    ink.line((x, y), (x + 6. * sx, y + 6. * sy));
                }
            }
            Self::Grid => {
                ink.rect((3., 3.), (21., 21.));
                for n in [9., 15.] {
                    ink.line((n, 3.), (n, 21.));
                    ink.line((3., n), (21., n));
                }
            }
            Self::Image => {
                ink.rect((3., 4.), (21., 20.));
                ink.circle((8., 9.), 1.5);
                ink.path(&[(3., 18.), (9., 12.), (13., 16.), (16., 13.), (21., 18.)]);
            }
            Self::Stitches => {
                ink.path(&[(3., 18.), (7., 6.), (12., 18.), (17., 6.), (21., 18.)]);
                for p in [(3., 18.), (7., 6.), (12., 18.), (17., 6.), (21., 18.)] {
                    ink.dot(p, 1.3);
                }
            }
            Self::Cube => {
                ink.path(&[
                    (12., 2.),
                    (22., 7.),
                    (22., 17.),
                    (12., 22.),
                    (2., 17.),
                    (2., 7.),
                    (12., 2.),
                ]);
                ink.path(&[(2., 7.), (12., 12.), (22., 7.)]);
                ink.line((12., 12.), (12., 22.));
            }
            Self::Layers => {
                ink.path(&[(2., 7.), (12., 2.), (22., 7.), (12., 12.), (2., 7.)]);
                ink.path(&[(2., 12.), (12., 17.), (22., 12.)]);
                ink.path(&[(2., 17.), (12., 22.), (22., 17.)]);
            }
            Self::Inspector => {
                for (x, y) in [(5., 8.), (12., 16.), (19., 10.)] {
                    ink.line((x, 3.), (x, y - 2.));
                    ink.line((x, y + 2.), (x, 21.));
                    ink.rect((x - 2., y - 2.), (x + 2., y + 2.));
                }
            }
            Self::Play => ink.path(&[(7., 3.), (21., 12.), (7., 21.), (7., 3.)]),
            Self::Pause => {
                ink.rect((5., 4.), (9., 20.));
                ink.rect((15., 4.), (19., 20.));
            }
            Self::Stop => ink.rect((5., 5.), (19., 19.)),
            Self::Eye | Self::Hidden => {
                ink.path(&[
                    (2., 12.),
                    (5., 8.),
                    (9., 6.),
                    (15., 6.),
                    (19., 8.),
                    (22., 12.),
                    (19., 16.),
                    (15., 18.),
                    (9., 18.),
                    (5., 16.),
                    (2., 12.),
                ]);
                ink.circle((12., 12.), 3.);
                if self == Self::Hidden {
                    ink.line((3., 3.), (21., 21.));
                }
            }
            Self::Lock | Self::Unlock => {
                ink.rect((5., 10.), (19., 21.));
                let mut shackle = Vec::new();
                for i in 0..=8 {
                    let angle = std::f32::consts::PI + i as f32 * std::f32::consts::PI / 8.;
                    shackle.push((12. + 4. * angle.cos(), 10. + 4. * angle.sin()));
                }
                if self == Self::Unlock {
                    shackle.pop();
                    shackle.pop();
                }
                ink.path(&shackle);
                ink.line((12., 14.), (12., 17.));
            }
            Self::Add => {
                ink.line((12., 4.), (12., 20.));
                ink.line((4., 12.), (20., 12.));
            }
            Self::Trash => {
                ink.path(&[(6., 7.), (7., 21.), (17., 21.), (18., 7.)]);
                ink.line((4., 7.), (20., 7.));
                ink.path(&[(9., 7.), (9., 3.), (15., 3.), (15., 7.)]);
                ink.line((10., 11.), (10., 17.));
                ink.line((14., 11.), (14., 17.));
            }
            Self::Up | Self::Down => {
                let tip = if self == Self::Up { 5. } else { 19. };
                let tail = 24. - tip;
                let base = if self == Self::Up { 11. } else { 13. };
                ink.line((12., tip), (12., tail));
                ink.path(&[(6., base), (12., tip), (18., base)]);
            }
            Self::Settings => {
                let mut teeth = Vec::with_capacity(33);
                for i in 0..=32 {
                    let angle = i as f32 * std::f32::consts::TAU / 32.;
                    let radius = if matches!(i % 4, 1 | 2) { 9. } else { 7. };
                    teeth.push((12. + radius * angle.cos(), 12. + radius * angle.sin()));
                }
                ink.path(&teeth);
                ink.circle((12., 12.), 3.);
            }
            Self::Warning => {
                ink.path(&[(12., 3.), (22., 21.), (2., 21.), (12., 3.)]);
                ink.line((12., 9.), (12., 14.));
                ink.dot((12., 17.5), 1.);
            }
            Self::Chevron => ink.path(&[(6., 9.), (12., 15.), (18., 9.)]),
        }
    }
}

struct IconInk<'a> {
    painter: &'a Painter,
    origin: Pos2,
    scale: f32,
    stroke: Stroke,
}
impl<'a> IconInk<'a> {
    fn new(painter: &'a Painter, rect: Rect, color: Color32) -> Self {
        let scale = rect.width().min(rect.height()) / 24.;
        Self {
            painter,
            origin: rect.center() - Vec2::splat(12. * scale),
            scale,
            stroke: Stroke::new((1.8 * scale).max(1.1), color),
        }
    }
    fn point(&self, p: (f32, f32)) -> Pos2 {
        self.origin + vec2(p.0, p.1) * self.scale
    }
    fn line(&self, a: (f32, f32), b: (f32, f32)) {
        self.painter
            .line_segment([self.point(a), self.point(b)], self.stroke);
    }
    fn path(&self, points: &[(f32, f32)]) {
        self.painter.add(Shape::line(
            points.iter().map(|&p| self.point(p)).collect(),
            self.stroke,
        ));
    }
    fn rect(&self, min: (f32, f32), max: (f32, f32)) {
        self.painter.rect_stroke(
            Rect::from_min_max(self.point(min), self.point(max)),
            1.,
            self.stroke,
            StrokeKind::Inside,
        );
    }
    fn circle(&self, center: (f32, f32), radius: f32) {
        self.painter
            .circle_stroke(self.point(center), radius * self.scale, self.stroke);
    }
    fn dot(&self, center: (f32, f32), radius: f32) {
        self.painter
            .circle_filled(self.point(center), radius * self.scale, self.stroke.color);
    }
}

fn button_foreground(ui: &Ui, response: &Response, selected: bool) -> Color32 {
    let color = ui
        .style()
        .interact_selectable(response, selected)
        .fg_stroke
        .color;
    // A disabled parent Ui already fades its painter. Only locally disabled
    // buttons need the explicit colour adjustment after add_enabled restores it.
    if response.enabled() || !ui.is_enabled() {
        color
    } else {
        ui.visuals().disable(color)
    }
}

fn finish_button(ui: &Ui, response: Response, selected: bool, label: &str) -> Response {
    response.widget_info(|| {
        WidgetInfo::selected(WidgetType::Button, response.enabled(), selected, label)
    });
    if response.has_focus() {
        let color = ui.visuals().selection.stroke.color;
        ui.painter().rect_stroke(
            response.rect.shrink(1.),
            4.,
            Stroke::new(1.5_f32, color),
            StrokeKind::Inside,
        );
    }
    response.on_hover_text(label).on_disabled_hover_text(label)
}

pub(super) fn icon_menu(ui: &mut Ui, icon: Icon, label: &str, contents: impl FnOnce(&mut Ui)) {
    let menu = ui.menu_button("", contents);
    icon.paint(
        ui.painter(),
        Rect::from_center_size(menu.response.rect.center(), Vec2::splat(ICON_SIZE)),
        button_foreground(ui, &menu.response, false),
    );
    let _ = finish_button(ui, menu.response, false, label);
}

/// Editing marks belong to the invariant light artwork plate, rather than
/// inheriting bright foregrounds intended for dark application chrome.
pub(super) fn artwork_ink() -> Tokens {
    Theme::Light.tokens()
}

pub(super) fn canvas_label(
    painter: &Painter,
    position: Pos2,
    anchor: egui::Align2,
    text: impl ToString,
    font: FontId,
    ink: Color32,
    background: Color32,
) {
    let galley = painter.layout_no_wrap(text.to_string(), font, ink);
    let rect = anchor.anchor_size(position, galley.size());
    painter.rect_filled(rect.expand(3.), 2., background);
    painter.galley(rect.min, galley, ink);
}

pub(super) fn icon_button(
    ui: &mut Ui,
    icon: Icon,
    selected: bool,
    enabled: bool,
    label: &str,
) -> Response {
    let response = ui.add_enabled(
        enabled,
        egui::Button::new("")
            .selected(selected)
            .min_size(Vec2::splat(CONTROL_HEIGHT)),
    );
    if ui.is_rect_visible(response.rect) {
        let rect = Rect::from_center_size(response.rect.center(), Vec2::splat(ICON_SIZE));
        icon.paint(
            ui.painter(),
            rect,
            button_foreground(ui, &response, selected),
        );
    }
    finish_button(ui, response, selected, label)
}

pub(super) fn action_button(ui: &mut Ui, icon: Icon, enabled: bool, label: &str) -> Response {
    let font = TextStyle::Button.resolve(ui.style());
    let galley = ui
        .painter()
        .layout_no_wrap(label.into(), font, Color32::PLACEHOLDER);
    let width = SPACE_ITEM * 2. + ICON_SIZE + SPACE_COMPACT + galley.size().x;
    let response = ui.add_enabled(
        enabled,
        egui::Button::new("").min_size(vec2(width, CONTROL_HEIGHT)),
    );
    if ui.is_rect_visible(response.rect) {
        let color = button_foreground(ui, &response, false);
        let icon_center = pos2(
            response.rect.left() + SPACE_ITEM + ICON_SIZE * 0.5,
            response.rect.center().y,
        );
        icon.paint(
            ui.painter(),
            Rect::from_center_size(icon_center, Vec2::splat(ICON_SIZE)),
            color,
        );
        let text = pos2(
            response.rect.left() + SPACE_ITEM + ICON_SIZE + SPACE_COMPACT,
            response.rect.center().y - galley.size().y * 0.5,
        );
        ui.painter().galley(text, galley, color);
    }
    finish_button(ui, response, false, label)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn contrast(a: Color32, b: Color32) -> f64 {
        let luminance = |c: Color32| {
            let linear = |v: u8| {
                let n = f64::from(v) / 255.;
                if n <= 0.04045 {
                    n / 12.92
                } else {
                    ((n + 0.055) / 1.055).powf(2.4)
                }
            };
            0.2126 * linear(c.r()) + 0.7152 * linear(c.g()) + 0.0722 * linear(c.b())
        };
        let (a, b) = (luminance(a), luminance(b));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    #[test]
    fn readable_tokens_and_keyboard_focus_hold_in_both_themes() {
        for theme in [Theme::Dark, Theme::Light] {
            let t = theme.tokens();
            for background in [t.canvas, t.panel, t.surface, t.elevated] {
                for foreground in [t.text, t.muted, t.warning, t.error, t.success] {
                    assert!(
                        contrast(foreground, background) >= 4.5,
                        "{theme:?}: {foreground:?} on {background:?}"
                    );
                }
            }
            assert!(contrast(t.accent, t.selection) >= 4.5);
            assert!(contrast(t.accent, t.surface) >= 3.);
            for ink in [
                artwork_ink().accent,
                artwork_ink().success,
                artwork_ink().warning,
            ] {
                assert!(contrast(ink, t.paper) >= 4.5);
            }
            let ctx = egui::Context::default();
            theme.apply(&ctx);
            let style = ctx.style();
            assert_eq!(style.spacing.interact_size.y, CONTROL_HEIGHT);
            assert_eq!(style.visuals.widgets.active.bg_stroke.color, t.accent);
            assert_eq!(style.visuals.selection.bg_fill, t.selection);
            assert_eq!(style.visuals.window_shadow, egui::epaint::Shadow::NONE);
        }
    }

    #[test]
    fn persisted_theme_is_explicit_and_defaults_to_dark() {
        assert_eq!(Theme::default(), Theme::Dark);
        for theme in [Theme::Dark, Theme::Light] {
            assert_eq!(
                serde_json::from_str::<Theme>(&serde_json::to_string(&theme).unwrap()).unwrap(),
                theme
            );
        }
    }
}
