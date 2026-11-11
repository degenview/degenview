//! The picker the Pine editor opens from a colour swatch: a header (the colour and the
//! expression it was written as), the saturation/brightness square, and two vertical strips,
//! transparency then hue. It shares the painting and the HSV maths of [`super::show`] and differs
//! only in layout, so the two never drift apart.
//!
//! The colour lives in an [`Hsv`] the caller keeps for as long as the picker is open (a grey has
//! no hue of its own and converting back and forth would lose it). [`show`] reports which of the
//! colour and the transparency this frame moved, and whether a gesture just finished, so the
//! caller can write only what the user touched.

use super::{
    Hsv, accessibility, checkerboard_cells, hue_stops, nudged, paint_square, paint_swatch_on,
    round_corners, square_to_sv, sv_to_square,
};
use crate::ui::theme;
use eframe::egui::{
    self, Color32, CursorIcon, FontId, Id, Key, Mesh, Painter, Pos2, Rect, Sense, Shape, Stroke,
    StrokeKind, Ui, Vec2, pos2, vec2,
};

const SQUARE_WIDTH: f32 = 208.0;
const SQUARE_HEIGHT: f32 = 160.0;
const STRIP_WIDTH: f32 = 16.0;
const GAP: f32 = 10.0;
const HEADER_HEIGHT: f32 = 22.0;
const HEADER_GAP: f32 = 10.0;
const SQUARE_RADIUS: f32 = 4.0;
const STRIP_RADIUS: f32 = 3.0;
const SQUARE_THUMB: f32 = 6.0;
const PREVIEW_RADIUS: f32 = 7.0;

/// Where everything sits, from the top-left corner of the picker's content.
#[derive(Clone, Copy, Debug)]
pub struct Layout {
    pub header: Rect,
    pub square: Rect,
    pub transparency: Rect,
    pub hue: Rect,
    pub total: Vec2,
}

impl Layout {
    pub fn new(origin: Pos2) -> Self {
        let header = Rect::from_min_size(origin, vec2(Self::width(), HEADER_HEIGHT));
        let top = header.bottom() + HEADER_GAP;
        let square = Rect::from_min_size(pos2(origin.x, top), vec2(SQUARE_WIDTH, SQUARE_HEIGHT));
        let transparency = Rect::from_min_size(
            pos2(square.right() + GAP, top),
            vec2(STRIP_WIDTH, SQUARE_HEIGHT),
        );
        let hue = Rect::from_min_size(
            pos2(transparency.right() + GAP, top),
            vec2(STRIP_WIDTH, SQUARE_HEIGHT),
        );
        Self {
            header,
            square,
            transparency,
            hue,
            total: vec2(Self::width(), hue.bottom() - origin.y),
        }
    }

    pub fn width() -> f32 {
        SQUARE_WIDTH + 2.0 * (GAP + STRIP_WIDTH)
    }
}

/// How far down a vertical strip `y` is, `0` at the top and `1` at the bottom; clamped, never
/// NaN, even for a strip of no height.
pub fn strip_position(rect: Rect, y: f32) -> f32 {
    let height = rect.height();
    if !(height.is_finite() && height > 0.0 && y.is_finite()) {
        return 0.0;
    }
    ((y - rect.top()) / height).clamp(0.0, 1.0)
}

/// The y of position `fraction` (`0` top, `1` bottom) on a strip: the inverse of
/// [`strip_position`].
pub fn strip_y(rect: Rect, fraction: f32) -> f32 {
    rect.top() + rect.height() * fraction.clamp(0.0, 1.0)
}

/// Pine transparency (0 opaque … 100 invisible) the transparency strip shows at `opacity`
/// (`0..=1`), a whole percent.
pub fn transparency_percent(opacity: f32) -> u8 {
    ((1.0 - opacity.clamp(0.0, 1.0)) * 100.0).round() as u8
}

/// The opacity (`0..=1`) of a Pine transparency percent.
pub fn opacity_of(transparency: u8) -> f32 {
    1.0 - f32::from(transparency.min(100)) / 100.0
}

/// How the header is set: the expression is worked out after this frame's interaction, so it
/// always matches the colour drawn beside it.
pub struct Header {
    pub font: FontId,
}

/// The colours the picker needs from its surroundings.
#[derive(Clone, Copy)]
pub struct Style {
    /// What is behind the picker: translucent corners are filled with it.
    pub background: Color32,
    /// The ring around a focused control.
    pub focus: Color32,
    /// The header's text.
    pub text: Color32,
}

/// What a frame of the picker did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Output {
    /// The square or the hue strip moved the colour.
    pub color_changed: bool,
    /// The transparency strip moved the transparency.
    pub transparency_changed: bool,
    /// A drag or click ended, or an arrow key was pressed: one gesture is complete.
    pub finished: bool,
}

/// Draws the picker at the top-left of `ui`'s cursor and edits `hsv` in place. Opacity is
/// snapped to a whole Pine transparency percent as it moves. `expression` is asked for the
/// header text once this frame's input has been applied.
pub fn show(
    ui: &mut Ui,
    id: Id,
    hsv: &mut Hsv,
    header: &Header,
    style: Style,
    expression: impl FnOnce(&Hsv, Output) -> String,
) -> Output {
    let layout = Layout::new(ui.cursor().min);
    ui.allocate_space(layout.total);
    let mut output = Output::default();

    // Saturation and brightness.
    let square = ui.interact(layout.square, id.with("square"), Sense::click_and_drag());
    let square = accessibility::label_as(
        square,
        egui::WidgetType::Slider,
        "Saturation and brightness",
    );
    square.widget_info(|| {
        egui::WidgetInfo::slider(true, f64::from(hsv.s * 100.0), "Saturation and brightness")
    });
    if square.hovered() || square.dragged() {
        ui.ctx().set_cursor_icon(CursorIcon::Crosshair);
    }
    if let Some(position) = square.interact_pointer_pos() {
        let (s, v) = square_to_sv(layout.square, position);
        output.color_changed |= (s, v) != (hsv.s, hsv.v);
        (hsv.s, hsv.v) = (s, v);
    }
    output.finished |= square.drag_stopped() || square.clicked();
    if square.has_focus() {
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
            output.color_changed = true;
            output.finished = true;
        }
    }

    // Transparency: opaque at the top.
    let mut opacity = hsv.a;
    let moved = strip(
        ui,
        id.with("transparency"),
        layout.transparency,
        &mut opacity,
        Direction::OpaqueAtTop,
        "Transparency",
    );
    if moved.changed {
        let snapped = opacity_of(transparency_percent(opacity));
        output.transparency_changed |= (snapped - hsv.a).abs() > f32::EPSILON;
        hsv.a = snapped;
    }
    output.finished |= moved.finished;
    let transparency_focus = moved.focused;

    // Hue: red at the top.
    let moved = strip(
        ui,
        id.with("hue"),
        layout.hue,
        &mut hsv.h,
        Direction::HueDown,
        "Hue",
    );
    output.color_changed |= moved.changed;
    output.finished |= moved.finished;
    let hue_focus = moved.focused;

    // Everything is painted from the values as they are now.
    let text = expression(hsv, output);
    let painter = ui.painter();
    paint_header(painter, layout.header, hsv, header, &text, style);

    paint_square(painter, layout.square, hsv.h);
    round_corners(painter, layout.square, SQUARE_RADIUS, style.background);
    painter.rect_stroke(
        layout.square,
        SQUARE_RADIUS,
        Stroke::new(1.0, theme::ink(30)),
        StrokeKind::Inside,
    );
    let picked = Color32::from(Hsv::new(hsv.h, hsv.s, hsv.v, 1.0));
    super::paint_thumb_sized(
        painter,
        sv_to_square(layout.square, hsv.s, hsv.v),
        SQUARE_THUMB,
        picked,
        square.has_focus().then_some(style.focus),
    );

    checkerboard_cells(
        painter,
        layout.transparency,
        layout.transparency.width() / 2.0,
    );
    paint_vertical(
        painter,
        layout.transparency,
        &[picked, picked.gamma_multiply(0.0)],
    );
    finish_strip(painter, layout.transparency, style);
    paint_marker(
        painter,
        layout.transparency,
        strip_y(layout.transparency, Direction::OpaqueAtTop.fraction(hsv.a)),
        style,
        transparency_focus,
    );

    paint_vertical(painter, layout.hue, &hue_stops());
    finish_strip(painter, layout.hue, style);
    paint_marker(
        painter,
        layout.hue,
        strip_y(layout.hue, Direction::HueDown.fraction(hsv.h)),
        style,
        hue_focus,
    );
    output
}

fn paint_header(
    painter: &Painter,
    rect: Rect,
    hsv: &Hsv,
    header: &Header,
    text: &str,
    style: Style,
) {
    let centre = pos2(rect.left() + PREVIEW_RADIUS, rect.center().y);
    let preview = Rect::from_center_size(centre, Vec2::splat(2.0 * PREVIEW_RADIUS));
    paint_swatch_on(
        painter,
        preview,
        Color32::from(*hsv),
        PREVIEW_RADIUS,
        style.background,
    );
    let left = preview.right() + 8.0;
    let room = (rect.right() - left).max(0.0);
    let color = style.text;
    let galley = fitted(painter, text, &header.font, color, room);
    painter.galley(
        pos2(left, rect.center().y - galley.size().y / 2.0),
        galley,
        color,
    );
}

/// `text` on one line, cut with an ellipsis to fit `width`.
fn fitted(
    painter: &Painter,
    text: &str,
    font: &FontId,
    color: Color32,
    width: f32,
) -> std::sync::Arc<egui::Galley> {
    let whole = painter.layout_no_wrap(text.to_owned(), font.clone(), color);
    if whole.size().x <= width {
        return whole;
    }
    let characters: Vec<char> = text.chars().collect();
    let (mut low, mut high) = (0, characters.len());
    // The most characters that still fit with the ellipsis.
    while low < high {
        let middle = (low + high).div_ceil(2);
        let candidate: String = characters[..middle].iter().collect::<String>() + "…";
        if painter
            .layout_no_wrap(candidate, font.clone(), color)
            .size()
            .x
            <= width
        {
            low = middle;
        } else {
            high = middle - 1;
        }
    }
    let cut: String = characters[..low].iter().collect::<String>() + "…";
    painter.layout_no_wrap(cut, font.clone(), color)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Direction {
    /// The value is opacity: `1` at the top, `0` at the bottom.
    OpaqueAtTop,
    /// The value is hue: `0` at the top, `1` at the bottom.
    HueDown,
}

impl Direction {
    /// The value under fraction `down` of the way down the strip.
    fn value(self, down: f32) -> f32 {
        match self {
            Self::OpaqueAtTop => 1.0 - down,
            Self::HueDown => down,
        }
    }

    /// How far down the strip a value sits.
    fn fraction(self, value: f32) -> f32 {
        match self {
            Self::OpaqueAtTop => 1.0 - value,
            Self::HueDown => value,
        }
    }
}

struct Moved {
    changed: bool,
    finished: bool,
    focused: bool,
}

/// The input of one vertical strip.
fn strip(
    ui: &mut Ui,
    id: Id,
    rect: Rect,
    value: &mut f32,
    direction: Direction,
    name: &str,
) -> Moved {
    let response = ui.interact(rect, id, Sense::click_and_drag());
    let response = accessibility::label_as(response, egui::WidgetType::Slider, name);
    let scale = match direction {
        Direction::OpaqueAtTop => 100.0,
        Direction::HueDown => 360.0,
    };
    let current = f64::from(*value) * scale;
    response.widget_info(|| egui::WidgetInfo::slider(true, current, name));
    if response.hovered() || response.dragged() {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    let before = *value;
    if let Some(position) = response.interact_pointer_pos() {
        *value = direction.value(strip_position(rect, position.y));
    }
    let mut finished = response.drag_stopped() || response.clicked();
    if response.has_focus() {
        let (step, far) = ui.input(|input| {
            let up =
                f32::from(input.key_pressed(Key::ArrowUp) || input.key_pressed(Key::ArrowRight));
            let down =
                f32::from(input.key_pressed(Key::ArrowDown) || input.key_pressed(Key::ArrowLeft));
            (up - down, input.modifiers.shift)
        });
        if step != 0.0 {
            // Up moves the handle up the strip, whichever way the value runs.
            let toward_top = match direction {
                Direction::OpaqueAtTop => step,
                Direction::HueDown => -step,
            };
            *value = nudged(*value, toward_top, far);
            finished = true;
        }
    }
    Moved {
        changed: *value != before,
        finished,
        focused: response.has_focus(),
    }
}

fn paint_vertical(painter: &Painter, rect: Rect, stops: &[Color32]) {
    let mut mesh = Mesh::default();
    let last = stops.len().saturating_sub(1).max(1);
    for (index, color) in stops.iter().enumerate() {
        let y = rect.top() + rect.height() * index as f32 / last as f32;
        mesh.colored_vertex(pos2(rect.left(), y), *color);
        mesh.colored_vertex(pos2(rect.right(), y), *color);
        if index > 0 {
            let at = (index as u32 - 1) * 2;
            mesh.add_triangle(at, at + 1, at + 2);
            mesh.add_triangle(at + 1, at + 2, at + 3);
        }
    }
    painter.add(Shape::mesh(mesh));
}

fn finish_strip(painter: &Painter, rect: Rect, style: Style) {
    round_corners(painter, rect, STRIP_RADIUS, style.background);
    painter.rect_stroke(
        rect,
        STRIP_RADIUS,
        Stroke::new(1.0, theme::ink(30)),
        StrokeKind::Inside,
    );
}

/// The thin horizontal handle across a strip: a white line with a dark edge either side, so it
/// reads on every colour.
fn paint_marker(painter: &Painter, rect: Rect, y: f32, style: Style, focused: bool) {
    let bar = Rect::from_min_max(
        pos2(rect.left() - 3.0, y - 2.0),
        pos2(rect.right() + 3.0, y + 2.0),
    );
    painter.rect_stroke(
        bar.expand(1.0),
        3.0,
        Stroke::new(1.0, Color32::from_black_alpha(90)),
        StrokeKind::Inside,
    );
    painter.rect_stroke(
        bar,
        2.0,
        Stroke::new(2.0, Color32::WHITE),
        StrokeKind::Inside,
    );
    if focused {
        painter.rect_stroke(
            bar.expand(3.0),
            4.0,
            Stroke::new(2.0, style.focus),
            StrokeKind::Outside,
        );
    }
}
