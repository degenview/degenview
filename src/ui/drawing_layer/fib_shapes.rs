//! The Fibonacci drawings projected onto the plot: their levels, fills, labels and handles. Each
//! function turns one geometry and the projection into a [`Screen`]; the levels themselves are
//! `model::fib` and the curves `ui::fib_geometry`.
//!
//! The preview while a drawing is placed and the finished drawing go through the same code, so one
//! never differs from the other. Levels are worked from the stored prices and bars (so they do not
//! move with the price scale mode), and only mapped to the screen here.

use super::Projection;
use super::line_shapes;
use super::shape::{Band, LabelLook, LabelPlace, PivotLabel, Screen, Styled, vertex_handles};
use crate::model::PitchforkVariant;
use crate::model::chart_type::Rgba;
use crate::model::drawing_style::{DrawingStyle, HorizontalAlign, VerticalAlign};
use crate::model::fib::{self, FibKind};
use crate::model::fib_style::{FibLevel, FibStyle};
use crate::model::{Anchor, Geometry};
use crate::ui::fib_geometry;
use crate::ui::line_geometry as lines;
use eframe::egui::{Pos2, Rect, Vec2, pos2, vec2};

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
        FibKind::Extension => extension(style, &fib, proj, points, &anchors),
        FibKind::Channel => channel(style, &fib, proj, &anchors),
        FibKind::TimeZone => {
            let origin = proj.times.index_of(points[0].time_ms);
            let interval = bars_between(proj, points[0], points[1]);
            time_tool(style, &fib, proj, &anchors, origin, interval)
        }
        FibKind::TrendTime => {
            let origin = proj.times.index_of(points[2].time_ms);
            let interval = bars_between(proj, points[0], points[1]);
            time_tool(style, &fib, proj, &anchors, origin, interval)
        }
        FibKind::SpeedFan => speed_fan(style, &fib, proj, &anchors),
        FibKind::Circles => circles(style, &fib, proj, &anchors),
        FibKind::Spiral => spiral(&fib, proj, &anchors),
        FibKind::SpeedArcs => speed_arcs(style, &fib, proj, &anchors),
        FibKind::Wedge => wedge(style, &fib, proj, &anchors),
        FibKind::Pitchfan => pitchfan(style, &fib, proj, &anchors),
    };
    let mut screen = screen;
    screen.backdrop = true;
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

/// The label of a level line from `from` to `to`, placed by the style's alignment against the
/// part of the line that is on the plot: its left end, middle or right end, and above it (bottom
/// aligned: the label's bottom edge on the line), on it or under it (top aligned). `text` is
/// given the height the label sits at, for the price there. A label that meets another gives way
/// along `push`, which is the way the levels' ratios grow on screen, so the labels keep their order.
fn segment_label(
    text: impl FnOnce(f32) -> Option<String>,
    color: Rgba,
    (from, to): (Pos2, Pos2),
    push: Vec2,
    fib: &FibStyle,
    plot: Rect,
) -> Option<PivotLabel> {
    let (from, to) = if from.x <= to.x {
        (from, to)
    } else {
        (to, from)
    };
    let (low, high) = (from.x.max(plot.left()), to.x.min(plot.right()));
    if low > high {
        return None;
    }
    let (x, edge_x) = match fib.label_h {
        HorizontalAlign::Left => (low + LABEL_GAP, -1.0),
        HorizontalAlign::Center => ((low + high) / 2.0, 0.0),
        HorizontalAlign::Right => (high - LABEL_GAP, 1.0),
    };
    let run = to.x - from.x;
    let y = if run.abs() > 1e-3 {
        from.y + (to.y - from.y) * (x - from.x) / run
    } else {
        from.y
    };
    if !y.is_finite() || y < plot.top() - OFF_PLOT || y > plot.bottom() + OFF_PLOT {
        return None;
    }
    let (dy, edge_y) = match fib.label_v {
        VerticalAlign::Bottom => (-LABEL_GAP, 1.0),
        VerticalAlign::Middle => (0.0, 0.0),
        VerticalAlign::Top => (LABEL_GAP, -1.0),
    };
    Some(PivotLabel {
        at: pos2(x, y + dy),
        text: text(y)?,
        look: LabelLook::Ink(color),
        place: LabelPlace::On,
        push,
        edge: vec2(edge_x, edge_y),
        on_select: false,
    })
}

/// The label of a vertical line at `x`: beside it (the side the style's horizontal alignment
/// names) and at the top, middle or bottom of the plot.
fn vertical_label(
    text: String,
    color: Rgba,
    x: f32,
    fib: &FibStyle,
    plot: Rect,
) -> Option<PivotLabel> {
    if !x.is_finite() || x < plot.left() - OFF_PLOT || x > plot.right() + OFF_PLOT {
        return None;
    }
    let (at_x, edge_x) = match fib.label_h {
        HorizontalAlign::Left => (x - LABEL_GAP, 1.0),
        HorizontalAlign::Center => (x, 0.0),
        HorizontalAlign::Right => (x + LABEL_GAP, -1.0),
    };
    let (at_y, edge_y) = match fib.label_v {
        VerticalAlign::Top => (plot.top() + LABEL_GAP, -1.0),
        VerticalAlign::Middle => (plot.center().y, 0.0),
        VerticalAlign::Bottom => (plot.bottom() - LABEL_GAP, 1.0),
    };
    Some(PivotLabel {
        at: pos2(at_x, at_y),
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
fn level_span(style: &DrawingStyle, plot: Rect, anchors: &[Pos2]) -> (f32, f32) {
    let mut left = anchors.iter().map(|a| a.x).fold(f32::INFINITY, f32::min);
    let mut right = anchors
        .iter()
        .map(|a| a.x)
        .fold(f32::NEG_INFINITY, f32::max);
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
    // The way the levels climb the screen: from the lowest ratio to the highest.
    let push = match (rows.first(), rows.last()) {
        (Some(first), Some(last)) if last.y != first.y => vec2(0.0, (last.y - first.y).signum()),
        _ => vec2(0.0, -1.0),
    };
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
        let text = |_: f32| label_text(proj, fib, f64::from(row.ratio), Some(row.price));
        let ends = (pos2(span.0, row.y), pos2(span.1, row.y));
        if let Some(label) = segment_label(text, color, ends, push, fib, proj.plot) {
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
    let span = level_span(style, proj.plot, anchors);
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

// MARK: Extension

fn extension(
    style: &DrawingStyle,
    fib: &FibStyle,
    proj: &Projection,
    points: &[Anchor],
    anchors: &[Pos2],
) -> Screen {
    let (mut first, mut second) = (points[0].price, points[1].price);
    // Reversed, the move is projected the other way: the first two prices change places.
    if fib.reverse {
        std::mem::swap(&mut first, &mut second);
    }
    let third = points[2].price;
    let log = fib.log_levels && proj.scale.logarithmic;
    let span = level_span(style, proj.plot, anchors);
    let mut screen = Screen::empty();
    if fib.trend_line {
        screen.lines.push(anchors.to_vec());
    }
    horizontal_levels(&mut screen, style, fib, proj, span, |ratio| {
        fib::extension_price(first, second, third, ratio, log)
    });
    screen.handles = vertex_handles(anchors);
    screen
}

// MARK: Channel

fn channel(style: &DrawingStyle, fib: &FibStyle, proj: &Projection, anchors: &[Pos2]) -> Screen {
    let (pa, pb, pc) = (anchors[0], anchors[1], anchors[2]);
    let plot = proj.plot;
    let mut screen = Screen::empty();
    screen.handles = vertex_handles(anchors);
    // A base line that runs straight up has no height to measure a width against.
    if (pb.x - pa.x).abs() <= 1e-3 {
        return screen;
    }
    // "Left" is the left of the screen, whichever way the base line was drawn.
    let (before, after) = if pa.x <= pb.x {
        (style.extend_left, style.extend_right)
    } else {
        (style.extend_right, style.extend_left)
    };
    let offset = lines::channel_offset(pa, pb, pc);
    let mut rows: Vec<&FibLevel> = fib.levels.active().collect();
    rows.sort_by(|a, b| a.ratio.total_cmp(&b.ratio));
    // Higher ratios are further along the offset, so a label that meets another gives way that way.
    let push = if offset.length() > 1e-3 {
        offset.normalized()
    } else {
        vec2(0.0, -1.0)
    };
    let alpha = style.fill.color[3];
    for pair in rows.windows(2) {
        let near = pa + offset * pair[0].ratio;
        let far = offset * (pair[1].ratio - pair[0].ratio);
        let polygon =
            line_shapes::band(near, pb + offset * pair[0].ratio, far, before, after, proj);
        if polygon.len() >= 3 {
            let color = look(pair[1], style);
            screen.bands.push(Band {
                polygon,
                color: [color[0], color[1], color[2], alpha],
            });
        }
    }
    for level in rows {
        let (from, to) = (pa + offset * level.ratio, pb + offset * level.ratio);
        let Some((from, to)) = lines::line_in_rect(from, to, plot, before, after) else {
            continue;
        };
        let color = look(level, style);
        screen.styled.push(styled(vec![from, to], level, color));
        let text =
            |y: f32| label_text(proj, fib, f64::from(level.ratio), Some(proj.scale.price(y)));
        if let Some(label) = segment_label(text, color, (from, to), push, fib, plot) {
            screen.labels.push(label);
        }
    }
    screen
}

// MARK: Time tools

/// The distance in bars from one anchor to another, fractional between bars.
fn bars_between(proj: &Projection, from: Anchor, to: Anchor) -> f64 {
    proj.times.index_of(to.time_ms) - proj.times.index_of(from.time_ms)
}

/// A time zone or a trend-based time: a vertical line at `origin + ratio * interval` bars for each
/// level, a fill between neighbouring lines, a label on each, and the trend line through the
/// anchors. At most the list's length of lines, however far the plot is zoomed out.
fn time_tool(
    style: &DrawingStyle,
    fib: &FibStyle,
    proj: &Projection,
    anchors: &[Pos2],
    origin: f64,
    interval: f64,
) -> Screen {
    let plot = proj.plot;
    let mut screen = Screen::empty();
    if fib.trend_line {
        screen.lines.push(anchors.to_vec());
    }
    screen.handles = vertex_handles(anchors);
    // With no interval every line is the same line: none is drawn.
    if !interval.is_finite() || interval.abs() < 1e-9 {
        return screen;
    }
    let mut rows: Vec<(f32, FibLevel)> = fib
        .levels
        .active()
        .filter_map(|level| {
            let index = fib::time_line(origin, interval, f64::from(level.ratio))?;
            let x = proj.x_at_index(index);
            x.is_finite().then_some((x, *level))
        })
        .collect();
    rows.sort_by(|a, b| a.1.ratio.total_cmp(&b.1.ratio));
    let alpha = style.fill.color[3];
    for pair in rows.windows(2) {
        let (low, high) = (pair[0].0.min(pair[1].0), pair[0].0.max(pair[1].0));
        let rect =
            Rect::from_min_max(pos2(low, plot.top()), pos2(high, plot.bottom())).intersect(plot);
        if rect.width() > 0.0 && rect.height() > 0.0 {
            let color = look(&pair[1].1, style);
            screen.bands.push(Band {
                polygon: vec![
                    rect.left_top(),
                    rect.right_top(),
                    rect.right_bottom(),
                    rect.left_bottom(),
                ],
                color: [color[0], color[1], color[2], alpha],
            });
        }
    }
    for (x, level) in rows {
        if x < plot.left() - OFF_PLOT || x > plot.right() + OFF_PLOT {
            continue;
        }
        let color = look(&level, style);
        screen.styled.push(styled(
            vec![pos2(x, plot.top()), pos2(x, plot.bottom())],
            &level,
            color,
        ));
        if fib.show_labels
            && let Some(label) = vertical_label(
                fib::level_label(f64::from(level.ratio), false),
                color,
                x,
                fib,
                plot,
            )
        {
            screen.labels.push(label);
        }
    }
    screen
}

// MARK: Rings

/// The levels that make a ring (a ratio of zero is a point, a negative one has no radius), by
/// growing ratio.
fn ring_levels(fib: &FibStyle) -> Vec<FibLevel> {
    let mut rows: Vec<FibLevel> = fib
        .levels
        .active()
        .filter(|level| level.ratio > 0.0)
        .copied()
        .collect();
    rows.sort_by(|a, b| a.ratio.total_cmp(&b.ratio));
    rows
}

/// One ring of a set of concentric rings: the ellipse, the stretch of it that is drawn (a whole
/// turn or an arc), the point the pie it bounds comes to (for an arc, its centre) and where its
/// label sits.
struct Ring {
    center: Pos2,
    rx: f32,
    ry: f32,
    range: (f64, f64),
    apex: Option<Pos2>,
    label: Pos2,
}

/// Draws the rings `ring_of` makes for each level: the curve in the level's look, the fill
/// between it and the ring before it (in the colour of the outer), and its label.
fn rings(
    screen: &mut Screen,
    style: &DrawingStyle,
    fib: &FibStyle,
    proj: &Projection,
    levels: &[FibLevel],
    ring_of: impl Fn(f32) -> Ring,
) {
    let plot = proj.plot;
    let alpha = style.fill.color[3];
    let mut inner: Vec<Pos2> = Vec::new();
    for (index, level) in levels.iter().enumerate() {
        let ring = ring_of(level.ratio);
        let color = look(level, style);
        if let Some(runs) = fib_geometry::arc(ring.center, ring.rx, ring.ry, ring.range, plot) {
            for run in runs.into_iter().filter(|run| run.len() >= 2) {
                screen.styled.push(styled(run, level, color));
            }
        }
        let disc = fib_geometry::disc(ring.center, ring.rx, ring.ry, ring.range, ring.apex, plot);
        if index > 0 {
            let polygon = fib_geometry::annulus(&disc, &inner);
            if polygon.len() >= 3 {
                screen.bands.push(Band {
                    polygon,
                    color: [color[0], color[1], color[2], alpha],
                });
            }
        }
        inner = disc;
        if fib.show_labels && plot.expand(OFF_PLOT).contains(ring.label) {
            screen.labels.push(PivotLabel {
                at: ring.label,
                text: fib::level_label(f64::from(level.ratio), fib.as_percent),
                look: LabelLook::Ink(color),
                place: LabelPlace::On,
                push: vec2(0.0, -1.0),
                edge: vec2(0.0, 0.0),
                on_select: false,
            });
        }
    }
}

// MARK: Circles

/// Ellipses about the middle of the two anchors: the ratio 1 one passes through both of them and
/// has the box they make for its bounding box scaled by the square root of two, so the trend
/// line at 45 degrees (Shift) makes true circles.
fn circles(style: &DrawingStyle, fib: &FibStyle, proj: &Projection, anchors: &[Pos2]) -> Screen {
    let (a, b) = (anchors[0], anchors[1]);
    let mut screen = Screen::empty();
    screen.handles = vertex_handles(anchors);
    if fib.trend_line {
        screen.lines.push(vec![a, b]);
    }
    let center = a.lerp(b, 0.5);
    let half = (b - a) * 0.5;
    let (rx, ry) = (
        half.x.abs() * std::f32::consts::SQRT_2,
        half.y.abs() * std::f32::consts::SQRT_2,
    );
    let levels = ring_levels(fib);
    rings(&mut screen, style, fib, proj, &levels, |ratio| Ring {
        center,
        rx: rx * ratio,
        ry: ry * ratio,
        range: (0.0, std::f64::consts::TAU),
        apex: None,
        label: pos2(center.x, center.y - ry * ratio),
    });
    screen
}

// MARK: Speed resistance arcs

/// Arcs about the second anchor at ratios of its distance from the first, the half facing the
/// first (or whole circles).
fn speed_arcs(style: &DrawingStyle, fib: &FibStyle, proj: &Projection, anchors: &[Pos2]) -> Screen {
    let (start, center) = (anchors[0], anchors[1]);
    let mut screen = Screen::empty();
    screen.handles = vertex_handles(anchors);
    if fib.trend_line {
        screen.lines.push(vec![start, center]);
    }
    let base = start - center;
    let length = base.length();
    if length <= 1e-3 {
        return screen;
    }
    let facing = f64::from(base.y).atan2(f64::from(base.x));
    let range = if fib.full_circles {
        (0.0, std::f64::consts::TAU)
    } else {
        (
            facing - std::f64::consts::FRAC_PI_2,
            facing + std::f64::consts::FRAC_PI_2,
        )
    };
    let levels = ring_levels(fib);
    rings(&mut screen, style, fib, proj, &levels, |ratio| {
        let radius = length * ratio;
        // The end of the half facing the first anchor, where the label sits.
        let end = fib_geometry::direction((facing - std::f64::consts::FRAC_PI_2) as f32);
        Ring {
            center,
            rx: radius,
            ry: radius,
            range,
            apex: (!fib.full_circles).then_some(center),
            label: center + end * radius,
        }
    });
    screen
}

// MARK: Wedge

/// Arcs about an apex between two arms: the second anchor sets the first arm and the radius, the
/// third the direction of the second arm; the arcs run the short way between them.
fn wedge(style: &DrawingStyle, fib: &FibStyle, proj: &Projection, anchors: &[Pos2]) -> Screen {
    let (apex, first, second) = (anchors[0], anchors[1], anchors[2]);
    let mut screen = Screen::empty();
    screen.handles = vertex_handles(anchors);
    let arm = first - apex;
    let radius = arm.length();
    let span_to = second - apex;
    if radius <= 1e-3 || span_to.length() <= 1e-3 {
        screen.lines.push(vec![apex, first]);
        return screen;
    }
    let start = f64::from(arm.y).atan2(f64::from(arm.x));
    let end = f64::from(span_to.y).atan2(f64::from(span_to.x));
    let sweep = fib_geometry::sweep(start, end);
    let range = (start, start + sweep);
    let levels = ring_levels(fib);
    if fib.trend_line {
        // The arms run out to the furthest ring drawn (or the anchor, if that is nearer).
        let furthest = levels.last().map_or(1.0, |level| level.ratio.max(1.0));
        for angle in [start, end] {
            let tip = apex + fib_geometry::direction(angle as f32) * (radius * furthest);
            if let Some((from, to)) = lines::line_in_rect(apex, tip, proj.plot, false, false) {
                screen.lines.push(vec![from, to]);
            }
        }
    }
    rings(&mut screen, style, fib, proj, &levels, |ratio| {
        let radius = radius * ratio;
        let middle = fib_geometry::direction((start + sweep / 2.0) as f32);
        Ring {
            center: apex,
            rx: radius,
            ry: radius,
            range,
            apex: Some(apex),
            label: apex + middle * radius,
        }
    });
    screen
}

// MARK: Spiral

/// The golden spiral about the first anchor through the second.
fn spiral(fib: &FibStyle, proj: &Projection, anchors: &[Pos2]) -> Screen {
    let (center, through) = (anchors[0], anchors[1]);
    let mut screen = Screen::empty();
    screen.handles = vertex_handles(anchors);
    let arm = through - center;
    let angle = f64::from(arm.y).atan2(f64::from(arm.x));
    match fib_geometry::golden_spiral(
        center,
        arm.length(),
        angle,
        !fib.counterclockwise,
        proj.plot,
    ) {
        Some(runs) => screen.lines = runs.into_iter().filter(|run| run.len() >= 2).collect(),
        None => screen.lines.push(vec![center, through]),
    }
    screen
}

// MARK: Speed resistance fan

/// Rays from the first anchor through each ratio of the box the anchors make: those through the
/// far vertical side (price levels, below the trend line when it rises), and through the far
/// horizontal one (time levels, above it); a grid of the same ratios fills the box.
fn speed_fan(style: &DrawingStyle, fib: &FibStyle, proj: &Projection, anchors: &[Pos2]) -> Screen {
    let plot = proj.plot;
    let mut screen = Screen::empty();
    screen.handles = vertex_handles(anchors);
    // Reversed, the fan opens from the second anchor back towards the first.
    let (origin, far) = if fib.reverse {
        (anchors[1], anchors[0])
    } else {
        (anchors[0], anchors[1])
    };
    let delta = far - origin;
    if delta.length() <= 1e-3 {
        return screen;
    }
    let boxed = Rect::from_two_pos(origin, far);
    let alpha = style.fill.color[3];
    let sorted = |levels: &mut dyn Iterator<Item = &FibLevel>| {
        let mut rows: Vec<FibLevel> = levels.copied().collect();
        rows.sort_by(|a, b| a.ratio.total_cmp(&b.ratio));
        rows
    };
    let families = [
        (true, sorted(&mut fib.levels.active())),
        (false, sorted(&mut fib.second.active())),
    ];
    let mut grid: Vec<Styled> = Vec::new();
    let mut rays: Vec<Styled> = Vec::new();
    for (price, levels) in &families {
        let mut previous: Option<Vec2> = None;
        for level in levels {
            let ratio = level.ratio;
            let (through, along) = if *price {
                let y = origin.y + delta.y * ratio;
                (pos2(far.x, y), [pos2(origin.x, y), pos2(far.x, y)])
            } else {
                let x = origin.x + delta.x * ratio;
                (pos2(x, far.y), [pos2(x, origin.y), pos2(x, far.y)])
            };
            let color = look(level, style);
            if fib.grid.enabled {
                grid.push(Styled::new(along.to_vec(), &fib.grid));
            }
            let direction = through - origin;
            if direction.length() > 1e-3 {
                let direction = direction.normalized();
                if let Some((from, to)) = lines::line_in_rect(origin, through, plot, false, true) {
                    rays.push(styled(vec![from, to], level, color));
                }
                if let Some(before) = previous {
                    let polygon = fib_geometry::fan_band(origin, before, direction, plot);
                    if polygon.len() >= 3 {
                        screen.bands.push(Band {
                            polygon,
                            color: [color[0], color[1], color[2], alpha],
                        });
                    }
                }
                previous = Some(direction);
            }
            if fib.show_labels {
                fan_labels(&mut screen, fib, plot, boxed, *price, level, color, through);
            }
        }
    }
    screen.styled.extend(grid);
    screen.styled.extend(rays);
    screen
}

/// The labels of one level of a fan, on the sides of the box the style asks for: price levels on
/// its left and right, time levels above and below.
#[allow(clippy::too_many_arguments)]
fn fan_labels(
    screen: &mut Screen,
    fib: &FibStyle,
    plot: Rect,
    boxed: Rect,
    price: bool,
    level: &FibLevel,
    color: Rgba,
    through: Pos2,
) {
    let text = fib::level_label(f64::from(level.ratio), false);
    let sides: [(bool, Pos2, Vec2); 2] = if price {
        [
            (
                fib.sides.left,
                pos2(boxed.left() - LABEL_GAP, through.y),
                vec2(1.0, 0.0),
            ),
            (
                fib.sides.right,
                pos2(boxed.right() + LABEL_GAP, through.y),
                vec2(-1.0, 0.0),
            ),
        ]
    } else {
        [
            (
                fib.sides.top,
                pos2(through.x, boxed.top() - LABEL_GAP),
                vec2(0.0, 1.0),
            ),
            (
                fib.sides.bottom,
                pos2(through.x, boxed.bottom() + LABEL_GAP),
                vec2(0.0, -1.0),
            ),
        ]
    };
    for (shown, at, edge) in sides {
        if shown && plot.expand(OFF_PLOT).contains(at) {
            screen.labels.push(PivotLabel {
                at,
                text: text.clone(),
                look: LabelLook::Ink(color),
                place: LabelPlace::On,
                push: vec2(0.0, -1.0),
                edge,
                on_select: false,
            });
        }
    }
}

// MARK: Pitchfan

/// Rays from the first anchor through the middle of the other two (the median) and through the
/// points either side of it at each ratio of half the span between them: the pitchfork's levels,
/// all meeting at its first pivot instead of running parallel.
fn pitchfan(style: &DrawingStyle, fib: &FibStyle, proj: &Projection, anchors: &[Pos2]) -> Screen {
    let (p1, p2, p3) = (anchors[0], anchors[1], anchors[2]);
    let plot = proj.plot;
    let mut screen = Screen::empty();
    screen.handles = vertex_handles(anchors);
    screen.guides = vec![[p1, p2], [p2, p3]];
    let Some(fork) = lines::fork(PitchforkVariant::Standard, p1, p2, p3) else {
        screen.lines.push(vec![p1, p2]);
        return screen;
    };
    // Every ray of the fan, with where it sits across it: the median is 0, a level its ratio on
    // either side.
    struct Ray {
        key: f32,
        direction: Vec2,
        through: Pos2,
        color: Rgba,
        level: Option<FibLevel>,
    }
    let mut fan: Vec<Ray> = Vec::new();
    let mut push = |key: f32, through: Pos2, color: Rgba, level: Option<FibLevel>| {
        let direction = through - p1;
        if direction.length() > 1e-3 {
            fan.push(Ray {
                key,
                direction: direction.normalized(),
                through,
                color,
                level,
            });
        }
    };
    push(0.0, fork.middle, style.stroke.color, None);
    for level in fib.levels.active().filter(|level| level.ratio > 0.0) {
        let color = look(level, style);
        for side in [-1.0_f32, 1.0] {
            push(
                level.ratio * side,
                fork.level_start(level.ratio, side),
                color,
                Some(*level),
            );
        }
    }
    fan.sort_by(|a, b| a.key.total_cmp(&b.key));
    let alpha = style.fill.color[3];
    for pair in fan.windows(2) {
        // The region between two neighbours, in the colour of the one further from the median.
        let outer = if pair[0].key.abs() >= pair[1].key.abs() {
            &pair[0]
        } else {
            &pair[1]
        };
        let polygon = fib_geometry::fan_band(p1, pair[0].direction, pair[1].direction, plot);
        if polygon.len() >= 3 {
            let color = outer.color;
            screen.bands.push(Band {
                polygon,
                color: [color[0], color[1], color[2], alpha],
            });
        }
    }
    for ray in &fan {
        let Some((from, to)) = lines::line_in_rect(p1, ray.through, plot, false, true) else {
            continue;
        };
        match &ray.level {
            Some(level) => screen.styled.push(styled(vec![from, to], level, ray.color)),
            None => screen.lines.push(vec![from, to]),
        }
    }
    screen
}
