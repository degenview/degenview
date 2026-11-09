//! Paints the Volume footprint chart type: each bar's candle with the volume traded at every price
//! row beside it, sell on the left and buy on the right, with the point of control, value area,
//! imbalances and the bar's summary.
//!
//! The numbers are built elsewhere (`model::market_profile::footprint`) and handed in as a finished
//! [`FootprintSet`]; this only turns them into shapes. How much is drawn depends on how much room a
//! cell has ([`Detail`]): numbers where they fit, intensity-only cells where they do not, and a plain
//! candle where even cells would be a smear. The financial meaning never changes with the zoom,
//! only the amount of writing.
//!
//! A bar the set has no footprint for (older than the set reaches, no volume, a row size that needs
//! too many rows) is a plain candle, and the chart says why in its status line. Nothing here
//! invents a number.

use super::{SeriesFrame, color, paint_candle};
use crate::model::chart_look;
use crate::model::chart_type::{
    FootprintDisplay, FootprintKind, FootprintSettings, FootprintSummary,
};
use crate::model::market_profile::footprint::{Cell, FootprintBar, FootprintSet, SetStatus, Side};
use crate::model::market_profile::quality::DataQuality;
use crate::ui::theme;
use eframe::egui::{Align2, Color32, FontId, Pos2, Rect, Stroke, StrokeKind};

/// How much of a bar is drawn, by the room a cell has.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Detail {
    /// Cells with their numbers.
    Numbers,
    /// Cells shaded by volume, no writing.
    Cells,
    /// Too small for cells: the candle alone.
    Candle,
}

/// Room a cell's text needs, in points.
const NUMBER_ROW_HEIGHT: f32 = 11.0;
const NUMBER_CELL_WIDTH: f32 = 26.0;
/// The least a cell is drawn at.
const MIN_ROW_HEIGHT: f32 = 2.0;
const MIN_CELL_WIDTH: f32 = 3.0;

/// The detail for cells `row_height` tall and `cell_width` wide (points).
pub fn detail(row_height: f32, cell_width: f32) -> Detail {
    if row_height >= NUMBER_ROW_HEIGHT && cell_width >= NUMBER_CELL_WIDTH {
        Detail::Numbers
    } else if row_height >= MIN_ROW_HEIGHT && cell_width >= MIN_CELL_WIDTH {
        Detail::Cells
    } else {
        Detail::Candle
    }
}

/// Where the parts of one bar sit, from its slot.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BarLayout {
    pub centre: f32,
    /// Half the width of the thin candle in the middle.
    pub candle_half: f32,
    /// Where the sell cells' outer edge is.
    pub left: f32,
    /// Where the buy cells' outer edge is.
    pub right: f32,
}

impl BarLayout {
    /// A bar centred on `x` in a slot `slot_width` wide: the cells use nine tenths of the slot.
    pub fn new(x: f32, slot_width: f32) -> Self {
        let half = slot_width * 0.45;
        Self {
            centre: x,
            candle_half: (slot_width * 0.03).clamp(1.0, 3.0),
            left: x - half,
            right: x + half,
        }
    }

    /// The width of one side's cells.
    pub fn cell_width(&self) -> f32 {
        ((self.right - self.left) / 2.0 - self.candle_half - 1.0).max(0.0)
    }
}

/// The volume steps of TradingView's gradient: below a quarter, a half, three quarters, and above.
pub fn gradient_step(ratio: f64) -> usize {
    if ratio < 0.25 {
        0
    } else if ratio < 0.5 {
        1
    } else if ratio < 0.75 {
        2
    } else {
        3
    }
}

const STEP_ALPHA: [f32; 4] = [0.20, 0.38, 0.58, 0.82];

fn tint(base: [u8; 4], alpha: f32) -> Color32 {
    let [r, g, b, a] = base;
    Color32::from_rgba_unmultiplied(
        r,
        g,
        b,
        (f32::from(a) * alpha).round().clamp(0.0, 255.0) as u8,
    )
}

/// The text of a volume: whole numbers where large, a few decimals where small (a bitcoin cell is
/// often a fraction), thousands and millions abbreviated.
pub fn format_volume(volume: f64) -> String {
    let magnitude = volume.abs();
    let (scaled, suffix) = if magnitude >= 1e9 {
        (volume / 1e9, "B")
    } else if magnitude >= 1e6 {
        (volume / 1e6, "M")
    } else if magnitude >= 1e4 {
        (volume / 1e3, "K")
    } else {
        (volume, "")
    };
    let body = if !suffix.is_empty() {
        format!("{scaled:.1}")
    } else if magnitude >= 100.0 {
        format!("{scaled:.0}")
    } else if magnitude >= 10.0 {
        format!("{scaled:.1}")
    } else if magnitude >= 1.0 {
        format!("{scaled:.2}")
    } else if magnitude >= 0.001 {
        format!("{scaled:.3}")
    } else if magnitude == 0.0 {
        return "0".to_owned();
    } else {
        format!("{scaled:.1e}")
    };
    let body = if body.contains('.') && !body.contains('e') {
        body.trim_end_matches('0').trim_end_matches('.').to_owned()
    } else {
        body
    };
    format!("{body}{suffix}")
}

/// A signed volume (delta): `+` for a positive one.
pub fn format_signed(volume: f64) -> String {
    if volume > 0.0 {
        format!("+{}", format_volume(volume))
    } else {
        format_volume(volume)
    }
}

/// The defaults of the look, from the chart's palette.
struct Look {
    buy: [u8; 4],
    sell: [u8; 4],
    buy_imbalance: [u8; 4],
    sell_imbalance: [u8; 4],
    poc: Color32,
    ink: Color32,
}

fn look(frame: &SeriesFrame, settings: &FootprintSettings) -> Look {
    Look {
        buy: settings.buy_color.unwrap_or(frame.palette.up),
        sell: settings.sell_color.unwrap_or(frame.palette.down),
        buy_imbalance: settings.buy_imbalance_color.unwrap_or([0, 230, 118, 255]),
        sell_imbalance: settings.sell_imbalance_color.unwrap_or([255, 23, 68, 255]),
        poc: Color32::from_rgb(240, 185, 11),
        ink: theme::text_primary(),
    }
}

pub fn paint(frame: &SeriesFrame) {
    let settings = frame.style.footprint;
    let feed = frame.profiles;
    let set = feed.and_then(|feed| feed.footprint.as_deref());
    let look = look(frame, &settings);
    // One scale for every bar in view, so a colour means the same volume everywhere on screen.
    let peak = set.map_or(0.0, |set| visible_peak(frame, set));
    for (index, bar) in frame.bars.iter().enumerate() {
        let footprint = set.and_then(|set| set.bar_at(bar.open_time));
        match footprint {
            Some(footprint) => paint_bar(
                frame,
                &settings,
                &look,
                set.unwrap(),
                index,
                footprint,
                peak,
            ),
            None => plain_candle(frame, index),
        }
    }
    if let Some(set) = set {
        paint_projections(frame, &settings, &look, set);
    }
    paint_status(frame, set);
}

/// The busiest single side of any cell of the bars in view.
fn visible_peak(frame: &SeriesFrame, set: &FootprintSet) -> f64 {
    frame
        .bars
        .iter()
        .filter_map(|bar| set.bar_at(bar.open_time))
        .flat_map(|footprint| footprint.cells.iter())
        .map(|cell| cell.buy.max(cell.sell))
        .fold(0.0, f64::max)
}

pub(super) fn plain_candle(frame: &SeriesFrame, index: usize) {
    let bar = &frame.bars[index];
    let settings = frame.style.candle_settings().copied().unwrap_or_default();
    let look = chart_look::candle_look(
        &settings,
        frame.palette,
        bar,
        frame.previous_close_of(index),
        frame.recolor_of(index),
    );
    let (high, low) = chart_look::drawn_extremes(bar.open, bar.high, bar.low, bar.close);
    paint_candle(
        frame,
        index,
        (frame.x_for)(index),
        super::candle_half_width(frame.slot_width),
        [bar.open, high, low, bar.close],
        &look,
    );
}

fn paint_bar(
    frame: &SeriesFrame,
    settings: &FootprintSettings,
    look: &Look,
    set: &FootprintSet,
    index: usize,
    footprint: &FootprintBar,
    peak: f64,
) {
    let Some(grid) = set.grid else {
        return plain_candle(frame, index);
    };
    let layout = BarLayout::new((frame.x_for)(index), frame.slot_width);
    let row_height = (frame.y(grid.bottom(footprint.cells[0].row))
        - frame.y(grid.top(footprint.cells[0].row)))
    .abs();
    let level = detail(row_height, layout.cell_width());
    if level == Detail::Candle {
        return plain_candle(frame, index);
    }
    let painter = frame.painter;
    let clip = frame.plot;
    // Value area behind the cells.
    if settings.show_value_area {
        let (low_row, high_row) = footprint.value_area_rows();
        let top = frame.y(grid.top(high_row));
        let bottom = frame.y(grid.bottom(low_row));
        if bottom >= clip.top() && top <= clip.bottom() {
            let area =
                Rect::from_min_max(Pos2::new(layout.left, top), Pos2::new(layout.right, bottom));
            painter.rect_filled(area, 0.0, tint([128, 128, 128, 255], 0.10));
            let line = Stroke::new(1.0, tint([160, 160, 160, 255], 0.55));
            painter.hline(layout.left..=layout.right, top, line);
            painter.hline(layout.left..=layout.right, bottom, line);
        }
    }
    let font = FontId::monospace((row_height - 2.0).clamp(7.0, 11.0));
    for cell in &footprint.cells {
        let (top, bottom) = (frame.y(grid.top(cell.row)), frame.y(grid.bottom(cell.row)));
        if bottom < clip.top() || top > clip.bottom() {
            continue;
        }
        paint_cell(
            frame,
            settings,
            look,
            &layout,
            cell,
            (top, bottom),
            peak,
            footprint.peak(),
            level,
            &font,
        );
    }
    if settings.show_poc {
        let row = footprint.poc_row();
        let (top, bottom) = (frame.y(grid.top(row)), frame.y(grid.bottom(row)));
        painter.rect_stroke(
            Rect::from_min_max(Pos2::new(layout.left, top), Pos2::new(layout.right, bottom)),
            0.0,
            Stroke::new(1.5, look.poc),
            StrokeKind::Inside,
        );
    }
    // The candle between the two halves.
    let candle_settings = frame.style.candle_settings().copied().unwrap_or_default();
    let bar = &frame.bars[index];
    let candle_look = chart_look::candle_look(
        &candle_settings,
        frame.palette,
        bar,
        frame.previous_close_of(index),
        frame.recolor_of(index),
    );
    let (high, low) = chart_look::drawn_extremes(bar.open, bar.high, bar.low, bar.close);
    paint_candle(
        frame,
        index,
        layout.centre,
        layout.candle_half,
        [bar.open, high, low, bar.close],
        &candle_look,
    );
    if level == Detail::Numbers && settings.summary == FootprintSummary::InfoBox {
        paint_summary(
            frame,
            footprint,
            &layout,
            grid.bottom(footprint.cells[0].row),
            look,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn paint_cell(
    frame: &SeriesFrame,
    settings: &FootprintSettings,
    look: &Look,
    layout: &BarLayout,
    cell: &Cell,
    (top, bottom): (f32, f32),
    peak: f64,
    bar_peak: f64,
    level: Detail,
    font: &FontId,
) {
    let painter = frame.painter;
    let width = layout.cell_width();
    let inner_left = layout.centre - layout.candle_half - 1.0;
    let inner_right = layout.centre + layout.candle_half + 1.0;
    // Profile display: a cell is as wide as its share of the bar's busiest row.
    let scaled = |volume: f64| -> f32 {
        match settings.display {
            FootprintDisplay::Cluster => width,
            FootprintDisplay::Profile if bar_peak > 0.0 => {
                (width * (volume / bar_peak) as f32).max(1.0)
            }
            FootprintDisplay::Profile => 0.0,
        }
    };
    let shade = |base: [u8; 4], volume: f64| -> Color32 {
        let ratio = if peak > 0.0 { volume / peak } else { 0.0 };
        let alpha = if settings.gradient {
            STEP_ALPHA[gradient_step(ratio)]
        } else {
            0.45
        };
        tint(base, if volume > 0.0 { alpha } else { 0.0 })
    };
    let text = |center: Pos2, label: String, strong: bool| {
        if level != Detail::Numbers {
            return;
        }
        painter.text(
            center,
            Align2::CENTER_CENTER,
            label,
            font.clone(),
            if strong {
                look.ink
            } else {
                look.ink.gamma_multiply(0.85)
            },
        );
    };
    let y = (top + bottom) / 2.0;
    let rect = |from: f32, to: f32| {
        Rect::from_min_max(
            Pos2::new(from.min(to), top),
            Pos2::new(from.max(to), bottom),
        )
    };
    match settings.kind {
        FootprintKind::BuyAndSell | FootprintKind::Ladder => {
            let sell_width = scaled(cell.sell);
            let buy_width = scaled(cell.buy);
            let sell_rect = rect(inner_left - sell_width, inner_left);
            let buy_rect = rect(inner_right, inner_right + buy_width);
            painter.rect_filled(sell_rect, 0.0, shade(look.sell, cell.sell));
            painter.rect_filled(buy_rect, 0.0, shade(look.buy, cell.buy));
            if settings.kind == FootprintKind::Ladder && cell.total() > 0.0 {
                let busiest = if cell.buy >= cell.sell {
                    buy_rect
                } else {
                    sell_rect
                };
                painter.rect_stroke(busiest, 0.0, Stroke::new(1.0, look.ink), StrokeKind::Inside);
            }
            text(
                Pos2::new(inner_left - width / 2.0, y),
                format_volume(cell.sell),
                cell.sell_imbalance && settings.highlight_imbalances,
            );
            text(
                Pos2::new(inner_right + width / 2.0, y),
                format_volume(cell.buy),
                cell.buy_imbalance && settings.highlight_imbalances,
            );
            if settings.highlight_imbalances {
                let edge = |x: f32, base: [u8; 4]| {
                    painter.rect_filled(rect(x - 1.0, x + 1.0), 0.0, color(base));
                };
                if cell.buy_imbalance {
                    edge(layout.right, look.buy_imbalance);
                }
                if cell.sell_imbalance {
                    edge(layout.left, look.sell_imbalance);
                }
            }
        }
        FootprintKind::Delta => {
            let delta = cell.delta();
            let base = if delta >= 0.0 { look.buy } else { look.sell };
            let reach = scaled(delta.abs());
            painter.rect_filled(
                rect(layout.centre - reach, layout.centre + reach),
                0.0,
                shade(base, delta.abs()),
            );
            text(Pos2::new(layout.centre, y), format_signed(delta), false);
        }
        FootprintKind::Total => {
            let reach = scaled(cell.total());
            painter.rect_filled(
                rect(layout.centre - reach, layout.centre + reach),
                0.0,
                shade([150, 150, 150, 255], cell.total()),
            );
            text(
                Pos2::new(layout.centre, y),
                format_volume(cell.total()),
                false,
            );
        }
    }
}

/// Two lines under a bar's lowest row: its delta and its total volume.
fn paint_summary(
    frame: &SeriesFrame,
    footprint: &FootprintBar,
    layout: &BarLayout,
    bottom_price: f64,
    look: &Look,
) {
    let y = frame.y(bottom_price) + 4.0;
    if y > frame.plot.bottom() - 24.0 {
        return;
    }
    let font = FontId::monospace(9.0);
    let delta_color = if footprint.delta() >= 0.0 {
        color(look.buy)
    } else {
        color(look.sell)
    };
    frame.painter.text(
        Pos2::new(layout.centre, y),
        Align2::CENTER_TOP,
        format!("Δ {}", format_signed(footprint.delta())),
        font.clone(),
        delta_color,
    );
    frame.painter.text(
        Pos2::new(layout.centre, y + 11.0),
        Align2::CENTER_TOP,
        format_volume(footprint.total()),
        font,
        look.ink.gamma_multiply(0.8),
    );
}

/// Stacked imbalances carried right until a later bar touches them.
fn paint_projections(
    frame: &SeriesFrame,
    settings: &FootprintSettings,
    look: &Look,
    set: &FootprintSet,
) {
    if !settings.highlight_stacked || !settings.highlight_imbalances {
        return;
    }
    // `set.projections` index the set's bars, which can reach further back than the view starts (and
    // start later than it does): each end is found in the view by its bar's open time.
    let plot = frame.plot;
    let slot_of = |time_ms: i64| -> Slot {
        let at = frame.bars.partition_point(|bar| bar.open_time < time_ms);
        if frame.bars.is_empty() || at == 0 && frame.bars[0].open_time != time_ms {
            Slot::Before
        } else if at >= frame.bars.len() {
            Slot::After
        } else {
            Slot::At(at)
        }
    };
    for projection in &set.projections {
        let Some(from_time) = set.open_times.get(projection.from).copied() else {
            continue;
        };
        let start_x = match slot_of(from_time) {
            Slot::Before => plot.left(),
            Slot::At(index) => (frame.x_for)(index) + frame.slot_width * 0.45,
            Slot::After => continue,
        };
        let end_x = match projection
            .until
            .and_then(|until| set.open_times.get(until).copied())
            .map(slot_of)
        {
            // Touched before the view starts: nothing of it is on screen.
            Some(Slot::Before) => continue,
            Some(Slot::At(index)) => (frame.x_for)(index),
            Some(Slot::After) | None => plot.right(),
        };
        let (top, bottom) = (frame.y(projection.high), frame.y(projection.low));
        if end_x <= start_x || bottom < plot.top() || top > plot.bottom() {
            continue;
        }
        let base = match projection.side {
            Side::Buy => look.buy_imbalance,
            Side::Sell => look.sell_imbalance,
        };
        let band = Rect::from_min_max(Pos2::new(start_x, top), Pos2::new(end_x, bottom));
        frame.painter.rect_filled(band, 0.0, tint(base, 0.10));
        let line = Stroke::new(1.0, tint(base, 0.6));
        frame.painter.hline(start_x..=end_x, top, line);
        frame.painter.hline(start_x..=end_x, bottom, line);
    }
}

/// The tooltip for the cell under `pointer`: its price range, buy and sell volume, delta, imbalance
/// and where it sits in the bar's value area, then the bar's totals and where its numbers came from.
/// `None` when the pointer is not over a drawn cell (the bar is only a candle at this zoom, or has no
/// footprint). Uses the geometry the painter used, so what is hovered is what is drawn.
pub fn hover(
    frame: &SeriesFrame,
    pointer: Pos2,
) -> Option<crate::ui::pine_chart_render::HoverTarget> {
    if !frame.plot.contains(pointer) || frame.bars.is_empty() {
        return None;
    }
    let set = frame.profiles?.footprint.as_deref()?;
    let grid = set.grid?;
    let first_x = (frame.x_for)(0);
    let index = (((pointer.x - first_x) / frame.slot_width).round().max(0.0) as usize)
        .min(frame.bars.len() - 1);
    let layout = BarLayout::new((frame.x_for)(index), frame.slot_width);
    if pointer.x < layout.left || pointer.x > layout.right {
        return None;
    }
    let footprint = set.bar_at(frame.bars[index].open_time)?;
    let row = grid.row_of(frame.scale.price(pointer.y))?;
    let cell = footprint.cell_at(row)?;
    let (top, bottom) = (frame.y(grid.top(row)), frame.y(grid.bottom(row)));
    if detail((bottom - top).abs(), layout.cell_width()) == Detail::Candle {
        return None;
    }
    let format = |price: f64| frame.price_format.format(price);
    let mut lines = vec![
        format!("{} – {}", format(grid.bottom(row)), format(grid.top(row))),
        format!(
            "Buy {}   Sell {}",
            format_volume(cell.buy),
            format_volume(cell.sell)
        ),
        format!(
            "Total {}   Delta {}",
            format_volume(cell.total()),
            format_signed(cell.delta())
        ),
    ];
    if cell.buy_imbalance {
        lines.push("Buy imbalance".to_owned());
    }
    if cell.sell_imbalance {
        lines.push("Sell imbalance".to_owned());
    }
    let (low_row, high_row) = footprint.value_area_rows();
    if row == footprint.poc_row() {
        lines.push("Point of control".to_owned());
    } else if (low_row..=high_row).contains(&row) {
        lines.push("In the value area".to_owned());
    }
    lines.push(format!(
        "Bar: volume {}, delta {} ({:+.1}%)",
        format_volume(footprint.total()),
        format_signed(footprint.delta()),
        footprint.delta_percent()
    ));
    lines.push(footprint.quality.label().to_owned());
    Some(crate::ui::pine_chart_render::HoverTarget {
        rect: Rect::from_min_max(
            Pos2::new(layout.left, top.min(bottom)),
            Pos2::new(layout.right, top.max(bottom)),
        ),
        text: lines.join("\n"),
    })
}

/// Where an instant falls in the bars in view.
enum Slot {
    Before,
    At(usize),
    After,
}

/// Where the data came from, or why there is none: bottom left, small.
fn paint_status(frame: &SeriesFrame, set: Option<&FootprintSet>) {
    let message = status_text(frame, set);
    let Some(message) = message else { return };
    frame.painter.text(
        Pos2::new(frame.plot.left() + 8.0, frame.plot.bottom() - 8.0),
        Align2::LEFT_BOTTOM,
        message,
        FontId::proportional(10.5),
        theme::text_secondary(),
    );
}

pub fn status_text(frame: &SeriesFrame, set: Option<&FootprintSet>) -> Option<String> {
    if let Some(SetStatus::Unsupported(reason)) = set.map(|set| set.status) {
        return Some(format!(
            "Volume footprint unavailable: {}",
            reason.explanation()
        ));
    }
    let feed = frame.profiles;
    let Some(set) = set else {
        return Some(match feed {
            Some(feed) if feed.error.is_some() => {
                format!("Volume footprint: {}", feed.error.as_deref().unwrap_or(""))
            }
            _ => "Loading order-flow data…".to_owned(),
        });
    };
    let counts = set.counts;
    if counts.refused > 0 {
        let reason = set.first_refusal.map_or("", |reason| reason.explanation());
        return Some(format!("{} bars not shown. {reason}", counts.refused));
    }
    let mut parts = Vec::new();
    for (count, quality) in [
        (counts.trades, DataQuality::TradeDerived),
        (counts.split, DataQuality::ExchangeSplit),
        (counts.intrabar, DataQuality::Intrabar),
        (counts.chart_bars, DataQuality::ChartBars),
    ] {
        if count > 0 {
            parts.push(format!("{} ({count})", quality.label()));
        }
    }
    if parts.is_empty() {
        return None;
    }
    Some(parts.join(" · "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detail_follows_the_room_a_cell_has() {
        assert_eq!(detail(14.0, 40.0), Detail::Numbers);
        assert_eq!(detail(10.0, 40.0), Detail::Cells, "too short for text");
        assert_eq!(detail(14.0, 20.0), Detail::Cells, "too narrow for text");
        assert_eq!(detail(1.5, 40.0), Detail::Candle);
        assert_eq!(detail(14.0, 2.0), Detail::Candle);
    }

    #[test]
    fn the_gradient_steps_are_tradingviews_quarters() {
        assert_eq!(gradient_step(0.0), 0);
        assert_eq!(gradient_step(0.249), 0);
        assert_eq!(gradient_step(0.25), 1);
        assert_eq!(gradient_step(0.5), 2);
        assert_eq!(gradient_step(0.75), 3);
        assert_eq!(gradient_step(2.0), 3);
    }

    #[test]
    fn volumes_read_compactly_without_losing_small_ones() {
        assert_eq!(format_volume(0.0), "0");
        assert_eq!(format_volume(0.0345), "0.035");
        assert_eq!(format_volume(1.5), "1.5");
        assert_eq!(format_volume(12.34), "12.3");
        assert_eq!(format_volume(125.0), "125");
        assert_eq!(format_volume(12_500.0), "12.5K");
        assert_eq!(format_volume(3_400_000.0), "3.4M");
        assert_eq!(format_volume(2_000_000_000.0), "2B");
        assert_eq!(format_signed(475.0), "+475");
        assert_eq!(format_signed(-475.0), "-475");
        assert_eq!(format_signed(0.0), "0");
        assert!(format_volume(0.000_000_5).contains('e'));
    }

    #[test]
    fn a_bar_layout_keeps_its_cells_inside_its_slot() {
        let layout = BarLayout::new(100.0, 60.0);
        assert!(layout.left >= 70.0 && layout.right <= 130.0);
        assert!(layout.cell_width() > 0.0);
        assert!(layout.cell_width() * 2.0 + layout.candle_half * 2.0 <= layout.right - layout.left);
        // A tiny slot has no room for cells at all.
        assert_eq!(
            detail(14.0, BarLayout::new(10.0, 4.0).cell_width()),
            Detail::Candle
        );
    }

    // MARK: Drawing

    use crate::model::chart_look::Palette;
    use crate::model::chart_type::{ChartStyle, ChartType};
    use crate::model::market_profile::feed::ProfileFeed;
    use crate::model::{ChartAppearance, Kline, PriceFormat};
    use crate::ui::chart_card::PriceScale;
    use crate::ui::chart_style_preview::{sample_candles, sample_footprint};
    use eframe::egui;

    fn bars() -> Vec<Kline> {
        let all = sample_candles();
        all[all.len() - 6..].to_vec()
    }

    /// Paints the footprint over `slot` points per bar and `height` points of plot; returns the
    /// number of shapes and the strings written.
    fn paint_with(
        feed: Option<&ProfileFeed>,
        style: &ChartStyle,
        slot: f32,
        height: f32,
    ) -> (usize, Vec<String>) {
        let bars = bars();
        let ctx = egui::Context::default();
        let plot = Rect::from_min_max(Pos2::ZERO, Pos2::new(slot * 6.0, height));
        let scale = PriceScale::linear(95.0, 125.0, 0.0, height);
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            let x_for = |index: usize| slot * (index as f32 + 0.5);
            paint(&SeriesFrame {
                painter: ui.painter(),
                plot,
                bars: &bars,
                synthetic: None,
                box_size: None,
                previous_close: None,
                slot_width: slot,
                x_for: &x_for,
                scale: &scale,
                palette: Palette::new([0, 200, 0], [200, 0, 0]),
                style,
                chart_type: ChartType::VolumeFootprint,
                recolor: None,
                price_format: PriceFormat::AUTO,
                profiles: feed,
            });
        });
        output.textures_delta.clear();
        let shapes: Vec<_> = std::mem::take(&mut output.shapes);
        let mut words = Vec::new();
        for clipped in &shapes {
            if let egui::epaint::Shape::Text(text) = &clipped.shape {
                words.push(text.galley.text().to_owned());
            }
        }
        (shapes.len(), words)
    }

    fn style() -> ChartStyle {
        ChartStyle {
            chart_type: ChartType::VolumeFootprint,
            ..ChartStyle::default()
        }
    }

    fn feed() -> ProfileFeed {
        sample_footprint(&ChartAppearance::default())
    }

    #[test]
    fn a_footprint_with_room_writes_its_numbers_and_one_without_draws_shaded_cells() {
        let feed = feed();
        let (wide, words) = paint_with(Some(&feed), &style(), 120.0, 600.0);
        assert!(
            words.iter().any(|w| w.starts_with('Δ')),
            "the summary is written: {words:?}"
        );
        assert!(
            words
                .iter()
                .filter(|w| w.parse::<f64>().is_ok() || w.ends_with('K'))
                .count()
                > 10,
            "cell volumes are written"
        );
        let (narrow, narrow_words) = paint_with(Some(&feed), &style(), 14.0, 600.0);
        assert!(
            narrow_words.iter().all(|w| !w.starts_with('Δ')),
            "no room for writing"
        );
        assert!(narrow > 0 && wide > narrow);
    }

    #[test]
    fn a_footprint_chart_with_no_feed_draws_candles_and_says_it_is_loading() {
        let (shapes, words) = paint_with(None, &style(), 60.0, 300.0);
        assert!(shapes > 0);
        assert!(words.iter().any(|w| w.contains("Loading")), "{words:?}");
        let loading = ProfileFeed::loading();
        let (_, words) = paint_with(Some(&loading), &style(), 60.0, 300.0);
        assert!(words.iter().any(|w| w.contains("Loading")));
    }

    #[test]
    fn a_failed_load_says_why() {
        let feed = ProfileFeed {
            error: Some("Network error: offline".into()),
            ..ProfileFeed::default()
        };
        let (_, words) = paint_with(Some(&feed), &style(), 60.0, 300.0);
        assert!(words.iter().any(|w| w.contains("offline")), "{words:?}");
    }

    #[test]
    fn the_status_line_names_where_the_numbers_came_from() {
        let feed = feed();
        let (_, words) = paint_with(Some(&feed), &style(), 60.0, 300.0);
        assert!(words.iter().any(|w| w.contains("Trade data")), "{words:?}");
    }

    #[test]
    fn an_unsupported_set_draws_candles_and_the_reason() {
        use crate::model::market_profile::footprint::{FootprintSet, SetStatus};
        use crate::model::market_profile::quality::Unsupported;
        let feed = ProfileFeed {
            footprint: Some(std::sync::Arc::new(FootprintSet::empty(
                SetStatus::Unsupported(Unsupported::BarTooShort),
            ))),
            ..ProfileFeed::default()
        };
        let (shapes, words) = paint_with(Some(&feed), &style(), 60.0, 300.0);
        assert!(shapes > 0);
        assert!(words.iter().any(|w| w.contains("too short")), "{words:?}");
    }

    #[test]
    fn every_display_and_kind_paints_without_panicking() {
        let feed = feed();
        for display in [FootprintDisplay::Cluster, FootprintDisplay::Profile] {
            for kind in [
                FootprintKind::BuyAndSell,
                FootprintKind::Delta,
                FootprintKind::Total,
                FootprintKind::Ladder,
            ] {
                for gradient in [true, false] {
                    let mut style = style();
                    style.footprint.display = display;
                    style.footprint.kind = kind;
                    style.footprint.gradient = gradient;
                    let (shapes, _) = paint_with(Some(&feed), &style, 90.0, 500.0);
                    assert!(shapes > 0, "{display:?} {kind:?}");
                }
            }
        }
    }

    #[test]
    fn the_hover_describes_the_cell_under_the_pointer_and_nothing_elsewhere() {
        let feed = feed();
        let bars = bars();
        let slot = 120.0;
        let plot = Rect::from_min_max(Pos2::ZERO, Pos2::new(slot * 6.0, 600.0));
        let scale = PriceScale::linear(95.0, 125.0, 0.0, 600.0);
        let x_for = |index: usize| slot * (index as f32 + 0.5);
        let style = style();
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            let frame = SeriesFrame {
                painter: ui.painter(),
                plot,
                bars: &bars,
                synthetic: None,
                box_size: None,
                previous_close: None,
                slot_width: slot,
                x_for: &x_for,
                scale: &scale,
                palette: Palette::new([0, 200, 0], [200, 0, 0]),
                style: &style,
                chart_type: ChartType::VolumeFootprint,
                recolor: None,
                price_format: PriceFormat::AUTO,
                profiles: Some(&feed),
            };
            // Over the busiest bar's close, on its buy side.
            let bar = &bars[3];
            let pointer = Pos2::new(x_for(3) + 20.0, scale.y(bar.close));
            let target = hover(&frame, pointer).expect("a cell is under the pointer");
            assert!(target.text.contains("Buy "), "{}", target.text);
            assert!(target.text.contains("Delta"), "{}", target.text);
            assert!(target.text.contains("Trade data"), "{}", target.text);
            assert!(target.rect.contains(pointer));
            // In the gap between bars, or outside the plot: nothing.
            assert!(
                hover(
                    &frame,
                    Pos2::new(x_for(3) + slot * 0.49, scale.y(bar.close))
                )
                .is_none()
            );
            assert!(hover(&frame, Pos2::new(-5.0, 10.0)).is_none());
            // No feed, no tooltip.
            let bare = SeriesFrame {
                profiles: None,
                ..frame
            };
            assert!(hover(&bare, pointer).is_none());
        });
        output.textures_delta.clear();
    }

    /// Pictures of the footprint at the zooms and settings worth a look.
    /// `DEGENVIEW_GALLERY_DIR=/tmp/g cargo test --lib gallery_of_the_footprint -- --ignored`.
    #[test]
    #[ignore = "writes images for a person to look at"]
    fn gallery_of_the_footprint() {
        use crate::ui::render_tests::{Screen, gallery_dir};
        if gallery_dir().is_none() {
            return;
        }
        let feed = feed();
        let bars = bars();
        type Tweak = fn(&mut ChartStyle);
        let cases: [(&str, f32, f32, Tweak); 7] = [
            ("footprint-wide", 120.0, 520.0, |_| {}),
            ("footprint-medium", 56.0, 520.0, |_| {}),
            ("footprint-narrow", 22.0, 520.0, |_| {}),
            ("footprint-short", 120.0, 120.0, |_| {}),
            ("footprint-delta", 120.0, 520.0, |s| {
                s.footprint.kind = FootprintKind::Delta;
            }),
            ("footprint-profile-ladder", 120.0, 520.0, |s| {
                s.footprint.display = FootprintDisplay::Profile;
                s.footprint.kind = FootprintKind::Ladder;
            }),
            ("footprint-plain", 120.0, 520.0, |s| {
                s.footprint.gradient = false;
                s.footprint.show_value_area = false;
                s.footprint.show_poc = false;
            }),
        ];
        for (name, slot, height, tweak) in cases {
            let mut style = style();
            tweak(&mut style);
            let mut screen = Screen::new(egui::vec2(slot * 6.0, height));
            let mut last = None;
            for _ in 0..2 {
                last = Some(screen.run(Vec::new(), |ui| {
                    let plot = Rect::from_min_size(Pos2::ZERO, egui::vec2(slot * 6.0, height));
                    ui.painter().rect_filled(plot, 0.0, theme::chrome());
                    let scale = PriceScale::linear(100.0, 124.0, 0.0, height);
                    let x_for = |index: usize| slot * (index as f32 + 0.5);
                    paint(&SeriesFrame {
                        painter: ui.painter(),
                        plot,
                        bars: &bars,
                        synthetic: None,
                        box_size: None,
                        previous_close: None,
                        slot_width: slot,
                        x_for: &x_for,
                        scale: &scale,
                        palette: Palette::new([38, 166, 154], [239, 83, 80]),
                        style: &style,
                        chart_type: ChartType::VolumeFootprint,
                        recolor: None,
                        price_format: PriceFormat::AUTO,
                        profiles: Some(&feed),
                    });
                }));
            }
            screen.save(name, last.expect("a frame"));
        }
    }

    #[test]
    fn stacked_imbalance_projections_are_drawn_when_the_view_starts_after_the_bar_that_owns_them() {
        use crate::model::market_profile::footprint::{Cache, SetInput, build_set};
        use crate::model::market_profile::trades::{Trade, TradeBook};
        // Bar 0 builds a stack of buy imbalances at 101..104; later bars stay clear of it.
        let hour = 3_600_000;
        let mk = |i: i64, low: f64, high: f64| Kline {
            open_time: i * hour,
            close_time: (i + 1) * hour - 1,
            open: low + 1.0,
            high,
            low,
            close: high - 1.0,
            volume: 10.0,
        };
        let chart = vec![
            mk(0, 100.0, 104.0),
            mk(1, 108.0, 112.0),
            mk(2, 108.0, 112.0),
            mk(3, 108.0, 112.0),
        ];
        let mut book = TradeBook::default();
        let t = |id: u64, time_ms: i64, price: f64, qty: f64, buy: bool| Trade {
            id,
            time_ms,
            price,
            quantity: qty,
            buyer_is_maker: !buy,
        };
        book.insert(&[
            t(1, -1, 100.0, 0.01, false),
            t(2, 5, 100.0, 4.0, false),
            t(3, 5, 100.0, 4.0, true),
            t(4, 5, 101.0, 12.0, true),
            t(5, 5, 101.0, 4.0, false),
            t(6, 5, 102.0, 12.0, true),
            t(7, 5, 102.0, 4.0, false),
            t(8, 5, 103.0, 12.0, true),
            t(9, hour + 5, 110.0, 1.0, true),
            t(10, 2 * hour + 5, 110.0, 1.0, true),
            t(11, 3 * hour + 5, 110.0, 1.0, true),
        ]);
        let set = build_set(
            SetInput {
                bars: &chart,
                bar_ms: hour,
                candles: None,
                trades: Some(&mut book),
                exchange_tick: Some(1.0),
                shape: ChartStyle::default().footprint.shape(),
            },
            &mut Cache::default(),
        );
        assert!(
            !set.projections.is_empty(),
            "the fixture makes a stack: {:?}",
            set.counts
        );
        let feed = ProfileFeed {
            footprint: Some(std::sync::Arc::new(set)),
            ..ProfileFeed::default()
        };
        let count = |highlight: bool, from: usize| {
            let mut style = style();
            style.footprint.highlight_stacked = highlight;
            let ctx = egui::Context::default();
            let shown = &chart[from..];
            let plot = Rect::from_min_max(Pos2::ZERO, Pos2::new(600.0, 600.0));
            let scale = PriceScale::linear(95.0, 115.0, 0.0, 600.0);
            let slot = 600.0 / shown.len() as f32;
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                let x_for = |index: usize| slot * (index as f32 + 0.5);
                paint(&SeriesFrame {
                    painter: ui.painter(),
                    plot,
                    bars: shown,
                    synthetic: None,
                    box_size: None,
                    previous_close: None,
                    slot_width: slot,
                    x_for: &x_for,
                    scale: &scale,
                    palette: Palette::new([0, 200, 0], [200, 0, 0]),
                    style: &style,
                    chart_type: ChartType::VolumeFootprint,
                    recolor: None,
                    price_format: PriceFormat::AUTO,
                    profiles: Some(&feed),
                });
            });
            output.textures_delta.clear();
            output.shapes.len()
        };
        // Seen from the first bar and from the second: the stack (never touched) is drawn in both.
        for from in [0, 1] {
            assert!(
                count(true, from) > count(false, from),
                "view from bar {from}"
            );
        }
    }
}
