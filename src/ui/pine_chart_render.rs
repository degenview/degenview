//! Paints a Pine script's visual output onto a chart: over the candles for `overlay=true`
//! scripts, or into a script pane for the rest. A port of `PineChartLayer` and its
//! `+Plots / +Drawings / +Markers / +Tables` extensions; the geometry it needs lives in
//! `pine_geometry` and is tested there.
//!
//! The caller owns the chart's coordinate system and hands it in as two closures: where a visible
//! candle slot sits horizontally (slots may be negative or past the last candle, for drawings
//! anchored outside the view) and where a price sits vertically. Per-bar series are right-aligned
//! with the candles; drawing objects carry absolute `bar_index` coordinates.
//!
//! Filled shapes are triangle meshes (egui's polygon fill assumes convex shapes): quad strips for
//! fills and areas, and a fan from the centroid for closed polylines, which is exact for convex and
//! star-shaped outlines.

// Transient: the chart card calls this once the Pine session produces output.
#![allow(dead_code)]

use crate::model::Kline;
use crate::model::chart_look::drawn_extremes;
use crate::pine::outputs::{
    DISPLAY_PANE, LabelOutput, MarkerKind, MarkerLocation, MarkerShape, PlotOutput, PlotStyle,
    TableOutput, YLocation,
};
use crate::pine::program::VisualOutput;
use crate::pine::strategy::StrategyReport;
use crate::ui::pine_geometry::{
    self as geometry, LabelGlyph, LabelShape, arrowhead, arrowheads, box_text_placement,
    candle_index_for_bar, dash_pattern, extended_line, faded, font_size, is_visible, label_layout,
    marker_radius, polyline_points, rgba, series_index,
};
use eframe::egui::{
    self, Align2, Color32, FontId, Painter, Pos2, Rect, Shape, Stroke, Vec2,
    epaint::{Mesh, Vertex, WHITE_UV},
};

/// Profit and loss colours, shared with the strategy report.
pub const WIN_COLOR: u32 = 0x26a6_9aff;
pub const LOSS_COLOR: u32 = 0xef53_50ff;

const AREA_OPACITY: f32 = 0.3;
const CROSS_HALF_SIZE: f32 = 3.0;
const MARKER_OFFSET: f32 = 8.0;
const LABEL_POINTER: f32 = 5.0;
const LABEL_PADDING: Vec2 = Vec2::new(10.0, 6.0);

/// How `plotcandle()` bodies and wicks are sized, to match the real candles they overlay.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CandleStyle {
    pub body_width: f32,
    pub wick_width: f32,
    pub min_body_height: f32,
}

impl Default for CandleStyle {
    fn default() -> Self {
        Self {
            body_width: 6.0,
            wick_width: 1.0,
            min_body_height: 1.0,
        }
    }
}

pub struct PineLayer<'a> {
    pub pine: &'a VisualOutput,
    /// The candles in view; per-bar series are right-aligned with these, ending `tail` bars before
    /// the last one when the chart is scrolled back.
    pub candles: &'a [Kline],
    /// How many bars lie after the last one in view.
    pub tail: usize,
    /// In its own pane there are no candles to anchor `abovebar` / `belowbar` markers to.
    pub in_pane: bool,
    /// The plot area that drawings are clipped to and tables are pinned inside.
    pub plot: Rect,
    /// The width of one candle slot.
    pub slot_width: f32,
    /// The x of the centre of visible candle slot `i` (negative or past the end allowed).
    pub x_for_index: &'a dyn Fn(i64) -> f32,
    /// The y of a price (or pane value).
    pub y_for: &'a dyn Fn(f64) -> f32,
    pub candle_style: CandleStyle,
}

/// `barcolor()` for visible candle `index` of `count` (the last of them `tail` bars before the
/// newest), if the script coloured it. A script may call `barcolor()` more than once; the last
/// call that gave the bar a colour (not `na`) wins.
pub fn bar_color_at(
    pine: &VisualOutput,
    count: usize,
    tail: usize,
    index: usize,
) -> Option<Color32> {
    pine.bar_colors
        .iter()
        .rev()
        .find_map(|series| {
            series_index(series.colors.len(), count + tail, index)
                .and_then(|slot| series.colors[slot])
        })
        .map(rgba)
}

// MARK: Mesh helpers

fn vertex(pos: Pos2, color: Color32) -> Vertex {
    Vertex {
        pos,
        uv: WHITE_UV,
        color,
    }
}

/// A filled quad `a, b, c, d` (in order around the outline).
fn quad(mesh: &mut Mesh, corners: [Pos2; 4], colors: [Color32; 4]) {
    let base = mesh.vertices.len() as u32;
    for (corner, color) in corners.into_iter().zip(colors) {
        mesh.vertices.push(vertex(corner, color));
    }
    mesh.indices
        .extend([base, base + 1, base + 2, base, base + 2, base + 3]);
}

/// A polygon filled as a fan from its centroid.
fn fan(mesh: &mut Mesh, points: &[Pos2], color: Color32) {
    if points.len() < 3 {
        return;
    }
    let count = points.len() as f32;
    let centre = Pos2::new(
        points.iter().map(|p| p.x).sum::<f32>() / count,
        points.iter().map(|p| p.y).sum::<f32>() / count,
    );
    let base = mesh.vertices.len() as u32;
    mesh.vertices.push(vertex(centre, color));
    for point in points {
        mesh.vertices.push(vertex(*point, color));
    }
    for index in 0..points.len() as u32 {
        let next = (index + 1) % points.len() as u32;
        mesh.indices
            .extend([base, base + 1 + index, base + 1 + next]);
    }
}

fn paint_mesh(painter: &Painter, mesh: Mesh) {
    if !mesh.is_empty() {
        painter.add(Shape::mesh(mesh));
    }
}

/// A stroke along `points` in a dash pattern (solid when the pattern is empty).
fn stroke_path(painter: &Painter, points: &[Pos2], stroke: Stroke, dashes: &[f32]) {
    if points.len() < 2 {
        return;
    }
    if let [dash, gap] = dashes {
        painter.extend(Shape::dashed_line(points, stroke, *dash, *gap));
    } else {
        painter.add(Shape::line(points.to_vec(), stroke));
    }
}

fn text_at(painter: &Painter, text: &str, size: f32, color: Color32, at: Pos2, anchor: Align2) {
    let galley = painter.layout_no_wrap(text.to_owned(), FontId::proportional(size), color);
    let rect = anchor.anchor_size(at, galley.size());
    painter.galley(rect.min, galley, color);
}

fn measure(painter: &Painter, text: &str, size: f32) -> Vec2 {
    painter
        .layout_no_wrap(text.to_owned(), FontId::proportional(size), Color32::WHITE)
        .size()
}

impl PineLayer<'_> {
    // MARK: Coordinates

    fn count(&self) -> usize {
        self.candles.len()
    }

    /// The value of a per-bar series at visible candle `i`.
    fn visible<'s, T>(&self, series: &'s [T], i: usize) -> Option<&'s T> {
        series_index(series.len(), self.count() + self.tail, i).map(|k| &series[k])
    }

    fn visible_opt<T: Copy>(&self, series: &[Option<T>], i: usize) -> Option<T> {
        self.visible(series, i).copied().flatten()
    }

    fn x_bar(&self, bar: i64) -> f32 {
        (self.x_for_index)(candle_index_for_bar(
            bar,
            self.pine.bar_count.saturating_sub(self.tail),
            self.count(),
        ))
    }

    fn x_at(&self, i: usize) -> f32 {
        (self.x_for_index)(i as i64)
    }

    fn y(&self, price: f64) -> f32 {
        (self.y_for)(price)
    }

    fn point_bar(&self, bar: i64, price: f64) -> Pos2 {
        Pos2::new(self.x_bar(bar), self.y(price))
    }

    /// `barcolor()` for the candle at `index`, if the script set one.
    pub fn bar_color(&self, index: usize) -> Option<Color32> {
        bar_color_at(self.pine, self.count(), self.tail, index)
    }

    /// Everything the script draws on the chart, behind the candles: `behind_chart`, or the user
    /// sending it to the back. Trade markers stay in front, see [`Self::draw_trades_in_front`].
    pub fn draw_behind(&self, painter: &Painter) {
        self.draw_visuals(&painter.with_clip_rect(self.plot), true);
    }

    /// The strategy's trades, which sit in front of the candles even when its plots do not.
    pub fn draw_trades_in_front(&self, painter: &Painter) {
        self.draw_strategy_trades(&painter.with_clip_rect(self.plot));
    }

    // MARK: Order

    /// Above the candles, in the chart's deterministic draw order.
    pub fn draw_foreground(&self, painter: &Painter) {
        let clipped = painter.with_clip_rect(self.plot);
        self.draw_visuals(&clipped, true);
        self.draw_strategy_trades(&clipped);
    }

    /// Everything but the strategy's trade markers, which stay in front of the candles even when
    /// the script's own plots sit behind them (`behind_chart`). `plotcandle()` is left out when
    /// `including_candles` is false: it recolours the real candles, so it has to be drawn after them.
    pub fn draw_visuals(&self, painter: &Painter, including_candles: bool) {
        self.draw_fills(painter);
        self.draw_boxes(painter);
        if including_candles {
            self.draw_candles(painter);
        }
        self.draw_plots(painter);
        self.draw_linefills(painter);
        self.draw_polylines(painter);
        self.draw_lines(painter);
        self.draw_horizontal_lines(painter);
        self.draw_markers(painter);
        self.draw_arrows(painter);
        self.draw_labels(painter);
    }

    // MARK: Background

    /// Behind the candles: one candle-width band for each non-empty `bgcolor()` result.
    pub fn draw_background(&self, painter: &Painter) {
        let painter = painter.with_clip_rect(self.plot);
        let mut mesh = Mesh::default();
        for output in &self.pine.backgrounds {
            for i in 0..self.count() {
                let Some(color) = self.visible_opt(&output.colors, i) else {
                    continue;
                };
                let x = self.x_at(i);
                let band = Rect::from_min_max(
                    Pos2::new(x - self.slot_width / 2.0, self.plot.min.y),
                    Pos2::new(x + self.slot_width / 2.0, self.plot.max.y),
                );
                mesh.add_colored_rect(band, rgba(color));
            }
        }
        paint_mesh(&painter, mesh);
    }

    // MARK: plot

    /// Contiguous `plot()` segments. A missing value ends the segment so Pine gaps are not
    /// bridged; the segment ending at a bar takes that bar's colour.
    pub fn draw_plots(&self, painter: &Painter) {
        for output in self
            .pine
            .plots
            .iter()
            .filter(|o| o.display & DISPLAY_PANE != 0)
        {
            let point = |i: usize| {
                self.visible_opt(&output.values, i)
                    .map(|value| Pos2::new(self.x_at(i), self.y(value)))
            };
            let color = |i: usize| self.visible_opt(&output.colors, i).unwrap_or(output.color);
            if matches!(
                output.style,
                PlotStyle::Line | PlotStyle::Stepline | PlotStyle::SteplineDiamond
            ) {
                self.draw_line_plot(painter, output, &point, &color);
            } else {
                self.draw_discrete_plot(painter, output, &point, &color);
            }
        }
    }

    fn draw_line_plot(
        &self,
        painter: &Painter,
        output: &PlotOutput,
        point: &dyn Fn(usize) -> Option<Pos2>,
        color: &dyn Fn(usize) -> u32,
    ) {
        let count = self.count();
        let mut i = 0;
        while i < count {
            let Some(start) = point(i) else {
                i += 1;
                continue;
            };
            let run_color = if i + 1 < count {
                color(i + 1)
            } else {
                color(i)
            };
            let mut path = vec![start];
            // Where the value is new: a run's first bar when no earlier run owns it (a run that
            // follows another starts on that one's last bar), and each bar whose value differs.
            let mut changes = if i == 0 || point(i - 1).is_none() {
                vec![start]
            } else {
                Vec::new()
            };
            let mut j = i;
            let mut previous = start;
            let stepped = matches!(
                output.style,
                PlotStyle::Stepline | PlotStyle::SteplineDiamond
            );
            while j + 1 < count
                && let Some(next) = point(j + 1)
                && color(j + 1) == run_color
            {
                if stepped {
                    path.push(Pos2::new(next.x, previous.y));
                }
                if next.y != previous.y {
                    changes.push(next);
                }
                path.push(next);
                previous = next;
                j += 1;
            }
            if is_visible(run_color) {
                stroke_path(
                    painter,
                    &path,
                    Stroke::new(output.line_width as f32, rgba(run_color)),
                    &[],
                );
                if output.style == PlotStyle::SteplineDiamond {
                    let radius = (output.line_width as f32 * 2.0).max(DIAMOND_RADIUS);
                    for centre in changes {
                        draw_diamond(painter, centre, radius, rgba(run_color));
                    }
                }
            }
            i = if j == i { i + 1 } else { j };
        }
    }

    /// Histogram, columns, area, circles and cross plot styles.
    fn draw_discrete_plot(
        &self,
        painter: &Painter,
        output: &PlotOutput,
        point: &dyn Fn(usize) -> Option<Pos2>,
        color: &dyn Fn(usize) -> u32,
    ) {
        let base = self.y(output.hist_base);
        if output.style == PlotStyle::Area {
            self.draw_area(painter, point, color, base);
            return;
        }
        let width = if output.style == PlotStyle::Columns {
            (self.slot_width * 0.8).max(1.0)
        } else {
            (self.slot_width * 0.35).max(1.0)
        };
        let size = output.line_width.max(1) as f32;
        let mut mesh = Mesh::default();
        for i in 0..self.count() {
            let Some(p) = point(i) else { continue };
            if !is_visible(color(i)) {
                continue;
            }
            let tint = rgba(color(i));
            match output.style {
                PlotStyle::Histogram | PlotStyle::Columns => {
                    let rect = Rect::from_min_size(
                        Pos2::new(p.x - width / 2.0, p.y.min(base)),
                        Vec2::new(width, (base - p.y).abs().max(1.0)),
                    );
                    mesh.add_colored_rect(rect, tint);
                }
                PlotStyle::Circles => {
                    painter.circle_filled(p, size + 1.0, tint);
                }
                _ => {
                    let h = CROSS_HALF_SIZE;
                    let stroke = Stroke::new(size, tint);
                    painter
                        .line_segment([Pos2::new(p.x - h, p.y), Pos2::new(p.x + h, p.y)], stroke);
                    painter
                        .line_segment([Pos2::new(p.x, p.y - h), Pos2::new(p.x, p.y + h)], stroke);
                }
            }
        }
        paint_mesh(painter, mesh);
    }

    /// One filled strip per contiguous run of values, tinted by the run's last bar.
    fn draw_area(
        &self,
        painter: &Painter,
        point: &dyn Fn(usize) -> Option<Pos2>,
        color: &dyn Fn(usize) -> u32,
        base: f32,
    ) {
        let count = self.count();
        let mut i = 0;
        while i < count {
            let Some(first) = point(i) else {
                i += 1;
                continue;
            };
            let mut run = vec![first];
            let mut j = i;
            while j + 1 < count
                && let Some(next) = point(j + 1)
            {
                run.push(next);
                j += 1;
            }
            let tint = faded(color(j), AREA_OPACITY);
            let mut mesh = Mesh::default();
            for pair in run.windows(2) {
                quad(
                    &mut mesh,
                    [
                        pair[0],
                        pair[1],
                        Pos2::new(pair[1].x, base),
                        Pos2::new(pair[0].x, base),
                    ],
                    [tint; 4],
                );
            }
            paint_mesh(painter, mesh);
            i = j + 1;
        }
    }

    // MARK: plotcandle and plotbar

    /// `plotcandle()` and `plotbar()`: the script's own OHLC, drawn over (or behind) the chart's
    /// candles at the same width. They are a layer of their own and look the same whatever type
    /// the chart is drawn as: a script that asked for candles gets candles on a Bars chart.
    ///
    /// The wicks reach at least the open and close and the high and low are ordered, whatever the
    /// script's values say; the values themselves are not changed.
    pub fn draw_candles(&self, painter: &Painter) {
        let style = self.candle_style;
        for output in self
            .pine
            .candles
            .iter()
            .filter(|o| o.display & DISPLAY_PANE != 0)
        {
            for i in 0..self.count() {
                let Some(bar) = self.visible(&output.bars, i).and_then(Option::as_ref) else {
                    continue;
                };
                let x = self.x_at(i);
                let (high, low) = drawn_extremes(bar.open, bar.high, bar.low, bar.close);
                if output.bar_style {
                    self.draw_ohlc_bar(painter, x, bar, (high, low));
                    continue;
                }
                if let Some(wick) = bar.wick_color.filter(|c| is_visible(*c)) {
                    let (top, bottom) = (self.y(high), self.y(low));
                    let rect = Rect::from_min_size(
                        Pos2::new(x - style.wick_width / 2.0, top),
                        Vec2::new(style.wick_width, (bottom - top).max(0.5)),
                    );
                    painter.rect_filled(rect, 0.0, rgba(wick));
                }
                let (top, bottom) = (
                    self.y(bar.open.max(bar.close)),
                    self.y(bar.open.min(bar.close)),
                );
                let body = Rect::from_min_size(
                    Pos2::new(x - style.body_width / 2.0, top),
                    Vec2::new(style.body_width, (bottom - top).max(style.min_body_height)),
                );
                if let Some(fill) = bar.color.filter(|c| is_visible(*c)) {
                    painter.rect_filled(body, 0.0, rgba(fill));
                }
                if let Some(border) = bar.border_color.filter(|c| is_visible(*c)) {
                    painter.rect_stroke(
                        body,
                        0.0,
                        Stroke::new(1.0, rgba(border)),
                        egui::StrokeKind::Inside,
                    );
                }
            }
        }
    }

    /// A conventional bar: a stem from the high to the low, the open as a tick to its left and
    /// the close as a tick to its right. Unlike a candle it has no wick or border of its own.
    fn draw_ohlc_bar(
        &self,
        painter: &Painter,
        x: f32,
        bar: &crate::pine::outputs::CandleBar,
        extremes: (f64, f64),
    ) {
        let Some(color) = bar.color.filter(|c| is_visible(*c)) else {
            return;
        };
        let stroke = Stroke::new((self.slot_width * 0.12).clamp(1.0, 2.5), rgba(color));
        let tick = (self.slot_width * 0.34).clamp(1.5, 7.0);
        let (open, close) = (self.y(bar.open), self.y(bar.close));
        painter.line_segment(
            [
                Pos2::new(x, self.y(extremes.0)),
                Pos2::new(x, self.y(extremes.1)),
            ],
            stroke,
        );
        painter.line_segment([Pos2::new(x - tick, open), Pos2::new(x, open)], stroke);
        painter.line_segment([Pos2::new(x, close), Pos2::new(x + tick, close)], stroke);
    }

    // MARK: fill and hline

    /// `fill()` between two plots: one strip per run of bars where both plots have values and
    /// the fill colour stays the same (or per-bar gradient quads).
    pub fn draw_fills(&self, painter: &Painter) {
        for fill in &self.pine.fills {
            let (Some(a), Some(b)) = (
                self.pine.plots.iter().find(|p| p.id == fill.plot_a),
                self.pine.plots.iter().find(|p| p.id == fill.plot_b),
            ) else {
                continue;
            };
            let pair = |i: usize| -> Option<(f32, f32, f32)> {
                let top = self.visible_opt(&a.values, i)?;
                let bottom = self.visible_opt(&b.values, i)?;
                Some((self.x_at(i), self.y(top), self.y(bottom)))
            };
            if fill.gradients.is_empty() {
                self.draw_flat_fill(painter, &fill.colors, &pair);
            } else {
                self.draw_gradient_fill(painter, &fill.gradients, &pair);
            }
        }
    }

    fn draw_gradient_fill(
        &self,
        painter: &Painter,
        gradients: &[Option<crate::pine::outputs::FillGradient>],
        pair: &dyn Fn(usize) -> Option<(f32, f32, f32)>,
    ) {
        let mut mesh = Mesh::default();
        for i in 0..self.count().saturating_sub(1) {
            let (Some(p), Some(q)) = (pair(i), pair(i + 1)) else {
                continue;
            };
            let Some(gradient) = self.visible_opt(gradients, i + 1) else {
                continue;
            };
            let (top_y, bottom_y) = (self.y(gradient.top), self.y(gradient.bottom));
            let color_at = |y: f32| {
                let span = bottom_y - top_y;
                let t = if span.abs() < f32::EPSILON {
                    0.0
                } else {
                    ((y - top_y) / span).clamp(0.0, 1.0)
                };
                let (a, b) = (rgba(gradient.top_color), rgba(gradient.bottom_color));
                let mix =
                    |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * t).round() as u8;
                Color32::from_rgba_premultiplied(
                    mix(a.r(), b.r()),
                    mix(a.g(), b.g()),
                    mix(a.b(), b.b()),
                    mix(a.a(), b.a()),
                )
            };
            quad(
                &mut mesh,
                [
                    Pos2::new(p.0, p.1),
                    Pos2::new(q.0, q.1),
                    Pos2::new(q.0, q.2),
                    Pos2::new(p.0, p.2),
                ],
                [color_at(p.1), color_at(q.1), color_at(q.2), color_at(p.2)],
            );
        }
        paint_mesh(painter, mesh);
    }

    fn draw_flat_fill(
        &self,
        painter: &Painter,
        colors: &[Option<u32>],
        pair: &dyn Fn(usize) -> Option<(f32, f32, f32)>,
    ) {
        let count = self.count();
        let color = |i: usize| self.visible_opt(colors, i);
        let mut i = 0;
        while i + 1 < count {
            let Some(run_color) = color(i + 1) else {
                i += 1;
                continue;
            };
            if pair(i).is_none() || pair(i + 1).is_none() {
                i += 1;
                continue;
            }
            let mut j = i + 1;
            while j + 1 < count && pair(j + 1).is_some() && color(j + 1) == Some(run_color) {
                j += 1;
            }
            let points: Vec<(f32, f32, f32)> = (i..=j).filter_map(pair).collect();
            let tint = rgba(run_color);
            let mut mesh = Mesh::default();
            for window in points.windows(2) {
                let (p, q) = (window[0], window[1]);
                quad(
                    &mut mesh,
                    [
                        Pos2::new(p.0, p.1),
                        Pos2::new(q.0, q.1),
                        Pos2::new(q.0, q.2),
                        Pos2::new(p.0, p.2),
                    ],
                    [tint; 4],
                );
            }
            paint_mesh(painter, mesh);
            i = j;
        }
    }

    pub fn draw_horizontal_lines(&self, painter: &Painter) {
        for line in &self.pine.hlines {
            let y = self.y(line.value);
            stroke_path(
                painter,
                &[Pos2::new(self.plot.min.x, y), Pos2::new(self.plot.max.x, y)],
                Stroke::new(1.0, faded(line.color, 0.8)),
                &[4.0, 3.0],
            );
        }
    }

    // MARK: Drawings

    pub fn draw_boxes(&self, painter: &Painter) {
        for item in self.pine.boxes.iter().filter(|b| b.is_complete()) {
            let mut left = self.x_bar(item.left);
            let mut right = self.x_bar(item.right);
            if item.extend.extends_left() {
                left = left.min(right) - self.plot.width() * 2.0;
            }
            if item.extend.extends_right() {
                right = left.max(right) + self.plot.width() * 2.0;
            }
            let (top, bottom) = (self.y(item.top), self.y(item.bottom));
            let rect = Rect::from_min_max(
                Pos2::new(left.min(right), top.min(bottom)),
                Pos2::new(left.max(right), top.max(bottom)),
            );
            if let Some(fill) = item.background_color {
                painter.rect_filled(rect, 0.0, rgba(fill));
            }
            if let Some(border) = item.border_color
                && item.border_width > 0
            {
                let corners = [
                    rect.left_top(),
                    rect.right_top(),
                    rect.right_bottom(),
                    rect.left_bottom(),
                    rect.left_top(),
                ];
                stroke_path(
                    painter,
                    &corners,
                    Stroke::new(item.border_width as f32, rgba(border)),
                    dash_pattern(item.border_style),
                );
            }
            if !item.text.is_empty() && is_visible(item.text_color) {
                let (point, anchor) = box_text_placement(
                    rect,
                    item.text_horizontal_align,
                    item.text_vertical_align,
                    4.0,
                );
                text_at(
                    &painter.with_clip_rect(rect.intersect(painter.clip_rect())),
                    &item.text,
                    font_size(item.text_size),
                    rgba(item.text_color),
                    point,
                    anchor,
                );
            }
        }
    }

    /// The quadrilateral between a linefill's two lines, under the lines themselves.
    pub fn draw_linefills(&self, painter: &Painter) {
        for fill in &self.pine.linefills {
            let (Some(first), Some(second)) = (
                self.pine.lines.iter().find(|l| l.id == fill.line1),
                self.pine.lines.iter().find(|l| l.id == fill.line2),
            ) else {
                continue;
            };
            if !first.is_complete() || !second.is_complete() {
                continue;
            }
            let a = self.point_bar(first.x1, first.y1);
            let b = self.point_bar(first.x2, first.y2);
            let c = self.point_bar(second.x1, second.y1);
            let d = self.point_bar(second.x2, second.y2);
            let mut mesh = Mesh::default();
            quad(&mut mesh, [a, b, d, c], [rgba(fill.color); 4]);
            paint_mesh(painter, mesh);
        }
    }

    /// Segments (or a spline) through each polyline's points: filled when closed with a fill
    /// colour, stroked in its line colour.
    pub fn draw_polylines(&self, painter: &Painter) {
        for polyline in self.pine.polylines.iter().filter(|p| p.points.len() > 1) {
            let locations: Vec<Pos2> = polyline
                .points
                .iter()
                .map(|point| self.point_bar(point.index, point.price))
                .collect();
            let mut path = polyline_points(&locations, polyline.curved, polyline.closed);
            if polyline.closed
                && let Some(fill) = polyline.fill_color
            {
                let mut mesh = Mesh::default();
                fan(&mut mesh, &path, rgba(fill));
                paint_mesh(painter, mesh);
            }
            let Some(color) = polyline.line_color else {
                continue;
            };
            if polyline.width == 0 {
                continue;
            }
            if polyline.closed
                && let Some(first) = path.first().copied()
            {
                path.push(first);
            }
            stroke_path(
                painter,
                &path,
                Stroke::new(polyline.width as f32, rgba(color)),
                dash_pattern(polyline.style),
            );
        }
    }

    pub fn draw_lines(&self, painter: &Painter) {
        for line in self.pine.lines.iter().filter(|l| l.is_complete()) {
            let start = self.point_bar(line.x1, line.y1);
            let end = self.point_bar(line.x2, line.y2);
            let (from, to) = extended_line(start, end, line.extend, self.plot);
            let width = line.width.max(1) as f32;
            let color = rgba(line.color);
            stroke_path(
                painter,
                &[from, to],
                Stroke::new(width, color),
                dash_pattern(line.style),
            );
            let (show_start, show_end) = arrowheads(line.style);
            let size = width * 2.0 + 6.0;
            for (show, tip, origin) in [(show_start, start, end), (show_end, end, start)] {
                if !show {
                    continue;
                }
                if let Some(points) = arrowhead(tip, origin, size, size / 2.5) {
                    painter.add(Shape::convex_polygon(points.to_vec(), color, Stroke::NONE));
                }
            }
        }
    }

    // MARK: Arrows

    /// `plotarrow()`: an up arrow whose tip sits just under a bar's low for each positive value and
    /// a down arrow just over its high for each negative one. The length is in proportion to the
    /// value against the series' largest (inferred: the reference only says that the larger the
    /// value, the longer the arrow); in a script pane the arrows pin to its edges.
    pub fn draw_arrows(&self, painter: &Painter) {
        for arrow in self
            .pine
            .arrows
            .iter()
            .filter(|a| a.display & DISPLAY_PANE != 0)
        {
            let largest = arrow.largest();
            for i in 0..self.count() {
                let Some(value) = self.visible_opt(&arrow.values, i).filter(|v| *v != 0.0) else {
                    continue;
                };
                let up = value > 0.0;
                let colors = if up {
                    &arrow.up_colors
                } else {
                    &arrow.down_colors
                };
                let Some(color) = self
                    .visible_opt(colors, i)
                    .filter(|color| is_visible(*color))
                else {
                    continue;
                };
                let location = if up {
                    MarkerLocation::BelowBar
                } else {
                    MarkerLocation::AboveBar
                };
                let tip = self.marker_y(location, &[], i);
                draw_arrow(
                    painter,
                    Pos2::new(self.x_at(i), tip),
                    up,
                    arrow.height(value, largest) as f32,
                    rgba(color),
                );
            }
        }
    }

    // MARK: Labels

    /// Labels draw as a bubble with a pointer toward their anchor; a transparent label colour
    /// leaves just text.
    pub fn draw_labels(&self, painter: &Painter) {
        for label in self.pine.labels.iter().filter(|l| l.is_complete()) {
            let Some(y) = self.label_y(label) else {
                continue;
            };
            let anchor = Pos2::new(self.x_bar(label.x), y);
            let size = font_size(label.size);
            let text = rgba(label.text_color);
            let measured = measure(painter, &label.text, size);
            let layout = label_layout(label.style, anchor, measured + LABEL_PADDING, LABEL_POINTER);
            if let Some(fill) = label.color.filter(|c| is_visible(*c)) {
                draw_label_shape(painter, &layout, rgba(fill));
            }
            text_at(
                painter,
                &label.text,
                size,
                text,
                layout.body.center(),
                Align2::CENTER_CENTER,
            );
        }
    }

    /// Where a label's anchor sits vertically: its own price, or just above the high / below the low
    /// of its bar for `yloc.abovebar` / `yloc.belowbar` (pinned to the pane's edges in a script
    /// pane, which has no candles). None when the bar is not on screen.
    fn label_y(&self, label: &LabelOutput) -> Option<f32> {
        let below = match label.yloc {
            YLocation::Price => return Some(self.y(label.y)),
            YLocation::AboveBar => false,
            YLocation::BelowBar => true,
        };
        if self.in_pane {
            return Some(if below {
                self.plot.max.y - MARKER_OFFSET
            } else {
                self.plot.min.y + MARKER_OFFSET
            });
        }
        let slot = geometry::candle_index_for_bar(
            label.x,
            self.pine.bar_count.saturating_sub(self.tail),
            self.count(),
        );
        let candle = self.candles.get(usize::try_from(slot).ok()?)?;
        Some(if below {
            self.y(candle.low) + MARKER_OFFSET
        } else {
            self.y(candle.high) - MARKER_OFFSET
        })
    }

    // MARK: Markers

    /// `plotshape` / `plotchar` markers.
    pub fn draw_markers(&self, painter: &Painter) {
        for marker in self
            .pine
            .markers
            .iter()
            .filter(|m| m.display & DISPLAY_PANE != 0)
        {
            for i in 0..self.count() {
                if self.visible(&marker.values, i) != Some(&true) {
                    continue;
                }
                let x = self.x_at(i);
                let y = self.marker_y(marker.location, &marker.prices, i);
                let color = rgba(self.visible_opt(&marker.colors, i).unwrap_or(marker.color));
                if marker.style == MarkerShape::Circle && marker.kind == MarkerKind::Shape {
                    painter.circle_filled(Pos2::new(x, y), marker_radius(marker.size), color);
                    continue;
                }
                let centre = Pos2::new(x, y);
                match &marker.character {
                    Some(glyph) => {
                        text_at(painter, glyph, 12.0, color, centre, Align2::CENTER_CENTER);
                    }
                    // Drawn, not typeset: "▲" and "▼" are in no font bundled with egui, so
                    // anywhere without the macOS symbol fonts they came out as tofu squares.
                    None => {
                        painter.add(Shape::convex_polygon(
                            geometry::triangle_points(
                                centre,
                                marker_radius(marker.size),
                                marker.style.points_down(),
                            )
                            .to_vec(),
                            color,
                            Stroke::NONE,
                        ));
                    }
                }
            }
        }
    }

    fn marker_y(&self, location: MarkerLocation, prices: &[Option<f64>], i: usize) -> f32 {
        if location == MarkerLocation::Absolute
            && let Some(price) = self.visible_opt(prices, i)
        {
            return self.y(price);
        }
        if self.in_pane {
            // No candles in a script pane: above/below bar pin to its edges.
            return if location.is_below() {
                self.plot.max.y - MARKER_OFFSET
            } else {
                self.plot.min.y + MARKER_OFFSET
            };
        }
        let candle = &self.candles[i];
        if location.is_below() {
            self.y(candle.low) + MARKER_OFFSET
        } else {
            self.y(candle.high) - MARKER_OFFSET
        }
    }

    // MARK: Strategy trades

    /// Strategy round trips: an arrow at each entry, a dot at each exit, and a dashed connector
    /// coloured by the trade's result. Prices are absolute, so this is chart-only.
    pub fn draw_strategy_trades(&self, painter: &Painter) {
        let Some(report) = self.pine.strategy.as_ref().filter(|_| !self.in_pane) else {
            return;
        };
        self.draw_trades(painter, report);
    }

    fn draw_trades(&self, painter: &Painter, report: &StrategyReport) {
        let end = self.pine.bar_count.saturating_sub(self.tail);
        let first = end.saturating_sub(self.count());
        let visible = |bar: usize| bar >= first && bar < end;
        let point = |bar: usize, price: f64| self.point_bar(bar as i64, price);
        let (win, loss) = (rgba(WIN_COLOR), rgba(LOSS_COLOR));
        for trade in report
            .trades
            .iter()
            .filter(|t| visible(t.entry_bar) || visible(t.exit_bar))
        {
            let entry = point(trade.entry_bar, trade.entry_price);
            let exit = point(trade.exit_bar, trade.exit_price);
            let tint = if trade.profit >= 0.0 { win } else { loss };
            stroke_path(
                painter,
                &[entry, exit],
                Stroke::new(1.0, tint.gamma_multiply(0.7)),
                &[3.0, 3.0],
            );
            entry_arrow(
                painter,
                trade.is_long,
                entry,
                if trade.is_long { win } else { loss },
            );
            painter.circle_filled(exit, 3.0, tint);
        }
        for trade in report.open_trades.iter().filter(|t| visible(t.entry_bar)) {
            let entry = point(trade.entry_bar, trade.entry_price);
            entry_arrow(
                painter,
                trade.is_long,
                entry,
                if trade.is_long { win } else { loss },
            );
        }
    }

    // MARK: Tables

    /// `table.new` grids, sized to their cell text and pinned to a plot corner or edge. Tables are
    /// never value-anchored, so callers draw them outside the series clip.
    pub fn draw_tables(&self, painter: &Painter) {
        for table in self
            .pine
            .tables
            .iter()
            .filter(|t| t.columns > 0 && t.rows > 0 && !t.cells.is_empty())
        {
            let area = self.plot.shrink(geometry::TABLE_MARGIN);
            let layout = geometry::measure_table(table, area.size(), &|text, size| {
                measure(painter, text, size)
            });
            let origin = geometry::table_origin(layout.size(), table.position, area);
            draw_table(painter, table, &layout, origin);
        }
    }
}

fn entry_arrow(painter: &Painter, long: bool, tip: Pos2, color: Color32) {
    let half = 5.0;
    let depth = half * 1.6 * if long { 1.0 } else { -1.0 };
    painter.add(Shape::convex_polygon(
        vec![
            tip,
            Pos2::new(tip.x - half, tip.y + depth),
            Pos2::new(tip.x + half, tip.y + depth),
        ],
        color,
        Stroke::NONE,
    ));
}

/// Half the width of the diamonds a `plot.style_stepline_diamond` plot marks its changes with, at
/// least.
const DIAMOND_RADIUS: f32 = 3.5;

fn draw_diamond(painter: &Painter, centre: Pos2, radius: f32, color: Color32) {
    painter.add(Shape::convex_polygon(
        geometry::diamond_points(centre, radius).to_vec(),
        color,
        Stroke::NONE,
    ));
}

/// An arrow of length `height` whose tip is at `tip`, pointing up (its tail below) or down.
fn draw_arrow(painter: &Painter, tip: Pos2, up: bool, height: f32, color: Color32) {
    let arrow = geometry::arrow_shape(tip, up, height);
    painter.add(Shape::convex_polygon(
        arrow.head.to_vec(),
        color,
        Stroke::NONE,
    ));
    if let Some(shaft) = arrow.shaft {
        painter.add(Shape::rect_filled(shaft, 0.0, color));
    }
}

fn draw_label_shape(painter: &Painter, layout: &geometry::LabelLayout, fill: Color32) {
    match &layout.shape {
        LabelShape::Bubble { pointer } => {
            painter.rect_filled(layout.body, 3.0, fill);
            if pointer.len() == 3 {
                painter.add(Shape::convex_polygon(pointer.clone(), fill, Stroke::NONE));
            }
        }
        LabelShape::Ellipse => {
            painter.circle_filled(layout.body.center(), layout.body.width() / 2.0, fill);
        }
        LabelShape::Rectangle => {
            painter.rect_filled(layout.body, 2.0, fill);
        }
        LabelShape::Diamond => {
            let b = layout.body;
            painter.add(Shape::convex_polygon(
                vec![
                    Pos2::new(b.center().x, b.min.y),
                    Pos2::new(b.max.x, b.center().y),
                    Pos2::new(b.center().x, b.max.y),
                    Pos2::new(b.min.x, b.center().y),
                ],
                fill,
                Stroke::NONE,
            ));
        }
        LabelShape::Marker(glyph) => {
            if let Some(rect) = layout.glyph {
                draw_glyph(painter, *glyph, rect, fill);
            }
        }
        LabelShape::TextOnly => {}
    }
}

/// Simplified glyphs for the marker label styles.
fn draw_glyph(painter: &Painter, glyph: LabelGlyph, b: Rect, fill: Color32) {
    let stroke = Stroke::new(2.0, fill);
    match glyph {
        LabelGlyph::TriangleUp | LabelGlyph::ArrowUp => {
            painter.add(Shape::convex_polygon(
                vec![
                    Pos2::new(b.center().x, b.min.y),
                    Pos2::new(b.max.x, b.max.y),
                    Pos2::new(b.min.x, b.max.y),
                ],
                fill,
                Stroke::NONE,
            ));
        }
        LabelGlyph::TriangleDown | LabelGlyph::ArrowDown => {
            painter.add(Shape::convex_polygon(
                vec![
                    Pos2::new(b.center().x, b.max.y),
                    Pos2::new(b.max.x, b.min.y),
                    Pos2::new(b.min.x, b.min.y),
                ],
                fill,
                Stroke::NONE,
            ));
        }
        LabelGlyph::Flag => {
            painter.add(Shape::line(
                vec![
                    Pos2::new(b.min.x, b.max.y),
                    Pos2::new(b.min.x, b.min.y),
                    Pos2::new(b.max.x, b.min.y + b.height() / 3.0),
                    Pos2::new(b.min.x, b.center().y),
                ],
                stroke,
            ));
        }
        LabelGlyph::Cross => {
            painter.line_segment(
                [
                    Pos2::new(b.center().x, b.min.y),
                    Pos2::new(b.center().x, b.max.y),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    Pos2::new(b.min.x, b.center().y),
                    Pos2::new(b.max.x, b.center().y),
                ],
                stroke,
            );
        }
        LabelGlyph::Xcross => {
            painter.line_segment([b.left_top(), b.right_bottom()], stroke);
            painter.line_segment([b.right_top(), b.left_bottom()], stroke);
        }
    }
}

fn draw_table(
    painter: &Painter,
    table: &TableOutput,
    layout: &geometry::TableLayout,
    origin: Pos2,
) {
    let frame = Rect::from_min_size(origin, layout.size());
    if let Some(background) = table.background_color {
        painter.rect_filled(frame, 0.0, rgba(background));
    }
    for cell in &table.cells {
        if cell.column >= table.columns || cell.row >= table.rows {
            continue;
        }
        let rect = layout.rect_of(cell, origin);
        if let Some(background) = cell.background_color {
            painter.rect_filled(rect, 0.0, rgba(background));
        }
        let (point, anchor) = box_text_placement(
            rect,
            cell.text_horizontal_align,
            cell.text_vertical_align,
            geometry::CELL_PADDING.x / 2.0,
        );
        text_at(
            painter,
            &cell.text,
            font_size(cell.text_size),
            rgba(cell.text_color),
            point,
            anchor,
        );
    }
    if let Some(border) = table.border_color
        && table.border_width > 0
    {
        for segment in geometry::grid_lines(table, layout, frame) {
            painter.line_segment(
                segment,
                Stroke::new(table.border_width as f32, rgba(border)),
            );
        }
    }
    if let Some(frame_color) = table.frame_color
        && table.frame_width > 0
    {
        painter.rect_stroke(
            frame,
            0.0,
            Stroke::new(table.frame_width as f32, rgba(frame_color)),
            egui::StrokeKind::Middle,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pine::outputs::{
        BoxOutput, ColorOutput, FillOutput, HorizontalLine, LabelOutput, LabelStyle, LineExtend,
        LineOutput, LineStyle, MarkerOutput, Size, TableCell, TablePosition, TextAlign, YLocation,
    };

    fn candles(count: usize) -> Vec<Kline> {
        (0..count)
            .map(|i| Kline {
                open_time: i as i64 * 60_000,
                close_time: i as i64 * 60_000 + 59_999,
                open: 100.0 + i as f64,
                high: 105.0 + i as f64,
                low: 95.0 + i as f64,
                close: 102.0 + i as f64,
                volume: 1.0,
            })
            .collect()
    }

    fn plot_output(values: Vec<Option<f64>>, style: PlotStyle) -> PlotOutput {
        PlotOutput {
            id: 0,
            title: None,
            values,
            color: 0xFF00_00FF,
            line_width: 2,
            style,
            colors: Vec::new(),
            display: DISPLAY_PANE,
            hist_base: 100.0,
            offset: 0,
            force_overlay: false,
        }
    }

    /// Runs `paint` on a headless egui frame and returns how many shapes it drew.
    fn shapes_drawn(paint: impl FnOnce(&Painter)) -> usize {
        let ctx = egui::Context::default();
        let mut paint = Some(paint);
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            if let Some(paint) = paint.take() {
                paint(ui.painter());
            }
        });
        // The frame's texture uploads go unused here; egui insists they are dropped on purpose.
        output.textures_delta.clear();
        output.shapes.len()
    }

    fn layer<'a>(
        pine: &'a VisualOutput,
        candles: &'a [Kline],
        x: &'a dyn Fn(i64) -> f32,
        y: &'a dyn Fn(f64) -> f32,
    ) -> PineLayer<'a> {
        PineLayer {
            pine,
            candles,
            tail: 0,
            in_pane: false,
            plot: Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(200.0, 100.0)),
            slot_width: 20.0,
            x_for_index: x,
            y_for: y,
            candle_style: CandleStyle::default(),
        }
    }

    fn x_of(index: i64) -> f32 {
        index as f32 * 20.0 + 10.0
    }

    fn y_of(price: f64) -> f32 {
        (200.0 - price) as f32
    }

    fn empty() -> VisualOutput {
        VisualOutput {
            overlay: true,
            bar_count: 5,
            ..VisualOutput::default()
        }
    }

    #[test]
    fn nothing_to_draw_draws_nothing() {
        let output = empty();
        let candles = candles(5);
        let (x, y) = (x_of as fn(i64) -> f32, y_of as fn(f64) -> f32);
        let layer = layer(&output, &candles, &x, &y);
        assert_eq!(shapes_drawn(|painter| layer.draw_foreground(painter)), 0);
        assert_eq!(shapes_drawn(|painter| layer.draw_background(painter)), 0);
        assert_eq!(shapes_drawn(|painter| layer.draw_tables(painter)), 0);
        assert!(layer.bar_color(0).is_none());
    }

    #[test]
    fn a_gap_splits_a_line_plot_into_two_segments() {
        let mut output = empty();
        output.plots.push(plot_output(
            vec![Some(100.0), Some(101.0), None, Some(103.0), Some(104.0)],
            PlotStyle::Line,
        ));
        let candles = candles(5);
        let (x, y) = (x_of as fn(i64) -> f32, y_of as fn(f64) -> f32);
        let layer = layer(&output, &candles, &x, &y);
        assert_eq!(
            shapes_drawn(|painter| layer.draw_plots(painter)),
            2,
            "one stroke per run"
        );
        let mut single = output.clone();
        single.plots[0].values = vec![Some(1.0); 5];
        let layer = self::layer(&single, &candles, &x, &y);
        assert_eq!(shapes_drawn(|painter| layer.draw_plots(painter)), 1);
        let mut hidden = single.clone();
        hidden.plots[0].display = 0;
        let layer = self::layer(&hidden, &candles, &x, &y);
        assert_eq!(
            shapes_drawn(|painter| layer.draw_plots(painter)),
            0,
            "display.none is not drawn"
        );
    }

    #[test]
    fn every_plot_style_paints_something() {
        let candles = candles(5);
        let (x, y) = (x_of as fn(i64) -> f32, y_of as fn(f64) -> f32);
        for style in [
            PlotStyle::Line,
            PlotStyle::Stepline,
            PlotStyle::Histogram,
            PlotStyle::Columns,
            PlotStyle::Area,
            PlotStyle::Circles,
            PlotStyle::Cross,
        ] {
            let mut output = empty();
            output.plots.push(plot_output(vec![Some(110.0); 5], style));
            let layer = layer(&output, &candles, &x, &y);
            assert!(
                shapes_drawn(|painter| layer.draw_plots(painter)) > 0,
                "{style:?}"
            );
        }
    }

    #[test]
    fn a_colour_change_ends_a_line_run_and_a_clear_colour_hides_it() {
        let candles = candles(4);
        let (x, y) = (x_of as fn(i64) -> f32, y_of as fn(f64) -> f32);
        let mut output = VisualOutput {
            bar_count: 4,
            ..empty()
        };
        let mut plot = plot_output(vec![Some(100.0); 4], PlotStyle::Line);
        plot.colors = vec![
            Some(0xFF00_00FF),
            Some(0xFF00_00FF),
            Some(0x00FF_00FF),
            Some(0x00FF_00FF),
        ];
        output.plots.push(plot);
        let layer = layer(&output, &candles, &x, &y);
        assert_eq!(shapes_drawn(|painter| layer.draw_plots(painter)), 2);
        output.plots[0].colors = vec![Some(0); 4];
        let layer = self::layer(&output, &candles, &x, &y);
        assert_eq!(
            shapes_drawn(|painter| layer.draw_plots(painter)),
            0,
            "alpha 0 is invisible"
        );
    }

    #[test]
    fn drawings_paint_and_incomplete_ones_are_skipped() {
        let candles = candles(5);
        let (x, y) = (x_of as fn(i64) -> f32, y_of as fn(f64) -> f32);
        let mut output = empty();
        output.lines.push(LineOutput {
            id: 1,
            x1: 0,
            y1: 100.0,
            x2: 4,
            y2: 110.0,
            color: 0x00FF_00FF,
            width: 2,
            style: LineStyle::ArrowBoth,
            extend: LineExtend::Right,
            time_anchored: false,
        });
        output.lines.push(LineOutput {
            id: 2,
            x1: crate::pine::outputs::MISSING_INDEX,
            y1: 100.0,
            x2: 4,
            y2: 110.0,
            color: 0x00FF_00FF,
            width: 1,
            style: LineStyle::Solid,
            extend: LineExtend::None,
            time_anchored: false,
        });
        output.boxes.push(BoxOutput {
            id: 1,
            left: 1,
            top: 120.0,
            right: 3,
            bottom: 105.0,
            border_color: Some(0xFFFF_00FF),
            border_width: 1,
            background_color: Some(0x0000_FF40),
            border_style: LineStyle::Dashed,
            extend: LineExtend::None,
            text: "zone".into(),
            text_color: 0xFFFF_FFFF,
            text_size: Size::Small,
            text_horizontal_align: TextAlign::Left,
            text_vertical_align: TextAlign::Top,
            time_anchored: false,
        });
        output.labels.push(LabelOutput {
            id: 1,
            x: 2,
            y: 108.0,
            text: "hi".into(),
            color: Some(0xFF00_00FF),
            text_color: 0xFFFF_FFFF,
            style: LabelStyle::LabelDown,
            size: Size::Normal,
            tooltip: None,
            text_align: TextAlign::Center,
            yloc: YLocation::Price,
            time_anchored: false,
        });
        let layer = layer(&output, &candles, &x, &y);
        assert!(
            shapes_drawn(|painter| layer.draw_lines(painter)) >= 2,
            "the line and its heads"
        );
        assert!(
            shapes_drawn(|painter| layer.draw_boxes(painter)) >= 3,
            "fill, dashed border, text"
        );
        assert!(
            shapes_drawn(|painter| layer.draw_labels(painter)) >= 2,
            "bubble, pointer, text"
        );
        let mut incomplete = empty();
        incomplete.lines.push(output.lines[1].clone());
        let layer = self::layer(&incomplete, &candles, &x, &y);
        assert_eq!(shapes_drawn(|painter| layer.draw_lines(painter)), 0);
    }

    #[test]
    fn arrows_draw_for_non_zero_values_with_a_visible_color() {
        use crate::pine::outputs::{ArrowOutput, DISPLAY_ALL};
        let candles = candles(4);
        let (x, y) = (x_of as fn(i64) -> f32, y_of as fn(f64) -> f32);
        let mut output = VisualOutput {
            bar_count: 4,
            ..empty()
        };
        let arrow = |values: Vec<Option<f64>>, up: u32| ArrowOutput {
            id: 0,
            title: None,
            up_colors: vec![Some(up); values.len()],
            down_colors: vec![Some(0xFF00_00FF); values.len()],
            values,
            offset: 0,
            show_last: None,
            min_height: 5.0,
            max_height: 100.0,
            display: DISPLAY_ALL,
            force_overlay: false,
        };
        // A long up arrow (head and shaft), a short down arrow (head only), `na` and zero (nothing).
        output.arrows.push(arrow(
            vec![Some(50.0), Some(-1.0), None, Some(0.0)],
            0x2196_F3FF,
        ));
        let layer = layer(&output, &candles, &x, &y);
        assert_eq!(shapes_drawn(|painter| layer.draw_arrows(painter)), 3);
        // A fully transparent colour draws nothing.
        let mut clear = VisualOutput {
            bar_count: 4,
            ..empty()
        };
        clear
            .arrows
            .push(arrow(vec![Some(50.0), None, None, None], 0x2196_F300));
        let layer = self::layer(&clear, &candles, &x, &y);
        assert_eq!(shapes_drawn(|painter| layer.draw_arrows(painter)), 0);
    }

    #[test]
    fn the_diamond_step_line_marks_each_change_of_value() {
        let candles = candles(4);
        let (x, y) = (x_of as fn(i64) -> f32, y_of as fn(f64) -> f32);
        let values = vec![Some(100.0), Some(100.0), Some(110.0), Some(110.0)];
        let drawn = |style| {
            let mut output = VisualOutput {
                bar_count: 4,
                ..empty()
            };
            output.plots.push(plot_output(values.clone(), style));
            let layer = layer(&output, &candles, &x, &y);
            shapes_drawn(|painter| layer.draw_plots(painter))
        };
        let plain = drawn(PlotStyle::Stepline);
        // The first bar and the one where the value moves to 110: two diamonds more.
        assert_eq!(drawn(PlotStyle::SteplineDiamond), plain + 2);
    }

    #[test]
    fn a_label_above_or_below_its_bar_sits_at_the_bars_high_or_low() {
        let candles = candles(3);
        let (x, y) = (x_of as fn(i64) -> f32, y_of as fn(f64) -> f32);
        let mut output = VisualOutput {
            bar_count: 3,
            ..empty()
        };
        let label = |yloc, price| LabelOutput {
            id: 1,
            x: 1,
            y: price,
            text: "go".into(),
            color: Some(0xFF00_00FF),
            text_color: 0xFFFF_FFFF,
            style: LabelStyle::LabelDown,
            size: Size::Normal,
            tooltip: None,
            text_align: TextAlign::Center,
            yloc,
            time_anchored: false,
        };
        // The price is ignored (here it is `na`) once the label hangs off its bar.
        output.labels.push(label(YLocation::AboveBar, f64::NAN));
        output.labels.push(label(YLocation::BelowBar, f64::NAN));
        output.labels.push(label(YLocation::Price, 120.0));
        let layer = layer(&output, &candles, &x, &y);
        let high = y_of(candles[1].high);
        let low = y_of(candles[1].low);
        assert_eq!(layer.label_y(&output.labels[0]), Some(high - MARKER_OFFSET));
        assert_eq!(layer.label_y(&output.labels[1]), Some(low + MARKER_OFFSET));
        assert_eq!(layer.label_y(&output.labels[2]), Some(y_of(120.0)));
        assert!(
            shapes_drawn(|painter| layer.draw_labels(painter)) >= 6,
            "all three labels draw: bubble, pointer and text each"
        );
        // A bar that is not on screen has no high to hang from.
        let mut offscreen = output.labels[0].clone();
        offscreen.x = 40;
        assert_eq!(layer.label_y(&offscreen), None);
        // In a script pane the label pins to the edge of the pane.
        let mut pane = self::layer(&output, &candles, &x, &y);
        pane.in_pane = true;
        assert_eq!(
            pane.label_y(&output.labels[0]),
            Some(pane.plot.min.y + MARKER_OFFSET)
        );
        assert_eq!(
            pane.label_y(&output.labels[1]),
            Some(pane.plot.max.y - MARKER_OFFSET)
        );
    }

    #[test]
    fn triangle_markers_are_polygons_not_text() {
        let candles = candles(3);
        let (x, y) = (x_of as fn(i64) -> f32, y_of as fn(f64) -> f32);
        let mut output = VisualOutput {
            bar_count: 3,
            ..empty()
        };
        for style in [MarkerShape::TriangleUp, MarkerShape::TriangleDown] {
            output.markers.push(MarkerOutput {
                id: 0,
                kind: MarkerKind::Shape,
                title: None,
                values: vec![true, false, true],
                character: None,
                color: 0xFF00_00FF,
                location: MarkerLocation::BelowBar,
                style,
                prices: Vec::new(),
                colors: Vec::new(),
                size: Size::Small,
                display: DISPLAY_PANE,
                force_overlay: false,
            });
        }
        let layer = layer(&output, &candles, &x, &y);
        let ctx = egui::Context::default();
        let mut frame = ctx.run_ui(egui::RawInput::default(), |ui| {
            layer.draw_markers(ui.painter());
        });
        frame.textures_delta.clear();
        let shapes: Vec<_> = frame.shapes.iter().map(|clipped| &clipped.shape).collect();
        assert_eq!(shapes.len(), 4, "two bars for each of two markers");
        assert!(
            shapes
                .iter()
                .all(|shape| matches!(shape, Shape::Path(path) if path.points.len() == 3)),
            "{shapes:?}"
        );
    }

    #[test]
    fn markers_hlines_backgrounds_and_barcolors() {
        let candles = candles(3);
        let (x, y) = (x_of as fn(i64) -> f32, y_of as fn(f64) -> f32);
        let mut output = VisualOutput {
            bar_count: 3,
            ..empty()
        };
        output.markers.push(MarkerOutput {
            id: 0,
            kind: MarkerKind::Shape,
            title: None,
            values: vec![false, true, true],
            character: None,
            color: 0xFF00_00FF,
            location: MarkerLocation::BelowBar,
            style: MarkerShape::Circle,
            prices: Vec::new(),
            colors: Vec::new(),
            size: Size::Small,
            display: DISPLAY_PANE,
            force_overlay: false,
        });
        output.hlines.push(HorizontalLine {
            id: 0,
            value: 100.0,
            color: 0x8080_80FF,
            title: None,
        });
        output.backgrounds.push(ColorOutput {
            id: 0,
            title: None,
            colors: vec![None, Some(0xFF00_0030), None],
            offset: 0,
            show_last: None,
        });
        output.bar_colors.push(ColorOutput {
            id: 0,
            title: None,
            colors: vec![None, None, Some(0x00FF_00FF)],
            offset: 0,
            show_last: None,
        });
        let layer = layer(&output, &candles, &x, &y);
        assert_eq!(
            shapes_drawn(|painter| layer.draw_markers(painter)),
            2,
            "two true bars"
        );
        assert!(shapes_drawn(|painter| layer.draw_horizontal_lines(painter)) > 0);
        assert_eq!(
            shapes_drawn(|painter| layer.draw_background(painter)),
            1,
            "one band mesh"
        );
        assert_eq!(layer.bar_color(2), Some(rgba(0x00FF_00FF)));
        assert_eq!(layer.bar_color(0), None);
        // Absolute markers sit at their price (series are right-aligned: one value is the last bar's); bar markers hang off the candle.
        assert_eq!(
            layer.marker_y(MarkerLocation::Absolute, &[Some(150.0)], 2),
            y_of(150.0)
        );
        assert_eq!(
            layer.marker_y(MarkerLocation::BelowBar, &[], 1),
            y_of(96.0) + 8.0
        );
        assert_eq!(
            layer.marker_y(MarkerLocation::AboveBar, &[], 1),
            y_of(106.0) - 8.0
        );
        let mut in_pane = self::layer(&output, &candles, &x, &y);
        in_pane.in_pane = true;
        assert_eq!(in_pane.marker_y(MarkerLocation::AboveBar, &[], 1), 8.0);
        assert_eq!(in_pane.marker_y(MarkerLocation::BelowBar, &[], 1), 92.0);
    }

    #[test]
    fn fills_between_plots_need_both_plots_and_a_colour() {
        let candles = candles(4);
        let (x, y) = (x_of as fn(i64) -> f32, y_of as fn(f64) -> f32);
        let mut output = VisualOutput {
            bar_count: 4,
            ..empty()
        };
        let mut a = plot_output(vec![Some(110.0); 4], PlotStyle::Line);
        a.id = 1;
        let mut b = plot_output(
            vec![Some(100.0), Some(100.0), None, Some(100.0)],
            PlotStyle::Line,
        );
        b.id = 2;
        output.plots.extend([a, b]);
        output.fills.push(FillOutput {
            id: 0,
            plot_a: 1,
            plot_b: 2,
            colors: vec![Some(0x00FF_0040); 4],
            title: None,
            gradients: Vec::new(),
        });
        let layer = layer(&output, &candles, &x, &y);
        // Bars 0-1 form a run; bar 2 has no second plot; bar 3 alone is too short to fill.
        assert_eq!(shapes_drawn(|painter| layer.draw_fills(painter)), 1);
        output.fills[0].plot_b = 99;
        let layer = self::layer(&output, &candles, &x, &y);
        assert_eq!(
            shapes_drawn(|painter| layer.draw_fills(painter)),
            0,
            "unknown plot"
        );
    }

    #[test]
    fn tables_pin_inside_the_plot_and_draw_cells() {
        let candles = candles(3);
        let (x, y) = (x_of as fn(i64) -> f32, y_of as fn(f64) -> f32);
        let mut output = VisualOutput {
            bar_count: 3,
            ..empty()
        };
        output.tables.push(TableOutput {
            id: 0,
            position: TablePosition::TopRight,
            columns: 2,
            rows: 1,
            background_color: Some(0x1020_30FF),
            border_color: Some(0xFFFF_FFFF),
            border_width: 1,
            frame_color: Some(0xFFFF_FFFF),
            frame_width: 1,
            cells: vec![
                TableCell {
                    column: 0,
                    row: 0,
                    text: "RSI".into(),
                    text_color: 0xFFFF_FFFF,
                    background_color: None,
                    text_size: Size::Normal,
                    column_span: 1,
                    row_span: 1,
                    text_horizontal_align: TextAlign::Center,
                    text_vertical_align: TextAlign::Center,
                    width: 0.0,
                    height: 0.0,
                },
                TableCell {
                    column: 1,
                    text: "55".into(),
                    ..TableCell {
                        column: 1,
                        row: 0,
                        text: String::new(),
                        text_color: 0xFFFF_FFFF,
                        background_color: Some(0x0000_00FF),
                        text_size: Size::Normal,
                        column_span: 1,
                        row_span: 1,
                        text_horizontal_align: TextAlign::Center,
                        text_vertical_align: TextAlign::Center,
                        width: 0.0,
                        height: 0.0,
                    }
                },
            ],
        });
        let layer = layer(&output, &candles, &x, &y);
        // Background, a cell background, two texts, one separator, the frame.
        assert!(shapes_drawn(|painter| layer.draw_tables(painter)) >= 5);
    }

    #[test]
    fn strategy_trades_draw_connectors_arrows_and_dots() {
        use crate::pine::strategy::{OpenTrade, StrategyReport, StrategySettings, Trade};
        let candles = candles(5);
        let (x, y) = (x_of as fn(i64) -> f32, y_of as fn(f64) -> f32);
        let mut output = empty();
        output.strategy = Some(StrategyReport {
            settings: StrategySettings::default(),
            trades: vec![Trade {
                id: 1,
                entry_id: "L".into(),
                exit_id: "X".into(),
                is_long: true,
                quantity: 1.0,
                entry_bar: 1,
                entry_time: 0,
                entry_price: 100.0,
                exit_bar: 3,
                exit_time: 0,
                exit_price: 110.0,
                commission: 0.0,
                profit: 10.0,
                ..Trade::default()
            }],
            open_trades: vec![OpenTrade {
                entry_id: "S".into(),
                is_long: false,
                quantity: 1.0,
                entry_bar: 4,
                entry_time: 0,
                entry_price: 108.0,
                entry_commission: 0.0,
                ..OpenTrade::default()
            }],
            equity: Vec::new(),
            open_profit: 0.0,
            chart: crate::model::ChartKind::Standard,
        });
        let layer = layer(&output, &candles, &x, &y);
        // A dashed connector (several dashes), the entry arrow, the exit dot and the open trade's arrow.
        assert!(shapes_drawn(|painter| layer.draw_strategy_trades(painter)) >= 4);
        let mut pane = self::layer(&output, &candles, &x, &y);
        pane.in_pane = true;
        assert_eq!(
            shapes_drawn(|painter| pane.draw_strategy_trades(painter)),
            0,
            "chart-only"
        );
    }

    fn one_candle_output(bar_style: bool, color: Option<u32>, border: Option<u32>) -> VisualOutput {
        use crate::pine::outputs::{CandleBar, CandleOutput, DISPLAY_ALL};
        let mut output = VisualOutput {
            bar_count: 1,
            ..empty()
        };
        output.candles.push(CandleOutput {
            id: 0,
            title: None,
            bars: vec![Some(CandleBar {
                open: 101.0,
                high: 98.0,
                low: 104.0,
                close: 103.0,
                color,
                wick_color: color,
                border_color: border,
            })],
            display: DISPLAY_ALL,
            bar_style,
            editable: true,
            show_last: None,
            force_overlay: false,
        });
        output
    }

    #[test]
    fn plotbar_draws_a_stem_and_two_ticks_and_plotcandle_a_wick_and_a_body() {
        let candles = candles(1);
        let (x, y) = (x_of as fn(i64) -> f32, y_of as fn(f64) -> f32);
        let bar = one_candle_output(true, Some(0x00FF_00FF), Some(0xFF00_00FF));
        let bar_layer = layer(&bar, &candles, &x, &y);
        assert_eq!(shapes_drawn(|painter| bar_layer.draw_candles(painter)), 3);
        // A candle with no border of its own: a wick and a body.
        let candle = one_candle_output(false, Some(0x00FF_00FF), None);
        let candle_layer = layer(&candle, &candles, &x, &y);
        assert_eq!(
            shapes_drawn(|painter| candle_layer.draw_candles(painter)),
            2
        );
        // A bar with no colour draws nothing; so does a bar the script left `na`.
        let invisible = one_candle_output(true, None, None);
        let invisible_layer = layer(&invisible, &candles, &x, &y);
        assert_eq!(
            shapes_drawn(|painter| invisible_layer.draw_candles(painter)),
            0
        );
    }

    #[test]
    fn a_script_may_colour_a_bar_more_than_once_and_the_last_colour_wins() {
        let mut output = empty();
        for colors in [
            vec![Some(0xFF00_00FF), Some(0xFF00_00FF), None, None, None],
            vec![None, Some(0x00FF_00FF), None, None, Some(0x0000_FFFF)],
        ] {
            output.bar_colors.push(ColorOutput {
                id: output.bar_colors.len(),
                title: None,
                colors,
                offset: 0,
                show_last: None,
            });
        }
        let red = rgba(0xFF00_00FF);
        let green = rgba(0x00FF_00FF);
        assert_eq!(bar_color_at(&output, 5, 0, 0), Some(red));
        assert_eq!(
            bar_color_at(&output, 5, 0, 1),
            Some(green),
            "the later call wins"
        );
        assert_eq!(
            bar_color_at(&output, 5, 0, 2),
            None,
            "na on every call: the default"
        );
        assert_eq!(bar_color_at(&output, 5, 0, 4), Some(rgba(0x0000_FFFF)));
    }

    #[test]
    fn a_scrolled_back_window_reads_the_colours_of_its_own_bars() {
        let mut output = empty();
        output.bar_colors.push(ColorOutput {
            id: 0,
            title: None,
            colors: vec![Some(0xFF00_00FF), None, None, None, Some(0x00FF_00FF)],
            offset: 0,
            show_last: None,
        });
        // Three bars in view, the two newest off screen to the right: bars 0..3.
        assert_eq!(bar_color_at(&output, 3, 2, 0), Some(rgba(0xFF00_00FF)));
        // Showing the newest three: bars 2..5.
        assert_eq!(bar_color_at(&output, 3, 0, 2), Some(rgba(0x00FF_00FF)));
        let candles = candles(3);
        let (x, y) = (x_of as fn(i64) -> f32, y_of as fn(f64) -> f32);
        let mut scrolled = layer(&output, &candles, &x, &y);
        scrolled.tail = 2;
        assert_eq!(scrolled.bar_color(0), Some(rgba(0xFF00_00FF)));
    }
}
