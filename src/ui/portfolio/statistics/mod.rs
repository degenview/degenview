//! The Portfolio's Statistics tab: performance, value range, activity and P&L by asset. A pure
//! view over the statistics the host computes.

use crate::model::portfolio::accounting::ClosedPosition;
use crate::model::portfolio::format::PortfolioValueFormatter;
use crate::model::portfolio::statistics::PortfolioStatistics;
use crate::model::portfolio::{PortfolioHolding, PortfolioSnapshot, PortfolioTransaction};
use crate::ui::icons::Icon;
use crate::ui::kit::time_text::date_text;
use crate::ui::portfolio::blocks::{
    CARD_HEIGHT, GRID_GAP, PAGE_INSET, SECTION_GAP, SIDE_WIDTH, StatCard, WIDE_MIN, grid,
    panel_height, split,
};
use eframe::egui::{self, Margin, Ui, vec2};
use hero::statistics_hero;
use pnl::pnl_panel;
use range::range_strip;
use win_rate::win_panel;

mod hero;
mod pnl;
mod range;
mod win_rate;

pub struct StatisticsInput<'a> {
    pub holdings: &'a [PortfolioHolding],
    /// Positions closed completely; their realized profit still counts.
    pub closed: &'a [ClosedPosition],
    pub history: &'a [PortfolioSnapshot],
    /// The converted transactions, for fees and activity.
    pub transactions: &'a [PortfolioTransaction],
    pub formatter: PortfolioValueFormatter,
    pub now_ms: i64,
    pub utc_offset_secs: i64,
}

/// "3 buys · 1 sell".
pub fn activity_caption(buys: usize, sells: usize) -> String {
    format!(
        "{buys} {} · {sells} {}",
        if buys == 1 { "buy" } else { "buys" },
        if sells == 1 { "sell" } else { "sells" }
    )
}

/// "Tracking since Jan 15, 2024" caption: how long ago, in words.
pub fn tracking_caption(since_ms: i64, now_ms: i64) -> String {
    let days = ((now_ms - since_ms) / 86_400_000).max(0);
    match days {
        0 => "today".to_owned(),
        1 => "1 day ago".to_owned(),
        2..=59 => format!("{days} days ago"),
        60..=729 => format!("{} months ago", days / 30),
        _ => format!("{} years ago", days / 365),
    }
}

/// The Win Rate panel's content height at least: beside the P&L panel it grows so the side column
/// (the panel over the activity cards) ends level with the bars.
const WIN_CONTENT: f32 = 156.0;
const RANGE_HEIGHT: f32 = 96.0;
/// The P&L rows share their panel's height, kept within these.
const PNL_ROW_MIN: f32 = 30.0;
const PNL_ROW_MAX: f32 = 54.0;
/// Below this row width the P&L percentage is dropped, so the bars keep some room.
const PNL_COMPACT: f32 = 520.0;

pub fn show_statistics(ui: &mut egui::Ui, input: &StatisticsInput) {
    let f = input.formatter;
    let stats = PortfolioStatistics::with_closed(
        input.holdings,
        input.closed,
        input.history,
        input.transactions,
    );
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            egui::Frame::new()
                .inner_margin(Margin::same(PAGE_INSET))
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing = vec2(GRID_GAP, SECTION_GAP);
                    let width = ui.available_width();
                    statistics_hero(ui, f, &stats);
                    value_cards(ui, input, &stats);
                    range_strip(ui, input, &stats);
                    let rows_height = stats.asset_results.len().max(1) as f32 * PNL_ROW_MIN;
                    if width >= WIDE_MIN {
                        // Results on the left. Beside them the win rate and, under it, the four
                        // activity cards two across; both columns end level, as tall as the
                        // longer of the bars and the side column needs.
                        let cards = 2.0 * CARD_HEIGHT + GRID_GAP;
                        let total = panel_height(rows_height)
                            .max(panel_height(WIN_CONTENT) + SECTION_GAP + cards);
                        split(
                            ui,
                            [width - SIDE_WIDTH - SECTION_GAP, SIDE_WIDTH],
                            SECTION_GAP,
                            &mut (),
                            |ui, _| {
                                pnl_panel(ui, &stats.asset_results, f, total - panel_height(0.0))
                            },
                            |ui, _| {
                                let win = total - cards - SECTION_GAP - panel_height(0.0);
                                win_panel(ui, &stats, win);
                                ui.scope(|ui| {
                                    ui.spacing_mut().item_spacing.y = GRID_GAP;
                                    activity_cards(ui, input, &stats);
                                });
                            },
                        );
                    } else {
                        win_panel(ui, &stats, WIN_CONTENT);
                        pnl_panel(ui, &stats.asset_results, f, rows_height.max(WIN_CONTENT));
                        activity_cards(ui, input, &stats);
                    }
                });
        });
}

fn value_cards(ui: &mut Ui, input: &StatisticsInput, stats: &PortfolioStatistics) {
    let f = input.formatter;
    let on = |ms: i64| date_text(ms + input.utc_offset_secs * 1_000);
    grid(
        ui,
        &[
            StatCard::new("Current Value", f.money(stats.total_value)).icon(Icon::Briefcase),
            StatCard::new("Cost Basis", f.money(stats.cost_basis)).icon(Icon::Banknote),
            StatCard::new(
                "Portfolio High",
                stats.high.map_or("—".to_owned(), |e| f.money(e.value)),
            )
            .caption(stats.high.map(|e| on(e.timestamp)))
            .placeholder(stats.high.is_none())
            .icon(Icon::ArrowUp),
            StatCard::new(
                "Portfolio Low",
                stats.low.map_or("—".to_owned(), |e| f.money(e.value)),
            )
            .caption(stats.low.map(|e| on(e.timestamp)))
            .placeholder(stats.low.is_none())
            .icon(Icon::ArrowDown),
        ],
    );
}

fn activity_cards(ui: &mut Ui, input: &StatisticsInput, stats: &PortfolioStatistics) {
    let f = input.formatter;
    let on = |ms: i64| date_text(ms + input.utc_offset_secs * 1_000);
    grid(
        ui,
        &[
            StatCard::new("Transactions", stats.transaction_count.to_string())
                .caption(Some(activity_caption(stats.buy_count, stats.sell_count)))
                .icon(Icon::Exchange),
            StatCard::new("Assets", stats.asset_count.to_string())
                .caption(Some(format!(
                    "{} in profit · {} at a loss",
                    stats.winners, stats.losers
                )))
                .icon(Icon::Layers),
            StatCard::new("Fees Paid", f.money(stats.fees_paid)).icon(Icon::Percent),
            StatCard::new(
                "Tracking Since",
                stats.first_transaction.map_or("—".to_owned(), on),
            )
            .caption(
                stats
                    .first_transaction
                    .map(|t| tracking_caption(t, input.now_ms)),
            )
            .placeholder(stats.first_transaction.is_none())
            .icon(Icon::List),
        ],
    );
}

#[cfg(test)]
mod tests;
