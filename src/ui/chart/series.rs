//! Paints the main price series of a chart in whichever way its chart type asks.
//!
//! What a bar looks like (hollow or filled, which colour, how wide a volume candle is, where a
//! step turns, where a baseline line crosses) is decided in `model::chart::look` and tested there;
//! this module turns those decisions into painter calls. The chart card owns the coordinate
//! system and hands it in as `x_for` and the price scale, so every type shares one set of axes:
//! one slot per bar, left to right. Price-based types have no time axis of their own: their
//! slots are bricks, columns or segments, in the order they formed.
//!
//! Everything here is the *main series*. A script's `plotcandle()` and `plotbar()` are drawn by
//! `pine_chart_render` as their own layer and are unaffected by the chart type.

use super::card::PriceScale;
use crate::model::chart;
use crate::model::chart::kind::{
    ChartStyle, ChartType, LineDash, LineSettings, PriceSource, RangeLook, Rgba,
};
use crate::model::chart::look::{
    CandleLook, Fill, Palette, drawn_extremes, split_at_level, step_points, volume_width_fraction,
};
use crate::model::chart::synthetic::{BarShape, SyntheticBar};
use crate::model::{Kline, PriceFormat};
use crate::ui::theme;
use eframe::egui::{
    Align2, Color32, FontId, Painter, Pos2, Rect, Shape, Stroke, StrokeKind, Vec2,
    epaint::{Mesh, Vertex, WHITE_UV},
};

/// How far a bar that can still change is faded.
const UNCONFIRMED_ALPHA: f32 = 0.55;

pub struct SeriesFrame<'a> {
    pub painter: &'a Painter,
    pub plot: Rect,
    /// The bars in view (the provider's candles, or the constructed bars of a price-based type).
    pub bars: &'a [Kline],
    /// For a price-based type, the constructed bars in view, aligned with `bars`.
    pub synthetic: Option<&'a [SyntheticBar]>,
    /// The box size a Point & Figure column is made of.
    pub box_size: Option<f64>,
    /// The close of the bar before `bars[0]`, for colouring by the previous close.
    pub previous_close: Option<f64>,
    pub slot_width: f32,
    pub x_for: &'a dyn Fn(usize) -> f32,
    pub scale: &'a PriceScale,
    pub palette: Palette,
    pub style: &'a ChartStyle,
    /// The type to draw, already checked against what the source can show.
    pub chart_type: ChartType,
    /// `barcolor()` of the bar at an index of `bars`.
    pub recolor: Option<&'a dyn Fn(usize) -> Option<Color32>>,
    /// How the chart writes a price (the high and low markers).
    pub price_format: PriceFormat,
}

pub fn color(rgba: Rgba) -> Color32 {
    Color32::from_rgba_unmultiplied(rgba[0], rgba[1], rgba[2], rgba[3])
}

fn rgba_of(color: Color32) -> Rgba {
    color.to_srgba_unmultiplied()
}

fn with_opacity(color: Color32, opacity: f32) -> Color32 {
    let [r, g, b, a] = color.to_srgba_unmultiplied();
    Color32::from_rgba_unmultiplied(r, g, b, (f32::from(a) * opacity).round() as u8)
}

/// The blue the line family defaults to, lifted a little on dark backgrounds.
pub fn line_default() -> Rgba {
    if theme::is_light() {
        [41, 98, 255, 255]
    } else {
        [76, 141, 255, 255]
    }
}

/// The grey the HLC Area's close line defaults to: it sits between the high and low lines.
pub fn neutral_line() -> Rgba {
    if theme::is_light() {
        [120, 126, 138, 255]
    } else {
        [160, 166, 178, 255]
    }
}

/// How strong the HLC Area's fills are against the line they follow.
pub const HLC_FILL_ALPHA: f32 = 0.18;

fn with_alpha(rgba: Rgba, alpha: f32) -> Rgba {
    [
        rgba[0],
        rgba[1],
        rgba[2],
        (f32::from(rgba[3]) * alpha).round() as u8,
    ]
}

pub fn paint(frame: &SeriesFrame) {
    if frame.bars.is_empty() {
        return;
    }
    let painter = frame.painter.with_clip_rect(frame.plot);
    let frame = SeriesFrame {
        painter: &painter,
        ..*frame
    };
    match frame.chart_type {
        ChartType::Candles
        | ChartType::HollowCandles
        | ChartType::VolumeCandles
        | ChartType::HeikinAshi
        | ChartType::Renko
        | ChartType::LineBreak => candles(&frame),
        ChartType::Range => {
            if frame.style.range.look == RangeLook::Candles {
                candles(&frame);
            } else {
                ohlc_bars(&frame, false);
            }
        }
        ChartType::Bars => ohlc_bars(&frame, false),
        ChartType::HighLowBars => ohlc_bars(&frame, true),
        ChartType::Line | ChartType::LineWithMarkers | ChartType::StepLine => line_family(&frame),
        ChartType::Area => area(&frame),
        ChartType::HlcArea => hlc_area(&frame),
        ChartType::Baseline => baseline(&frame),
        ChartType::Columns => columns(&frame),
        ChartType::Kagi => kagi(&frame),
        ChartType::PointAndFigure => point_figure(&frame),
    }
}

impl SeriesFrame<'_> {
    fn confirmed(&self, index: usize) -> bool {
        self.synthetic
            .and_then(|bars| bars.get(index))
            .is_none_or(|bar| bar.confirmed)
    }

    fn previous_close_of(&self, index: usize) -> Option<f64> {
        match index.checked_sub(1) {
            Some(previous) => Some(self.bars[previous].close),
            None => self.previous_close,
        }
    }

    fn recolor_of(&self, index: usize) -> Option<Rgba> {
        self.recolor.and_then(|recolor| recolor(index)).map(rgba_of)
    }

    fn fade(&self, index: usize, color: Color32) -> Color32 {
        if self.confirmed(index) {
            color
        } else {
            with_opacity(color, UNCONFIRMED_ALPHA)
        }
    }

    fn y(&self, price: f64) -> f32 {
        self.scale.y(price)
    }
}

// MARK: Candles

/// Half the width of a plain candle's body for a slot.
///
/// TradingView's proportions at every zoom (`docs/chart-types.md`, Bar widths): a narrow slot is
/// filled but for the gap [`snap_candle`] keeps, and a wide one tends to four fifths. There is no
/// widest candle, or bars would drift apart as the chart zooms in.
pub fn candle_half_width(slot_width: f32) -> f32 {
    let past_narrow = (slot_width - 4.0).max(0.0);
    let share = 1.0 - 0.2 * past_narrow.atan() / std::f32::consts::FRAC_PI_2;
    slot_width * share / 2.0
}

/// The left and right edges (points) of a column centred on `x` that fills its slot: volume and
/// column plots. Laid on the physical pixel grid, neighbours are one point apart at any phase;
/// a slot too narrow for that leaves one pixel.
pub fn snap_column(x: f32, slot_width: f32, pixels_per_point: f32) -> (f32, f32) {
    let ppp = sane_pixels_per_point(pixels_per_point);
    let gap = ppp.round().max(1.0);
    let lead = (gap / 2.0).floor();
    let boundary = |at: f32| (at * ppp).round();
    let left = boundary(x - slot_width / 2.0) + lead;
    let right = (boundary(x + slot_width / 2.0) - (gap - lead)).max(left + 1.0);
    (left / ppp, right / ppp)
}

/// How wide a candle's wick is, in points.
pub const WICK_WIDTH: f32 = 1.0;

fn sane_pixels_per_point(pixels_per_point: f32) -> f32 {
    if pixels_per_point.is_finite() && pixels_per_point > 0.0 {
        pixels_per_point
    } else {
        1.0
    }
}

/// A candle's body and wick edges (points, left and right), laid on the physical pixel grid.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SnappedCandle {
    pub body: (f32, f32),
    pub wick: (f32, f32),
}

fn wick_pixels(wick_width: f32, ppp: f32) -> f32 {
    (wick_width * ppp).round().max(1.0)
}

/// The physical pixel column a wick of `wick_px` pixels centred on `x` starts in.
fn wick_column(x: f32, wick_px: f32, ppp: f32) -> f32 {
    (x * ppp - wick_px / 2.0).round()
}

/// Lays a candle centred on `x` on whole physical pixels, so every body of one width paints the
/// same number of pixels and the wick sits in the middle of it.
///
/// A fractional centre and half width otherwise land each candle on a different sub-pixel phase:
/// the same body shows 5 px wide in one slot and 6 in the next, and the wick is off-centre. The
/// body gets the widest width within `2 * half_width` that has the wick's parity (an odd body for a
/// one-pixel wick), which is what lets the wick be exactly centred.
///
/// A body is never wider than the slot less one pixel, so neighbours keep a gap: centres are
/// snapped separately and can land a pixel closer than the slot is wide. In a slot too narrow for
/// that the body is the wick.
pub fn snap_candle(
    x: f32,
    half_width: f32,
    wick_width: f32,
    slot_width: f32,
    pixels_per_point: f32,
) -> SnappedCandle {
    let ppp = sane_pixels_per_point(pixels_per_point);
    let wick_px = wick_pixels(wick_width, ppp);
    // The nudge keeps a whole number of pixels whole through float error.
    let ideal = (2.0 * half_width * ppp + 1e-3).floor();
    let room = (slot_width * ppp).floor() - 1.0;
    let mut body_px = ideal.min(room).max(wick_px);
    if (body_px - wick_px) % 2.0 != 0.0 {
        body_px -= 1.0;
    }
    let wick_left = wick_column(x, wick_px, ppp);
    let body_left = wick_left - (body_px - wick_px) / 2.0;
    SnappedCandle {
        body: (body_left / ppp, (body_left + body_px) / ppp),
        wick: (wick_left / ppp, (wick_left + wick_px) / ppp),
    }
}

/// A candle's body and wick extents (points, top and bottom), laid on the physical pixel grid.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SnappedRows {
    pub body: (f32, f32),
    pub wick: (f32, f32),
}

/// Lays a candle's rows on whole physical pixels from the screen `y` of its open, close, high and
/// low (in any order).
///
/// Fractional edges render as two half-strength rows, shimmer while the scale changes, and leave a
/// seam where a wick meets a body that were anti-aliased separately. Here the wick ends on the
/// body's own rows, and a body (a doji's) is at least one pixel tall, so it never vanishes.
pub fn snap_rows(
    open_y: f32,
    close_y: f32,
    high_y: f32,
    low_y: f32,
    pixels_per_point: f32,
) -> SnappedRows {
    let ppp = sane_pixels_per_point(pixels_per_point);
    let row = |y: f32| (y * ppp).round();
    let body_top = row(open_y.min(close_y));
    let body_bottom = row(open_y.max(close_y)).max(body_top + 1.0);
    let wick_top = row(high_y.min(low_y)).min(body_top);
    let wick_bottom = row(high_y.max(low_y)).max(body_bottom);
    SnappedRows {
        body: (body_top / ppp, body_bottom / ppp),
        wick: (wick_top / ppp, wick_bottom / ppp),
    }
}

/// The share of its slot a brick, or the widest volume candle, takes: 0.92 at every zoom.
fn brick_half_width(slot_width: f32) -> f32 {
    (slot_width * 0.46).max(1.0)
}

/// A candle ready to paint: where it sits, how wide it is and how it looks.
struct Candle {
    index: usize,
    x: f32,
    half: f32,
    ohlc: [f64; 4],
    look: CandleLook,
}

impl Candle {
    /// Folds the next bar of the same pixel column in: the column reaches every high and low and
    /// looks like the later bar.
    fn absorb(&mut self, next: Candle) {
        let high = self.ohlc[1].max(next.ohlc[1]);
        let low = self.ohlc[2].min(next.ohlc[2]);
        *self = Candle {
            ohlc: [next.ohlc[0], high, low, next.ohlc[3]],
            ..next
        };
    }
}

/// Zoomed out far enough, several bars share a pixel column. Painting each would stack up to four
/// shapes per bar on top of one another, the later hiding the earlier; a column is painted once
/// instead, so the shapes of a frame never outnumber the plot's pixel columns.
fn candles(frame: &SeriesFrame) {
    let style = frame.style;
    let settings = style.candle_settings().copied().unwrap_or_default();
    let brick = matches!(frame.chart_type, ChartType::Renko | ChartType::LineBreak);
    let ppp = sane_pixels_per_point(frame.painter.ctx().pixels_per_point());
    let wick_px = wick_pixels(WICK_WIDTH, ppp);
    let max_volume = frame
        .bars
        .iter()
        .map(|bar| bar.volume)
        .fold(0.0_f64, f64::max);
    let mut open: Option<(f32, Candle)> = None;
    for (index, bar) in frame.bars.iter().enumerate() {
        let previous = frame.previous_close_of(index);
        let recolor = frame.recolor_of(index);
        let look = if frame.chart_type == ChartType::HollowCandles {
            chart::look::hollow_look(&settings, frame.palette, bar, previous, recolor)
        } else {
            chart::look::candle_look(&settings, frame.palette, bar, previous, recolor)
        };
        let half = match frame.chart_type {
            ChartType::VolumeCandles => {
                let full = brick_half_width(frame.slot_width);
                (full * volume_width_fraction(bar.volume, max_volume) as f32).max(1.0)
            }
            _ if brick => brick_half_width(frame.slot_width),
            _ => candle_half_width(frame.slot_width),
        };
        let (high, low) = if brick {
            (bar.high, bar.low)
        } else {
            drawn_extremes(bar.open, bar.high, bar.low, bar.close)
        };
        let x = (frame.x_for)(index);
        let candle = Candle {
            index,
            x,
            half,
            ohlc: [bar.open, high, low, bar.close],
            look,
        };
        let column = wick_column(x, wick_px, ppp);
        match open.as_mut() {
            Some((open_column, merged)) if *open_column == column => merged.absorb(candle),
            _ => {
                if let Some((_, done)) = open.replace((column, candle)) {
                    paint_candle(frame, done.index, done.x, done.half, done.ohlc, &done.look);
                }
            }
        }
    }
    if let Some((_, last)) = open {
        paint_candle(frame, last.index, last.x, last.half, last.ohlc, &last.look);
    }
}

/// One candle: wicks above and below the body (never through a hollow one), then the body.
pub(crate) fn paint_candle(
    frame: &SeriesFrame,
    index: usize,
    x: f32,
    half_width: f32,
    ohlc: [f64; 4],
    look: &CandleLook,
) {
    let painter = frame.painter;
    let ppp = painter.ctx().pixels_per_point();
    let [open, high, low, close] = ohlc;
    let rows = snap_rows(
        frame.y(open),
        frame.y(close),
        frame.y(high),
        frame.y(low),
        ppp,
    );
    let snapped = snap_candle(x, half_width, WICK_WIDTH, frame.slot_width, ppp);
    if let Some(wick) = look.wick {
        let tint = frame.fade(index, color(wick));
        let (left, right) = snapped.wick;
        for (top, bottom) in [(rows.wick.0, rows.body.0), (rows.body.1, rows.wick.1)] {
            if bottom > top {
                painter.rect_filled(
                    Rect::from_min_max(Pos2::new(left, top), Pos2::new(right, bottom)),
                    0.0,
                    tint,
                );
            }
        }
    }
    let body = Rect::from_min_max(
        Pos2::new(snapped.body.0, rows.body.0),
        Pos2::new(snapped.body.1, rows.body.1),
    );
    if let Fill::Solid(fill) = look.body {
        painter.rect_filled(body, 0.5, frame.fade(index, color(fill)));
    }
    if let Some(border) = look.border {
        painter.rect_stroke(
            body,
            0.5,
            Stroke::new(1.0, frame.fade(index, color(border))),
            StrokeKind::Inside,
        );
    }
}

// MARK: OHLC and high-low bars

fn ohlc_bars(frame: &SeriesFrame, high_low_only: bool) {
    let settings = frame.style.bar_settings().copied().unwrap_or_default();
    let painter = frame.painter;
    let tick = (frame.slot_width * 0.34).clamp(1.5, 7.0);
    let width = if settings.thin {
        1.0
    } else {
        (frame.slot_width * 0.12).clamp(1.0, 2.5)
    };
    for (index, bar) in frame.bars.iter().enumerate() {
        let previous = frame.previous_close_of(index);
        let tint = color(chart::look::bar_color(
            &settings,
            frame.palette,
            bar,
            previous,
            frame.recolor_of(index),
        ));
        let tint = frame.fade(index, tint);
        let stroke = Stroke::new(width, tint);
        let x = (frame.x_for)(index);
        let (high, low) = drawn_extremes(bar.open, bar.high, bar.low, bar.close);
        painter.line_segment(
            [Pos2::new(x, frame.y(high)), Pos2::new(x, frame.y(low))],
            stroke,
        );
        if !high_low_only && settings.ticks {
            let open_y = frame.y(bar.open);
            let close_y = frame.y(bar.close);
            painter.line_segment([Pos2::new(x - tick, open_y), Pos2::new(x, open_y)], stroke);
            painter.line_segment(
                [Pos2::new(x, close_y), Pos2::new(x + tick, close_y)],
                stroke,
            );
        }
    }
    if high_low_only && settings.labels {
        extreme_labels(frame);
    }
}

/// The highest high and lowest low in view, named at the bars that made them.
fn extreme_labels(frame: &SeriesFrame) {
    let mut high: Option<(usize, f64)> = None;
    let mut low: Option<(usize, f64)> = None;
    for (index, bar) in frame.bars.iter().enumerate() {
        if high.is_none_or(|(_, value)| bar.high > value) {
            high = Some((index, bar.high));
        }
        if low.is_none_or(|(_, value)| bar.low < value) {
            low = Some((index, bar.low));
        }
    }
    let ink = theme::text_primary();
    for (extreme, above) in [(high, true), (low, false)] {
        let Some((index, value)) = extreme else {
            continue;
        };
        let x = (frame.x_for)(index);
        let y = frame.y(value);
        super::price_text::paint(
            frame.painter,
            Pos2::new(x, if above { y - 4.0 } else { y + 4.0 }),
            if above {
                Align2::CENTER_BOTTOM
            } else {
                Align2::CENTER_TOP
            },
            &frame.price_format.format(value),
            FontId::monospace(9.5),
            ink,
        );
    }
}

// MARK: Lines and areas

/// The y of each bar's value, or `None` where it is not a number.
fn value_points(frame: &SeriesFrame, source: PriceSource) -> Vec<Option<Pos2>> {
    frame
        .bars
        .iter()
        .enumerate()
        .map(|(index, bar)| {
            let value = source.value(bar);
            value
                .is_finite()
                .then(|| Pos2::new((frame.x_for)(index), frame.y(value)))
        })
        .collect()
}

/// Runs of consecutive points: a missing value ends the run so a gap is never bridged.
pub(crate) fn runs(points: &[Option<Pos2>]) -> Vec<Vec<Pos2>> {
    let mut runs: Vec<Vec<Pos2>> = Vec::new();
    let mut open = false;
    for point in points {
        match point {
            Some(point) if open => runs.last_mut().expect("run").push(*point),
            Some(point) => {
                runs.push(vec![*point]);
                open = true;
            }
            None => open = false,
        }
    }
    runs
}

pub(crate) fn stroke_line(painter: &Painter, points: &[Pos2], stroke: Stroke, dash: LineDash) {
    if points.len() < 2 {
        return;
    }
    match dash {
        LineDash::Solid => {
            painter.add(Shape::line(points.to_vec(), stroke));
        }
        LineDash::Dashed => {
            painter.extend(Shape::dashed_line(points, stroke, 7.0, 4.0));
        }
        LineDash::Dotted => {
            painter.extend(Shape::dashed_line(points, stroke, stroke.width, 3.0));
        }
    }
}

fn line_family(frame: &SeriesFrame) {
    let settings: LineSettings = frame.style.line_settings().copied().unwrap_or_default();
    let tint = color(settings.color.unwrap_or_else(line_default));
    let stroke = Stroke::new(f32::from(settings.width), tint);
    let points = value_points(frame, settings.source);
    for run in runs(&points) {
        let path: Vec<Pos2> = if frame.chart_type == ChartType::StepLine {
            let raw: Vec<(f32, f32)> = run.iter().map(|point| (point.x, point.y)).collect();
            step_points(&raw)
                .into_iter()
                .map(|(x, y)| Pos2::new(x, y))
                .collect()
        } else {
            run.clone()
        };
        if path.len() == 1 {
            frame.painter.circle_filled(path[0], stroke.width, tint);
        }
        stroke_line(frame.painter, &path, stroke, settings.dash);
        if frame.chart_type == ChartType::LineWithMarkers {
            marker_dots(frame, &run, settings, tint);
        }
    }
}

/// A dot on each data point, thinned out so markers never overlap when zoomed far out.
fn marker_dots(frame: &SeriesFrame, run: &[Pos2], settings: LineSettings, tint: Color32) {
    let radius = f32::from(settings.marker_size);
    let spacing = radius * 2.0 + 2.0;
    let every = (spacing / frame.slot_width.max(0.1)).ceil().max(1.0) as usize;
    let fill = theme::surface();
    for point in run.iter().step_by(every) {
        frame.painter.circle(
            *point,
            radius,
            fill,
            Stroke::new(f32::from(settings.width).min(2.0), tint),
        );
    }
}

fn vertex(pos: Pos2, color: Color32) -> Vertex {
    Vertex {
        pos,
        uv: WHITE_UV,
        color,
    }
}

/// A vertical gradient strip under (or between) two polylines sharing x positions.
fn strip(
    mesh: &mut Mesh,
    top: &[Pos2],
    bottom: &[Pos2],
    top_color: Color32,
    bottom_color: Color32,
) {
    for (upper, lower) in top.windows(2).zip(bottom.windows(2)) {
        let base = mesh.vertices.len() as u32;
        mesh.vertices.push(vertex(upper[0], top_color));
        mesh.vertices.push(vertex(upper[1], top_color));
        mesh.vertices.push(vertex(lower[1], bottom_color));
        mesh.vertices.push(vertex(lower[0], bottom_color));
        mesh.indices
            .extend([base, base + 1, base + 2, base, base + 2, base + 3]);
    }
}

fn area(frame: &SeriesFrame) {
    let settings = frame.style.area;
    let line = settings.line.unwrap_or_else(line_default);
    let top = color(settings.top.unwrap_or_else(|| with_alpha(line, 0.38)));
    let bottom = color(settings.bottom.unwrap_or_else(|| with_alpha(line, 0.0)));
    let foot = frame.plot.bottom();
    let points = value_points(frame, settings.source);
    let mut mesh = Mesh::default();
    for run in runs(&points) {
        let floor: Vec<Pos2> = run.iter().map(|point| Pos2::new(point.x, foot)).collect();
        strip(&mut mesh, &run, &floor, top, bottom);
    }
    frame.painter.add(Shape::mesh(mesh));
    let stroke = Stroke::new(f32::from(settings.width), color(line));
    for run in runs(&points) {
        stroke_line(frame.painter, &run, stroke, LineDash::Solid);
    }
}

fn hlc_area(frame: &SeriesFrame) {
    let settings = frame.style.hlc_area;
    let high_line = settings.high_line.unwrap_or(frame.palette.up);
    let low_line = settings.low_line.unwrap_or(frame.palette.down);
    let close_line = settings.close_line.unwrap_or_else(neutral_line);
    let high_fill = color(
        settings
            .high_fill
            .unwrap_or_else(|| with_alpha(high_line, HLC_FILL_ALPHA)),
    );
    let low_fill = color(
        settings
            .low_fill
            .unwrap_or_else(|| with_alpha(low_line, HLC_FILL_ALPHA)),
    );
    let highs = value_points(frame, PriceSource::High);
    let lows = value_points(frame, PriceSource::Low);
    let closes = value_points(frame, PriceSource::Close);
    let (high_runs, low_runs, close_runs) = (runs(&highs), runs(&lows), runs(&closes));
    let mut mesh = Mesh::default();
    for ((high_run, low_run), close_run) in high_runs.iter().zip(&low_runs).zip(&close_runs) {
        strip(&mut mesh, high_run, close_run, high_fill, high_fill);
        strip(&mut mesh, close_run, low_run, low_fill, low_fill);
    }
    frame.painter.add(Shape::mesh(mesh));
    let width = f32::from(settings.width);
    // The close line last: it sits between the other two and stays readable where they meet.
    for (runs, tint) in [
        (&high_runs, high_line),
        (&low_runs, low_line),
        (&close_runs, close_line),
    ] {
        let stroke = Stroke::new(width, color(tint));
        for run in runs {
            stroke_line(frame.painter, run, stroke, LineDash::Solid);
        }
    }
}

fn baseline(frame: &SeriesFrame) {
    let settings = frame.style.baseline;
    let plot = frame.plot;
    let level_y = plot.bottom() - settings.level_fraction() as f32 * plot.height();
    let up = settings.top_line.unwrap_or(frame.palette.up);
    let down = settings.bottom_line.unwrap_or(frame.palette.down);
    let top_fill = settings.top_fill.unwrap_or_else(|| with_alpha(up, 0.3));
    let bottom_fill = settings
        .bottom_fill
        .unwrap_or_else(|| with_alpha(down, 0.3));
    let points = value_points(frame, settings.source);
    let width = f32::from(settings.width);
    let mut mesh = Mesh::default();
    let mut strokes: Vec<(Vec<Pos2>, Stroke)> = Vec::new();
    for run in runs(&points) {
        let raw: Vec<(f32, f32)> = run.iter().map(|point| (point.x, point.y)).collect();
        for side in split_at_level(&raw, level_y) {
            let path: Vec<Pos2> = side.points.iter().map(|(x, y)| Pos2::new(*x, *y)).collect();
            let (line, fill) = if side.above {
                (up, top_fill)
            } else {
                (down, bottom_fill)
            };
            let base: Vec<Pos2> = path
                .iter()
                .map(|point| Pos2::new(point.x, level_y))
                .collect();
            strip(
                &mut mesh,
                &path,
                &base,
                color(fill),
                color(with_alpha(fill, 0.1)),
            );
            strokes.push((path, Stroke::new(width, color(line))));
        }
    }
    frame.painter.add(Shape::mesh(mesh));
    frame.painter.extend(Shape::dashed_line(
        &[
            Pos2::new(plot.left(), level_y),
            Pos2::new(plot.right(), level_y),
        ],
        Stroke::new(1.0, theme::text_secondary().gamma_multiply(0.6)),
        4.0,
        4.0,
    ));
    for (path, stroke) in strokes {
        stroke_line(frame.painter, &path, stroke, LineDash::Solid);
    }
}

fn columns(frame: &SeriesFrame) {
    let settings = frame.style.columns;
    let up = color(settings.up.unwrap_or(frame.palette.up));
    let down = color(settings.down.unwrap_or(frame.palette.down));
    let ppp = frame.painter.ctx().pixels_per_point();
    let foot = frame.plot.bottom();
    for (index, bar) in frame.bars.iter().enumerate() {
        let value = settings.source.value(bar);
        if !value.is_finite() {
            continue;
        }
        // The first bar in view is compared with the bar before it when that is its close.
        let previous = match index.checked_sub(1) {
            Some(before) => Some(settings.source.value(&frame.bars[before])),
            None => frame
                .previous_close
                .filter(|_| settings.source == PriceSource::Close),
        };
        let rising = if settings.color_by_previous_close {
            previous.is_none_or(|previous| value >= previous)
        } else {
            bar.close >= bar.open
        };
        let tint = match frame.recolor_of(index) {
            Some(recolor) => color(recolor),
            None if rising => up,
            None => down,
        };
        let (left, right) = snap_column((frame.x_for)(index), frame.slot_width, ppp);
        let top = frame.y(value).min(foot - 1.0);
        frame.painter.rect_filled(
            Rect::from_min_max(Pos2::new(left, top), Pos2::new(right, foot)),
            0.0,
            tint,
        );
    }
}

// MARK: Kagi

fn kagi(frame: &SeriesFrame) {
    let (Some(bars), settings) = (frame.synthetic, frame.style.kagi) else {
        return;
    };
    let up = color(settings.up.unwrap_or(frame.palette.up));
    let down = color(settings.down.unwrap_or(frame.palette.down));
    let thick_width = f32::from(settings.width).max(2.0);
    let thin_width = (thick_width / 2.0).max(1.0);
    // Thick is the "yang" line, drawn in the up colour; thin the "yin", in the down colour.
    let pen = |thick: bool, index: usize| {
        let (tint, width) = if thick {
            (up, thick_width)
        } else {
            (down, thin_width)
        };
        Stroke::new(width, frame.fade(index, tint))
    };
    let painter = frame.painter;
    for (index, bar) in bars.iter().enumerate() {
        let BarShape::Kagi { thick, switch } = bar.shape else {
            continue;
        };
        let x = (frame.x_for)(index);
        let started_thick = if switch.is_some() { !thick } else { thick };
        // The step across from the previous segment, in the thickness the line had there.
        if index > 0 {
            let from = (frame.x_for)(index - 1);
            painter.line_segment(
                [
                    Pos2::new(from, frame.y(bar.open)),
                    Pos2::new(x, frame.y(bar.open)),
                ],
                pen(started_thick, index),
            );
        }
        match switch {
            Some(level) => {
                painter.line_segment(
                    [
                        Pos2::new(x, frame.y(bar.open)),
                        Pos2::new(x, frame.y(level)),
                    ],
                    pen(started_thick, index),
                );
                painter.line_segment(
                    [
                        Pos2::new(x, frame.y(level)),
                        Pos2::new(x, frame.y(bar.close)),
                    ],
                    pen(thick, index),
                );
            }
            None => {
                painter.line_segment(
                    [
                        Pos2::new(x, frame.y(bar.open)),
                        Pos2::new(x, frame.y(bar.close)),
                    ],
                    pen(thick, index),
                );
            }
        }
    }
}

// MARK: Point & Figure

fn point_figure(frame: &SeriesFrame) {
    let (Some(bars), Some(size)) = (frame.synthetic, frame.box_size) else {
        return;
    };
    let settings = frame.style.point_figure;
    let up = color(settings.up.unwrap_or(frame.palette.up));
    let down = color(settings.down.unwrap_or(frame.palette.down));
    let painter = frame.painter;
    let outline = Stroke::new(1.0, theme::grid());
    for (index, bar) in bars.iter().enumerate() {
        let BarShape::Column { rising, boxes } = bar.shape else {
            continue;
        };
        let x = (frame.x_for)(index);
        let tint = frame.fade(index, if rising { up } else { down });
        let box_height = (frame.y(bar.low) - frame.y(bar.low + size)).abs();
        let extent = (frame.slot_width * 0.86).min(box_height * 0.9).max(1.0) / 2.0;
        if box_height < 2.5 {
            // Too dense to tell the symbols apart: the column as a bar.
            painter.line_segment(
                [
                    Pos2::new(x, frame.y(bar.high)),
                    Pos2::new(x, frame.y(bar.low)),
                ],
                Stroke::new((frame.slot_width * 0.5).clamp(1.0, 3.0), tint),
            );
            continue;
        }
        for step in 0..boxes {
            let low = bar.low + f64::from(step) * size;
            let (y_bottom, y_top) = (frame.y(low), frame.y(low + size));
            let centre = Pos2::new(x, (y_bottom + y_top) / 2.0);
            if settings.outlines {
                painter.rect_stroke(
                    Rect::from_center_size(centre, Vec2::new(frame.slot_width, box_height)),
                    0.0,
                    outline,
                    StrokeKind::Inside,
                );
            }
            let stroke = Stroke::new(1.4, tint);
            if rising {
                painter.line_segment(
                    [
                        centre + Vec2::new(-extent, -extent),
                        centre + Vec2::new(extent, extent),
                    ],
                    stroke,
                );
                painter.line_segment(
                    [
                        centre + Vec2::new(-extent, extent),
                        centre + Vec2::new(extent, -extent),
                    ],
                    stroke,
                );
            } else {
                painter.circle_stroke(centre, extent, stroke);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::chart::kind::CandleSettings;
    use crate::model::chart::synthetic::{Construction, build_with};
    use eframe::egui::epaint::RectShape;
    use eframe::egui::{self, pos2};

    fn bar(index: usize, open: f64, high: f64, low: f64, close: f64, volume: f64) -> Kline {
        Kline {
            open_time: index as i64 * 3_600_000,
            close_time: (index as i64 + 1) * 3_600_000 - 1,
            open,
            high,
            low,
            close,
            volume,
        }
    }

    fn market() -> Vec<Kline> {
        (0..40)
            .map(|index| {
                let base = 100.0 + (index as f64 * 0.6).sin() * 8.0;
                bar(
                    index,
                    base,
                    base + 2.0,
                    base - 2.5,
                    base + 1.0,
                    5.0 + index as f64,
                )
            })
            .collect()
    }

    /// What `paint` drew on a headless frame, `slot` points per bar at `ppp` pixels per point.
    fn painted(
        chart_type: ChartType,
        style: &ChartStyle,
        source: &[Kline],
        bars: &[Kline],
        (slot, ppp): (f32, f32),
    ) -> Vec<Shape> {
        let ctx = egui::Context::default();
        ctx.set_pixels_per_point(ppp);
        let plot = Rect::from_min_max(pos2(0.0, 0.0), pos2(400.0, 200.0));
        let scale = PriceScale::linear(80.0, 120.0, 0.0, 200.0);
        let series = Construction::of(&ChartStyle {
            chart_type,
            ..*style
        })
        .map(|construction| build_with(construction, source, i64::MAX / 2));
        let synthetic = series.as_ref().map(|series| series.bars.as_slice());
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            assert_eq!(ui.ctx().pixels_per_point(), ppp);
            let x_for = |index: usize| slot * (index as f32 + 0.5);
            paint(&SeriesFrame {
                painter: ui.painter(),
                plot,
                bars,
                synthetic,
                box_size: series.as_ref().and_then(|series| series.size),
                previous_close: None,
                slot_width: slot,
                x_for: &x_for,
                scale: &scale,
                palette: Palette::new([0, 200, 0], [200, 0, 0]),
                style,
                chart_type,
                recolor: None,
                price_format: PriceFormat::AUTO,
            });
        });
        output.textures_delta.clear();
        output
            .shapes
            .into_iter()
            .map(|clipped| clipped.shape)
            .collect()
    }

    /// How many shapes `paint` put on a headless frame for `chart_type` over `bars`.
    fn shapes(
        chart_type: ChartType,
        style: &ChartStyle,
        source: &[Kline],
        bars: &[Kline],
    ) -> usize {
        let slot = 400.0 / bars.len().max(1) as f32;
        painted(chart_type, style, source, bars, (slot, 1.0)).len()
    }

    /// The rectangles of `shapes` that are filled, and the ones that are only outlined.
    fn rects(shapes: &[Shape]) -> (Vec<RectShape>, Vec<RectShape>) {
        shapes
            .iter()
            .filter_map(|shape| match shape {
                Shape::Rect(rect) => Some(rect.clone()),
                _ => None,
            })
            .partition(|rect| rect.fill != Color32::TRANSPARENT)
    }

    fn styled(chart_type: ChartType) -> ChartStyle {
        ChartStyle {
            chart_type,
            ..ChartStyle::default()
        }
    }

    #[test]
    fn every_type_paints_something_for_ordinary_data() {
        let candles = market();
        for kind in ChartType::ALL {
            let style = styled(kind);
            let constructed = Construction::of(&style)
                .map(|construction| build_with(construction, &candles, i64::MAX / 2).klines());
            let bars = constructed.as_deref().unwrap_or(&candles);
            assert!(
                shapes(kind, &style, &candles, bars) > 0,
                "{kind:?} painted nothing"
            );
        }
    }

    #[test]
    fn awkward_data_never_panics_and_gaps_are_not_bridged() {
        let doji = vec![bar(0, 100.0, 100.0, 100.0, 100.0, 0.0); 3];
        let nothing: Vec<Kline> = Vec::new();
        let with_gap = vec![
            bar(0, 100.0, 101.0, 99.0, 100.5, 1.0),
            bar(1, f64::NAN, f64::NAN, f64::NAN, f64::NAN, 0.0),
            bar(2, 101.0, 102.0, 100.0, 101.5, 1.0),
        ];
        for kind in ChartType::ALL {
            let style = styled(kind);
            for data in [&doji, &nothing, &with_gap] {
                shapes(kind, &style, data, data);
            }
        }
        // A missing value ends the line: two runs of one point each draw dots, not a bridge.
        let style = styled(ChartType::Line);
        let gap_shapes = shapes(ChartType::Line, &style, &with_gap, &with_gap);
        let solid = shapes(ChartType::Line, &style, &market()[..3], &market()[..3]);
        assert!(gap_shapes >= 2 && solid >= 1);
    }

    /// The colours of every vertex the HLC Area's fills put on a headless frame.
    fn hlc_fill_colours(style: &ChartStyle, bars: &[Kline]) -> Vec<Color32> {
        let ctx = egui::Context::default();
        let plot = Rect::from_min_max(pos2(0.0, 0.0), pos2(400.0, 200.0));
        let scale = PriceScale::linear(80.0, 120.0, 0.0, 200.0);
        let slot = plot.width() / bars.len().max(1) as f32;
        let mut colours = Vec::new();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            let x_for = |index: usize| slot * (index as f32 + 0.5);
            paint(&SeriesFrame {
                painter: ui.painter(),
                plot,
                bars,
                synthetic: None,
                box_size: None,
                previous_close: None,
                slot_width: slot,
                x_for: &x_for,
                scale: &scale,
                palette: Palette::new([0, 200, 0], [200, 0, 0]),
                style,
                chart_type: ChartType::HlcArea,
                recolor: None,
                price_format: PriceFormat::AUTO,
            });
        });
        for clipped in &output.shapes {
            if let Shape::Mesh(mesh) = &clipped.shape {
                colours.extend(mesh.vertices.iter().map(|vertex| vertex.color));
            }
        }
        output.textures_delta.clear();
        colours
    }

    #[test]
    fn the_hlc_area_fills_above_and_below_the_close_line_in_their_own_colours() {
        let candles = market();
        let style = styled(ChartType::HlcArea);
        let colours = hlc_fill_colours(&style, &candles);
        let up = color(with_alpha([0, 200, 0, 255], HLC_FILL_ALPHA));
        let down = color(with_alpha([200, 0, 0, 255], HLC_FILL_ALPHA));
        assert!(colours.contains(&up), "no fill above the close line");
        assert!(colours.contains(&down), "no fill below the close line");
        assert!(
            colours
                .iter()
                .all(|colour| *colour == up || *colour == down)
        );

        let mut custom = style;
        custom.hlc_area.high_fill = Some([10, 20, 30, 60]);
        custom.hlc_area.low_fill = Some([40, 50, 60, 70]);
        let colours = hlc_fill_colours(&custom, &candles);
        assert!(colours.contains(&color([10, 20, 30, 60])));
        assert!(colours.contains(&color([40, 50, 60, 70])));
    }

    #[test]
    fn a_hollow_candle_leaves_its_body_unfilled_and_a_filled_one_does_not() {
        let rising = vec![bar(0, 100.0, 104.0, 98.0, 103.0, 1.0)];
        let filled = shapes(
            ChartType::Candles,
            &styled(ChartType::Candles),
            &rising,
            &rising,
        );
        let hollow = shapes(
            ChartType::HollowCandles,
            &styled(ChartType::HollowCandles),
            &rising,
            &rising,
        );
        // Both draw two wick halves and the body's border; only the filled one paints the body.
        assert_eq!(filled, hollow + 1);
        let look = chart::look::hollow_look(
            &CandleSettings::default(),
            Palette::new([0, 200, 0], [200, 0, 0]),
            &rising[0],
            Some(99.0),
            None,
        );
        assert_eq!(look.body, Fill::Hollow);
    }

    #[test]
    fn snapped_candles_keep_one_width_and_a_centred_wick_at_every_phase() {
        for ppp in [1.0_f32, 1.25, 1.5, 2.0, 3.0] {
            for slot in 3..40 {
                let half = candle_half_width(slot as f32 * 0.7);
                let mut width = None;
                for step in 0..200 {
                    let x = 100.0 + step as f32 * 0.137;
                    let c = snap_candle(x, half, 1.0, slot as f32, ppp);
                    let px = |v: f32| v * ppp;
                    for edge in [c.body.0, c.body.1, c.wick.0, c.wick.1] {
                        assert!((px(edge) - px(edge).round()).abs() < 1e-3, "off grid");
                    }
                    let body = (px(c.body.1) - px(c.body.0)).round();
                    let wick = (px(c.wick.1) - px(c.wick.0)).round();
                    assert_eq!(*width.get_or_insert(body), body, "ppp {ppp} slot {slot}");
                    assert!(wick >= 1.0 && body >= wick);
                    assert!(
                        (px(c.body.0) + px(c.body.1) - px(c.wick.0) - px(c.wick.1)).abs() < 1e-3,
                        "wick not centred (ppp {ppp}, slot {slot}, x {x})"
                    );
                    assert!((px(c.wick.0 + c.wick.1) / 2.0 - px(x)).abs() <= 0.5 + 1e-3);
                }
            }
        }
    }

    fn on_grid(value: f32, ppp: f32) -> bool {
        (value * ppp - (value * ppp).round()).abs() < 1e-3
    }

    #[test]
    fn rows_land_on_whole_pixels_and_a_doji_keeps_one() {
        for ppp in [1.0_f32, 1.5, 2.0, 3.0] {
            for step in 0..60 {
                let y = 40.0 + step as f32 * 0.137;
                let rows = snap_rows(y, y + 0.2, y - 7.3, y + 9.1, ppp);
                for edge in [rows.body.0, rows.body.1, rows.wick.0, rows.wick.1] {
                    assert!(on_grid(edge, ppp), "ppp {ppp} y {y}: {edge}");
                }
                assert!((rows.body.1 - rows.body.0) * ppp >= 1.0 - 1e-3);
                assert!(rows.wick.0 <= rows.body.0 && rows.wick.1 >= rows.body.1);
            }
        }
        let doji = snap_rows(50.3, 50.3, 50.3, 50.3, 2.0);
        assert_eq!(doji.body, (50.5, 51.0));
        assert_eq!(doji.wick, doji.body);
    }

    #[test]
    fn a_wick_ends_on_the_bodys_rows_and_never_stops_short_of_it() {
        let short = snap_rows(10.0, 30.0, 12.0, 28.0, 1.0);
        assert_eq!(short.wick, short.body);
        let swapped = snap_rows(30.4, 10.4, 40.2, 2.6, 2.0);
        assert_eq!(swapped, snap_rows(10.4, 30.4, 2.6, 40.2, 2.0));
    }

    #[test]
    fn a_painted_candle_shares_its_pixel_rows_between_wick_and_body() {
        let bars = [
            bar(0, 100.3, 104.7, 98.2, 103.1, 1.0),
            bar(1, 100.0, 101.0, 99.0, 100.0, 1.0),
        ];
        for ppp in [1.0_f32, 2.0] {
            for bar in &bars {
                let one = std::slice::from_ref(bar);
                let shapes = painted(
                    ChartType::Candles,
                    &styled(ChartType::Candles),
                    one,
                    one,
                    (12.0, ppp),
                );
                let (filled, _) = rects(&shapes);
                let [upper, lower, body] = &filled[..] else {
                    panic!("two wicks and a body, got {}", filled.len());
                };
                assert_eq!(upper.rect.max.y, body.rect.min.y, "ppp {ppp}");
                assert_eq!(body.rect.max.y, lower.rect.min.y, "ppp {ppp}");
                for edge in [
                    upper.rect.min.y,
                    body.rect.min.y,
                    body.rect.max.y,
                    lower.rect.max.y,
                ] {
                    assert!(on_grid(edge, ppp), "ppp {ppp}: {edge}");
                }
                assert!(body.rect.height() * ppp >= 1.0 - 1e-3);
                assert_eq!(
                    upper.rect.min.x + upper.rect.max.x,
                    body.rect.min.x + body.rect.max.x
                );
            }
        }
    }

    /// Pixel widths read off TradingView's canvas on a 2x display: slot, candle body, column.
    const TRADINGVIEW_WIDTHS: [(f32, f32, f32); 7] = [
        (7.0, 6.0, 5.0),
        (8.0, 6.0, 6.0),
        (13.0, 10.0, 11.0),
        (25.0, 20.0, 23.0),
        (47.0, 36.0, 45.0),
        (83.0, 66.0, 81.0),
        (120.0, 96.0, 118.0),
    ];

    #[test]
    fn candles_and_columns_are_as_wide_as_tradingviews() {
        for (slot_px, body_px, column_px) in TRADINGVIEW_WIDTHS {
            let slot = slot_px / 2.0;
            let candle = snap_candle(100.0, candle_half_width(slot), WICK_WIDTH, slot, 2.0);
            let column = snap_column(100.0, slot, 2.0);
            assert_eq!(
                (candle.body.1 - candle.body.0) * 2.0,
                body_px,
                "slot {slot_px}"
            );
            assert_eq!((column.1 - column.0) * 2.0, column_px, "slot {slot_px}");
        }
    }

    #[test]
    fn neighbouring_columns_stay_one_point_apart_at_every_phase() {
        for ppp in [1.0_f32, 2.0, 3.0] {
            for step in 0..200 {
                let slot = 4.0 + step as f32 * 0.37;
                let x = 100.0 + step as f32 * 0.173;
                let here = snap_column(x, slot, ppp);
                let next = snap_column(x + slot, slot, ppp);
                for edge in [here.0, here.1, next.0, next.1] {
                    assert!(on_grid(edge, ppp), "ppp {ppp} slot {slot}: {edge}");
                }
                // Two boundaries rounded apart by float error may give or take a pixel.
                let gap = (next.0 - here.1) * ppp;
                assert!(
                    (gap - ppp).abs() <= 1.0 + 1e-3,
                    "ppp {ppp} slot {slot}: {gap}"
                );
                assert!(gap >= 1.0 - 1e-3, "ppp {ppp} slot {slot}: touching");
                assert!(((here.0 + here.1) / 2.0 - x).abs() * ppp <= 1.0 + 1e-3);
            }
        }
    }

    #[test]
    fn neighbouring_bodies_keep_a_gap_at_every_slot_width() {
        for ppp in [1.0_f32, 2.0] {
            for step in 0..=56 {
                let slot = 2.0 + step as f32 * 0.5;
                let half = candle_half_width(slot);
                let slot_px = (slot * ppp).floor();
                for phase in 0..50 {
                    let x = 100.0 + phase as f32 * 0.173;
                    let here = snap_candle(x, half, WICK_WIDTH, slot, ppp);
                    let next = snap_candle(x + slot, half, WICK_WIDTH, slot, ppp);
                    let width = |edges: (f32, f32)| ((edges.1 - edges.0) * ppp).round();
                    let (body, wick) = (width(here.body), width(here.wick));
                    let at = format!("ppp {ppp} slot {slot} x {x}");
                    assert!(body <= (slot_px - 1.0).max(wick), "{at}: body {body}");
                    assert!(body >= wick && (body - wick) % 2.0 == 0.0, "{at}: parity");
                    assert!(
                        (here.body.0 + here.body.1 - here.wick.0 - here.wick.1).abs() * ppp < 1e-3,
                        "{at}: wick off-centre"
                    );
                    if slot_px - 1.0 >= wick {
                        assert!(
                            (next.body.0 - here.body.1) * ppp >= 1.0 - 1e-3,
                            "{at}: touching"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn bars_sharing_a_pixel_column_paint_once_as_their_combined_range() {
        let first = bar(0, 100.0, 101.0, 99.0, 100.5, 1.0);
        let highest = bar(1, 100.5, 112.0, 100.0, 101.0, 1.0);
        let lowest = bar(2, 101.0, 102.0, 88.0, 99.0, 1.0);
        let last = bar(3, 99.0, 100.0, 97.0, 97.5, 1.0);
        let bars = [first, highest, lowest, last];
        let scale = PriceScale::linear(80.0, 120.0, 0.0, 200.0);
        // Four bars to a pixel column.
        let shapes = painted(
            ChartType::Candles,
            &styled(ChartType::Candles),
            &bars,
            &bars,
            (0.25, 1.0),
        );
        let (filled, _) = rects(&shapes);
        let [upper, lower, body] = &filled[..] else {
            panic!("one candle, got {} filled shapes", filled.len());
        };
        assert_eq!(upper.rect.min.y, scale.y(112.0).round());
        assert_eq!(lower.rect.max.y, scale.y(88.0).round());
        assert_eq!(
            body.fill,
            Color32::from_rgb(200, 0, 0),
            "the later bar falls"
        );
        assert_eq!(shapes.len(), 4);
    }

    #[test]
    fn a_dense_chart_paints_no_more_than_a_candle_per_pixel_column() {
        let bars: Vec<Kline> = (0..1600)
            .map(|index| {
                let base = 100.0 + (index % 9) as f64;
                bar(index, base, base + 3.0, base - 3.0, base + 1.0, 1.0)
            })
            .collect();
        for ppp in [1.0_f32, 2.0] {
            let shapes = painted(
                ChartType::Candles,
                &styled(ChartType::Candles),
                &bars,
                &bars,
                (0.25, ppp),
            );
            let columns = (400.0 * ppp) as usize + 1;
            assert!(
                shapes.len() <= 4 * columns,
                "ppp {ppp}: {} shapes",
                shapes.len()
            );
        }
    }

    #[test]
    fn colours_convert_with_their_alpha() {
        assert_eq!(
            color([1, 2, 3, 128]),
            Color32::from_rgba_unmultiplied(1, 2, 3, 128)
        );
        assert_eq!(rgba_of(color([9, 8, 7, 255])), [9, 8, 7, 255]);
        // egui keeps colours premultiplied, so a translucent one may come back a step off.
        let back = rgba_of(color([90, 80, 70, 200]));
        assert!(
            back.iter()
                .zip([90_u8, 80, 70, 200])
                .all(|(a, b)| a.abs_diff(b) <= 1)
        );
        let faded = with_opacity(Color32::from_rgb(10, 20, 30), 0.5);
        assert_eq!(faded.to_srgba_unmultiplied()[3], 128);
    }

    #[test]
    fn a_dense_point_and_figure_falls_back_to_a_bar_instead_of_unreadable_symbols() {
        let candles = market();
        let mut style = styled(ChartType::PointAndFigure);
        style.point_figure.size.method = crate::model::chart::kind::SizeMethod::Traditional;
        style.point_figure.size.amount = 0.01;
        let constructed = build_with(Construction::of(&style).unwrap(), &candles, i64::MAX / 2);
        let bars = constructed.klines();
        // Columns this tall in boxes this small draw one stroke each, not thousands of Xs.
        let count = shapes(ChartType::PointAndFigure, &style, &candles, &bars);
        assert!(count <= constructed.bars.len() * 2, "{count} shapes");
    }
}
