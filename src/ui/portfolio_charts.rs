//! The Overview's two charts, drawn by hand like the candle charts: the allocation donut
//! (with its legend) and the portfolio-value line. The geometry and data preparation are
//! pure functions with tests; the `show_*` functions only paint them.

// Transient: the Portfolio window hosts these once the portfolio store is wired.
#![allow(dead_code)]

use crate::model::portfolio::{PortfolioCurrency, PortfolioHolding, PortfolioSnapshot};
use crate::model::portfolio_format::PortfolioValueFormatter;
use crate::model::portfolio_statistics::PortfolioHistoryRange;
use crate::ui::accessibility;
use crate::ui::alert_center::clock_time;
use crate::ui::icons::{self, Icon};
use crate::ui::kit;
use crate::ui::portfolio_empty;
use crate::ui::replay_bar::date_text;
use crate::ui::theme;
use eframe::egui::{self, Align2, Color32, FontId, Mesh, Pos2, Rect, Sense, Shape, Stroke, Vec2};
use std::f32::consts::{PI, TAU};

// MARK: Allocation

const PALETTE: [Color32; 8] = [
    Color32::from_rgb(10, 132, 255),
    Color32::from_rgb(255, 159, 10),
    Color32::from_rgb(48, 209, 88),
    Color32::from_rgb(191, 90, 242),
    Color32::from_rgb(255, 55, 95),
    Color32::from_rgb(100, 210, 255),
    Color32::from_rgb(255, 214, 10),
    Color32::from_rgb(94, 92, 230),
];
const OTHER_COLOR: Color32 = Color32::from_gray(130);

#[derive(Clone, Debug, PartialEq)]
pub struct Slice {
    pub id: String,
    pub label: String,
    /// Fraction of the portfolio, 0…=1.
    pub share: f64,
    pub value: Option<f64>,
    pub color: Color32,
}

/// The biggest positions get their own slice; the long tail is folded into "Other" so a
/// hundred-asset portfolio still reads, and colours never repeat.
pub fn slices(holdings: &[PortfolioHolding]) -> Vec<Slice> {
    let mut sorted: Vec<&PortfolioHolding> =
        holdings.iter().filter(|h| h.allocation > 0.0).collect();
    sorted.sort_by(|a, b| {
        b.allocation
            .total_cmp(&a.allocation)
            .then_with(|| a.asset.key.cmp(&b.asset.key))
    });
    let mut result: Vec<Slice> = sorted
        .iter()
        .take(PALETTE.len())
        .enumerate()
        .map(|(index, holding)| Slice {
            id: holding.asset.key.clone(),
            label: holding.asset.display_ticker(),
            share: holding.allocation,
            value: holding.current_value,
            color: PALETTE[index],
        })
        .collect();
    let rest = &sorted[sorted.len().min(PALETTE.len())..];
    if !rest.is_empty() {
        result.push(Slice {
            id: "other".to_owned(),
            label: format!("Other ({})", rest.len()),
            share: rest.iter().map(|h| h.allocation).sum(),
            value: Some(rest.iter().filter_map(|h| h.current_value).sum()),
            color: OTHER_COLOR,
        });
    }
    result
}

/// Which slice a pointer at `offset` from the donut's centre is over: on the ring, within
/// `half_width` of `radius`. Slices run clockwise from twelve o'clock.
pub fn slice_at(slices: &[Slice], offset: Vec2, radius: f32, half_width: f32) -> Option<usize> {
    if (offset.length() - radius).abs() > half_width {
        return None;
    }
    // atan2 puts zero at three o'clock, increasing clockwise on screen (y grows down).
    let mut angle = offset.y.atan2(offset.x) + PI / 2.0;
    if angle < 0.0 {
        angle += TAU;
    }
    let mut start = 0.0_f32;
    for (index, slice) in slices.iter().enumerate() {
        let end = start + slice.share as f32 * TAU;
        if angle >= start && angle < end {
            return Some(index);
        }
        start = end;
    }
    None
}

/// The donut and its legend, centred in a block `height` tall. `hovered` is the slice under the
/// pointer (donut or legend).
pub fn show_allocation(
    ui: &mut egui::Ui,
    holdings: &[PortfolioHolding],
    formatter: PortfolioValueFormatter,
    hovered: &mut Option<usize>,
    height: f32,
) {
    let slices = slices(holdings);
    let (area, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::hover());
    if slices.is_empty() {
        portfolio_empty::compact(
            ui,
            area,
            portfolio_empty::Scene::Allocation,
            "No Allocations",
            "Priced holdings appear here.",
        );
        return;
    }
    let mut next_hover = None;
    let primary = theme::text_primary();
    let secondary = theme::text_secondary();

    // The donut and its legend sit together in the middle of the panel; a legend never grows
    // wider than its longest row needs.
    let legend_width = (area.width() - DONUT_SIZE - DONUT_GAP).clamp(120.0, LEGEND_MAX_WIDTH);
    let group_left =
        area.left() + (area.width() - DONUT_SIZE - DONUT_GAP - legend_width).max(0.0) / 2.0;
    let donut = Rect::from_min_size(
        Pos2::new(group_left, area.center().y - DONUT_SIZE / 2.0),
        Vec2::splat(DONUT_SIZE),
    );
    let radius = DONUT_SIZE / 2.0 - 18.0;
    let painter = ui.painter_at(donut.expand(4.0));
    let mut start = -PI / 2.0;
    for (index, slice) in slices.iter().enumerate() {
        let end = start + slice.share as f32 * TAU;
        let dimmed = hovered.is_some() && *hovered != Some(index);
        let width = if *hovered == Some(index) { 28.0 } else { 24.0 };
        let gap = if slices.len() > 1 { 0.026 } else { 0.0 };
        let points = arc(donut.center(), radius, start, (end - gap).max(start));
        if points.len() >= 2 {
            painter.add(Shape::line(
                points,
                Stroke::new(
                    width,
                    slice.color.gamma_multiply(if dimmed { 0.3 } else { 1.0 }),
                ),
            ));
        }
        start = end;
    }
    let donut_response = ui.interact(donut, ui.id().with("allocation-donut"), Sense::hover());
    if let Some(pointer) = donut_response.hover_pos() {
        next_hover = slice_at(&slices, pointer - donut.center(), radius, 18.0);
    }

    // Centre: the hovered slice, or the asset count.
    let centre = donut.center();
    match hovered.and_then(|index| slices.get(index)) {
        Some(slice) => {
            let label = kit::galley(ui, &slice.label, 11.0, kit::SEMIBOLD, primary, 100.0);
            let share = kit::galley(
                ui,
                &formatter.percent(slice.share, 1),
                15.0,
                kit::SEMIBOLD,
                primary,
                110.0,
            );
            let value = kit::galley(
                ui,
                &slice.value.map_or("—".to_owned(), |v| formatter.money(v)),
                10.0,
                kit::REGULAR,
                secondary,
                110.0,
            );
            let total = label.size().y + share.size().y + value.size().y + 4.0;
            let mut y = centre.y - total / 2.0;
            for galley in [&label, &share, &value] {
                painter.galley(
                    Pos2::new(centre.x - galley.size().x / 2.0, y),
                    galley.clone(),
                    Color32::PLACEHOLDER,
                );
                y += galley.size().y + 2.0;
            }
        }
        None => {
            let count = kit::galley(
                ui,
                &holdings.len().to_string(),
                22.0,
                kit::SEMIBOLD,
                primary,
                110.0,
            );
            let caption = kit::galley(
                ui,
                if holdings.len() == 1 {
                    "Asset"
                } else {
                    "Assets"
                },
                10.0,
                kit::REGULAR,
                secondary,
                110.0,
            );
            let total = count.size().y + 2.0 + caption.size().y;
            let top = centre.y - total / 2.0;
            painter.galley(
                Pos2::new(centre.x - count.size().x / 2.0, top),
                count.clone(),
                Color32::PLACEHOLDER,
            );
            painter.galley(
                Pos2::new(
                    centre.x - caption.size().x / 2.0,
                    top + count.size().y + 2.0,
                ),
                caption,
                Color32::PLACEHOLDER,
            );
        }
    }

    // Legend: one row per slice, centred as a block when it fits, scrolling when it does not.
    let legend = Rect::from_min_max(
        Pos2::new(donut.right() + DONUT_GAP, area.top()),
        Pos2::new(donut.right() + DONUT_GAP + legend_width, area.bottom()),
    );
    let block = slices.len() as f32 * (LEGEND_ROW + LEGEND_GAP) - LEGEND_GAP;
    let offset = ((legend.height() - block) / 2.0).max(0.0);
    let mut legend_ui = ui.new_child(egui::UiBuilder::new().max_rect(legend));
    egui::ScrollArea::vertical()
        .id_salt("allocation-legend")
        .auto_shrink([false, false])
        .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::VisibleWhenNeeded)
        .show(&mut legend_ui, |ui| {
            ui.add_space(offset);
            ui.spacing_mut().item_spacing.y = LEGEND_GAP;
            for (index, slice) in slices.iter().enumerate() {
                let (row, response) = ui.allocate_exact_size(
                    Vec2::new(ui.available_width(), LEGEND_ROW),
                    Sense::hover(),
                );
                if response.hovered() {
                    next_hover = Some(index);
                }
                let active = *hovered == Some(index);
                let fade = if hovered.is_none() || active {
                    1.0
                } else {
                    0.45
                };
                if active {
                    ui.painter().rect_filled(row, 6.0, theme::ink(18));
                }
                ui.painter().circle_filled(
                    Pos2::new(row.left() + 12.0, row.center().y),
                    4.0,
                    slice.color.gamma_multiply(fade),
                );
                let share = kit::galley(
                    ui,
                    &formatter.percent(slice.share, 1),
                    kit::size::SUBHEADLINE,
                    kit::REGULAR,
                    secondary.gamma_multiply(fade),
                    80.0,
                );
                let label = kit::galley(
                    ui,
                    &slice.label,
                    kit::size::SUBHEADLINE,
                    kit::MEDIUM,
                    primary.gamma_multiply(fade),
                    (row.width() - 24.0 - share.size().x - 16.0).max(20.0),
                );
                ui.painter().galley(
                    Pos2::new(row.left() + 24.0, row.center().y - label.size().y / 2.0),
                    label,
                    Color32::PLACEHOLDER,
                );
                ui.painter().galley(
                    Pos2::new(
                        row.right() - 8.0 - share.size().x,
                        row.center().y - share.size().y / 2.0,
                    ),
                    share,
                    Color32::PLACEHOLDER,
                );
                accessibility::label_as(
                    response,
                    egui::WidgetType::Label,
                    format!(
                        "{}, {} of portfolio",
                        slice.label,
                        formatter.percent(slice.share, 1)
                    ),
                );
            }
        });
    *hovered = next_hover;
}

const DONUT_SIZE: f32 = 190.0;
const DONUT_GAP: f32 = 20.0;
const LEGEND_MAX_WIDTH: f32 = 240.0;
/// Legend rows: 5 pt of padding around an 11 pt line, 2 pt apart.
const LEGEND_ROW: f32 = 24.0;
const LEGEND_GAP: f32 = 2.0;

/// A centred glyph, title and one-line explanation: the empty state of a chart.
pub(crate) fn empty_message(ui: &egui::Ui, area: Rect, icon: Icon, title: &str, detail: &str) {
    let heading = kit::galley(
        ui,
        title,
        kit::size::HEADLINE,
        kit::SEMIBOLD,
        theme::text_primary(),
        area.width(),
    );
    let body = kit::galley(
        ui,
        detail,
        kit::size::SUBHEADLINE,
        kit::REGULAR,
        theme::text_secondary(),
        area.width() - 32.0,
    );
    let total = 30.0 + 10.0 + heading.size().y + 4.0 + body.size().y;
    let top = area.center().y - total / 2.0;
    icons::paint(
        ui.painter(),
        Rect::from_center_size(Pos2::new(area.center().x, top + 15.0), Vec2::splat(30.0)),
        icon,
        theme::text_tertiary(),
    );
    let mut y = top + 40.0;
    for galley in [heading, body] {
        ui.painter().galley(
            Pos2::new(area.center().x - galley.size().x / 2.0, y),
            galley.clone(),
            Color32::PLACEHOLDER,
        );
        y += galley.size().y + 4.0;
    }
}

/// Points along an arc, for a thick stroke. Angles in radians.
pub(crate) fn arc(centre: Pos2, radius: f32, start: f32, end: f32) -> Vec<Pos2> {
    let steps = (((end - start).abs() * radius / 3.0).ceil() as usize).max(2);
    (0..=steps)
        .map(|step| {
            let angle = start + (end - start) * step as f32 / steps as f32;
            centre + Vec2::new(angle.cos(), angle.sin()) * radius
        })
        .collect()
}

// MARK: Value history

const LEFT_MARGIN: f32 = 12.0;
const RIGHT_MARGIN: f32 = 76.0;
const TOP_MARGIN: f32 = 12.0;
const BOTTOM_MARGIN: f32 = 28.0;
const LABEL_SPACING: f32 = 110.0;

/// The history in `range`, with a live point at today's value ending the line (unless the
/// data is still loading).
pub fn prepared_points(
    snapshots: &[PortfolioSnapshot],
    current_value: f64,
    range: PortfolioHistoryRange,
    now_ms: i64,
    is_loading: bool,
) -> Vec<PortfolioSnapshot> {
    let mut ranged = range.filter(snapshots, now_ms);
    // Days before the first purchase are an empty portfolio, not a value of zero to climb from.
    let first_held = ranged
        .iter()
        .position(|point| point.value != 0.0 || point.net_contributions != 0.0)
        .unwrap_or(ranged.len().saturating_sub(1));
    ranged.drain(..first_held);
    if is_loading {
        return ranged;
    }
    let Some(latest) = ranged.last().or(snapshots.last()).cloned() else {
        return ranged;
    };
    ranged.push(PortfolioSnapshot {
        portfolio_id: latest.portfolio_id,
        timestamp: now_ms,
        value: current_value,
        net_contributions: latest.net_contributions,
        realized_pnl: latest.realized_pnl,
        unrealized_pnl: current_value - latest.net_contributions,
        is_complete: latest.is_complete,
    });
    ranged
}

/// The value range of the chart: padded 8 % either side; a flat line is given a spread of 2.
pub fn value_bounds(points: &[PortfolioSnapshot]) -> Option<(f64, f64)> {
    let minimum = points.iter().map(|p| p.value).fold(f64::INFINITY, f64::min);
    let maximum = points
        .iter()
        .map(|p| p.value)
        .fold(f64::NEG_INFINITY, f64::max);
    if !minimum.is_finite() || !maximum.is_finite() {
        return None;
    }
    let (minimum, maximum) = if minimum == maximum {
        (minimum - 1.0, maximum + 1.0)
    } else {
        (minimum, maximum)
    };
    let padding = (maximum - minimum) * 0.08;
    Some((minimum - padding, maximum + padding))
}

/// Whether the portfolio is above what was put in at this point.
pub fn is_profit(point: &PortfolioSnapshot) -> bool {
    point.value >= point.net_contributions
}

/// The stretches of same-coloured segments, as inclusive index ranges. One polygon per
/// stretch rather than per segment, so adjacent fills show no anti-aliasing seams. Each
/// segment takes the colour of the point it ends on, like the stroke.
pub fn profit_runs(points: &[PortfolioSnapshot]) -> Vec<(usize, usize, bool)> {
    let mut runs = Vec::new();
    if points.len() < 2 {
        return runs;
    }
    let mut run_start = 0;
    for index in 1..points.len() {
        let profit = is_profit(&points[index]);
        let is_last = index == points.len() - 1;
        let next_profit = if is_last {
            profit
        } else {
            is_profit(&points[index + 1])
        };
        if is_last || next_profit != profit {
            runs.push((run_start, index, profit));
            run_start = index;
        }
    }
    runs
}

/// About one label per `LABEL_SPACING` points of width, evenly spread over the points.
pub fn axis_label_indices(count: usize, width: f32) -> Vec<usize> {
    if count <= 1 {
        return vec![0];
    }
    let labels = (width / LABEL_SPACING).floor().max(2.0).min(count as f32) as usize;
    (0..labels)
        .map(|label| ((label as f64 * (count - 1) as f64 / (labels - 1) as f64).round()) as usize)
        .collect()
}

/// The index under a pointer at `x`, snapping to the nearest point.
pub fn hover_index(count: usize, x: f32, plot_left: f32, plot_width: f32) -> Option<usize> {
    if count == 0 {
        return None;
    }
    if count == 1 {
        return Some(0);
    }
    let fraction = ((x - plot_left) / plot_width.max(1.0)).clamp(0.0, 1.0);
    Some((fraction * (count - 1) as f32).round() as usize)
}

/// The label under the time axis: a clock time for a day, a day for weeks, a month for years.
pub fn date_label(range: PortfolioHistoryRange, ms: i64, utc_offset_secs: i64) -> String {
    match range {
        PortfolioHistoryRange::OneDay => clock_time(ms, utc_offset_secs),
        PortfolioHistoryRange::OneWeek | PortfolioHistoryRange::OneMonth => {
            let text = date_text(ms + utc_offset_secs * 1_000);
            // "Oct 9, 2026" → "Oct 9"
            text.split(',').next().unwrap_or(&text).to_owned()
        }
        PortfolioHistoryRange::OneYear | PortfolioHistoryRange::All => {
            let text = date_text(ms + utc_offset_secs * 1_000);
            // "Oct 9, 2026" → "Oct 2026"
            let month = text.split(' ').next().unwrap_or("");
            let year = text.rsplit(' ').next().unwrap_or("");
            format!("{month} {year}")
        }
    }
}

pub struct HistoryInput<'a> {
    pub snapshots: &'a [PortfolioSnapshot],
    pub current_value: f64,
    pub currency: PortfolioCurrency,
    pub range: PortfolioHistoryRange,
    pub privacy: bool,
    pub is_loading: bool,
    pub loading_message: &'a str,
    pub now_ms: i64,
    pub utc_offset_secs: i64,
}

pub fn show_history(ui: &mut egui::Ui, input: &HistoryInput, height: f32) {
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::hover());
    let painter = ui.painter_at(rect);
    let points = prepared_points(
        input.snapshots,
        input.current_value,
        input.range,
        input.now_ms,
        input.is_loading,
    );

    if input.is_loading {
        let label = kit::galley(
            ui,
            input.loading_message,
            12.0,
            kit::REGULAR,
            theme::text_primary(),
            rect.width() - 80.0,
        );
        let pill = Rect::from_center_size(
            rect.center(),
            Vec2::new(14.0 + 8.0 + label.size().x + 28.0, label.size().y + 20.0),
        );
        painter.rect_filled(pill, 10.0, theme::ink(14));
        painter.rect_stroke(
            pill,
            10.0,
            Stroke::new(1.0, theme::separator().gamma_multiply(0.5)),
            egui::StrokeKind::Inside,
        );
        ui.put(
            Rect::from_center_size(
                Pos2::new(pill.left() + 14.0 + 7.0, pill.center().y),
                Vec2::splat(14.0),
            ),
            egui::Spinner::new().size(14.0),
        );
        painter.galley(
            Pos2::new(
                pill.left() + 14.0 + 14.0 + 8.0,
                pill.center().y - label.size().y / 2.0,
            ),
            label,
            Color32::PLACEHOLDER,
        );
        return;
    }
    if input.privacy {
        let label = kit::galley(
            ui,
            "Values hidden",
            12.0,
            kit::REGULAR,
            theme::text_secondary(),
            rect.width(),
        );
        painter.galley(
            rect.center() - label.size() / 2.0,
            label,
            Color32::PLACEHOLDER,
        );
        return;
    }
    let Some((minimum, maximum)) = value_bounds(&points) else {
        portfolio_empty::compact(
            ui,
            rect,
            portfolio_empty::Scene::History,
            "No History Yet",
            "History builds from transaction dates and available market candles.",
        );
        return;
    };

    let plot = Rect::from_min_max(
        Pos2::new(rect.left() + LEFT_MARGIN, rect.top() + TOP_MARGIN),
        Pos2::new(rect.right() - RIGHT_MARGIN, rect.bottom() - BOTTOM_MARGIN),
    );
    let spread = maximum - minimum;
    let position = |index: usize| -> Pos2 {
        let x = if points.len() == 1 {
            plot.center().x
        } else {
            plot.left() + plot.width() * index as f32 / (points.len() - 1) as f32
        };
        let normalized = ((points[index].value - minimum) / spread) as f32;
        Pos2::new(x, plot.bottom() - plot.height() * normalized)
    };

    // Grid and value labels.
    for tick in 0..=4 {
        let fraction = tick as f32 / 4.0;
        let y = plot.bottom() - plot.height() * fraction;
        painter.line_segment(
            [Pos2::new(plot.left(), y), Pos2::new(plot.right(), y)],
            Stroke::new(1.0, theme::grid()),
        );
        painter.text(
            Pos2::new(plot.right() + 6.0, y),
            Align2::LEFT_CENTER,
            input
                .currency
                .format_compact(minimum + spread * f64::from(fraction)),
            FontId::proportional(10.0),
            ui.visuals().weak_text_color(),
        );
    }

    if points.len() > 1 {
        for (from, to, profit) in profit_runs(&points) {
            let colour = if profit { theme::UP } else { theme::DOWN };
            let mut mesh = Mesh::default();
            for index in from..=to {
                let top = position(index);
                mesh.colored_vertex(top, colour.gamma_multiply(0.22));
                mesh.colored_vertex(Pos2::new(top.x, plot.bottom()), colour.gamma_multiply(0.02));
            }
            for step in 0..(to - from) {
                let base = (step * 2) as u32;
                mesh.add_triangle(base, base + 1, base + 2);
                mesh.add_triangle(base + 1, base + 3, base + 2);
            }
            painter.add(Shape::mesh(mesh));
        }
        for (index, point) in points.iter().enumerate().skip(1) {
            let colour = if is_profit(point) {
                theme::UP
            } else {
                theme::DOWN
            };
            painter.line_segment(
                [position(index - 1), position(index)],
                Stroke::new(2.25, colour),
            );
        }
    }

    // The live point.
    let last = points.len() - 1;
    let live_colour = if is_profit(&points[last]) {
        theme::UP
    } else {
        theme::DOWN
    };
    painter.circle_filled(position(last), 3.5, live_colour);

    // Axis labels, a repeated coarse label said once.
    let mut previous: Option<String> = None;
    for index in axis_label_indices(points.len(), plot.width()) {
        let label = date_label(input.range, points[index].timestamp, input.utc_offset_secs);
        if previous.as_deref() == Some(label.as_str()) {
            continue;
        }
        let anchor = if index == 0 && points.len() > 1 {
            Align2::LEFT_TOP
        } else {
            Align2::CENTER_TOP
        };
        painter.text(
            Pos2::new(position(index).x, plot.bottom() + 8.0),
            anchor,
            &label,
            FontId::proportional(10.0),
            ui.visuals().weak_text_color(),
        );
        previous = Some(label);
    }

    // Hover.
    if let Some(pointer) = response
        .hover_pos()
        .filter(|p| plot.expand2(Vec2::new(0.0, 8.0)).contains(*p))
        && let Some(index) = hover_index(points.len(), pointer.x, plot.left(), plot.width())
    {
        let spot = position(index);
        let dash = Stroke::new(1.0, Color32::from_gray(150).gamma_multiply(0.6));
        painter.line_segment(
            [
                Pos2::new(spot.x, plot.top()),
                Pos2::new(spot.x, plot.bottom()),
            ],
            dash,
        );
        painter.line_segment(
            [
                Pos2::new(plot.left(), spot.y),
                Pos2::new(plot.right(), spot.y),
            ],
            dash,
        );
        let tint = if is_profit(&points[index]) {
            theme::UP
        } else {
            theme::DOWN
        };
        painter.circle(spot, 4.5, tint, Stroke::new(1.5, theme::background()));

        let value = input.currency.format(points[index].value);
        let when = format!(
            "{} {}",
            date_text(points[index].timestamp + input.utc_offset_secs * 1_000),
            clock_time(points[index].timestamp, input.utc_offset_secs)
        );
        let value_galley = painter.layout_no_wrap(
            value,
            FontId::proportional(13.0),
            ui.visuals().strong_text_color(),
        );
        let when_galley = painter.layout_no_wrap(
            when,
            FontId::proportional(11.0),
            ui.visuals().weak_text_color(),
        );
        let size = Vec2::new(
            value_galley.size().x.max(when_galley.size().x) + 20.0,
            value_galley.size().y + when_galley.size().y + 14.0,
        );
        let x = spot
            .x
            .clamp(plot.left() + size.x / 2.0, plot.right() - size.x / 2.0);
        let above = spot.y - size.y - 12.0;
        let y = if above >= plot.top() - 4.0 {
            above
        } else {
            spot.y + 12.0
        };
        let tip = Rect::from_min_size(Pos2::new(x - size.x / 2.0, y), size);
        painter.rect_filled(tip, 8.0, theme::chrome().gamma_multiply(0.96));
        painter.rect_stroke(
            tip,
            8.0,
            Stroke::new(1.0, theme::border()),
            egui::StrokeKind::Inside,
        );
        painter.galley(
            Pos2::new(
                tip.center().x - value_galley.size().x / 2.0,
                tip.top() + 6.0,
            ),
            value_galley.clone(),
            Color32::PLACEHOLDER,
        );
        painter.galley(
            Pos2::new(
                tip.center().x - when_galley.size().x / 2.0,
                tip.top() + 8.0 + value_galley.size().y,
            ),
            when_galley,
            Color32::PLACEHOLDER,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::DataSource;
    use crate::model::portfolio::PortfolioAsset;
    use std::collections::HashMap;

    fn holding(ticker: &str, allocation: f64, value: Option<f64>) -> PortfolioHolding {
        PortfolioHolding {
            asset: PortfolioAsset {
                key: format!("binance:{ticker}"),
                symbol: format!("{ticker}/USDT"),
                name: ticker.to_owned(),
                source: DataSource::Binance,
                quote_currency: PortfolioCurrency::Usd,
                metadata: HashMap::new(),
            },
            quantity: 1.0,
            cost_basis: 1.0,
            average_cost: 1.0,
            realized_pnl: 0.0,
            current_price: value,
            previous_day_price: None,
            current_value: value,
            unrealized_pnl: None,
            allocation,
            is_price_stale: false,
        }
    }

    fn snapshot(timestamp: i64, value: f64, contributions: f64) -> PortfolioSnapshot {
        PortfolioSnapshot {
            portfolio_id: 1,
            timestamp,
            value,
            net_contributions: contributions,
            realized_pnl: 0.0,
            unrealized_pnl: 0.0,
            is_complete: true,
        }
    }

    #[test]
    fn the_biggest_positions_get_slices_and_the_tail_becomes_other() {
        let many: Vec<PortfolioHolding> = (0..12)
            .map(|n| {
                holding(
                    &format!("A{n}"),
                    0.01 * (12 - n) as f64,
                    Some(100.0 - n as f64),
                )
            })
            .collect();
        let made = slices(&many);
        assert_eq!(made.len(), 9, "eight named plus Other");
        assert_eq!(made[0].label, "A0");
        assert!(
            made.windows(2)
                .take(7)
                .all(|pair| pair[0].share >= pair[1].share)
        );
        let other = made.last().unwrap();
        assert_eq!(other.label, "Other (4)");
        assert!((other.share - (0.04 + 0.03 + 0.02 + 0.01)).abs() < 1e-12);
        assert_eq!(other.color, OTHER_COLOR);
        let colours: std::collections::HashSet<_> =
            made.iter().map(|s| s.color.to_array()).collect();
        assert_eq!(colours.len(), 9, "colours never repeat");
    }

    #[test]
    fn unallocated_holdings_get_no_slice() {
        let found = slices(&[holding("A", 0.0, None), holding("B", 1.0, Some(5.0))]);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].label, "B");
        assert!(slices(&[]).is_empty());
    }

    #[test]
    fn a_pointer_on_the_ring_finds_its_slice_clockwise_from_twelve() {
        let made = vec![
            Slice {
                id: "a".into(),
                label: "A".into(),
                share: 0.5,
                value: None,
                color: PALETTE[0],
            },
            Slice {
                id: "b".into(),
                label: "B".into(),
                share: 0.25,
                value: None,
                color: PALETTE[1],
            },
            Slice {
                id: "c".into(),
                label: "C".into(),
                share: 0.25,
                value: None,
                color: PALETTE[2],
            },
        ];
        let at = |x: f32, y: f32| slice_at(&made, Vec2::new(x, y), 77.0, 18.0);
        assert_eq!(at(10.0, -77.0), Some(0), "just right of twelve o'clock");
        assert_eq!(
            at(77.0, 0.0),
            Some(0),
            "three o'clock is still the first half"
        );
        assert_eq!(
            at(0.0, 77.0),
            Some(1),
            "six o'clock starts the second slice"
        );
        assert_eq!(at(-77.0, 10.0), Some(1));
        assert_eq!(
            at(-77.0, -10.0),
            Some(2),
            "left of twelve is the last quarter"
        );
        assert_eq!(at(-10.0, -77.0), Some(2));
        assert_eq!(at(0.0, 0.0), None, "the hole");
        assert_eq!(at(150.0, 0.0), None, "outside the ring");
        assert_eq!(at(77.0 + 17.0, 0.0), Some(0));
        assert_eq!(at(77.0 + 19.0, 0.0), None);
    }

    #[test]
    fn the_live_point_ends_the_line_unless_loading() {
        let history = vec![snapshot(1_000, 100.0, 90.0), snapshot(2_000, 110.0, 90.0)];
        let day = 86_400_000;
        let now = 2_000 + day / 2;
        let points = prepared_points(&history, 125.0, PortfolioHistoryRange::OneDay, now, false);
        assert_eq!(points.len(), 3);
        let live = points.last().unwrap();
        assert_eq!(
            (live.timestamp, live.value, live.net_contributions),
            (now, 125.0, 90.0)
        );
        assert_eq!(live.unrealized_pnl, 35.0);
        assert_eq!(
            prepared_points(&history, 125.0, PortfolioHistoryRange::OneDay, now, true).len(),
            2
        );
        // A range with no snapshots still ends at "now", built from the latest known one.
        let far = 400 * day;
        let only_live = prepared_points(&history, 125.0, PortfolioHistoryRange::OneDay, far, false);
        assert_eq!(only_live.len(), 1);
        assert_eq!(only_live[0].net_contributions, 90.0);
        assert!(prepared_points(&[], 5.0, PortfolioHistoryRange::All, 0, false).is_empty());
    }

    #[test]
    fn value_bounds_pad_and_give_a_flat_line_some_height() {
        let (low, high) =
            value_bounds(&[snapshot(0, 100.0, 0.0), snapshot(1, 200.0, 0.0)]).unwrap();
        assert!((low - 92.0).abs() < 1e-9 && (high - 208.0).abs() < 1e-9);
        let (flat_low, flat_high) = value_bounds(&[snapshot(0, 50.0, 0.0)]).unwrap();
        assert!(flat_low < 50.0 && flat_high > 50.0);
        assert!((flat_high - flat_low - 2.0 * 1.16).abs() < 1e-9);
        assert_eq!(value_bounds(&[]), None);
    }

    #[test]
    fn colour_runs_follow_profit_and_loss_without_gaps() {
        // value vs contributions: profit, profit, loss, loss, profit
        let points = vec![
            snapshot(0, 110.0, 100.0),
            snapshot(1, 120.0, 100.0),
            snapshot(2, 90.0, 100.0),
            snapshot(3, 80.0, 100.0),
            snapshot(4, 105.0, 100.0),
        ];
        let runs = profit_runs(&points);
        assert_eq!(runs, vec![(0, 1, true), (1, 3, false), (3, 4, true)]);
        // Each run starts where the previous ended, so the fills meet.
        assert!(runs.windows(2).all(|pair| pair[0].1 == pair[1].0));
        assert!(profit_runs(&points[..1]).is_empty());
        let steady = vec![
            snapshot(0, 110.0, 100.0),
            snapshot(1, 120.0, 100.0),
            snapshot(2, 130.0, 100.0),
        ];
        assert_eq!(profit_runs(&steady), vec![(0, 2, true)]);
    }

    #[test]
    fn axis_labels_are_spread_evenly_about_110_points_apart() {
        assert_eq!(axis_label_indices(0, 500.0), vec![0]);
        assert_eq!(axis_label_indices(1, 500.0), vec![0]);
        assert_eq!(axis_label_indices(100, 220.0), vec![0, 99], "at least two");
        assert_eq!(axis_label_indices(100, 440.0), vec![0, 33, 66, 99]);
        assert_eq!(
            axis_label_indices(3, 1_000.0),
            vec![0, 1, 2],
            "never more labels than points"
        );
    }

    #[test]
    fn hovering_snaps_to_the_nearest_point() {
        assert_eq!(hover_index(0, 5.0, 0.0, 100.0), None);
        assert_eq!(hover_index(1, 5.0, 0.0, 100.0), Some(0));
        assert_eq!(hover_index(11, 0.0, 0.0, 100.0), Some(0));
        assert_eq!(hover_index(11, 54.0, 0.0, 100.0), Some(5));
        assert_eq!(hover_index(11, 56.0, 0.0, 100.0), Some(6));
        assert_eq!(
            hover_index(11, 500.0, 0.0, 100.0),
            Some(10),
            "clamped to the plot"
        );
        assert_eq!(hover_index(11, -50.0, 0.0, 100.0), Some(0));
    }

    #[test]
    fn date_labels_get_coarser_with_the_range() {
        let ms = 1_705_361_400_000; // 2024-01-15 23:30 UTC
        assert_eq!(date_label(PortfolioHistoryRange::OneDay, ms, 0), "23:30");
        assert_eq!(date_label(PortfolioHistoryRange::OneWeek, ms, 0), "Jan 15");
        assert_eq!(
            date_label(PortfolioHistoryRange::OneMonth, ms, 2 * 3_600),
            "Jan 16"
        );
        assert_eq!(
            date_label(PortfolioHistoryRange::OneYear, ms, 0),
            "Jan 2024"
        );
        assert_eq!(date_label(PortfolioHistoryRange::All, ms, 0), "Jan 2024");
    }

    #[test]
    fn arcs_start_and_end_on_the_circle() {
        let points = arc(Pos2::new(100.0, 100.0), 50.0, 0.0, PI);
        assert!(points.len() >= 3);
        for point in &points {
            assert!(((*point - Pos2::new(100.0, 100.0)).length() - 50.0).abs() < 1e-3);
        }
        assert!(
            (points[0].x - 150.0).abs() < 1e-3 && (points.last().unwrap().x - 50.0).abs() < 1e-3
        );
    }
}
