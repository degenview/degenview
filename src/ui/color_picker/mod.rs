//! The colour picker every swatch in the app opens: a saturation/brightness square, hue and
//! opacity sliders, a hex field, the macOS system colours and the colours picked last. Drawn
//! with the painter in the app's own style, so it looks the same on every platform and matches
//! the other controls; egui's stock picker is not used.
//!
//! [`show`] edits a straight-alpha `Color32` in place. The geometry (where the square and the
//! sliders are, what a pointer position means) and the text handling (hex) are pure functions
//! below, tested without a window.

use super::accessibility;
use super::kit;
use super::theme;
use eframe::egui::color_picker::Alpha;
use eframe::egui::{
    self, Color32, CursorIcon, Id, Key, Mesh, Painter, Pos2, Rect, Sense, Shape, Stroke,
    StrokeKind, Ui, Vec2, pos2, vec2,
};
use serde::{Deserialize, Serialize};

/// The width of the square and the sliders.
pub const WIDTH: f32 = 208.0;
const SQUARE_HEIGHT: f32 = 150.0;
const SQUARE_RADIUS: f32 = 6.0;
const SLIDER_HEIGHT: f32 = 14.0;
const THUMB_RADIUS: f32 = 8.0;
const GAP: f32 = 12.0;
const FIELD_HEIGHT: f32 = 24.0;
/// What a field adds around its text: a point of border and eight of padding each side.
const FIELD_CHROME: f32 = 18.0;
const PREVIEW: f32 = 24.0;
const COLUMNS: usize = 10;
const SWATCH_GAP: f32 = 4.0;
/// How many recently picked colours are kept.
pub const RECENT_LIMIT: usize = 10;

/// What one arrow-key press moves a value by, and with Shift.
const NUDGE: f32 = 0.01;
const NUDGE_FAR: f32 = 0.1;

// MARK: Pure helpers

/// The macOS system colours (light appearance) and a ramp of greys.
pub const SYSTEM_SWATCHES: [[u8; 3]; 20] = [
    [255, 59, 48],
    [255, 149, 0],
    [255, 204, 0],
    [52, 199, 89],
    [0, 199, 190],
    [48, 176, 199],
    [50, 173, 230],
    [0, 122, 255],
    [88, 86, 214],
    [175, 82, 222],
    [255, 45, 85],
    [162, 132, 94],
    [255, 255, 255],
    [209, 209, 214],
    [152, 152, 157],
    [99, 99, 102],
    [58, 58, 60],
    [28, 28, 30],
    [0, 0, 0],
    [142, 142, 147],
];

/// `text` as a colour: `#` optional, 3, 6 or 8 hex digits, any case. Eight digits carry alpha and
/// are refused when `alpha` is off; three and six are opaque.
pub fn parse_hex(text: &str, alpha: bool) -> Option<Color32> {
    let trimmed = text.trim();
    let digits = trimmed.strip_prefix('#').unwrap_or(trimmed);
    if !digits.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let pair = |range: std::ops::Range<usize>| u8::from_str_radix(digits.get(range)?, 16).ok();
    match digits.len() {
        3 => {
            let each = |index: usize| {
                u8::from_str_radix(&digits[index..=index], 16)
                    .ok()
                    .map(|value| value * 17)
            };
            Some(Color32::from_rgb(each(0)?, each(1)?, each(2)?))
        }
        6 => Some(Color32::from_rgb(pair(0..2)?, pair(2..4)?, pair(4..6)?)),
        8 if alpha => Some(Color32::from_rgba_unmultiplied(
            pair(0..2)?,
            pair(2..4)?,
            pair(4..6)?,
            pair(6..8)?,
        )),
        _ => None,
    }
}

/// `color` as `#RRGGBB`, or `#RRGGBBAA` when `alpha` is on and the colour is not opaque.
pub fn format_hex(color: Color32, alpha: bool) -> String {
    let [r, g, b, a] = color.to_srgba_unmultiplied();
    if alpha && a != 255 {
        format!("#{r:02X}{g:02X}{b:02X}{a:02X}")
    } else {
        format!("#{r:02X}{g:02X}{b:02X}")
    }
}

/// A position in `0..=1` along `rect`, clamped; never NaN, even for a rect of no width.
pub fn slider_value(rect: Rect, x: f32) -> f32 {
    let width = rect.width();
    if !(width.is_finite() && width > 0.0 && x.is_finite()) {
        return 0.0;
    }
    ((x - rect.left()) / width).clamp(0.0, 1.0)
}

/// Saturation (left to right) and brightness (bottom to top) under a pointer in `rect`.
pub fn square_to_sv(rect: Rect, position: Pos2) -> (f32, f32) {
    let height = rect.height();
    let v = if height.is_finite() && height > 0.0 && position.y.is_finite() {
        ((rect.bottom() - position.y) / height).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (slider_value(rect, position.x), v)
}

/// Where saturation `s` and brightness `v` sit in `rect`: the inverse of [`square_to_sv`].
pub fn sv_to_square(rect: Rect, s: f32, v: f32) -> Pos2 {
    pos2(
        rect.left() + rect.width() * s.clamp(0.0, 1.0),
        rect.bottom() - rect.height() * v.clamp(0.0, 1.0),
    )
}

/// `value` moved by one arrow press (`far` with Shift), kept in `0..=1`.
pub fn nudged(value: f32, direction: f32, far: bool) -> f32 {
    (value + direction * if far { NUDGE_FAR } else { NUDGE }).clamp(0.0, 1.0)
}

/// A colour as the picker edits it: hue, saturation and brightness in the *display* (gamma)
/// space, so the square's gradients look like what they pick (halfway across is halfway on
/// screen), and opacity. Hue is kept when the colour is grey or black.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hsv {
    pub h: f32,
    pub s: f32,
    pub v: f32,
    pub a: f32,
}

impl Hsv {
    pub fn new(h: f32, s: f32, v: f32, a: f32) -> Self {
        Self { h, s, v, a }
    }

    /// Plain HSV of the sRGB bytes, as every design tool does it. A grey keeps no hue of its own:
    /// the caller supplies the one to keep.
    pub fn from_srgba_unmultiplied([r, g, b, a]: [u8; 4]) -> Self {
        let (r, g, b) = (
            f32::from(r) / 255.0,
            f32::from(g) / 255.0,
            f32::from(b) / 255.0,
        );
        let max = r.max(g).max(b);
        let min = r.min(g).min(b);
        let delta = max - min;
        let hue = if delta <= f32::EPSILON {
            0.0
        } else if max == r {
            ((g - b) / delta).rem_euclid(6.0) / 6.0
        } else if max == g {
            ((b - r) / delta + 2.0) / 6.0
        } else {
            ((r - g) / delta + 4.0) / 6.0
        };
        let saturation = if max <= f32::EPSILON {
            0.0
        } else {
            delta / max
        };
        Self::new(hue, saturation, max, f32::from(a) / 255.0)
    }

    pub fn to_srgba_unmultiplied(self) -> [u8; 4] {
        let h = self.h.clamp(0.0, 1.0) * 6.0;
        let (s, v) = (self.s.clamp(0.0, 1.0), self.v.clamp(0.0, 1.0));
        let chroma = v * s;
        let x = chroma * (1.0 - (h.rem_euclid(2.0) - 1.0).abs());
        let (r, g, b) = match h as u32 {
            0 => (chroma, x, 0.0),
            1 => (x, chroma, 0.0),
            2 => (0.0, chroma, x),
            3 => (0.0, x, chroma),
            4 => (x, 0.0, chroma),
            _ => (chroma, 0.0, x),
        };
        let m = v - chroma;
        let byte = |value: f32| ((value + m) * 255.0).round().clamp(0.0, 255.0) as u8;
        [
            byte(r),
            byte(g),
            byte(b),
            (self.a.clamp(0.0, 1.0) * 255.0).round() as u8,
        ]
    }
}

impl From<Hsv> for Color32 {
    fn from(hsv: Hsv) -> Self {
        let [r, g, b, a] = hsv.to_srgba_unmultiplied();
        Color32::from_rgba_unmultiplied(r, g, b, a)
    }
}

/// The colours picked last, newest first, kept across restarts with egui's persisted memory.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct Recents(pub Vec<[u8; 4]>);

impl Recents {
    /// Puts `color` first; a colour already there moves up instead of repeating.
    pub fn push(&mut self, color: [u8; 4]) {
        self.0.retain(|known| *known != color);
        self.0.insert(0, color);
        self.0.truncate(RECENT_LIMIT);
    }
}

fn recents_id() -> Id {
    Id::new("degenview-recent-colors")
}

fn remember(ctx: &egui::Context, color: Color32) {
    ctx.data_mut(|data| {
        data.get_persisted_mut_or_default::<Recents>(recents_id())
            .push(color.to_srgba_unmultiplied());
    });
}

// MARK: Layout

/// Where everything of one picker sits, from its top-left corner.
#[derive(Clone, Copy, Debug)]
pub struct Layout {
    pub square: Rect,
    pub hue: Rect,
    pub opacity: Option<Rect>,
    pub fields: Rect,
    pub swatches: Rect,
    pub recents: Rect,
    pub total: Vec2,
}

impl Layout {
    pub fn new(origin: Pos2, alpha: bool) -> Self {
        let width = WIDTH;
        let square = Rect::from_min_size(origin, vec2(width, SQUARE_HEIGHT));
        let mut y = square.bottom() + GAP;
        let hue = Rect::from_min_size(pos2(origin.x, y), vec2(width, SLIDER_HEIGHT));
        y = hue.bottom() + 8.0;
        let opacity = alpha.then(|| {
            let rect = Rect::from_min_size(pos2(origin.x, y), vec2(width, SLIDER_HEIGHT));
            y = rect.bottom() + 8.0;
            rect
        });
        y += GAP - 8.0;
        let fields = Rect::from_min_size(pos2(origin.x, y), vec2(width, PREVIEW.max(FIELD_HEIGHT)));
        y = fields.bottom() + GAP;
        let cell = Self::cell();
        let swatches = Rect::from_min_size(pos2(origin.x, y), vec2(width, 2.0 * cell + SWATCH_GAP));
        y = swatches.bottom() + GAP - 2.0;
        // A caption line, then a row of recents.
        let recents = Rect::from_min_size(pos2(origin.x, y + 14.0), vec2(width, cell));
        let total = vec2(width, recents.bottom() - origin.y);
        Self {
            square,
            hue,
            opacity,
            fields,
            swatches,
            recents,
            total,
        }
    }

    /// The side of a swatch.
    pub fn cell() -> f32 {
        (WIDTH - (COLUMNS as f32 - 1.0) * SWATCH_GAP) / COLUMNS as f32
    }

    /// The rect of swatch `index` in a row or grid that starts at `area`'s top-left.
    pub fn swatch(area: Rect, index: usize) -> Rect {
        let cell = Self::cell();
        let (column, row) = (index % COLUMNS, index / COLUMNS);
        Rect::from_min_size(
            pos2(
                area.left() + column as f32 * (cell + SWATCH_GAP),
                area.top() + row as f32 * (cell + SWATCH_GAP),
            ),
            Vec2::splat(cell),
        )
    }
}

// MARK: Painting

/// Paints the four corners of `rect` that a `radius` rounding takes away in `background`, so a
/// square mesh reads as a rounded one (egui cannot clip to a rounded rectangle).
fn round_corners(painter: &Painter, rect: Rect, radius: f32, background: Color32) {
    let radius = radius.min(rect.width() / 2.0).min(rect.height() / 2.0);
    if radius <= 0.0 {
        return;
    }
    let corners = [
        (rect.left_top(), pos2(radius, radius), 180.0),
        (rect.right_top(), pos2(-radius, radius), 270.0),
        (rect.right_bottom(), pos2(-radius, -radius), 0.0),
        (rect.left_bottom(), pos2(radius, -radius), 90.0),
    ];
    for (corner, centre_offset, start) in corners {
        let centre = corner + centre_offset.to_vec2();
        let mut mesh = Mesh::default();
        mesh.colored_vertex(corner, background);
        let steps = 8;
        for step in 0..=steps {
            let angle = (start + 90.0 * step as f32 / steps as f32).to_radians();
            mesh.colored_vertex(centre + vec2(angle.cos(), angle.sin()) * radius, background);
        }
        for index in 0..steps as u32 {
            mesh.add_triangle(0, 1 + index, 2 + index);
        }
        painter.add(Shape::mesh(mesh));
    }
}

/// The two-tone grid behind a translucent colour.
fn checkerboard(painter: &Painter, rect: Rect) {
    checkerboard_cells(painter, rect, (rect.height() / 2.0).clamp(3.0, 6.0));
}

/// [`checkerboard`] with cells of a given side, from the rect's top-left corner.
fn checkerboard_cells(painter: &Painter, rect: Rect, cell: f32) {
    let (light, dark) = (
        theme::pick(Color32::from_gray(96), Color32::WHITE),
        theme::pick(Color32::from_gray(58), Color32::from_gray(214)),
    );
    painter.rect_filled(rect, 0.0, light);
    let mut mesh = Mesh::default();
    let (columns, rows) = (
        (rect.width() / cell).ceil() as i32,
        (rect.height() / cell).ceil() as i32,
    );
    for row in 0..rows {
        for column in 0..columns {
            if (row + column) % 2 == 1 {
                let cell_rect = Rect::from_min_size(
                    rect.min + vec2(column as f32 * cell, row as f32 * cell),
                    Vec2::splat(cell),
                )
                .intersect(rect);
                if cell_rect.is_positive() {
                    mesh.add_colored_rect(cell_rect, dark);
                }
            }
        }
    }
    painter.add(Shape::mesh(mesh));
}

/// A colour over its checkerboard, in a rounded rectangle with a hairline.
pub fn paint_swatch(painter: &Painter, rect: Rect, color: Color32, radius: f32) {
    paint_swatch_on(painter, rect, color, radius, kit::menu_fill());
}

/// [`paint_swatch`] on a surface of another colour: the corners a translucent swatch cuts away
/// are filled with `background`, which must be what is behind it.
pub fn paint_swatch_on(
    painter: &Painter,
    rect: Rect,
    color: Color32,
    radius: f32,
    background: Color32,
) {
    if color.a() < 255 {
        checkerboard(painter, rect);
        painter.rect_filled(rect, 0.0, color);
        round_corners(painter, rect, radius, background);
    } else {
        painter.rect_filled(rect, radius, color);
    }
    painter.rect_stroke(
        rect,
        radius,
        Stroke::new(1.0, theme::ink(36)),
        StrokeKind::Inside,
    );
}

fn paint_square(painter: &Painter, rect: Rect, hue: f32) {
    let pure = Color32::from(Hsv::new(hue, 1.0, 1.0, 1.0));
    let mut across = Mesh::default();
    across.colored_vertex(rect.left_top(), Color32::WHITE);
    across.colored_vertex(rect.right_top(), pure);
    across.colored_vertex(rect.right_bottom(), pure);
    across.colored_vertex(rect.left_bottom(), Color32::WHITE);
    across.add_triangle(0, 1, 2);
    across.add_triangle(0, 2, 3);
    painter.add(Shape::mesh(across));
    let mut down = Mesh::default();
    down.colored_vertex(rect.left_top(), Color32::TRANSPARENT);
    down.colored_vertex(rect.right_top(), Color32::TRANSPARENT);
    down.colored_vertex(rect.right_bottom(), Color32::BLACK);
    down.colored_vertex(rect.left_bottom(), Color32::BLACK);
    down.add_triangle(0, 1, 2);
    down.add_triangle(0, 2, 3);
    painter.add(Shape::mesh(down));
}

fn paint_track(painter: &Painter, rect: Rect, stops: &[Color32], checkers: bool) {
    if checkers {
        checkerboard(painter, rect);
    }
    let mut mesh = Mesh::default();
    let last = stops.len().saturating_sub(1).max(1);
    for (index, color) in stops.iter().enumerate() {
        let x = rect.left() + rect.width() * index as f32 / last as f32;
        mesh.colored_vertex(pos2(x, rect.top()), *color);
        mesh.colored_vertex(pos2(x, rect.bottom()), *color);
        if index > 0 {
            let at = (index as u32 - 1) * 2;
            mesh.add_triangle(at, at + 1, at + 2);
            mesh.add_triangle(at + 1, at + 2, at + 3);
        }
    }
    painter.add(Shape::mesh(mesh));
}

/// The round handle on a slider or the square: a white ring with a soft shadow.
fn paint_thumb(painter: &Painter, centre: Pos2, fill: Color32, focused: bool) {
    let ring = focused.then(|| theme::with_alpha(kit::ACCENT, 0.8));
    paint_thumb_sized(painter, centre, THUMB_RADIUS, fill, ring);
}

/// [`paint_thumb`] with its own radius, and a focus ring of a given colour (`None` for no ring).
fn paint_thumb_sized(
    painter: &Painter,
    centre: Pos2,
    radius: f32,
    fill: Color32,
    focus_ring: Option<Color32>,
) {
    painter.circle_filled(
        centre + vec2(0.0, 1.0),
        radius + 2.0,
        Color32::from_black_alpha(80),
    );
    painter.circle(centre, radius, fill, Stroke::new(2.5, Color32::WHITE));
    // A hairline outside the white ring, so a light colour on a light ground still has an edge.
    painter.circle_stroke(
        centre,
        radius + 1.5,
        Stroke::new(1.0, Color32::from_black_alpha(70)),
    );
    if let Some(color) = focus_ring {
        painter.circle_stroke(centre, radius + 3.5, Stroke::new(2.0, color));
    }
}

// MARK: Controls

/// One slider over `rect` (a taller hit area than its track). Returns whether the value changed
/// and whether a drag or key press just finished.
#[allow(clippy::too_many_arguments)]
fn slider(
    ui: &mut Ui,
    id: Id,
    rect: Rect,
    value: &mut f32,
    stops: &[Color32],
    checkers: bool,
    thumb: Color32,
    name: &str,
    scale: f64,
) -> (bool, bool) {
    let response = ui.interact(rect, id, Sense::click_and_drag());
    let response = accessibility::label_as(response, egui::WidgetType::Slider, name);
    let current = f64::from(*value) * scale;
    response.widget_info(|| egui::WidgetInfo::slider(true, current, name));
    let before = *value;
    if response.hovered() || response.dragged() {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    if let Some(position) = response.interact_pointer_pos() {
        *value = slider_value(rect, position.x);
    }
    let mut finished = response.drag_stopped() || response.clicked();
    if response.has_focus() {
        let (step, far) = ui.input(|input| {
            let direction =
                f32::from(input.key_pressed(Key::ArrowRight) || input.key_pressed(Key::ArrowUp))
                    - f32::from(
                        input.key_pressed(Key::ArrowLeft) || input.key_pressed(Key::ArrowDown),
                    );
            (direction, input.modifiers.shift)
        });
        if step != 0.0 {
            *value = nudged(*value, step, far);
            finished = true;
        }
    }
    let track = rect.shrink2(vec2(
        0.0,
        (rect.height() - SLIDER_HEIGHT.min(rect.height())) / 2.0,
    ));
    let painter = ui.painter();
    paint_track(painter, track, stops, checkers);
    round_corners(painter, track, track.height() / 2.0, kit::menu_fill());
    painter.rect_stroke(
        track,
        track.height() / 2.0,
        Stroke::new(1.0, theme::ink(30)),
        StrokeKind::Inside,
    );
    let inset = THUMB_RADIUS.min(track.width() / 2.0);
    let x = track.left() + inset + (track.width() - 2.0 * inset) * *value;
    // The thumb rides inside the track's ends, so its centre never leaves the capsule.
    paint_thumb(
        painter,
        pos2(x, track.center().y),
        thumb,
        response.has_focus(),
    );
    (*value != before, finished)
}

/// A text field that keeps what is typed until Return or leaving it, then hands the text back.
fn text_field(ui: &mut Ui, id: Id, shown: &str, width: f32, name: &str) -> Option<String> {
    let text_id = id.with("text");
    let mut text = ui
        .data(|data| data.get_temp::<String>(text_id))
        .unwrap_or_else(|| shown.to_owned());
    let focused = ui.memory(|memory| memory.has_focus(id));
    let frame = kit::field_frame(focused)
        .corner_radius(6.0)
        .inner_margin(egui::Margin::symmetric(8, 0));
    let response = frame
        .show(ui, |ui| {
            ui.allocate_ui_with_layout(
                vec2(width, FIELD_HEIGHT - 2.0),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| kit::plain_field(ui, id, &mut text, "", 12.0, kit::REGULAR, width),
            )
            .inner
        })
        .inner;
    accessibility::label_as(response.clone(), egui::WidgetType::TextEdit, name);
    let committed = (response.lost_focus() && !text.trim().is_empty()).then(|| text.clone());
    if response.has_focus() {
        ui.data_mut(|data| data.insert_temp(text_id, text));
    } else {
        ui.data_mut(|data| data.remove::<String>(text_id));
    }
    committed
}

/// A swatch button. Returns whether it was clicked.
fn swatch_button(ui: &mut Ui, id: Id, rect: Rect, color: Color32, name: &str) -> bool {
    let response = ui
        .interact(rect, id, Sense::click())
        .on_hover_cursor(CursorIcon::PointingHand);
    let response = accessibility::label(response, name);
    paint_swatch(ui.painter(), rect, color, 5.0);
    if response.hovered() || response.has_focus() {
        ui.painter().rect_stroke(
            rect.expand(1.5),
            6.0,
            Stroke::new(
                2.0,
                theme::with_alpha(kit::ACCENT, if response.has_focus() { 0.9 } else { 0.55 }),
            ),
            StrokeKind::Outside,
        );
    }
    response.clicked()
}

/// The hue as the colour a slider shows at `position` (`0..=1`).
fn hue_stops() -> Vec<Color32> {
    (0..=6)
        .map(|step| Color32::from(Hsv::new(step as f32 / 6.0, 1.0, 1.0, 1.0)))
        .collect()
}

/// Edits `color` in place; returns whether it changed. `alpha` says whether opacity is offered:
/// without it the colour stays opaque.
pub fn show(ui: &mut Ui, color: &mut Color32, alpha: Alpha) -> bool {
    let with_alpha = alpha != Alpha::Opaque;
    let id = ui.id().with("degenview-color-picker");
    let layout = Layout::new(ui.cursor().min, with_alpha);
    ui.allocate_space(layout.total);

    // The hue, saturation and brightness live apart from the colour while the picker is open: a
    // grey or black has no hue of its own, and converting back and forth would lose it.
    let state_id = id.with("hsv");
    let current = color.to_srgba_unmultiplied();
    let mut hsv = ui
        .data(|data| data.get_temp::<Hsv>(state_id))
        .filter(|known| known.to_srgba_unmultiplied() == current)
        .unwrap_or_else(|| Hsv::from_srgba_unmultiplied(current));
    let mut edited = false;
    let mut finished = false;

    // Saturation and brightness.
    let square_response = ui.interact(layout.square, id.with("square"), Sense::click_and_drag());
    let square_response = accessibility::label_as(
        square_response,
        egui::WidgetType::Slider,
        "Saturation and brightness",
    );
    square_response.widget_info(|| {
        egui::WidgetInfo::slider(true, f64::from(hsv.s * 100.0), "Saturation and brightness")
    });
    if square_response.hovered() || square_response.dragged() {
        ui.ctx().set_cursor_icon(CursorIcon::Crosshair);
    }
    if let Some(position) = square_response.interact_pointer_pos() {
        let (s, v) = square_to_sv(layout.square, position);
        edited |= (s, v) != (hsv.s, hsv.v);
        (hsv.s, hsv.v) = (s, v);
    }
    finished |= square_response.drag_stopped() || square_response.clicked();
    if square_response.has_focus() {
        let (dx, dy, far) = ui.input(|input| {
            (
                f32::from(input.key_pressed(Key::ArrowRight))
                    - f32::from(input.key_pressed(Key::ArrowLeft)),
                f32::from(input.key_pressed(Key::ArrowUp))
                    - f32::from(input.key_pressed(Key::ArrowDown)),
                input.modifiers.shift,
            )
        });
        if dx != 0.0 || dy != 0.0 {
            hsv.s = nudged(hsv.s, dx, far);
            hsv.v = nudged(hsv.v, dy, far);
            edited = true;
            finished = true;
        }
    }
    {
        let painter = ui.painter();
        paint_square(painter, layout.square, hsv.h);
        round_corners(painter, layout.square, SQUARE_RADIUS, kit::menu_fill());
        painter.rect_stroke(
            layout.square,
            SQUARE_RADIUS,
            Stroke::new(1.0, theme::ink(30)),
            StrokeKind::Inside,
        );
        let picked = Color32::from(Hsv::new(hsv.h, hsv.s, hsv.v, 1.0));
        paint_thumb(
            painter,
            sv_to_square(layout.square, hsv.s, hsv.v),
            picked,
            square_response.has_focus(),
        );
    }

    // Hue.
    let hue_thumb = Color32::from(Hsv::new(hsv.h, 1.0, 1.0, 1.0));
    let (changed, done) = slider(
        ui,
        id.with("hue"),
        layout.hue,
        &mut hsv.h,
        &hue_stops(),
        false,
        hue_thumb,
        "Hue",
        360.0,
    );
    edited |= changed;
    finished |= done;

    // Opacity.
    if let Some(rect) = layout.opacity {
        let opaque = Color32::from(Hsv::new(hsv.h, hsv.s, hsv.v, 1.0));
        let (changed, done) = slider(
            ui,
            id.with("opacity"),
            rect,
            &mut hsv.a,
            &[opaque.gamma_multiply(0.0), opaque],
            true,
            opaque,
            "Opacity",
            100.0,
        );
        edited |= changed;
        finished |= done;
    } else {
        hsv.a = 1.0;
    }

    let [r, g, b, a] = hsv.to_srgba_unmultiplied();
    let mut next = Color32::from_rgba_unmultiplied(r, g, b, a);

    // The preview, the hex field and the opacity field.
    let fields = layout.fields;
    let preview = Rect::from_min_size(fields.min, Vec2::splat(PREVIEW));
    paint_swatch(ui.painter(), preview, next, 6.0);
    let mut cursor = fields.left() + PREVIEW + 8.0;
    // Each field is its text plus 18 points of border and padding; together with the preview and
    // the gaps they fill the width exactly.
    let hex_width = if with_alpha {
        86.0
    } else {
        WIDTH - PREVIEW - 8.0 - FIELD_CHROME
    };
    let hex_rect = Rect::from_min_size(
        pos2(cursor, fields.top()),
        vec2(hex_width + FIELD_CHROME, FIELD_HEIGHT),
    );
    let typed = ui
        .scope_builder(egui::UiBuilder::new().max_rect(hex_rect), |ui| {
            text_field(
                ui,
                id.with("hex"),
                &format_hex(next, with_alpha),
                hex_width,
                "Hex color",
            )
        })
        .inner;
    if let Some(text) = typed
        && let Some(parsed) = parse_hex(&text, with_alpha)
        && parsed != next
    {
        let [r, g, b, a] = parsed.to_srgba_unmultiplied();
        hsv = Hsv::from_srgba_unmultiplied([r, g, b, if with_alpha { a } else { 255 }]);
        next = parsed;
        edited = true;
        finished = true;
    }
    cursor += hex_width + FIELD_CHROME + 8.0;
    if with_alpha {
        let percent = (hsv.a * 100.0).round() as i64;
        let rect = Rect::from_min_size(
            pos2(cursor, fields.top()),
            vec2(WIDTH - (cursor - fields.left()), FIELD_HEIGHT),
        );
        let typed = ui
            .scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                text_field(
                    ui,
                    id.with("percent"),
                    &format!("{percent}%"),
                    38.0,
                    "Opacity percent",
                )
            })
            .inner;
        if let Some(text) = typed
            && let Ok(value) = text.trim().trim_end_matches('%').trim().parse::<f32>()
            && value.is_finite()
        {
            hsv.a = (value / 100.0).clamp(0.0, 1.0);
            let [r, g, b, a] = hsv.to_srgba_unmultiplied();
            next = Color32::from_rgba_unmultiplied(r, g, b, a);
            edited = true;
            finished = true;
        }
    }

    // The system colours.
    let mut swatch_pick = None;
    for (index, rgb) in SYSTEM_SWATCHES.iter().enumerate() {
        let rect = Layout::swatch(layout.swatches, index);
        let swatch = Color32::from_rgb(rgb[0], rgb[1], rgb[2]);
        let name = format!("Color {}", format_hex(swatch, false));
        if swatch_button(ui, id.with(("system", index)), rect, swatch, &name) {
            swatch_pick = Some(Color32::from_rgba_unmultiplied(
                rgb[0],
                rgb[1],
                rgb[2],
                if with_alpha {
                    (hsv.a * 255.0).round() as u8
                } else {
                    255
                },
            ));
        }
    }

    // The colours picked last.
    ui.painter().text(
        pos2(layout.recents.left(), layout.recents.top() - 12.0),
        egui::Align2::LEFT_TOP,
        "Recent",
        egui::FontId::proportional(10.0),
        theme::text_tertiary(),
    );
    let recents = ui
        .data_mut(|data| data.get_persisted::<Recents>(recents_id()))
        .unwrap_or_default();
    if recents.0.is_empty() {
        ui.painter().text(
            layout.recents.left_center(),
            egui::Align2::LEFT_CENTER,
            "Colors you pick appear here",
            egui::FontId::proportional(11.0),
            theme::text_tertiary(),
        );
    }
    for (index, [r, g, b, a]) in recents.0.iter().enumerate() {
        let rect = Layout::swatch(layout.recents, index);
        let recent = Color32::from_rgba_unmultiplied(*r, *g, *b, if with_alpha { *a } else { 255 });
        let name = format!("Recent color {}", format_hex(recent, with_alpha));
        if swatch_button(ui, id.with(("recent", index)), rect, recent, &name) {
            swatch_pick = Some(recent);
        }
    }
    if let Some(picked) = swatch_pick {
        let [r, g, b, a] = picked.to_srgba_unmultiplied();
        hsv = Hsv::from_srgba_unmultiplied([r, g, b, a]);
        next = picked;
        edited = true;
        finished = true;
    }

    ui.data_mut(|data| data.insert_temp(state_id, hsv));
    if finished {
        remember(ui.ctx(), next);
    }
    let changed = edited && next != *color;
    if edited {
        *color = next;
    }
    changed
}

pub mod inline;

#[cfg(test)]
mod tests;
