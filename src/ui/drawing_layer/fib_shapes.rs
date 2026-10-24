//! The Fibonacci drawings projected onto the plot: their levels, fills, labels and handles. Each
//! function turns one geometry and the projection into a [`Screen`]; the levels themselves are
//! `model::fib` and the curves `ui::fib_geometry`.
//!
//! The preview while a drawing is placed and the finished drawing go through the same code, so one
//! never differs from the other. Levels are worked from the stored prices and bars (so they do not
//! move with the price scale mode), and only mapped to the screen here.

use super::Projection;
use super::shape::{Band, LabelLook, LabelPlace, PivotLabel, Screen, Styled, vertex_handles};
use crate::model::chart_type::Rgba;
use crate::model::drawing_style::{DrawingStyle, HorizontalAlign, VerticalAlign};
use crate::model::fib::{self, FibKind};
use crate::model::fib_style::{FibLevel, FibStyle};
use crate::model::{Anchor, Geometry};
use eframe::egui::{Pos2, Rect, pos2, vec2};

/// How far (px) beyond the plot a level may be and still be built: its stroke may reach in.
const OFF_PLOT: f32 = 12.0;
/// The space (px) between a level's line and its label, and between the label and the line's end.
const LABEL_GAP: f32 = 3.0;

/// The style a `kind` drawing is drawn with: its own, or the tool's defaults for a style that
/// carries none (a drawing made by code, or by a client that sends no levels).
pub(super) fn fib_style(kind: FibKind, style: &DrawingStyle) -> FibStyle {
    style
        .fib
        .unwrap_or_else(|| FibStyle::for_tool(kind.tool()).unwrap_or_default())
}

/// Builds the screen shape of a Fibonacci drawing. `anchors` are the geometry's anchors on
/// screen, already checked to be finite.
pub(super) fn project(
    geometry: &Geometry,
    style: &DrawingStyle,
    proj: &Projection,
    anchors: Vec<Pos2>,
) -> Option<Screen> {
    let Geometry::Fib { kind, points } = geometry else {
        return None;
    };
    if anchors.len() != kind.points() || points.len() != kind.points() {
        return None;
    }
    let fib = fib_style(*kind, style);
    let screen = match kind {
        FibKind::Retracement => retracement(style, &fib, proj, points, &anchors),
    };
    screen.finish(style.stroke.width / 2.0 + 8.0)
}

// MARK: Shared pieces

/// How a level is drawn: its own colour, or the drawing's when it is set to use one colour.
fn look(level: &FibLevel, style: &DrawingStyle) -> Rgba {
    if style.use_one_color {
        style.stroke.color
    } else {
        level.color
    }
}

fn styled(path: Vec<Pos2>, level: &FibLevel, color: Rgba) -> Styled {
    Styled {
        path,
        color,
        width: level.width,
        dash: level.dash,
    }
}

/// What a level's label says: its ratio, its price, or both (`0.618 (123.4)`), as the style asks;
/// `None` when it asks for neither.
fn label_text(proj: &Projection, fib: &FibStyle, ratio: f64, price: Option<f64>) -> Option<String> {
    let value = fib
        .show_labels
        .then(|| fib::level_label(ratio, fib.as_percent));
    let price = price
        .filter(|_| fib.show_prices)
        .map(|price| proj.format.format(price));
    match (value, price) {
        (Some(value), Some(price)) => Some(format!("{value} ({price})")),
        (Some(text), None) | (None, Some(text)) => Some(text),
        (None, None) => None,
    }
}

/// The label of a horizontal line from `left` to `right` at `y`, placed by the style's alignment
/// against the part of the line that is on the plot. A bottom-aligned label sits on the line (its
/// bottom edge on it), a top-aligned one hangs under it.
fn horizontal_label(
    text: String,
    color: Rgba,
    (left, right): (f32, f32),
    y: f32,
    fib: &FibStyle,
    plot: Rect,
) -> Option<PivotLabel> {
    let (from, to) = (left.max(plot.left()), right.min(plot.right()));
    if from > to || y < plot.top() - OFF_PLOT || y > plot.bottom() + OFF_PLOT {
        return None;
    }
    let (x, edge_x) = match fib.label_h {
        HorizontalAlign::Left => (from + LABEL_GAP, -1.0),
        HorizontalAlign::Center => ((from + to) / 2.0, 0.0),
        HorizontalAlign::Right => (to - LABEL_GAP, 1.0),
    };
    let (dy, edge_y) = match fib.label_v {
        VerticalAlign::Bottom => (-LABEL_GAP, 1.0),
        VerticalAlign::Middle => (0.0, 0.0),
        VerticalAlign::Top => (LABEL_GAP, -1.0),
    };
    Some(PivotLabel {
        at: pos2(x, y + dy),
        text,
        look: LabelLook::Ink(color),
        place: LabelPlace::On,
        push: vec2(0.0, if edge_y > 0.0 { -1.0 } else { 1.0 }),
        edge: vec2(edge_x, edge_y),
        on_select: false,
    })
}

/// The x range a drawing's horizontal levels cover: between its anchors, run on to the plot's
/// edges where the style extends them.
fn level_span(style: &DrawingStyle, plot: Rect, a: Pos2, b: Pos2) -> (f32, f32) {
    let mut left = a.x.min(b.x);
    let mut right = a.x.max(b.x);
    if style.extend_left {
        left = plot.left() - OFF_PLOT;
    }
    if style.extend_right {
        right = plot.right() + OFF_PLOT;
    }
    (
        left.clamp(plot.left() - OFF_PLOT, plot.right() + OFF_PLOT),
        right.clamp(plot.left() - OFF_PLOT, plot.right() + OFF_PLOT),
    )
}

/// The rectangle between two horizontal levels over `span`, cut to the plot, as a fill band.
fn band_between(
    plot: Rect,
    span: (f32, f32),
    ys: (f32, f32),
    color: Rgba,
    alpha: u8,
) -> Option<Band> {
    let (top, bottom) = (ys.0.min(ys.1), ys.0.max(ys.1));
    let rect = Rect::from_min_max(pos2(span.0, top), pos2(span.1, bottom)).intersect(plot);
    (rect.width() > 0.0 && rect.height() > 0.0).then(|| Band {
        polygon: vec![
            rect.left_top(),
            rect.right_top(),
            rect.right_bottom(),
            rect.left_bottom(),
        ],
        color: [color[0], color[1], color[2], alpha],
    })
}

/// One level of a set of horizontal levels: its price, where it falls, and how it is drawn.
struct Row {
    ratio: f32,
    price: f64,
    y: f32,
    level: FibLevel,
}

/// Lays the horizontal levels `price_of` gives out as lines, fills between neighbouring levels
/// (in ratio order, each in the colour of the farther one) and labels, over `span`.
fn horizontal_levels(
    screen: &mut Screen,
    style: &DrawingStyle,
    fib: &FibStyle,
    proj: &Projection,
    span: (f32, f32),
    price_of: impl Fn(f64) -> Option<f64>,
) {
    let mut rows: Vec<Row> = fib
        .levels
        .active()
        .filter_map(|level| {
            let price = price_of(f64::from(level.ratio))?;
            let y = proj.scale.y(price);
            y.is_finite().then_some(Row {
                ratio: level.ratio,
                price,
                y,
                level: *level,
            })
        })
        .collect();
    rows.sort_by(|a, b| a.ratio.total_cmp(&b.ratio));
    let alpha = style.fill.color[3];
    for pair in rows.windows(2) {
        let color = look(&pair[1].level, style);
        if let Some(band) = band_between(proj.plot, span, (pair[0].y, pair[1].y), color, alpha) {
            screen.bands.push(band);
        }
    }
    for row in &rows {
        let color = look(&row.level, style);
        let near_plot =
            row.y >= proj.plot.top() - OFF_PLOT && row.y <= proj.plot.bottom() + OFF_PLOT;
        if near_plot && span.0 < span.1 {
            screen.styled.push(styled(
                vec![pos2(span.0, row.y), pos2(span.1, row.y)],
                &row.level,
                color,
            ));
        }
        if let Some(text) = label_text(proj, fib, f64::from(row.ratio), Some(row.price))
            && let Some(label) = horizontal_label(text, color, span, row.y, fib, proj.plot)
        {
            screen.labels.push(label);
        }
    }
}

// MARK: Retracement

fn retracement(
    style: &DrawingStyle,
    fib: &FibStyle,
    proj: &Projection,
    points: &[Anchor],
    anchors: &[Pos2],
) -> Screen {
    let (mut start, mut end) = (points[0].price, points[1].price);
    if fib.reverse {
        std::mem::swap(&mut start, &mut end);
    }
    // The ratio of the prices only means something on a logarithmic chart.
    let log = fib.log_levels && proj.scale.logarithmic;
    let span = level_span(style, proj.plot, anchors[0], anchors[1]);
    let mut screen = Screen::empty();
    if fib.trend_line {
        screen.lines.push(anchors.to_vec());
    }
    horizontal_levels(&mut screen, style, fib, proj, span, |ratio| {
        fib::retracement_price(start, end, ratio, log)
    });
    screen.handles = vertex_handles(anchors);
    screen
}
