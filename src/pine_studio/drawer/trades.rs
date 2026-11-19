//! The Report tab's closed trades: one row each, newest first, in aligned columns (number, side,
//! entry to exit, how long it was held, what it made). Hovering a row says when it ran, its orders
//! and their comments.

use super::tab_chrome::{self as chrome, card, hairline, panel};
use crate::pine::strategy::{StrategyReport, Trade};
use crate::ui::icons::{self, Icon};
use crate::ui::pine::report::{self, PineReportFormat};
use crate::ui::{kit, theme};
use eframe::egui::{Color32, Galley, Rect, Sense, Ui, Vec2, pos2, vec2};
use std::sync::Arc;

/// The most trades listed; the rest are counted under the list.
const TRADE_LIMIT: usize = 100;
const ROW_HEIGHT: f32 = 30.0;
const PAD_X: f32 = 12.0;
/// The space between two columns.
const GAP: f32 = 12.0;
const SIDE_ICON: f32 = 10.0;
/// The narrowest the prices may get before the held column, then the prices, give way.
const MIN_PRICES: f32 = 64.0;

/// The trades card, or a word on where trades will show while there are none.
pub(super) fn trades_card(ui: &mut Ui, report: &StrategyReport) {
    if report.trades.is_empty() {
        chrome::caption(ui, "Trades");
        panel(ui, |ui| {
            ui.label(kit::text(
                "No closed trades yet",
                kit::size::CALLOUT,
                kit::MEDIUM,
                theme::text_secondary(),
            ));
            ui.add_space(3.0);
            ui.label(kit::text(
                "Each trade is listed here once the strategy closes it.",
                kit::size::CAPTION,
                kit::REGULAR,
                theme::text_tertiary(),
            ));
        });
        return;
    }
    chrome::caption(ui, &format!("Trades ({})", report.trades.len()));
    let shown: Vec<&Trade> = report.trades.iter().rev().take(TRADE_LIMIT).collect();
    let columns = Columns::measure(ui, &shown);
    card(ui, |ui| {
        for (index, trade) in shown.into_iter().enumerate() {
            if index > 0 {
                hairline(ui);
            }
            trade_row(ui, trade, &columns);
        }
    });
    if report.trades.len() > TRADE_LIMIT {
        chrome::footnote(
            ui,
            &format!(
                "Showing the latest {TRADE_LIMIT} of {}",
                report.trades.len()
            ),
        );
    }
}

/// "1 bar", "12 bars": how long a trade was held.
pub(super) fn held(trade: &Trade) -> String {
    let bars = trade.exit_bar.saturating_sub(trade.entry_bar);
    format!("{bars} bar{}", if bars == 1 { "" } else { "s" })
}

/// What hovering a trade says: when it opened and closed, then its orders, size and comments.
fn hover_text(trade: &Trade) -> String {
    let summary = report::trade_summary(trade);
    if trade.entry_time > 0 && trade.exit_time > 0 {
        format!(
            "{} \u{2192} {}\n{summary}",
            super::alert_time(trade.entry_time),
            super::alert_time(trade.exit_time)
        )
    } else {
        summary
    }
}

/// The widths the rows share, so their columns line up.
struct Columns {
    number: f32,
    side: f32,
    held: f32,
    profit: f32,
}

impl Columns {
    fn measure(ui: &Ui, trades: &[&Trade]) -> Self {
        let width = |text: &str, size: f32, weight: f32| {
            kit::digits(ui, text, size, weight, Color32::PLACEHOLDER, f32::INFINITY)
                .size()
                .x
        };
        let widest = |text: &dyn Fn(&Trade) -> String, size: f32, weight: f32| {
            trades
                .iter()
                .map(|trade| width(&text(trade), size, weight))
                .fold(0.0, f32::max)
        };
        Self {
            number: widest(&number, kit::size::CAPTION, kit::REGULAR),
            side: SIDE_ICON + 4.0 + width("Short", kit::size::SUBHEADLINE, kit::REGULAR),
            held: widest(&held, kit::size::CAPTION, kit::REGULAR),
            profit: widest(&profit, kit::size::SUBHEADLINE, kit::MEDIUM),
        }
    }
}

fn number(trade: &Trade) -> String {
    format!("#{}", trade.id)
}

fn profit(trade: &Trade) -> String {
    PineReportFormat::signed_money(trade.profit)
}

/// One trade: its number and side, where it went in and out, how long it was held, and what it
/// made. In a narrow card the held column, then the prices, give way to the profit.
fn trade_row(ui: &mut Ui, trade: &Trade, columns: &Columns) {
    let (rect, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), ROW_HEIGHT), Sense::hover());
    if response.hovered() {
        ui.painter()
            .rect_filled(rect.shrink2(vec2(4.0, 1.0)), 6.0, theme::ink(10));
    }
    let painter = ui.painter().with_clip_rect(rect);
    let mid = rect.center().y;
    let paint = |left: f32, galley: Arc<Galley>| {
        let top = mid - galley.size().y / 2.0;
        painter.galley(pos2(left, top), galley, Color32::PLACEHOLDER);
    };
    let caption = |text: &str| {
        kit::digits(
            ui,
            text,
            kit::size::CAPTION,
            kit::REGULAR,
            theme::text_tertiary(),
            f32::INFINITY,
        )
    };

    let number_x = rect.left() + PAD_X;
    paint(number_x, caption(&number(trade)));

    let side_x = number_x + columns.number + GAP;
    let (icon, side) = if trade.is_long {
        (Icon::ArrowUpRight, "Long")
    } else {
        (Icon::ArrowDownRight, "Short")
    };
    icons::paint(
        &painter,
        Rect::from_min_size(pos2(side_x, mid - SIDE_ICON / 2.0), Vec2::splat(SIDE_ICON)),
        icon,
        theme::text_secondary(),
    );
    paint(
        side_x + SIDE_ICON + 4.0,
        kit::galley(
            ui,
            side,
            kit::size::SUBHEADLINE,
            kit::REGULAR,
            theme::text_primary(),
            f32::INFINITY,
        ),
    );

    let profit_x = rect.right() - PAD_X - columns.profit;
    let amount = kit::digits(
        ui,
        &profit(trade),
        kit::size::SUBHEADLINE,
        kit::MEDIUM,
        report::tint(trade.profit, theme::text_primary()),
        f32::INFINITY,
    );
    paint(rect.right() - PAD_X - amount.size().x, amount);

    let prices_x = side_x + columns.side + GAP;
    let held_x = profit_x - GAP - columns.held;
    let with_held = held_x - GAP - prices_x >= MIN_PRICES;
    if with_held {
        let galley = caption(&held(trade));
        paint(held_x + columns.held - galley.size().x, galley);
    }
    let prices_right = if with_held { held_x } else { profit_x } - GAP;
    let room = prices_right - prices_x;
    if room >= MIN_PRICES / 2.0 {
        paint(
            prices_x,
            kit::digits(
                ui,
                &format!(
                    "{} \u{2192} {}",
                    PineReportFormat::price(trade.entry_price),
                    PineReportFormat::price(trade.exit_price)
                ),
                kit::size::SUBHEADLINE,
                kit::REGULAR,
                theme::text_secondary(),
                room,
            ),
        );
    }
    response.on_hover_text(hover_text(trade));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_trade_is_held_for_the_bars_between_its_entry_and_exit() {
        let trade = |entry_bar, exit_bar| Trade {
            entry_bar,
            exit_bar,
            ..Trade::default()
        };
        assert_eq!(held(&trade(10, 10)), "0 bars");
        assert_eq!(held(&trade(10, 11)), "1 bar");
        assert_eq!(held(&trade(10, 22)), "12 bars");
    }

    #[test]
    fn hovering_names_the_times_only_when_the_trade_has_them() {
        let mut trade = Trade {
            entry_id: "Long".into(),
            exit_id: "Exit".into(),
            quantity: 1.0,
            ..Trade::default()
        };
        assert_eq!(hover_text(&trade), "Long \u{2192} Exit \u{b7} qty 1");
        trade.entry_time = 1_700_000_000_000;
        trade.exit_time = 1_700_003_600_000;
        let text = hover_text(&trade);
        assert_eq!(text.lines().count(), 2);
        assert!(text.ends_with("Long \u{2192} Exit \u{b7} qty 1"));
    }
}
