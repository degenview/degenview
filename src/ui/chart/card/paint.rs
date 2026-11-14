//! The live chart's layers under the crosshair: the grid, the series, volume, the built-in
//! indicators, trend-flip markers, the last price and the time axis.

use super::*;

impl ChartCard {
    /// The price labels and the grid lines behind the series. One tick list serves both, so a
    /// line is always on its label; the vertical lines stand on the time axis' own ticks. The
    /// grid is drawn from the scale and never feeds back into it.
    pub(super) fn draw_grid(
        &self,
        painter: &egui::Painter,
        plot: Rect,
        rect: Rect,
        scale: &PriceScale,
        style: &CanvasStyle,
        tick_xs: &[f32],
    ) {
        let budget = chart::canvas::tick_budget(plot.height(), style.price_tick_spacing());
        let AxisTicks { prices, step } = scale.axis_ticks(budget);
        let half = style.tag_half_height();
        for price in prices {
            let y = scale.y(price);
            if !y.is_finite() || y < plot.top() || y > plot.bottom() {
                continue;
            }
            if let Some(grid) = style.horizontal_grid {
                canvas::horizontal(painter, (plot.left(), plot.right()), y, grid);
            }
            // A label that would hang over the plot's edge waits for the scale to move.
            if y - half >= plot.top() - half && y + half <= plot.bottom() + half {
                price_text::paint(
                    painter,
                    Pos2::new(rect.right() - 7.0, y),
                    Align2::RIGHT_CENTER,
                    &scale.tick_label(self.appearance.price_format(), price, step),
                    style.price_font(),
                    style.text_color,
                );
            }
        }
        if let Some(grid) = style.vertical_grid {
            for x in tick_xs {
                if *x >= plot.left() && *x <= plot.right() {
                    canvas::vertical(painter, *x, (plot.top(), plot.bottom()), grid);
                }
            }
        }
    }

    pub(super) fn draw_line(
        &self,
        painter: &egui::Painter,
        candles: &[Kline],
        x_for: &dyn Fn(usize) -> f32,
        scale: &PriceScale,
    ) {
        if candles.len() < 2 {
            if let Some(candle) = candles.first() {
                painter.circle_filled(
                    Pos2::new(x_for(0), scale.y(candle.close)),
                    2.0,
                    self.bullish_color(),
                );
            }
            return;
        }
        let points: Vec<Pos2> = candles
            .iter()
            .enumerate()
            .map(|(index, candle)| Pos2::new(x_for(index), scale.y(candle.close)))
            .collect();
        for pair in points.windows(2) {
            let color = if pair[1].y <= pair[0].y {
                self.bullish_color()
            } else {
                self.bearish_color()
            };
            painter.line_segment([pair[0], pair[1]], Stroke::new(1.5, color));
        }
    }

    /// Each choice as a plain line, labelled at its right end.
    pub(super) fn draw_series(
        &self,
        painter: &egui::Painter,
        plot: Rect,
        shown: &[Kline],
        x_for: &dyn Fn(usize) -> f32,
        scale: &PriceScale,
    ) {
        let (Some(first), Some(last)) = (shown.first(), shown.last()) else {
            return;
        };
        let mut ends: Vec<(f32, &SeriesLine)> = Vec::new();
        for line in &self.series {
            let mut previous: Option<Pos2> = None;
            for candle in self
                .series_points(line)
                .filter(|candle| (first.open_time..=last.open_time).contains(&candle.open_time))
            {
                let index = shown
                    .partition_point(|shown| shown.open_time < candle.open_time)
                    .min(shown.len() - 1);
                let point = Pos2::new(x_for(index), scale.y(candle.close));
                if let Some(previous) = previous {
                    painter.line_segment([previous, point], Stroke::new(1.5, line.color));
                }
                previous = Some(point);
            }
            if let Some(end) = previous {
                ends.push((end.y, line));
            }
        }
        // Top to bottom, pushed apart so neighbouring labels do not stack, and kept in the plot.
        ends.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut top = f32::MIN;
        for (y, line) in ends {
            let y = y
                .max(top + SERIES_LABEL_GAP)
                .clamp(plot.top() + 6.0, plot.bottom() - 6.0);
            top = y;
            let label = painter.layout_no_wrap(
                truncated(&line.label, SERIES_LABEL_CHARS),
                FontId::proportional(9.0),
                theme::text_primary(),
            );
            let text = Rect::from_min_size(
                Pos2::new(
                    plot.right() - 4.0 - label.size().x,
                    y - label.size().y / 2.0,
                ),
                label.size(),
            );
            painter.rect_filled(
                text.expand2(Vec2::new(11.0, 2.0))
                    .translate(Vec2::new(-4.0, 0.0)),
                4.0,
                theme::with_alpha(theme::surface(), 0.85),
            );
            painter.circle_filled(Pos2::new(text.left() - 6.0, y), 3.0, line.color);
            painter.galley(text.min, label, theme::text_primary());
        }
    }

    pub(super) fn draw_volume(
        &self,
        painter: &egui::Painter,
        plot: Rect,
        x_for: &dyn Fn(usize) -> f32,
        slot_width: f32,
        candles: &[Kline],
    ) {
        let peak = candles
            .iter()
            .map(|candle| candle.volume)
            .fold(0.0, f64::max);
        if peak <= 0.0 {
            return;
        }
        let pane_height = plot.height() * 0.22;
        let pane_bottom = plot.bottom();
        let bar_width = (slot_width * 0.6).clamp(1.0, 8.0);
        for (index, candle) in candles.iter().enumerate() {
            if candle.volume <= 0.0 {
                continue;
            }
            let height = (pane_height * (candle.volume / peak) as f32).max(1.0);
            let x = x_for(index);
            let color = if candle.is_up() {
                self.bullish_color()
            } else {
                self.bearish_color()
            };
            let bar = Rect::from_min_max(
                Pos2::new(x - bar_width / 2.0, pane_bottom - height),
                Pos2::new(x + bar_width / 2.0, pane_bottom),
            );
            painter.rect_filled(bar, 0.0, theme::with_alpha_byte(color, 89));
        }
    }

    pub(super) fn draw_ema(
        &self,
        painter: &egui::Painter,
        x_for: &dyn Fn(usize) -> f32,
        scale: &PriceScale,
        values: &[Option<f64>],
    ) {
        let stroke = Stroke::new(1.5, theme::EMA);
        let mut previous: Option<Pos2> = None;
        for (index, value) in values.iter().enumerate() {
            let Some(value) = value else { continue };
            let point = Pos2::new(x_for(index), scale.y(*value));
            if let Some(previous) = previous {
                painter.line_segment([previous, point], stroke);
            }
            previous = Some(point);
        }
    }

    pub(super) fn draw_bollinger(
        &self,
        painter: &egui::Painter,
        x_for: &dyn Fn(usize) -> f32,
        scale: &PriceScale,
        bands: &BollingerSeries,
    ) {
        let mut upper_points = Vec::new();
        let mut lower_points = Vec::new();
        for index in 0..bands.upper.len() {
            if let (Some(upper), Some(lower)) = (bands.upper[index], bands.lower[index]) {
                let x = x_for(index);
                upper_points.push(Pos2::new(x, scale.y(upper)));
                lower_points.push(Pos2::new(x, scale.y(lower)));
            }
        }
        if upper_points.len() < 2 {
            return;
        }

        // Tint the channel as a strip of quads. One `PathShape` around the whole band is a
        // concave polygon, which egui fills as a fan from its first vertex, so it paints
        // spokes from the far-left point to every other vertex.
        let tint = theme::with_alpha_byte(theme::BOLLINGER, 26);
        let mut channel = egui::epaint::Mesh::default();
        for (upper, lower) in upper_points.iter().zip(&lower_points) {
            channel.colored_vertex(*upper, tint);
            channel.colored_vertex(*lower, tint);
        }
        for column in 0..(upper_points.len() as u32 - 1) {
            let top = column * 2;
            channel.add_triangle(top, top + 1, top + 2);
            channel.add_triangle(top + 1, top + 3, top + 2);
        }
        painter.add(egui::Shape::mesh(channel));

        let rail = Stroke::new(1.0, theme::with_alpha_byte(theme::BOLLINGER, 180));
        for pair in upper_points.windows(2) {
            painter.line_segment([pair[0], pair[1]], rail);
        }
        for pair in lower_points.windows(2) {
            painter.line_segment([pair[0], pair[1]], rail);
        }

        let mut previous: Option<Pos2> = None;
        for (index, value) in bands.middle.iter().enumerate() {
            let Some(value) = value else { continue };
            let point = Pos2::new(x_for(index), scale.y(*value));
            if let Some(previous) = previous {
                painter.add(egui::Shape::dashed_line(
                    &[previous, point],
                    Stroke::new(1.0, theme::with_alpha_byte(theme::BOLLINGER, 115)),
                    4.0,
                    3.0,
                ));
            }
            previous = Some(point);
        }
    }

    pub(super) fn draw_rsi(
        &self,
        painter: &egui::Painter,
        plot: Rect,
        x_for: &dyn Fn(usize) -> f32,
        values: &[Option<f64>],
    ) {
        if !values.iter().any(|value| value.is_some()) {
            return;
        }
        let pane = Rect::from_min_max(
            Pos2::new(plot.left(), plot.bottom() - plot.height() * 0.28),
            plot.right_bottom(),
        );
        let y_for = |reading: f64| pane.bottom() - (reading as f32 / 100.0) * pane.height();

        for level in [crate::model::RSI_OVERBOUGHT, crate::model::RSI_OVERSOLD] {
            let y = y_for(level);
            painter.add(egui::Shape::dashed_line(
                &[Pos2::new(pane.left(), y), Pos2::new(pane.right(), y)],
                Stroke::new(1.0, theme::grid()),
                4.0,
                3.0,
            ));
        }

        let stroke = Stroke::new(1.0, theme::RSI);
        let mut previous: Option<Pos2> = None;
        for (index, value) in values.iter().enumerate() {
            let Some(value) = value else { continue };
            let point = Pos2::new(x_for(index), y_for(*value));
            if let Some(previous) = previous {
                painter.line_segment([previous, point], stroke);
            }
            previous = Some(point);
        }
    }

    /// Labelled up/down triangles at confirmed Supertrend regime changes. Each marker (triangle
    /// plus "Bullish"/"Bearish" pill) is clamped inside the plot so a flip at a price extreme
    /// stays readable.
    pub(super) fn draw_trend_flips(
        &self,
        painter: &egui::Painter,
        plot: Rect,
        x_for: &dyn Fn(usize) -> f32,
        scale: &PriceScale,
        candles: &[Kline],
        flips: &[Option<bool>],
    ) {
        let half = TREND_FLIP_MARKER_SIZE / 2.0;
        for (index, flip) in flips.iter().enumerate() {
            let Some(bullish) = *flip else { continue };
            let Some(candle) = candles.get(index) else {
                continue;
            };
            let x = x_for(index);
            let color = if bullish {
                self.bullish_color()
            } else {
                self.bearish_color()
            };
            let galley = painter.layout_no_wrap(
                if bullish { "Bullish" } else { "Bearish" }.to_owned(),
                FontId::proportional(TREND_FLIP_LABEL_FONT_SIZE),
                Color32::WHITE,
            );
            let label_size = galley.size() + TREND_FLIP_LABEL_PAD * 2.0;
            let group_height = TREND_FLIP_MARKER_SIZE + TREND_FLIP_LABEL_GAP + label_size.y;
            let price_y = scale.y(if bullish { candle.low } else { candle.high });
            let raw_center = if bullish {
                price_y + TREND_FLIP_MARKER_GAP + group_height / 2.0
            } else {
                price_y - TREND_FLIP_MARKER_GAP - group_height / 2.0
            };
            let (Some(group_center), Some(label_center_x)) = (
                contained_center(raw_center, plot.top(), plot.bottom(), group_height),
                contained_center(x, plot.left(), plot.right(), label_size.x),
            ) else {
                continue;
            };
            // The triangle sits on the candle's side of the group, the label beyond it.
            let side = if bullish { 1.0 } else { -1.0 };
            let triangle_y = group_center - side * (group_height - TREND_FLIP_MARKER_SIZE) / 2.0;
            let label_y = group_center + side * (group_height - label_size.y) / 2.0;
            let points = if bullish {
                vec![
                    Pos2::new(x, triangle_y - half),
                    Pos2::new(x - half, triangle_y + half),
                    Pos2::new(x + half, triangle_y + half),
                ]
            } else {
                vec![
                    Pos2::new(x, triangle_y + half),
                    Pos2::new(x - half, triangle_y - half),
                    Pos2::new(x + half, triangle_y - half),
                ]
            };
            painter.add(egui::Shape::convex_polygon(points, color, Stroke::NONE));

            let pill = Rect::from_center_size(Pos2::new(label_center_x, label_y), label_size);
            painter.rect_filled(pill, label_size.y / 2.0, color);
            painter.galley(pill.min + TREND_FLIP_LABEL_PAD, galley, Color32::WHITE);
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn draw_last_price(
        &self,
        painter: &egui::Painter,
        plot: Rect,
        rect: Rect,
        scale: &PriceScale,
        candles: &[Kline],
        constructed_last: Option<&Kline>,
        style: &CanvasStyle,
    ) {
        let half = style.tag_half_height();
        // A price-based chart's own last close is not what the market last traded at: it gets its
        // own dashed line and an outlined tag, and the solid tag stays the market's.
        if let Some(bar) = constructed_last {
            let y = scale.y(bar.close);
            if plot.y_range().contains(y) {
                let tint = theme::text_secondary();
                painter.extend(Shape::dashed_line(
                    &[Pos2::new(plot.left(), y), Pos2::new(plot.right(), y)],
                    Stroke::new(1.0, tint.gamma_multiply(0.7)),
                    3.0,
                    3.0,
                ));
                let tag = Rect::from_min_max(
                    Pos2::new(plot.right() + 3.0, y - half),
                    Pos2::new(rect.right() - 3.0, y + half),
                );
                painter.rect_stroke(tag, 3.0, Stroke::new(1.0, tint), StrokeKind::Inside);
                price_text::paint(
                    painter,
                    tag.center(),
                    Align2::CENTER_CENTER,
                    &scale.tag_label(self.appearance.price_format(), bar.close),
                    style.price_font(),
                    tint,
                );
            }
        }
        let Some(last) = candles.last() else {
            return;
        };
        let y = scale.y(last.close);
        if !plot.y_range().contains(y) {
            return;
        }
        let color = if last.is_up() {
            self.bullish_color()
        } else {
            self.bearish_color()
        };
        // Dotted, so the line that follows the price reads apart from the grid and the drawings.
        canvas::horizontal(
            painter,
            (plot.left(), plot.right()),
            y,
            canvas::LineStyle {
                stroke: Stroke::new(1.0, color.gamma_multiply(0.65)),
                dash: crate::model::chart::kind::LineDash::Dotted,
            },
        );
        let tag = Rect::from_min_max(
            Pos2::new(plot.right() + 3.0, y - half),
            Pos2::new(rect.right() - 3.0, y + half),
        );
        painter.rect_filled(tag, 3.0, color);
        price_text::paint(
            painter,
            tag.center(),
            Align2::CENTER_CENTER,
            &scale.tag_label(self.appearance.price_format(), last.close),
            style.price_font(),
            Color32::WHITE,
        );
    }

    /// The time labels, on the bars the vertical grid stands on.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn draw_time_axis(
        &self,
        painter: &egui::Painter,
        rect: Rect,
        candles: &[Kline],
        ticks: &[usize],
        x_for: &dyn Fn(usize) -> f32,
        plot_right: f32,
        style: &CanvasStyle,
    ) {
        let y = rect.bottom() - style.time_axis_height() / 2.0;
        for index in ticks {
            let Some(candle) = candles.get(*index) else {
                continue;
            };
            let label = format::timestamp(candle.open_time);
            let x = x_for(*index);
            // A label that would hang over the plot's right edge waits for the chart to move, as a
            // price label does at the plot's top and bottom: it would run into the price axis and
            // the scale button in the corner.
            let galley =
                painter.layout_no_wrap(label[5..].to_owned(), style.time_font(), style.text_color);
            if x + galley.size().x / 2.0 > plot_right {
                continue;
            }
            let at = Pos2::new(x, y) - galley.size() / 2.0;
            painter.galley(at, galley, style.text_color);
        }
    }
}

const TREND_FLIP_MARKER_SIZE: f32 = 7.0;
const TREND_FLIP_MARKER_GAP: f32 = 4.0;
const TREND_FLIP_LABEL_FONT_SIZE: f32 = 10.5;
const TREND_FLIP_LABEL_PAD: Vec2 = Vec2::new(5.0, 2.0);
const TREND_FLIP_LABEL_GAP: f32 = 2.0;

/// `center` moved so an item `length` long stays within `lower..=upper`, or `None` when it does
/// not fit at all.
pub(super) fn contained_center(center: f32, lower: f32, upper: f32, length: f32) -> Option<f32> {
    let (low, high) = (lower + length / 2.0, upper - length / 2.0);
    (length >= 0.0 && low <= high).then(|| center.clamp(low, high))
}
