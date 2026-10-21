//! Painting a drawing's screen shape: fill, middle line, strokes, glyphs, text, statistics and
//! handles. Colours come from the drawing's own style; nothing here reads the theme except the
//! neutral plates behind labels and the handle faces.

use super::Projection;
use super::shape::{HandleSpot, Screen, TextPlace};
use crate::model::chart_type::LineDash;
use crate::model::drawing_style::{DrawingStyle, HorizontalAlign, TextStyle, VerticalAlign};
use crate::model::{Drawing, Geometry};
use crate::ui::canvas_render::dash_pattern;
use crate::ui::drawing_geometry as geo;
use crate::ui::drawing_render::{ArrowReadout, HANDLE_RADIUS};
use crate::ui::{series_render, theme};
use eframe::egui::epaint::text::VariationCoords;
use eframe::egui::{
    self, Align2, Color32, FontId, Painter, Pos2, Rect, Shape, Stroke, StrokeKind, TextFormat,
    text::LayoutJob, vec2,
};

/// How a drawing is emphasized this frame.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Emphasis {
    /// Hovered or selected: a halo behind the strokes.
    pub halo: bool,
    /// Selected: handles and construction lines.
    pub selected: bool,
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

/// A trend line in its own style: colour, width and dash, prolonged to the plot edge where the
/// style says so. Handles stay on the two anchors, and `emphasized` adds the halo and handles.
pub(super) fn draw_trend_line(
    painter: &Painter,
    a: Pos2,
    b: Pos2,
    plot: Rect,
    style: &DrawingStyle,
    emphasized: bool,
) {
    let color = series_render::color(style.stroke.color);
    let (from, to) = geo::extend_to_rect(a, b, plot, style.extend_left, style.extend_right);
    if emphasized {
        let halo = Stroke::new(style.stroke.width + 5.0, with_alpha(color, 40));
        stroke_path(painter, &[from, to], false, halo, LineDash::Solid);
    }
    stroke_path(
        painter,
        &[from, to],
        false,
        Stroke::new(style.stroke.width, color),
        style.stroke.dash,
    );
    if emphasized {
        for at in [a, b] {
            painter.circle(at, HANDLE_RADIUS, theme::surface(), Stroke::new(1.5, color));
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
    let mut job = LayoutJob::default();
    job.append(
        text,
        0.0,
        TextFormat {
            font_id: FontId::proportional(style.size),
            color: series_render::color(style.color),
            italics: style.italic,
            coords: VariationCoords::new([("wght", if style.bold { 700.0 } else { 400.0 })]),
            ..Default::default()
        },
    );
    job.wrap.max_width = wrap.max(20.0);
    job.halign = match style.align {
        HorizontalAlign::Left => eframe::egui::Align::LEFT,
        HorizontalAlign::Center => eframe::egui::Align::Center,
        HorizontalAlign::Right => eframe::egui::Align::RIGHT,
    };
    painter.layout_job(job)
}

/// A drawing's own text, placed where its shape says.
/// `clear_middle` lifts centred text off a middle line drawn through the same spot.
fn draw_text(
    painter: &Painter,
    place: TextPlace,
    text: &str,
    style: &TextStyle,
    clear_middle: bool,
) {
    if text.trim().is_empty() {
        return;
    }
    const PAD: f32 = 5.0;
    match place {
        TextPlace::Inside(rect) => {
            let galley = text_galley(painter, text, style, rect.width() - PAD * 2.0);
            let size = galley.size();
            let x = match style.align {
                HorizontalAlign::Left => rect.left() + PAD,
                HorizontalAlign::Center => rect.center().x - size.x / 2.0,
                HorizontalAlign::Right => rect.right() - PAD - size.x,
            };
            let y = match style.valign {
                VerticalAlign::Top => rect.top() + PAD,
                VerticalAlign::Middle if clear_middle => rect.center().y - size.y - 2.0,
                VerticalAlign::Middle => rect.center().y - size.y / 2.0,
                VerticalAlign::Bottom => rect.bottom() - PAD - size.y,
            };
            painter.galley(Pos2::new(x, y), galley, series_render::color(style.color));
        }
        TextPlace::At { point, above } => {
            let galley = text_galley(painter, text, style, 320.0);
            let size = galley.size();
            let y = if above {
                point.y - 6.0 - size.y
            } else {
                point.y + 6.0
            };
            painter.galley(
                Pos2::new(point.x - size.x / 2.0, y),
                galley,
                series_render::color(style.color),
            );
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

/// Paints one projected drawing. `painter` is already clipped to the plot.
pub(super) fn paint(
    painter: &Painter,
    screen: &Screen,
    drawing: &Drawing,
    proj: &Projection,
    emphasis: Emphasis,
) {
    let Some(style) = drawing.style() else {
        return;
    };
    let color = series_render::color(style.stroke.color);
    let width = style.stroke.width;

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
        return;
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
        return;
    }

    if let Some(mesh) = &screen.fill_mesh {
        painter.add(Shape::mesh(mesh.clone()));
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

    if let Some(place) = screen.text {
        draw_text(
            painter,
            place,
            drawing.text(),
            &style.text,
            style.middle_line.enabled,
        );
    }
    if matches!(
        drawing,
        Drawing::Shape {
            geometry: Geometry::Arrow { .. },
            ..
        }
    ) {
        draw_arrow_extras(painter, screen, drawing, style, proj, emphasis.selected);
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
