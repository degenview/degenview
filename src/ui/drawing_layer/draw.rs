//! Painting a drawing's screen shape: fill, middle line, strokes, glyphs, text, statistics and
//! handles. Colours come from the drawing's own style; nothing here reads the theme except the
//! neutral plates behind labels and the handle faces.

use super::Projection;
use super::shape::{HandleSpot, Screen, Tag, TextPlace};
use crate::model::Drawing;
use crate::model::chart_type::LineDash;
use crate::model::drawing_style::{
    DrawingStyle, HorizontalAlign, TextOrientation, TextStyle, VerticalAlign,
};
use crate::ui::canvas_render::CanvasStyle;
use crate::ui::canvas_render::dash_pattern;
use crate::ui::drawing_geometry as geo;
use crate::ui::drawing_render::{ArrowReadout, HANDLE_RADIUS};
use crate::ui::line_geometry as lines;
use crate::ui::price_text;
use crate::ui::{series_render, theme};
use eframe::egui::{
    self, Align2, Color32, FontId, Painter, Pos2, Rect, Shape, Stroke, StrokeKind,
    epaint::TextShape, pos2, vec2,
};
use std::f32::consts::FRAC_PI_2;

/// How a drawing is emphasized this frame.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Emphasis {
    /// Hovered or selected: a halo behind the strokes.
    pub halo: bool,
    /// Selected: handles and construction lines.
    pub selected: bool,
    /// What the drawing's text place shows.
    pub text: TextMode,
    /// The drawing's note is open (a pin's).
    pub open: bool,
    /// The selected cell of a table, as `(row, column)`.
    pub cell: Option<(usize, usize)>,
    /// The row or column border of a table the pointer is on.
    pub divider: Option<super::edit::Part>,
}

/// What a drawing's text place shows this frame.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum TextMode {
    /// The drawing's text, if it has any.
    #[default]
    Text,
    /// The text, or a faint "Add text" where there is none: the drawing is selected and can
    /// take text, so the place to click is shown.
    Invite,
    /// Nothing: the text is being edited in place.
    Editing,
}

fn with_alpha(color: Color32, alpha: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), alpha)
}

/// A polyline or closed loop in `stroke`, dashed per `dash`.
fn stroke_path(painter: &Painter, points: &[Pos2], closed: bool, stroke: Stroke, dash: LineDash) {
    if points.len() < 2 {
        return;
    }
    match dash_pattern(dash, stroke.width) {
        None => {
            painter.add(if closed {
                Shape::closed_line(points.to_vec(), stroke)
            } else {
                Shape::line(points.to_vec(), stroke)
            });
        }
        Some((dash, gap)) => {
            let mut path = points.to_vec();
            if closed {
                path.push(points[0]);
            }
            painter.extend(Shape::dashed_line(&path, stroke, dash, gap));
        }
    }
}

/// The glyph of a handle: a filled disc with a coloured rim, or a hollow ring for a control.
pub(super) fn draw_handle(painter: &Painter, spot: &HandleSpot, color: Color32) {
    let radius = HANDLE_RADIUS;
    if spot.control {
        painter.circle(
            spot.pos,
            radius,
            with_alpha(theme::surface(), 120),
            Stroke::new(1.5, color),
        );
    } else {
        painter.circle(spot.pos, radius, theme::surface(), Stroke::new(1.5, color));
    }
}

fn text_galley(
    painter: &Painter,
    text: &str,
    style: &TextStyle,
    wrap: f32,
) -> std::sync::Arc<eframe::egui::Galley> {
    painter.layout_job(super::text::layout_job(text, style, wrap))
}

/// Where a drawing's text was last laid out: its unrotated size, its centre and the angle it is
/// turned by. The layer keeps it so a click can find the text, which only the painter can size.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct TextBox {
    pub center: Pos2,
    pub size: egui::Vec2,
    /// Radians clockwise.
    pub angle: f32,
}

impl TextBox {
    /// Whether `point` is within `slack` of the text's (turned) box.
    pub(super) fn contains(&self, point: Pos2, slack: f32) -> bool {
        let (sin, cos) = self.angle.sin_cos();
        let from_center = point - self.center;
        // Into the box's own frame: undo the turn.
        let local = vec2(
            from_center.x * cos + from_center.y * sin,
            -from_center.x * sin + from_center.y * cos,
        );
        local.x.abs() <= self.size.x / 2.0 + slack && local.y.abs() <= self.size.y / 2.0 + slack
    }
}

/// Space (px) between text and the line or edge it sits against.
const TEXT_GAP: f32 = 4.0;
/// Space (px) kept between text and the end of the line it sits on.
pub(super) const TEXT_PAD: f32 = 5.0;

/// Paints `galley` with the top-left corner of its text at `at`, whatever its alignment: egui
/// anchors a galley at its alignment point (the middle of a centred one), so the position it is
/// painted at is moved back by where its rectangle starts.
fn paint_galley_at(
    painter: &Painter,
    at: Pos2,
    galley: std::sync::Arc<egui::Galley>,
    color: Color32,
) {
    let origin = at - galley.rect.min.to_vec2();
    painter.galley(origin, galley, color);
}

/// Paints `galley` centred on `center` and turned by `angle` (radians, clockwise), pixel-snapped
/// at the pivot so a level label stays as sharp as any other text.
fn paint_turned(
    painter: &Painter,
    galley: std::sync::Arc<egui::Galley>,
    center: Pos2,
    angle: f32,
    color: Color32,
) -> TextBox {
    let size = galley.size();
    let ppp = painter.ctx().pixels_per_point();
    let snap = |value: f32| (value * ppp).round() / ppp;
    let center = if angle == 0.0 {
        pos2(snap(center.x), snap(center.y))
    } else {
        center
    };
    let shape = TextShape::new(center - size / 2.0, galley, color)
        .with_angle_and_anchor(angle, Align2::CENTER_CENTER);
    painter.add(shape);
    TextBox {
        center,
        size,
        angle,
    }
}

/// A drawing's own text, placed where its shape says.
/// `clear_middle` lifts centred text off a middle line drawn through the same spot.
fn draw_text(
    painter: &Painter,
    place: TextPlace,
    text: &str,
    style: &TextStyle,
    clear_middle: bool,
) -> Option<TextBox> {
    if text.trim().is_empty() {
        return None;
    }
    let color = series_render::color(style.color);
    match place {
        TextPlace::Inside(rect) => {
            let galley = text_galley(painter, text, style, rect.width() - TEXT_PAD * 2.0);
            let size = galley.size();
            let x = match style.align {
                HorizontalAlign::Left => rect.left() + TEXT_PAD,
                HorizontalAlign::Center => rect.center().x - size.x / 2.0,
                HorizontalAlign::Right => rect.right() - TEXT_PAD - size.x,
            };
            let y = match style.valign {
                VerticalAlign::Top => rect.top() + TEXT_PAD,
                VerticalAlign::Middle if clear_middle => rect.center().y - size.y - 2.0,
                VerticalAlign::Middle => rect.center().y - size.y / 2.0,
                VerticalAlign::Bottom => rect.bottom() - TEXT_PAD - size.y,
            };
            let at = Pos2::new(x, y);
            paint_galley_at(painter, at, galley, color);
            Some(TextBox {
                center: at + size / 2.0,
                size,
                angle: 0.0,
            })
        }
        TextPlace::At { point, above } => {
            let galley = text_galley(painter, text, style, 320.0);
            let size = galley.size();
            let y = if above {
                point.y - 6.0 - size.y
            } else {
                point.y + 6.0
            };
            let at = Pos2::new(point.x - size.x / 2.0, y);
            paint_galley_at(painter, at, galley, color);
            Some(TextBox {
                center: at + size / 2.0,
                size,
                angle: 0.0,
            })
        }
        TextPlace::Line { from, to } => {
            let galley = text_galley(painter, text, style, 320.0);
            let size = galley.size();
            // Read from the left end; a vertical line from the bottom.
            let (left, right) = if from.x < to.x || (from.x == to.x && from.y > to.y) {
                (from, to)
            } else {
                (to, from)
            };
            let length = (right - left).length();
            let direction = if length > 1e-3 {
                (right - left) / length
            } else {
                vec2(1.0, 0.0)
            };
            let angle = match style.orientation {
                TextOrientation::Along => lines::text_angle(left, right).unwrap_or(0.0),
                TextOrientation::Horizontal => 0.0,
                TextOrientation::Vertical => -FRAC_PI_2,
            };
            let normal = vec2(-angle.sin(), angle.cos());
            let along = if length < size.x + TEXT_PAD * 2.0 {
                length / 2.0
            } else {
                match style.align {
                    HorizontalAlign::Left => TEXT_PAD + size.x / 2.0,
                    HorizontalAlign::Center => length / 2.0,
                    HorizontalAlign::Right => length - TEXT_PAD - size.x / 2.0,
                }
            };
            let on_line = left + direction * along;
            let lift = size.y / 2.0 + TEXT_GAP;
            let center = match style.valign {
                VerticalAlign::Top => on_line - normal * lift,
                VerticalAlign::Middle => on_line,
                VerticalAlign::Bottom => on_line + normal * lift,
            };
            Some(paint_turned(painter, galley, center, angle, color))
        }
        TextPlace::Column { x, top, bottom } => {
            let galley = text_galley(painter, text, style, 320.0);
            let size = galley.size();
            let turned = style.orientation != TextOrientation::Horizontal;
            // The text's extent down the line and across it.
            let (down, across) = if turned {
                (size.x, size.y)
            } else {
                (size.y, size.x)
            };
            let y = match style.valign {
                VerticalAlign::Top => top + TEXT_PAD + down / 2.0,
                VerticalAlign::Middle => (top + bottom) / 2.0,
                VerticalAlign::Bottom => bottom - TEXT_PAD - down / 2.0,
            };
            let center_x = match style.align {
                HorizontalAlign::Left => x - TEXT_GAP - across / 2.0,
                HorizontalAlign::Center => x,
                HorizontalAlign::Right => x + TEXT_GAP + across / 2.0,
            };
            let angle = if turned { -FRAC_PI_2 } else { 0.0 };
            Some(paint_turned(
                painter,
                galley,
                pos2(center_x, y),
                angle,
                color,
            ))
        }
    }
}

/// A small plate with lines of text, anchored by `align` at `at`.
fn label_card(painter: &Painter, at: Pos2, align: Align2, lines: &[String], accent: Color32) {
    if lines.is_empty() {
        return;
    }
    let font = FontId::monospace(10.0);
    let galleys: Vec<_> = lines
        .iter()
        .map(|line| painter.layout_no_wrap(line.clone(), font.clone(), theme::text_primary()))
        .collect();
    let width = galleys.iter().map(|g| g.size().x).fold(0.0, f32::max) + 12.0;
    let height: f32 = galleys.iter().map(|g| g.size().y).sum::<f32>() + 8.0;
    let rect = align.anchor_size(at, vec2(width, height));
    painter.rect(
        rect,
        4.0,
        theme::chrome().gamma_multiply(0.92),
        Stroke::new(1.0, accent),
        StrokeKind::Inside,
    );
    let mut y = rect.top() + 4.0;
    for galley in galleys {
        let line_height = galley.size().y;
        painter.galley(Pos2::new(rect.left() + 6.0, y), galley, accent);
        y += line_height;
    }
}

/// The arrow's statistics and price labels.
fn draw_arrow_extras(
    painter: &Painter,
    screen: &Screen,
    drawing: &Drawing,
    style: &DrawingStyle,
    proj: &Projection,
    selected: bool,
) {
    let Some((start, end)) = screen.arrow else {
        return;
    };
    let accent = series_render::color(style.stroke.color);
    let anchors = drawing.anchors();
    let [first, second] = [anchors[0], anchors[1]];
    if style.price_labels {
        for (point, other, anchor) in [(start, end, first), (end, start, second)] {
            let away = (point - other).normalized();
            let direction = if away.x.is_finite() {
                away
            } else {
                vec2(1.0, 0.0)
            };
            let at = point + direction * 8.0;
            let align = if direction.x >= 0.0 {
                Align2::LEFT_CENTER
            } else {
                Align2::RIGHT_CENTER
            };
            label_card(
                painter,
                at,
                align,
                &[proj.format.format(anchor.price)],
                accent,
            );
        }
    }
    if style.stats.any() && (selected || style.stats.always_show) {
        let bars = proj.times.bars_between(first.time_ms, second.time_ms);
        let periods = (proj.times.index_of(second.time_ms).round()
            - proj.times.index_of(first.time_ms).round()) as i64;
        let readout = ArrowReadout::new(
            first.price,
            second.price,
            bars,
            periods,
            (second.time_ms - first.time_ms) / 1000,
            start,
            end,
        );
        let lines = readout.lines(&style.stats, proj.format);
        let at = match style.stats.position {
            crate::model::drawing_style::StatsPosition::Left => start,
            crate::model::drawing_style::StatsPosition::Center => start.lerp(end, 0.5),
            crate::model::drawing_style::StatsPosition::Right => end,
        };
        // Beside the line, on the side away from the text, so the two never collide.
        let offset = if end.y <= start.y {
            vec2(0.0, 14.0)
        } else {
            vec2(0.0, -14.0)
        };
        let align = if end.y <= start.y {
            Align2::CENTER_TOP
        } else {
            Align2::CENTER_BOTTOM
        };
        label_card(painter, at + offset, align, &lines, accent);
    }
}

/// The textures of the pictures being painted, kept between frames so a picture is uploaded once
/// however often the chart is panned, and dropped when nothing shows it any more.
#[derive(Default)]
pub(super) struct Textures {
    held: std::collections::HashMap<String, egui::TextureHandle>,
    used: std::collections::HashSet<String>,
}

impl Textures {
    #[cfg(test)]
    pub(super) fn holds(&self, asset: &str) -> bool {
        self.held.contains_key(asset)
    }

    /// The texture of a picture, to tell a reused one from a new upload.
    #[cfg(test)]
    pub(super) fn id_of(&self, asset: &str) -> Option<egui::TextureId> {
        self.held.get(asset).map(egui::TextureHandle::id)
    }

    /// Starts a frame: nothing is in use yet.
    pub(super) fn begin(&mut self) {
        self.used.clear();
    }

    /// Ends it: the pictures nothing painted are let go.
    pub(super) fn end(&mut self) {
        let used = std::mem::take(&mut self.used);
        self.held.retain(|asset, _| used.contains(asset));
    }

    fn get(&mut self, painter: &Painter, asset: &str) -> Option<egui::TextureId> {
        self.used.insert(asset.to_owned());
        if let Some(texture) = self.held.get(asset) {
            return Some(texture.id());
        }
        let crate::service::asset_host::AssetImage::Ready(pixels) =
            crate::service::asset_host::lookup(painter.ctx(), asset)
        else {
            return None;
        };
        let texture = painter.ctx().load_texture(
            format!("drawing-picture-{asset}"),
            (*pixels).clone(),
            egui::TextureOptions::LINEAR,
        );
        let id = texture.id();
        self.held.insert(asset.to_owned(), texture);
        Some(id)
    }
}

/// What a picture that cannot be shown says, and in what state it is.
fn picture_status(painter: &Painter, asset: &str) -> &'static str {
    use crate::service::asset_host::{AssetImage, lookup};
    match lookup(painter.ctx(), asset) {
        AssetImage::Loading => "Loading image…",
        AssetImage::Missing => "Image unavailable (waiting to sync)",
        AssetImage::Corrupt => "This image could not be read",
        AssetImage::Ready(_) => "",
    }
}

/// Paints a picture in `rect`, or a placeholder that says why it is not there.
pub(super) fn draw_picture(
    painter: &Painter,
    textures: &mut Textures,
    rect: Rect,
    asset: &str,
    name: &str,
    opacity: f32,
) {
    if let Some(texture) = textures.get(painter, asset) {
        painter.image(
            texture,
            rect,
            Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)),
            Color32::WHITE.gamma_multiply(opacity),
        );
        return;
    }
    painter.rect_filled(rect, 4.0, with_alpha(theme::surface(), 200));
    stroke_path(
        painter,
        &[
            rect.left_top(),
            rect.right_top(),
            rect.right_bottom(),
            rect.left_bottom(),
        ],
        true,
        Stroke::new(1.0, theme::text_tertiary()),
        LineDash::Dashed,
    );
    let status = picture_status(painter, asset);
    let label = if name.is_empty() {
        status.to_owned()
    } else {
        format!("{status}\n{name}")
    };
    let galley = painter.layout(
        label,
        FontId::proportional(12.0),
        theme::text_secondary(),
        (rect.width() - 12.0).max(20.0),
    );
    let at = rect.center() - galley.size() / 2.0;
    painter
        .with_clip_rect(rect)
        .galley(at, galley, theme::text_secondary());
}

/// Paints one projected drawing. `painter` is already clipped to the plot.
pub(super) fn paint(
    painter: &Painter,
    screen: &Screen,
    drawing: &Drawing,
    proj: &Projection,
    emphasis: Emphasis,
    textures: &mut Textures,
) -> Option<TextBox> {
    let style = drawing.style()?;
    let color = series_render::color(style.stroke.color);
    let width = style.stroke.width;

    if let Some(picture) = &screen.picture {
        draw_picture(
            painter,
            textures,
            picture.rect,
            &picture.asset,
            &picture.name,
            picture.opacity,
        );
        if emphasis.halo && !emphasis.selected {
            painter.rect_stroke(
                picture.rect,
                0.0,
                Stroke::new(1.0, with_alpha(crate::ui::kit::ACCENT, 160)),
                StrokeKind::Outside,
            );
        } else if emphasis.selected {
            painter.rect_stroke(
                picture.rect,
                0.0,
                Stroke::new(1.0, crate::ui::kit::ACCENT),
                StrokeKind::Outside,
            );
        }
        return None;
    }

    if let Some(card) = &screen.card {
        let avatar = card
            .card
            .meta
            .as_ref()
            .and_then(|meta| meta.avatar.as_deref())
            .and_then(|asset| textures.get(painter, asset));
        // The line to the post's candle goes under the card, so it meets the card's edge.
        for styled in &screen.styled {
            stroke_path(
                painter,
                &styled.path,
                false,
                Stroke::new(styled.width, series_render::color(styled.color)),
                styled.dash,
            );
        }
        for (center, radius, fill) in &screen.discs {
            painter.circle_filled(*center, *radius, series_render::color(*fill));
        }
        crate::ui::content_card::paint(painter, card.rect, &card.card, 1.0, avatar);
        if emphasis.selected || emphasis.halo {
            let alpha = if emphasis.selected { 255 } else { 160 };
            painter.rect_stroke(
                card.rect,
                8.0,
                Stroke::new(1.0, with_alpha(crate::ui::kit::ACCENT, alpha)),
                StrokeKind::Outside,
            );
        }
        return None;
    }

    if let Some(mask) = &screen.mask {
        let texture = screen.mask_texture.get_or_init(|| {
            let [r, g, b, a] = style.stroke.color;
            let pixels: Vec<Color32> = mask
                .coverage
                .iter()
                .map(|coverage| {
                    let alpha = (u32::from(a) * u32::from(*coverage) + 127) / 255;
                    Color32::from_rgba_unmultiplied(r, g, b, alpha as u8)
                })
                .collect();
            painter.ctx().load_texture(
                "drawing-stroke",
                egui::ColorImage::new(mask.size, pixels),
                egui::TextureOptions::LINEAR,
            )
        });
        painter.image(
            texture.id(),
            mask.rect(),
            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
            Color32::WHITE,
        );
        paint_stroke_frame(painter, screen.bounds, color, emphasis);
        return None;
    }
    if let Some(mesh) = &screen.stroke {
        if emphasis.halo
            && let Some(path) = screen.lines.first()
            && let Some(halo) = geo::stroke_mesh(path, width + 6.0, with_alpha(color, 40), 1.0)
        {
            painter.add(Shape::mesh(halo));
        }
        painter.add(Shape::mesh(mesh.clone()));
        paint_stroke_frame(painter, screen.bounds, color, emphasis);
        return None;
    }

    for mesh in &screen.band_meshes {
        painter.add(Shape::mesh(mesh.clone()));
    }
    if let Some(mesh) = &screen.fill_mesh {
        painter.add(Shape::mesh(mesh.clone()));
    }
    if emphasis.open
        && let Some(popup) = &screen.popup
    {
        if let Some(mesh) = &screen.popup_mesh {
            painter.add(Shape::mesh(mesh.clone()));
        }
        let border = &style.border;
        if border.enabled {
            stroke_path(
                painter,
                &popup.outline,
                true,
                Stroke::new(border.width, series_render::color(border.color)),
                border.dash,
            );
        }
    }
    for frame in &screen.frames {
        let border = &style.border;
        if border.enabled {
            stroke_path(
                painter,
                frame,
                true,
                Stroke::new(border.width, series_render::color(border.color)),
                border.dash,
            );
        } else if emphasis.halo {
            // A box without a border still shows how far it reaches while it is hovered or
            // selected.
            stroke_path(
                painter,
                frame,
                true,
                Stroke::new(1.0, with_alpha(color, 120)),
                LineDash::Solid,
            );
        }
    }
    if style.border.enabled {
        let border = &style.border;
        for rule in &screen.rules {
            stroke_path(
                painter,
                rule,
                false,
                Stroke::new(border.width, series_render::color(border.color)),
                border.dash,
            );
        }
    }
    for (cell, text) in &screen.cell_texts {
        // A cell never spills into its neighbours.
        let clipped = painter.with_clip_rect(cell.shrink(0.5).intersect(painter.clip_rect()));
        draw_text(&clipped, TextPlace::Inside(*cell), text, &style.text, false);
    }
    if let (Some((row, column)), Some(grid)) = (emphasis.cell, &screen.grid)
        && let Some(cell) = grid.cell_rect(row, column)
    {
        painter.rect_stroke(
            cell,
            0.0,
            Stroke::new(2.0, crate::ui::kit::ACCENT),
            StrokeKind::Inside,
        );
    }
    if let Some(part) = emphasis.divider
        && let Some(divider) = screen.dividers.iter().find(|divider| divider.part == part)
    {
        painter.line_segment(
            [divider.a, divider.b],
            Stroke::new(2.5, crate::ui::kit::ACCENT),
        );
    }
    for styled in &screen.styled {
        stroke_path(
            painter,
            &styled.path,
            false,
            Stroke::new(styled.width, series_render::color(styled.color)),
            styled.dash,
        );
    }
    if let Some(line) = screen.middle {
        let middle = &style.middle_line;
        stroke_path(
            painter,
            &line,
            false,
            Stroke::new(middle.width, series_render::color(middle.color)),
            middle.dash,
        );
    }

    if emphasis.halo {
        let halo = Stroke::new(width + 5.0, with_alpha(color, 40));
        for path in &screen.lines {
            stroke_path(painter, path, false, halo, LineDash::Solid);
        }
        for path in &screen.outlines {
            stroke_path(painter, path, true, halo, LineDash::Solid);
        }
        for solid in &screen.solids {
            stroke_path(
                painter,
                solid,
                true,
                Stroke::new(4.0, with_alpha(color, 40)),
                LineDash::Solid,
            );
        }
    }

    let stroke = Stroke::new(width, color);
    for path in &screen.lines {
        stroke_path(painter, path, false, stroke, style.stroke.dash);
    }
    for path in &screen.outlines {
        stroke_path(painter, path, true, stroke, style.stroke.dash);
    }
    for solid in &screen.solids {
        painter.add(Shape::mesh(geo::fill_mesh(solid, color)));
        // The outline antialiases the mesh's edge.
        stroke_path(
            painter,
            solid,
            true,
            Stroke::new(1.0, color),
            LineDash::Solid,
        );
    }
    if let Some(dot) = screen.middle_dot {
        painter.circle_filled(dot, (width + 2.0).max(3.0), color);
    }
    for (center, radius) in &screen.studs {
        painter.circle_filled(*center, *radius, Color32::WHITE);
    }
    for (center, radius, fill) in &screen.discs {
        painter.circle_filled(*center, *radius, series_render::color(*fill));
    }
    if let Some((center, glyph, size)) = screen.glyph {
        painter.text(
            center,
            Align2::CENTER_CENTER,
            glyph,
            FontId::proportional(size),
            Color32::WHITE,
        );
    }

    if emphasis.selected {
        for guide in &screen.guides {
            painter.add(Shape::dashed_line(
                &[guide[0], guide[1]],
                Stroke::new(1.0, with_alpha(color, 160)),
                4.0,
                3.0,
            ));
        }
    }

    let accent = series_render::color(style.stroke.color);
    if let Some(mark) = screen.angle {
        draw_angle(painter, mark, accent, style.text.size);
    }
    for label in &screen.end_labels {
        let at = label.at + label.outward * 8.0;
        let align = if label.outward.x.abs() < label.outward.y.abs() {
            if label.outward.y < 0.0 {
                Align2::CENTER_BOTTOM
            } else {
                Align2::CENTER_TOP
            }
        } else if label.outward.x >= 0.0 {
            Align2::LEFT_CENTER
        } else {
            Align2::RIGHT_CENTER
        };
        label_card(
            painter,
            at,
            align,
            &[proj.format.format(label.price)],
            accent,
        );
    }
    if let Some(readout) = &screen.readout {
        painter.text(
            readout.at + vec2(-4.0, 6.0),
            Align2::RIGHT_TOP,
            &readout.text,
            FontId::proportional(11.0),
            accent,
        );
    }
    if screen.arrow.is_some() {
        draw_arrow_extras(painter, screen, drawing, style, proj, emphasis.selected);
    }
    if style.hide_text && emphasis.text != TextMode::Invite {
        return None;
    }
    // A note that opens from a marker has nothing to show while it is shut.
    if screen.popup.is_some() && !emphasis.open {
        return None;
    }
    let place = screen.text?;
    let words = shown_text(screen, drawing);
    // A note that opens from a marker is cut to its box, which is held on the plot.
    let clipped;
    let painter = match &screen.popup {
        Some(popup) => {
            clipped = painter.with_clip_rect(popup.rect.intersect(painter.clip_rect()));
            &clipped
        }
        None => painter,
    };
    match emphasis.text {
        TextMode::Editing => None,
        // The same place and look as the text will have, faint, so the click lands on it: the
        // invitation where there is no text, and the text itself where it is hidden.
        TextMode::Invite if drawing.text().trim().is_empty() || style.hide_text => {
            let mut faint = style.text;
            faint.color[3] = (u16::from(faint.color[3]) * 45 / 100) as u8;
            faint.italic = true;
            let words = if drawing.text().trim().is_empty() {
                INVITATION
            } else {
                drawing.text()
            };
            draw_text(painter, place, words, &faint, false)
        }
        _ => draw_text(
            painter,
            place,
            &words,
            &style.text,
            style.middle_line.enabled,
        ),
    }
}

/// The text a drawing shows: what its tool writes (a price) and then its own text.
fn shown_text(screen: &Screen, drawing: &Drawing) -> String {
    match (&screen.heading, drawing.text().trim().is_empty()) {
        (Some(heading), true) => heading.clone(),
        (Some(heading), false) => format!("{heading}\n{}", drawing.text()),
        (None, _) => drawing.text().to_owned(),
    }
}

/// What the empty text place of a selected drawing says.
const INVITATION: &str = "Add text";

/// The trend angle's mark: a level reference line from the left end, an arc up (or down) to the
/// line, and the angle in degrees beside it.
fn draw_angle(painter: &Painter, mark: super::shape::AngleMark, color: Color32, size: f32) {
    let delta = mark.toward - mark.origin;
    let length = delta.length();
    if length < 12.0 {
        return;
    }
    let faint = with_alpha(color, 150);
    // Up on screen is positive, as a rising line reads.
    let degrees = (-delta.y).atan2(delta.x).to_degrees();
    let reference = (length * 0.9).min(160.0).max(length.min(40.0));
    stroke_path(
        painter,
        &[mark.origin, mark.origin + vec2(reference, 0.0)],
        false,
        Stroke::new(1.0, faint),
        LineDash::Dashed,
    );
    let radius = (length * 0.45).clamp(14.0, 42.0);
    let sweep = delta.y.atan2(delta.x);
    let steps = 24;
    let arc: Vec<Pos2> = (0..=steps)
        .map(|step| {
            let t = sweep * step as f32 / steps as f32;
            mark.origin + vec2(t.cos(), t.sin()) * radius
        })
        .collect();
    painter.add(Shape::line(arc, Stroke::new(1.0, faint)));
    let middle = sweep / 2.0;
    painter.text(
        mark.origin + vec2(middle.cos(), middle.sin()) * (radius + 6.0),
        Align2::LEFT_CENTER,
        format!("{degrees:.1}°"),
        FontId::proportional(size.min(13.0)),
        color,
    );
}

/// Paints a line's tag on the axis it belongs to: its price on the price axis, its time on the
/// time axis. `rect` is the chart with its axes, `plot` the part inside them.
pub(super) fn paint_tag(
    painter: &Painter,
    tag: Tag,
    rect: Rect,
    plot: Rect,
    proj: &Projection,
    style: &CanvasStyle,
) {
    match tag {
        Tag::Price { y, price, color } => {
            let half = style.tag_half_height();
            let area = Rect::from_min_max(
                Pos2::new(plot.right() + 3.0, y - half),
                Pos2::new(rect.right() - 3.0, y + half),
            );
            painter.rect_filled(area, 3.0, series_render::color(color).to_opaque());
            price_text::paint(
                painter,
                area.center(),
                Align2::CENTER_CENTER,
                // On the axis it reads as the axis does (a change in Percent), while the line
                // stays anchored to the real price.
                &proj.scale.tag_label(proj.format, price),
                style.price_font(),
                Color32::WHITE,
            );
        }
        Tag::Time { x, time_ms, color } => {
            let width = 116.0 * style.time_size / 9.0;
            let height = style.time_axis_height() - 6.0;
            // Kept on the chart, so a line at its edge still shows a whole tag.
            let x = x.clamp(plot.left() + width / 2.0, plot.right() - width / 2.0);
            let area = Rect::from_center_size(
                Pos2::new(x, rect.bottom() - style.time_axis_height() / 2.0),
                vec2(width, height),
            );
            painter.rect_filled(area, 3.0, series_render::color(color).to_opaque());
            painter.text(
                area.center(),
                Align2::CENTER_CENTER,
                crate::format::timestamp(time_ms),
                style.time_font(),
                Color32::WHITE,
            );
        }
    }
}

/// What marks a selected or hovered freehand stroke: it has no handles, so its bounds are
/// outlined (dashed while selected, faint while hovered).
fn paint_stroke_frame(painter: &Painter, bounds: Rect, color: Color32, emphasis: Emphasis) {
    let corners = [
        bounds.left_top(),
        bounds.right_top(),
        bounds.right_bottom(),
        bounds.left_bottom(),
        bounds.left_top(),
    ];
    if emphasis.selected {
        painter.extend(Shape::dashed_line(
            &corners,
            Stroke::new(1.0, color.to_opaque()),
            4.0,
            3.0,
        ));
    } else if emphasis.halo {
        painter.add(Shape::line(
            corners.to_vec(),
            Stroke::new(1.0, with_alpha(color, 90)),
        ));
    }
}

/// The handles of a selected drawing.
pub(super) fn paint_handles(painter: &Painter, screen: &Screen, color: Color32) {
    for spot in &screen.handles {
        draw_handle(painter, spot, color);
    }
}

/// A rectangle's bounds as the marching hint shown around a locked selection.
pub(super) fn paint_locked_frame(painter: &Painter, bounds: Rect, color: Color32) {
    painter.rect_stroke(
        bounds,
        2.0,
        Stroke::new(1.0, with_alpha(color, 120)),
        StrokeKind::Outside,
    );
}
