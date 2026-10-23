//! Paints the main price series of a chart in whichever way its chart type asks.
//!
//! What a bar looks like (hollow or filled, which colour, how wide a volume candle is, where a
//! step turns, where a baseline line crosses) is decided in `model::chart_look` and tested there;
//! this module turns those decisions into painter calls. The chart card owns the coordinate
//! system and hands it in as `x_for` and the price scale, so every type shares one set of axes:
//! one slot per bar, left to right. Price-based types have no time axis of their own: their
//! slots are bricks, columns or segments, in the order they formed.
//!
//! Everything here is the *main series*. A script's `plotcandle()` and `plotbar()` are drawn by
//! `pine_chart_render` as their own layer and are unaffected by the chart type.

use super::chart_card::PriceScale;
use super::theme;
use crate::model::chart_look::{
    self, CandleLook, Fill, Palette, drawn_extremes, split_at_level, step_points,
    volume_width_fraction,
};
use crate::model::chart_type::{
    ChartStyle, ChartType, LineDash, LineSettings, PriceSource, RangeLook, Rgba,
};
use crate::model::synthetic::{BarShape, SyntheticBar};
use crate::model::{Kline, PriceFormat};
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
pub fn candle_half_width(slot_width: f32) -> f32 {
    (slot_width * 0.34).clamp(1.0, 8.0)
}

/// Bricks and lines fill more of their slot than a candle does.
fn brick_half_width(slot_width: f32) -> f32 {
    (slot_width * 0.46).clamp(1.0, 24.0)
}

fn candles(frame: &SeriesFrame) {
    let style = frame.style;
    let settings = style.candle_settings().copied().unwrap_or_default();
    let brick = matches!(frame.chart_type, ChartType::Renko | ChartType::LineBreak);
    let max_volume = frame
        .bars
        .iter()
        .map(|bar| bar.volume)
        .fold(0.0_f64, f64::max);
    for (index, bar) in frame.bars.iter().enumerate() {
        let previous = frame.previous_close_of(index);
        let recolor = frame.recolor_of(index);
        let look = if frame.chart_type == ChartType::HollowCandles {
            chart_look::hollow_look(&settings, frame.palette, bar, previous, recolor)
        } else {
            chart_look::candle_look(&settings, frame.palette, bar, previous, recolor)
        };
        let half = match frame.chart_type {
            ChartType::VolumeCandles => {
                let full = (frame.slot_width * 0.46).clamp(1.0, 24.0);
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
        paint_candle(
            frame,
            index,
            (frame.x_for)(index),
            half,
            [bar.open, high, low, bar.close],
            &look,
        );
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
    let [open, high, low, close] = ohlc;
    let (open_y, close_y) = (frame.y(open), frame.y(close));
    let (body_top, body_bottom) = (open_y.min(close_y), open_y.max(close_y));
    let body_bottom = body_bottom.max(body_top + 1.0);
    if let Some(wick) = look.wick {
        let stroke = Stroke::new(1.0, frame.fade(index, color(wick)));
        painter.line_segment(
            [Pos2::new(x, frame.y(high)), Pos2::new(x, body_top)],
            stroke,
        );
        painter.line_segment(
            [Pos2::new(x, body_bottom), Pos2::new(x, frame.y(low))],
            stroke,
        );
    }
    let body = Rect::from_min_max(
        Pos2::new(x - half_width, body_top),
        Pos2::new(x + half_width, body_bottom),
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
        let tint = color(chart_look::bar_color(
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
fn runs(points: &[Option<Pos2>]) -> Vec<Vec<Pos2>> {
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

fn stroke_line(painter: &Painter, points: &[Pos2], stroke: Stroke, dash: LineDash) {
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
    let close_line = settings.close_line.unwrap_or_else(line_default);
    let fill = color(
        settings
            .fill
            .unwrap_or_else(|| with_alpha(close_line, 0.16)),
    );
    let highs = value_points(frame, PriceSource::High);
    let lows = value_points(frame, PriceSource::Low);
    let closes = value_points(frame, PriceSource::Close);
    let mut mesh = Mesh::default();
    for (high_run, low_run) in runs(&highs).iter().zip(runs(&lows).iter()) {
        strip(&mut mesh, high_run, low_run, fill, fill);
    }
    frame.painter.add(Shape::mesh(mesh));
    let width = f32::from(settings.width);
    for (points, tint, stroke_width) in [
        (&highs, high_line, width),
        (&lows, low_line, width),
        (&closes, close_line, width),
    ] {
        let stroke = Stroke::new(stroke_width, color(tint));
        for run in runs(points) {
            stroke_line(frame.painter, &run, stroke, LineDash::Solid);
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
    let half = (frame.slot_width * 0.4).clamp(0.75, 20.0);
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
        let x = (frame.x_for)(index);
        let top = frame.y(value).min(foot - 1.0);
        frame.painter.rect_filled(
            Rect::from_min_max(Pos2::new(x - half, top), Pos2::new(x + half, foot)),
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
    use crate::model::chart_type::CandleSettings;
    use crate::model::synthetic::{Construction, build_with};
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

    /// How many shapes `paint` put on a headless frame for `chart_type` over `bars`.
    fn shapes(
        chart_type: ChartType,
        style: &ChartStyle,
        source: &[Kline],
        bars: &[Kline],
    ) -> usize {
        let ctx = egui::Context::default();
        let plot = Rect::from_min_max(pos2(0.0, 0.0), pos2(400.0, 200.0));
        let scale = PriceScale::linear(80.0, 120.0, 0.0, 200.0);
        let slot = plot.width() / bars.len().max(1) as f32;
        let series = Construction::of(&ChartStyle {
            chart_type,
            ..*style
        })
        .map(|construction| build_with(construction, source, i64::MAX / 2));
        let synthetic = series.as_ref().map(|series| series.bars.as_slice());
        let mut count = 0;
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
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
        count += output.shapes.len();
        output.textures_delta.clear();
        count
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
        let look = chart_look::hollow_look(
            &CandleSettings::default(),
            Palette::new([0, 200, 0], [200, 0, 0]),
            &rising[0],
            Some(99.0),
            None,
        );
        assert_eq!(look.body, Fill::Hollow);
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
        style.point_figure.size.method = crate::model::chart_type::SizeMethod::Traditional;
        style.point_figure.size.amount = 0.01;
        let constructed = build_with(Construction::of(&style).unwrap(), &candles, i64::MAX / 2);
        let bars = constructed.klines();
        // Columns this tall in boxes this small draw one stroke each, not thousands of Xs.
        let count = shapes(ChartType::PointAndFigure, &style, &candles, &bars);
        assert!(count <= constructed.bars.len() * 2, "{count} shapes");
    }
}
