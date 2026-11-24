//! The crosshair: its lines and tags, and the readout naming the bar under it.

use super::*;
use crate::i18n::{tr, tr_c};
use crate::model::chart::sync::TimeAxis;
use crate::model::chart::viewport::zoom::Slots;

impl ChartCard {
    /// Where the crosshair's column lies on this chart's plot. The owner's is under its pointer.
    /// Any other chart's is at the same instant, wherever that falls on its own bars; a chart
    /// without a time scale, or one that does not reach the instant on screen, has none.
    pub(super) fn crosshair_column(
        &self,
        crosshair: &CrosshairInfo,
        chart_id: u64,
        source: DataSource,
        plot: Rect,
        slots: Slots,
        bars: &[Kline],
    ) -> Option<f32> {
        if crosshair.owner_id == chart_id {
            return Some(plot.left() + plot.width() * crosshair.x_fraction as f32);
        }
        if !self.takes_part_in_sync(source) {
            return None;
        }
        let x = slots.x(TimeAxis::new(bars).x_at(crosshair.time?)?);
        plot.x_range().contains(x).then_some(x)
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn draw_crosshair(
        &self,
        painter: &egui::Painter,
        plot: Rect,
        rect: Rect,
        pane_rects: &[Rect],
        crosshair: &CrosshairInfo,
        chart_id: u64,
        x: f32,
        scale: &PriceScale,
        candles: &[Kline],
        slot_width: f32,
        lead: f32,
        show_volume: bool,
        style: &CanvasStyle,
        main_visible: bool,
        info: &CrosshairBars,
    ) {
        let line = style.crosshair;
        let caps = self.cursor.caps();

        // Vertical line — every chart draws it at the shared column, through the price chart and
        // on through each script pane under it.
        if caps.lines && main_visible && !crosshair.frozen {
            canvas::vertical(painter, x, (plot.top(), plot.bottom()), line);
        }
        for pane in pane_rects
            .iter()
            .filter(|_| caps.lines && !crosshair.frozen)
        {
            if pane.height() > crate::model::chart::canvas::COLLAPSED_PANE_HEIGHT {
                canvas::vertical(painter, x, (pane.top(), pane.bottom()), line);
            }
        }

        // Horizontal line + price pill — only the chart the pointer is over.
        if main_visible && crosshair.owner_id == chart_id && !crosshair.frozen {
            let y = scale.y(crosshair.price);
            if plot.y_range().contains(y) {
                if caps.lines {
                    canvas::horizontal(painter, (plot.left(), plot.right()), y, line);
                }
                if caps.axis_tags {
                    let half = style.tag_half_height();
                    let price_tag = Rect::from_min_max(
                        Pos2::new(plot.right() + 3.0, y - half),
                        Pos2::new(rect.right() - 3.0, y + half),
                    );
                    painter.rect_filled(price_tag, 3.0, Color32::from_gray(79));
                    price_text::paint(
                        painter,
                        price_tag.center(),
                        Align2::CENTER_CENTER,
                        &scale.tag_label(self.appearance.price_format(), crosshair.price),
                        style.price_font(),
                        Color32::WHITE,
                    );
                }
            }
        }

        let raw_index = ((x - plot.left()) / slot_width - lead).floor() as isize;
        let Some((index, candle)) = usize::try_from(raw_index)
            .ok()
            .and_then(|index| candles.get(index).map(|candle| (index, candle)))
        else {
            return;
        };

        if caps.axis_tags && !crosshair.frozen {
            let date = self.time_text(candle.open_time);
            let time_tag_width = self.time_tag_width(style);
            let time_tag = Rect::from_center_size(
                Pos2::new(x, rect.bottom() - style.time_axis_height() / 2.0),
                Vec2::new(time_tag_width, style.time_axis_height() - 6.0),
            )
            .intersect(Rect::from_min_max(
                Pos2::new(plot.left(), plot.bottom()),
                Pos2::new(plot.right(), rect.bottom()),
            ));
            painter.rect_filled(time_tag, 3.0, Color32::from_gray(79));
            painter.text(
                time_tag.center(),
                Align2::CENTER_CENTER,
                date,
                style.time_font(),
                Color32::WHITE,
            );
        }

        // The header covers the plot's top, so the readouts start under it.
        let below_header = self.below_header(plot);
        let readout_bottom = if style.ohlc_readout && caps.readout {
            let previous_close = match index.checked_sub(1) {
                Some(before) => Some(candles[before].close),
                None => info.previous_close,
            };
            self.draw_readout(
                painter,
                below_header,
                info,
                (index, candle, previous_close),
                show_volume,
            )
        } else {
            below_header.top() - 4.0
        };
        let (overlay_frame, overlay_view) = info.overlays;
        self.paint_overlay_rings(painter, below_header, overlay_frame, overlay_view, index, x);
        overlay_card::remember(painter.ctx(), chart_id, index, readout_bottom);
        if self.shows_values_tooltip() && crosshair.owner_id == chart_id && !crosshair.frozen {
            let previous_close = match index.checked_sub(1) {
                Some(before) => Some(candles[before].close),
                None => info.previous_close,
            };
            self.draw_values_tooltip(
                painter,
                plot,
                info,
                (index, candle, previous_close),
                show_volume,
            );
        }

        // Line, area and baseline charts follow one value: ring it on the bar under the pointer.
        if info.chart_type.data_class() == DataClass::StandardLine
            && info.chart_type != ChartType::Columns
        {
            let value = if info.chart_type == ChartType::HlcArea {
                candle.close
            } else {
                self.appearance.style.value_source().value(candle)
            };
            let centre = Pos2::new(
                plot.left() + slot_width * (index as f32 + 0.5) + slot_width * lead,
                scale.y(value),
            );
            if plot.contains(centre) {
                painter.circle(
                    centre,
                    4.0,
                    theme::surface(),
                    Stroke::new(2.0, theme::text_primary()),
                );
            }
        }
    }

    /// The box of values for the bar under the crosshair, in the plot's top-left corner.
    fn draw_readout(
        &self,
        painter: &egui::Painter,
        plot: Rect,
        info: &CrosshairBars,
        bar: (usize, &Kline, Option<f64>),
        show_volume: bool,
    ) -> f32 {
        let color = self.bar_color(bar.1, bar.2);
        let items = self.readout_rows(info, bar.0, bar.1, show_volume);
        paint_ohlc_readout(painter, plot, &items, color)
    }

    /// The `(label, value)` pairs naming the bar at `index`: what the box in the corner shows, and
    /// the first rows of the values tooltip.
    pub(super) fn readout_rows(
        &self,
        info: &CrosshairBars,
        index: usize,
        candle: &Kline,
        show_volume: bool,
    ) -> Vec<(String, String)> {
        let constructed = info.synthetic.and_then(|bars| bars.get(index));
        let price_format = self.appearance.price_format();
        let mut items = readout_items(info, constructed, candle, price_format, &|time| {
            self.time_text(time)
        });
        if show_volume {
            items.push((
                tr_c!("readout label", "Vol").to_owned(),
                crate::model::watchlist::quote::compact_number(candle.volume),
            ));
        }
        // The market's own price, apart from the constructed bar's.
        if constructed.is_some()
            && let Some(last) = info.source_candles.last()
        {
            items.push((
                tr_c!("readout label", "Mkt").to_owned(),
                price_format.format(last.close),
            ));
        }
        // The script's own candles and bars under the pointer, apart from the chart's.
        let hovered_from_end = info.shown_len + info.tail;
        for pine in &info.pine {
            for output in pine
                .candles
                .iter()
                .filter(|output| output.display & crate::pine::outputs::DISPLAY_PANE != 0)
            {
                let Some(bar) = crate::ui::pine::geometry::series_index(
                    output.bars.len(),
                    hovered_from_end,
                    index,
                )
                .and_then(|slot| output.bars[slot].as_ref()) else {
                    continue;
                };
                let name = output
                    .title
                    .clone()
                    .filter(|title| !title.is_empty())
                    .unwrap_or_else(|| {
                        if output.bar_style {
                            tr!("Bars")
                        } else {
                            tr!("Candles")
                        }
                        .to_owned()
                    });
                items.push((
                    name,
                    format!(
                        "{} {} {} {}",
                        price_format.format(bar.open),
                        price_format.format(bar.high),
                        price_format.format(bar.low),
                        price_format.format(bar.close)
                    ),
                ));
            }
        }
        items
    }
}

/// What the crosshair needs to name the bar under it: how the chart is drawn and, for a
/// price-based type, the bars it constructed with the candles they came from.
pub(super) struct CrosshairBars<'a> {
    pub(super) chart_type: ChartType,
    pub(super) synthetic: Option<&'a [SyntheticBar]>,
    /// The close of the bar before the first one in view, for colouring by the previous close.
    pub(super) previous_close: Option<f64>,
    /// The market's own candles, whole.
    pub(super) source_candles: &'a [Kline],
    /// The scripts drawing on the chart: their `plotcandle()` and `plotbar()` values show too.
    pub(super) pine: Vec<&'a VisualOutput>,
    /// Bars in view, and how many lie after them.
    pub(super) shown_len: usize,
    pub(super) tail: usize,
    /// The comparison lines, and the view they were resolved for.
    pub(super) overlays: (&'a overlays::OverlayFrame<'a>, &'a View),
    /// The built-in indicators over the bars in view, already computed for the frame, and the
    /// settings they were computed with.
    pub(super) indicators: &'a IndicatorSeries,
    pub(super) indicator_config: &'a IndicatorConfig,
    /// The scripts with a pane of their own, whose plots the values tooltip lists too.
    pub(super) pane_pine: Vec<&'a VisualOutput>,
}

/// The crosshair's values for one bar. A candle chart reads open, high, low and close; a
/// constructed bar says what it is first, then what is meaningful for its shape (a Kagi segment
/// has a start and an end, not a body), then which candles made it and whether it can still
/// change.
fn readout_items(
    info: &CrosshairBars,
    constructed: Option<&SyntheticBar>,
    bar: &Kline,
    price_format: PriceFormat,
    time_text: &dyn Fn(i64) -> String,
) -> Vec<(String, String)> {
    let ohlc = || {
        vec![
            (
                tr_c!("readout label: open", "O").to_owned(),
                price_format.format(bar.open),
            ),
            (
                tr_c!("readout label: high", "H").to_owned(),
                price_format.format(bar.high),
            ),
            (
                tr_c!("readout label: low", "L").to_owned(),
                price_format.format(bar.low),
            ),
            (
                tr_c!("readout label: close", "C").to_owned(),
                price_format.format(bar.close),
            ),
        ]
    };
    let Some(made) = constructed else {
        return ohlc();
    };
    let mut items = vec![(
        match info.chart_type {
            ChartType::HeikinAshi => tr!("Heikin Ashi"),
            ChartType::Renko => tr_c!("readout label", "Brick"),
            ChartType::LineBreak => tr_c!("readout label", "Line"),
            ChartType::Range => tr!("Range bar"),
            ChartType::Kagi => tr!("Kagi"),
            _ => tr!("Point & Figure"),
        }
        .to_owned(),
        String::new(),
    )];
    match made.shape {
        BarShape::Ohlc => items.extend(ohlc()),
        BarShape::Kagi { thick, .. } => {
            items.push((
                tr_c!("readout label", "From").to_owned(),
                price_format.format(made.open),
            ));
            items.push((
                tr_c!("readout label", "To").to_owned(),
                price_format.format(made.close),
            ));
            items.push((
                tr_c!("readout label", "Line").to_owned(),
                if thick { tr!("thick") } else { tr!("thin") }.to_owned(),
            ));
        }
        BarShape::Column { rising, boxes } => {
            items.push((
                tr_c!("readout label", "From").to_owned(),
                price_format.format(made.open),
            ));
            items.push((
                tr_c!("readout label", "To").to_owned(),
                price_format.format(made.close),
            ));
            items.push((if rising { "X" } else { "O" }.to_owned(), boxes.to_string()));
        }
    }
    if let (Some(first), Some(last)) = (
        info.source_candles.get(made.first_source),
        info.source_candles.get(made.last_source),
    ) {
        let (first, last) = (time_text(first.open_time), time_text(last.open_time));
        items.push((tr!("From candle").to_owned(), first[5..].to_owned()));
        if made.last_source != made.first_source {
            items.push((
                tr_c!("readout label", "to").to_owned(),
                last[5..].to_owned(),
            ));
        }
    }
    if !made.confirmed {
        items.push((tr!("Live").to_owned(), String::new()));
    }
    items
}

pub(super) const READOUT_MARGIN: f32 = 8.0;
pub(super) const READOUT_PAD: Vec2 = Vec2::new(8.0, 5.0);
const READOUT_GAP: f32 = 12.0;
pub(super) const READOUT_ROW_GAP: f32 = 2.0;

/// Flows items of the given widths left to right, starting a new row whenever the next one would
/// pass `max_width`. Returns each item's `(row, x offset)` within its row. An item wider than a
/// whole row sits alone on its own row.
pub(super) fn flow_readout(widths: &[f32], gap: f32, max_width: f32) -> Vec<(usize, f32)> {
    let mut placed = Vec::with_capacity(widths.len());
    let (mut row, mut x) = (0, 0.0_f32);
    for &width in widths {
        if x > 0.0 && x + width > max_width {
            row += 1;
            x = 0.0;
        }
        placed.push((row, x));
        x += width + gap;
    }
    placed
}

/// The crosshair's O/H/L/C(/Vol) box in the plot's top-left corner. Each item is a `(label,
/// value)` pair: the label in the theme's ink (white on dark, black on light), the value in
/// `color`. The box is measured from the text, so it hugs it at any price magnitude, and the
/// items wrap onto more rows when the plot is too narrow for one line — it never spills past
/// the plot.
fn paint_ohlc_readout(
    painter: &egui::Painter,
    plot: Rect,
    items: &[(String, String)],
    color: Color32,
) -> f32 {
    let font = FontId::monospace(10.0);
    let label_color = theme::pick(Color32::WHITE, Color32::BLACK);
    let galleys: Vec<_> = items
        .iter()
        .map(|(label, value)| {
            let mut job = egui::text::LayoutJob::default();
            job.append(
                &format!("{label} "),
                0.0,
                egui::TextFormat::simple(font.clone(), label_color),
            );
            job.append(value, 0.0, egui::TextFormat::simple(font.clone(), color));
            painter.layout_job(job)
        })
        .collect();
    let widths: Vec<f32> = galleys.iter().map(|galley| galley.size().x).collect();
    let max_width = (plot.width() - 2.0 * (READOUT_MARGIN + READOUT_PAD.x)).max(0.0);
    let placed = flow_readout(&widths, READOUT_GAP, max_width);

    let rows = placed.last().map_or(0, |(row, _)| row + 1);
    let row_height = galleys.iter().map(|g| g.size().y).fold(0.0, f32::max);
    let content_width = placed
        .iter()
        .zip(&widths)
        .map(|((_, x), width)| x + width)
        .fold(0.0, f32::max);
    let content = Vec2::new(
        content_width,
        rows as f32 * row_height + rows.saturating_sub(1) as f32 * READOUT_ROW_GAP,
    );

    let origin = plot.min + Vec2::splat(READOUT_MARGIN) - Vec2::new(0.0, 1.0);
    let boxed = Rect::from_min_size(origin, content + READOUT_PAD * 2.0);
    painter.rect(
        boxed,
        4.0,
        theme::chrome().gamma_multiply(0.94),
        Stroke::new(1.0, theme::border()),
        StrokeKind::Inside,
    );
    for (galley, (row, x)) in galleys.into_iter().zip(placed) {
        let at = origin + READOUT_PAD + Vec2::new(x, row as f32 * (row_height + READOUT_ROW_GAP));
        painter.galley(at, galley, color);
    }
    boxed.bottom()
}
