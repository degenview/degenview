//! The Top Performers card: the holdings ranked by how their markets moved over the card's
//! range, best first, each as a bar growing from a shared zero line (gains right, losses left).
//! The bars and the row count are pure and tested; hovering a row shows where its market
//! started and where it stands.

use super::{chip, loading_pill, tone_color, tooltip};
use crate::model::portfolio::card::{CardData, PortfolioCard};
use crate::ui::icons::Icon;
use crate::ui::kit;
use crate::ui::portfolio::charts::empty_message;
use crate::ui::portfolio::table::paint_asset_icon;
use crate::ui::theme;
use eframe::egui::{Color32, Id, Pos2, Rect, Ui, Vec2};

const ROW_HEIGHT: f32 = 38.0;
const FOOTER_HEIGHT: f32 = 22.0;
const ICON_SIZE: f32 = 24.0;
/// Right column: the change and, under it, what it was worth.
const RIGHT_COLUMN: f32 = 76.0;
const TICKER_COLUMN: f32 = 84.0;
const MIN_BAR_ZONE: f32 = 56.0;

/// One bar: where it starts and how long it is, both as fractions of the bar zone's width.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Bar {
    pub start: f32,
    pub length: f32,
}

/// Bars for `changes`, scaled to the largest move on each side so the zero line sits where the
/// two reaches meet. A move that rounds to nothing keeps a sliver so its row still reads.
pub(super) fn bars(changes: &[f64]) -> (f32, Vec<Bar>) {
    let gain_reach = changes.iter().copied().fold(0.0_f64, f64::max);
    let loss_reach = changes
        .iter()
        .map(|change| (-change).max(0.0))
        .fold(0.0, f64::max);
    let span = (gain_reach + loss_reach).max(1e-9);
    let zero = (loss_reach / span) as f32;
    let bars = changes
        .iter()
        .map(|change| {
            let length = ((change.abs() / span) as f32).max(0.008);
            Bar {
                start: if *change >= 0.0 { zero } else { zero - length },
                length,
            }
        })
        .collect();
    (zero, bars)
}

/// How many rows fit `height`, leaving a footer when some are cut or some are unpriced.
pub(super) fn visible_rows(height: f32, total: usize, has_note: bool) -> usize {
    let all = (height / ROW_HEIGHT).floor().max(0.0) as usize;
    if total <= all && !has_note {
        return total;
    }
    let with_footer = ((height - FOOTER_HEIGHT) / ROW_HEIGHT).floor().max(0.0) as usize;
    total.min(with_footer)
}

/// "Best BTC +12.4%" beside the range switch.
pub(super) fn paint_summary(ui: &Ui, row: Rect, card: &PortfolioCard, data: &CardData) {
    let Some(best) = data.performance.ranked.first().filter(|_| !data.loading) else {
        let galley = kit::galley(
            ui,
            &format!("Over {}", card.range.period_name()),
            12.0,
            kit::MEDIUM,
            theme::text_secondary(),
            row.width(),
        );
        ui.painter().galley(
            Pos2::new(row.left(), row.center().y - galley.size().y / 2.0),
            galley,
            Color32::PLACEHOLDER,
        );
        return;
    };
    let color = tone_color(Some(if best.change >= 0.0 {
        crate::model::portfolio::format::Tone::Gain
    } else {
        crate::model::portfolio::format::Tone::Loss
    }));
    let label = kit::galley(
        ui,
        &format!("Best · {}", best.ticker),
        12.0,
        kit::SEMIBOLD,
        theme::text_primary(),
        (row.width() * 0.5).max(40.0),
    );
    ui.painter().galley(
        Pos2::new(row.left(), row.center().y - label.size().y / 2.0),
        label.clone(),
        Color32::PLACEHOLDER,
    );
    let remaining = row.width() - label.size().x - 10.0;
    if remaining > 56.0 {
        chip(
            ui,
            Pos2::new(row.left() + label.size().x + 10.0, row.center().y),
            &data.formatter.signed_percent(best.change, 2),
            color,
            remaining,
        );
    }
}

pub(super) fn paint(
    ui: &Ui,
    area: Rect,
    _id: Id,
    card: &PortfolioCard,
    data: &CardData,
    pointer: Option<Pos2>,
) {
    let performance = &data.performance;
    if data.loading || (data.loading_performance && performance.ranked.is_empty()) {
        paint_skeleton(ui, area);
        let message = if data.loading {
            "Loading market data…"
        } else {
            "Reading price history…"
        };
        loading_pill(ui, area, message);
        return;
    }
    if data.holdings.is_empty() {
        empty_message(
            ui,
            area,
            Icon::BarChart,
            "No Holdings",
            "Add transactions to see how your assets perform.",
        );
        return;
    }
    if performance.ranked.is_empty() {
        empty_message(
            ui,
            area,
            Icon::BarChart,
            "No Price History",
            "None of the holdings has price history for this range.",
        );
        return;
    }

    let painter = ui.painter_at(area);
    let ranked = &performance.ranked;
    let has_note = performance.unpriced > 0;
    let rows = visible_rows(area.height(), ranked.len(), has_note).max(1);
    let shown = &ranked[..rows.min(ranked.len())];
    let changes: Vec<f64> = shown.iter().map(|performer| performer.change).collect();
    let (zero, bars) = bars(&changes);

    let wide = area.width() >= 330.0;
    let ticker_column = if wide { TICKER_COLUMN } else { 58.0 };
    let bar_left = area.left() + 12.0 + ICON_SIZE + 8.0 + ticker_column;
    let bar_right = area.right() - 12.0 - RIGHT_COLUMN - 10.0;
    let zone = Rect::from_min_max(
        Pos2::new(bar_left, area.top()),
        Pos2::new(bar_right.max(bar_left), area.bottom()),
    );
    let show_bars = zone.width() >= MIN_BAR_ZONE;
    let hovered = pointer.and_then(|p| {
        (area.contains(p) && p.y >= area.top()).then(|| ((p.y - area.top()) / ROW_HEIGHT) as usize)
    });

    if show_bars {
        // The zero line, behind the rows.
        let x = zone.left() + zone.width() * zero;
        painter.line_segment(
            [
                Pos2::new(x, area.top() + 4.0),
                Pos2::new(x, area.top() + rows as f32 * ROW_HEIGHT - 4.0),
            ],
            eframe::egui::Stroke::new(1.0, theme::separator()),
        );
    }

    for (index, performer) in shown.iter().enumerate() {
        let row = Rect::from_min_size(
            Pos2::new(area.left() + 4.0, area.top() + index as f32 * ROW_HEIGHT),
            Vec2::new(area.width() - 8.0, ROW_HEIGHT),
        );
        let active = hovered == Some(index);
        if active {
            painter.rect_filled(row.shrink2(Vec2::new(0.0, 2.0)), 8.0, theme::ink(10));
        }
        let holding = data
            .holdings
            .iter()
            .find(|holding| holding.asset.key == performer.asset_key);
        if let Some(holding) = holding {
            paint_asset_icon(
                ui,
                Rect::from_center_size(
                    Pos2::new(area.left() + 12.0 + ICON_SIZE / 2.0, row.center().y),
                    Vec2::splat(ICON_SIZE),
                ),
                &holding.asset,
            );
        }
        let tone = data.formatter.tone(Some(performer.change));
        let color = tone_color(tone);
        let ticker = kit::galley(
            ui,
            &performer.ticker,
            12.5,
            kit::SEMIBOLD,
            theme::text_primary(),
            ticker_column,
        );
        let text_left = area.left() + 12.0 + ICON_SIZE + 8.0;
        let name = holding.map(|holding| {
            if holding.asset.name.is_empty() {
                holding
                    .asset
                    .embedded_name()
                    .unwrap_or(&holding.asset.symbol)
                    .to_owned()
            } else {
                holding.asset.name.clone()
            }
        });
        let detail = name
            .filter(|name| wide && !name.eq_ignore_ascii_case(&performer.ticker))
            .map(|name| {
                kit::galley(
                    ui,
                    &name,
                    10.0,
                    kit::REGULAR,
                    theme::text_secondary(),
                    ticker_column,
                )
            });
        let block = ticker.size().y + detail.as_ref().map_or(0.0, |d| d.size().y);
        let top = row.center().y - block / 2.0;
        painter.galley(
            Pos2::new(text_left, top),
            ticker.clone(),
            Color32::PLACEHOLDER,
        );
        if let Some(detail) = detail {
            painter.galley(
                Pos2::new(text_left, top + ticker.size().y),
                detail,
                Color32::PLACEHOLDER,
            );
        }

        if show_bars {
            let bar = bars[index];
            let track = Rect::from_min_size(
                Pos2::new(zone.left(), row.center().y - 4.0),
                Vec2::new(zone.width(), 8.0),
            );
            painter.rect_filled(track, 4.0, theme::ink(9));
            let fill = Rect::from_min_size(
                Pos2::new(zone.left() + zone.width() * bar.start, track.top()),
                Vec2::new((zone.width() * bar.length).max(2.0), 8.0),
            );
            painter.rect_filled(
                fill,
                4.0,
                color.gamma_multiply(if active { 1.0 } else { 0.85 }),
            );
        }

        let percent = kit::galley(
            ui,
            &data.formatter.signed_percent(performer.change, 2),
            12.5,
            kit::SEMIBOLD,
            color,
            RIGHT_COLUMN,
        );
        let amount = performer.amount.map(|amount| {
            kit::galley(
                ui,
                &data.formatter.signed_money(amount),
                10.0,
                kit::REGULAR,
                theme::text_secondary(),
                RIGHT_COLUMN,
            )
        });
        let right = area.right() - 12.0;
        let block = percent.size().y + amount.as_ref().map_or(0.0, |a| a.size().y);
        let top = row.center().y - block / 2.0;
        painter.galley(
            Pos2::new(right - percent.size().x, top),
            percent.clone(),
            Color32::PLACEHOLDER,
        );
        if let Some(amount) = amount {
            painter.galley(
                Pos2::new(right - amount.size().x, top + percent.size().y),
                amount,
                Color32::PLACEHOLDER,
            );
        }
    }

    // Footer: what was cut and what could not be ranked.
    let hidden = ranked.len() - shown.len();
    if hidden > 0 || has_note {
        let mut parts = Vec::new();
        if hidden > 0 {
            parts.push(format!("+{hidden} more"));
        }
        if has_note {
            parts.push(format!("{} unpriced", performance.unpriced));
        }
        let galley = kit::galley(
            ui,
            &parts.join("  ·  "),
            10.5,
            kit::REGULAR,
            theme::text_tertiary(),
            area.width() - 24.0,
        );
        painter.galley(
            Pos2::new(
                area.left() + 14.0,
                area.bottom() - FOOTER_HEIGHT / 2.0 - galley.size().y / 2.0 - 2.0,
            ),
            galley,
            Color32::PLACEHOLDER,
        );
    }

    if let (Some(index), Some(pointer)) = (hovered, pointer)
        && let Some(performer) = shown.get(index)
    {
        let color = tone_color(data.formatter.tone(Some(performer.change)));
        tooltip(
            ui,
            Pos2::new(pointer.x, area.top() + index as f32 * ROW_HEIGHT),
            area,
            &[
                (
                    format!(
                        "{} · {}",
                        performer.ticker,
                        data.formatter.signed_percent(performer.change, 2)
                    ),
                    13.0,
                    kit::SEMIBOLD,
                    color,
                ),
                (
                    format!(
                        "{} → {}",
                        crate::format::price(performer.start_price),
                        crate::format::price(performer.price)
                    ),
                    11.0,
                    kit::REGULAR,
                    theme::text_secondary(),
                ),
                (
                    format!("over {}", card.range.period_name()),
                    10.5,
                    kit::REGULAR,
                    theme::text_tertiary(),
                ),
            ],
        );
    }
}

/// Pulsing placeholder rows while the first prices arrive.
fn paint_skeleton(ui: &Ui, area: Rect) {
    let time = ui.ctx().input(|input| input.time);
    let pulse = 0.5 + 0.5 * (time * 3.0).sin() as f32;
    ui.ctx().request_repaint();
    let rows = ((area.height() / ROW_HEIGHT).floor() as usize).clamp(1, 6);
    for index in 0..rows {
        let y = area.top() + index as f32 * ROW_HEIGHT + ROW_HEIGHT / 2.0;
        let alpha = (0.05 + 0.05 * pulse) * (1.0 - index as f32 * 0.12);
        let tint = Color32::from_white_alpha((alpha * 255.0) as u8);
        let painter = ui.painter_at(area);
        painter.circle_filled(Pos2::new(area.left() + 24.0, y), ICON_SIZE / 2.0, tint);
        painter.rect_filled(
            Rect::from_min_size(
                Pos2::new(area.left() + 48.0, y - 5.0),
                Vec2::new(60.0, 10.0),
            ),
            5.0,
            tint,
        );
        painter.rect_filled(
            Rect::from_min_size(
                Pos2::new(area.left() + 120.0, y - 4.0),
                Vec2::new(
                    (area.width() - 220.0).max(20.0) * (1.0 - index as f32 * 0.12),
                    8.0,
                ),
            ),
            4.0,
            tint,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gains_grow_right_and_losses_left_from_a_shared_zero() {
        let (zero, bars) = bars(&[0.30, 0.10, -0.10]);
        // Reaches are 0.30 and 0.10, so zero sits a quarter of the way in.
        assert!((zero - 0.25).abs() < 1e-6);
        assert!((bars[0].start - 0.25).abs() < 1e-6);
        assert!((bars[0].length - 0.75).abs() < 1e-6);
        assert!((bars[2].start - 0.0).abs() < 1e-6);
        assert!((bars[2].length - 0.25).abs() < 1e-6);
    }

    #[test]
    fn all_gains_start_at_the_left_edge() {
        let (zero, bars) = bars(&[0.2, 0.1]);
        assert_eq!(zero, 0.0);
        assert!((bars[0].length - 1.0).abs() < 1e-6);
    }

    #[test]
    fn a_flat_move_keeps_a_sliver_and_nothing_divides_by_zero() {
        let (zero, bars) = bars(&[0.0, 0.0]);
        assert_eq!(zero, 0.0);
        assert!(bars.iter().all(|bar| bar.length > 0.0 && bar.length < 0.05));
        assert!(super::bars(&[]).1.is_empty());
    }

    #[test]
    fn a_footer_costs_a_row() {
        assert_eq!(visible_rows(400.0, 5, false), 5);
        assert_eq!(visible_rows(400.0, 5, true), 5);
        assert_eq!(visible_rows(150.0, 5, false), 3);
        assert_eq!(visible_rows(150.0, 5, true), 3);
        assert_eq!(visible_rows(200.0, 6, false), 4);
        assert_eq!(visible_rows(0.0, 5, false), 0);
    }
}
