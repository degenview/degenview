//! The Portfolio Value Chart card: total value over time, coloured by whether the portfolio is
//! above what was put in, with a live point, a dashed line at today's value and a snapped
//! crosshair. The range switch and the headline figures are the card's controls row.

use super::{area_under, axis_text, dashed_h, dashed_v, loading_pill, reveal, tone_color, tooltip};
use crate::model::portfolio::card::{CardData, PortfolioCard};
use crate::ui::icons::Icon;
use crate::ui::kit::{
    self,
    time_text::{clock_time, date_text},
};
use crate::ui::portfolio::charts::{
    axis_label_indices, date_label, empty_message, hover_index, is_profit, prepared_points,
    profit_runs, value_bounds,
};
use crate::ui::theme;
use eframe::egui::{Align2, Color32, Id, Pos2, Rect, Stroke, Ui, Vec2};

const LEFT_MARGIN: f32 = 6.0;
const TOP_MARGIN: f32 = 8.0;
const BOTTOM_MARGIN: f32 = 20.0;
/// A tick for about every this much height.
const TICK_SPACING: f32 = 46.0;

/// How many value ticks fit `height`: at least a floor and a ceiling.
pub(super) fn tick_count(height: f32) -> usize {
    ((height / TICK_SPACING).floor() as usize + 1).clamp(2, 5)
}

/// The plot inside `area`, leaving `label_width` of gutter on the right for the value labels.
pub(super) fn plot_rect(area: Rect, label_width: f32) -> Rect {
    Rect::from_min_max(
        Pos2::new(area.left() + LEFT_MARGIN, area.top() + TOP_MARGIN),
        Pos2::new(
            (area.right() - label_width - 6.0).max(area.left() + LEFT_MARGIN + 20.0),
            (area.bottom() - BOTTOM_MARGIN).max(area.top() + TOP_MARGIN + 20.0),
        ),
    )
}

/// The headline: today's total and what the range gained or lost.
pub(super) fn paint_summary(ui: &Ui, row: Rect, card: &PortfolioCard, data: &CardData) {
    let centre_y = row.center().y;
    let total = if data.loading {
        "—".to_owned()
    } else {
        data.formatter.money(data.total_value)
    };
    let galley = super::fitted(
        ui,
        &total,
        17.0,
        12.0,
        kit::SEMIBOLD,
        theme::text_primary(),
        (row.width() * 0.6).max(40.0),
    );
    let width = galley.size().x;
    ui.painter().galley(
        Pos2::new(row.left(), centre_y - galley.size().y / 2.0),
        galley,
        Color32::PLACEHOLDER,
    );
    let Some(change) = data.period_change.filter(|_| !data.loading) else {
        return;
    };
    let tone = data.formatter.tone(Some(change.profit));
    let color = tone_color(tone);
    let arrow = if change.profit >= 0.0 { "↗" } else { "↘" };
    let remaining = row.width() - width - 10.0;
    let full = match change.percentage {
        Some(fraction) => format!(
            "{arrow} {} · {}",
            data.formatter.signed_money(change.profit),
            data.formatter.signed_percent(fraction, 2)
        ),
        None => format!("{arrow} {}", data.formatter.signed_money(change.profit)),
    };
    let short = match change.percentage {
        Some(fraction) => format!("{arrow} {}", data.formatter.signed_percent(fraction, 2)),
        None => full.clone(),
    };
    let probe = kit::galley(ui, &full, 11.0, kit::SEMIBOLD, color, f32::INFINITY);
    let text = if probe.size().x + 14.0 <= remaining {
        full
    } else {
        short
    };
    if remaining > 50.0 {
        super::chip(
            ui,
            Pos2::new(row.left() + width + 10.0, centre_y),
            &text,
            color,
            remaining,
        );
    }
    let _ = card;
}

pub(super) fn paint(
    ui: &Ui,
    area: Rect,
    id: Id,
    card: &PortfolioCard,
    data: &CardData,
    pointer: Option<Pos2>,
) {
    if data.loading {
        loading_pill(ui, area, "Loading market data…");
        return;
    }
    let points = prepared_points(
        &data.history,
        data.total_value,
        card.range,
        data.now_ms,
        data.loading_history && data.history.is_empty(),
    );
    if data.loading_history && data.history.is_empty() {
        loading_pill(ui, area, "Building history…");
        return;
    }
    let Some((minimum, maximum)) = value_bounds(&points) else {
        empty_message(
            ui,
            area,
            Icon::ChartUptrend,
            "No History Yet",
            "History builds from transaction dates and market candles.",
        );
        return;
    };
    let privacy = data.formatter.privacy;
    let spread = maximum - minimum;
    let ticks = tick_count(area.height() - TOP_MARGIN - BOTTOM_MARGIN);
    let labels: Vec<String> = (0..ticks)
        .map(|tick| {
            let fraction = tick as f64 / (ticks - 1) as f64;
            data.formatter
                .currency
                .format_compact(minimum + spread * fraction)
        })
        .collect();
    let label_width = if privacy {
        0.0
    } else {
        labels
            .iter()
            .map(|label| {
                kit::galley(ui, label, 10.0, kit::REGULAR, Color32::WHITE, f32::INFINITY)
                    .size()
                    .x
            })
            .fold(0.0, f32::max)
            + 6.0
    };
    let plot = plot_rect(area, label_width);
    let painter = ui.painter_at(area);

    // Grid and value labels, bottom to top. A label the badge of today's value would sit on is
    // left out: the badge says it better.
    let last_y = points.last().map_or(plot.bottom(), |point| {
        plot.bottom() - plot.height() * ((point.value - minimum) / spread) as f32
    });
    for (tick, label) in labels.iter().enumerate() {
        let fraction = tick as f32 / (ticks - 1) as f32;
        let y = plot.bottom() - plot.height() * fraction;
        painter.line_segment(
            [Pos2::new(plot.left(), y), Pos2::new(plot.right(), y)],
            Stroke::new(1.0, theme::grid().gamma_multiply(0.7)),
        );
        if !privacy && (y - last_y).abs() >= 12.0 {
            axis_text(
                ui,
                Pos2::new(plot.right() + 6.0, y),
                Align2::LEFT_CENTER,
                label,
            );
        }
    }

    let position = |index: usize| -> Pos2 {
        let x = if points.len() == 1 {
            plot.center().x
        } else {
            plot.left() + plot.width() * index as f32 / (points.len() - 1) as f32
        };
        let normalized = ((points[index].value - minimum) / spread) as f32;
        Pos2::new(x, plot.bottom() - plot.height() * normalized)
    };
    let positions: Vec<Pos2> = (0..points.len()).map(position).collect();

    // The line draws in from the left when the range or the card first appears.
    let progress = reveal(ui.ctx(), id.with(("draw", card.range.title())), 0.55);
    let clip = Rect::from_min_max(
        area.min,
        Pos2::new(plot.left() + plot.width() * progress + 3.0, area.bottom()),
    )
    .intersect(area);
    let drawing = painter.with_clip_rect(clip);
    if points.len() > 1 {
        for (from, to, profit) in profit_runs(&points) {
            let colour = if profit { theme::UP } else { theme::DOWN };
            area_under(&drawing, &positions[from..=to], plot.bottom(), colour, 0.24);
        }
        for index in 1..points.len() {
            let colour = if is_profit(&points[index]) {
                theme::UP
            } else {
                theme::DOWN
            };
            drawing.line_segment(
                [positions[index - 1], positions[index]],
                Stroke::new(2.0, colour),
            );
        }
    }

    // Today's value: a dashed line out to a badge in the gutter, and a dot where the line ends.
    let last = points.len() - 1;
    let live = if is_profit(&points[last]) {
        theme::UP
    } else {
        theme::DOWN
    };
    if progress >= 1.0 {
        painter.circle_filled(positions[last], 7.0, live.gamma_multiply(0.22));
        painter.circle_filled(positions[last], 3.5, live);
        if !privacy {
            dashed_h(
                &painter,
                positions[last].y,
                plot.left(),
                plot.right(),
                Stroke::new(1.0, live.gamma_multiply(0.45)),
            );
            let text = data.formatter.currency.format_compact(points[last].value);
            let galley = kit::galley(ui, &text, 10.0, kit::SEMIBOLD, Color32::WHITE, 80.0);
            let badge = Rect::from_min_size(
                Pos2::new(
                    plot.right() + 3.0,
                    (positions[last].y - 8.0).clamp(plot.top() - 6.0, plot.bottom() - 10.0),
                ),
                Vec2::new(galley.size().x + 8.0, 16.0),
            );
            painter.rect_filled(badge, 4.0, live);
            painter.galley(
                Pos2::new(badge.left() + 4.0, badge.center().y - galley.size().y / 2.0),
                galley,
                Color32::PLACEHOLDER,
            );
        }
    }

    // Time axis; a repeated coarse label is said once.
    let mut previous: Option<String> = None;
    for index in axis_label_indices(points.len(), plot.width()) {
        let label = date_label(card.range, points[index].timestamp, data.utc_offset_secs);
        if previous.as_deref() == Some(label.as_str()) {
            continue;
        }
        let anchor = if index == 0 && points.len() > 1 {
            Align2::LEFT_TOP
        } else {
            Align2::CENTER_TOP
        };
        axis_text(
            ui,
            Pos2::new(positions[index].x, plot.bottom() + 6.0),
            anchor,
            &label,
        );
        previous = Some(label);
    }

    // Crosshair.
    let Some(pointer) = pointer.filter(|p| plot.expand2(Vec2::new(0.0, 8.0)).contains(*p)) else {
        return;
    };
    let Some(index) = hover_index(points.len(), pointer.x, plot.left(), plot.width()) else {
        return;
    };
    let spot = positions[index];
    let point = &points[index];
    let guide = Stroke::new(1.0, theme::text_secondary().gamma_multiply(0.5));
    dashed_v(&painter, spot.x, plot.top(), plot.bottom(), guide);
    let tint = if is_profit(point) {
        theme::UP
    } else {
        theme::DOWN
    };
    painter.circle_filled(spot, 6.5, theme::background());
    painter.circle_filled(spot, 4.0, tint);
    let when = format!(
        "{} · {}",
        date_text(point.timestamp + data.utc_offset_secs * 1_000),
        clock_time(point.timestamp, data.utc_offset_secs)
    );
    let versus = point.value - point.net_contributions;
    let versus_tone = data.formatter.tone(Some(versus));
    let mut lines = vec![
        (
            data.formatter.money(point.value),
            14.0,
            kit::SEMIBOLD,
            theme::text_primary(),
        ),
        (when, 11.0, kit::REGULAR, theme::text_secondary()),
    ];
    if point.net_contributions != 0.0 {
        lines.push((
            format!("{} vs invested", data.formatter.signed_money(versus)),
            11.0,
            kit::MEDIUM,
            tone_color(versus_tone),
        ));
    }
    tooltip(ui, spot, plot.expand2(Vec2::new(8.0, 0.0)), &lines);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_short_card_still_has_a_floor_and_a_ceiling() {
        assert_eq!(tick_count(10.0), 2);
        assert_eq!(tick_count(120.0), 3);
        assert_eq!(tick_count(1_000.0), 5);
    }

    #[test]
    fn the_plot_leaves_room_for_labels_and_never_inverts() {
        let area = Rect::from_min_size(Pos2::ZERO, Vec2::new(300.0, 200.0));
        let plot = plot_rect(area, 50.0);
        assert!(plot.right() <= area.right() - 50.0);
        assert!(plot.bottom() <= area.bottom() - BOTTOM_MARGIN + 0.01);
        let tiny = Rect::from_min_size(Pos2::ZERO, Vec2::new(40.0, 20.0));
        let plot = plot_rect(tiny, 80.0);
        assert!(plot.width() > 0.0 && plot.height() > 0.0);
    }
}
