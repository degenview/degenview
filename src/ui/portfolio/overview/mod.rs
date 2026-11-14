//! The portfolio's front page: what it is worth, how it has moved, and where the money sits.
//! Pure views over the holdings, history and statistics the host computes.
//!
//! Every block here has a fixed height and paints its text at measured positions, so nothing
//! reflows while prices stream in and cards in one row always end level.

use crate::model::portfolio::format::PortfolioValueFormatter;
use crate::model::portfolio::statistics::{
    PortfolioHistoryRange, PortfolioPeriodChange, PortfolioStatistics,
};
use crate::model::portfolio::{PortfolioAsset, PortfolioHolding, PortfolioSnapshot};
use crate::ui::icons::Icon;
use crate::ui::kit;
use crate::ui::portfolio::blocks::{
    CARD_HEIGHT, GRID_GAP, PAGE_INSET, SECTION_GAP, SIDE_WIDTH, WIDE_MIN, panel, panel_height,
    split,
};
use crate::ui::portfolio::charts;
use eframe::egui::{self, Color32, Margin, Ui, vec2};
use hero::hero;
use history::history_card;
use summary::{performer_card, summary_cards};
use top_holdings::top_holdings_panel;

pub(super) mod hero;
mod history;
mod summary;
mod top_holdings;

/// How many holdings the "Top Holdings" panel lists.
pub const TOP_COUNT: usize = 6;

/// The value chart's content height beside the allocation, a little taller than when stacked.
const WIDE_CHART_HEIGHT: f32 = 300.0;
const HISTORY_CHART_HEIGHT: f32 = 250.0;
/// Both lower panels size their content to this, so they always end level: six Top Holdings
/// rows fill it exactly.
const PANEL_CONTENT_HEIGHT: f32 = 270.0;
const TOP_ROW_HEIGHT: f32 = 43.0;
const TOP_ROW_GAP: f32 = 2.0;

#[derive(Clone, Debug, PartialEq)]
pub enum OverviewAction {
    /// Open this asset's transactions.
    Select(String),
    /// "View All" in Top Holdings.
    ShowHoldings,
}

pub struct OverviewInput<'a> {
    /// `None` is "All Portfolios".
    pub portfolio_name: Option<&'a str>,
    pub holdings: &'a [PortfolioHolding],
    /// The value history for the selected portfolio (intraday for 1D when there is some).
    pub history: &'a [PortfolioSnapshot],
    pub total_value: f64,
    /// Money put in less money taken out, to today, in the reporting currency.
    pub net_contributions: f64,
    pub formatter: PortfolioValueFormatter,
    pub subtitle: &'a dyn Fn(&PortfolioAsset) -> String,
    /// "Prices as of 14:03" and the like.
    pub market_value_caption: &'a str,
    pub price_status: PriceStatus,
    pub is_loading_initial: bool,
    pub is_loading_history: bool,
    pub now_ms: i64,
    pub utc_offset_secs: i64,
}

#[derive(Default)]
pub struct OverviewState {
    pub range: PortfolioHistoryRange,
    hovered_slice: Option<usize>,
}

/// The top holdings by value, largest first; unpriced ones last. Equal values keep a fixed
/// order (by market), so rows never swap places between frames.
pub fn top_holdings(holdings: &[PortfolioHolding]) -> Vec<&PortfolioHolding> {
    let mut ranked: Vec<&PortfolioHolding> = holdings.iter().collect();
    ranked.sort_by(|a, b| {
        b.current_value
            .unwrap_or(0.0)
            .total_cmp(&a.current_value.unwrap_or(0.0))
            .then_with(|| a.asset.key.cmp(&b.asset.key))
    });
    ranked.truncate(TOP_COUNT);
    ranked
}

/// "Portfolio · 3 assets".
pub fn asset_count_text(count: usize) -> String {
    format!("{count} {}", if count == 1 { "asset" } else { "assets" })
}

/// How current the prices behind the total are; colours the dot beside the caption.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PriceStatus {
    /// Every holding has a live price.
    #[default]
    Live,
    /// Some price is a cached one.
    Cached,
    /// Some holding has no price.
    Partial,
    /// The valuation failed.
    Unavailable,
}

impl PriceStatus {
    fn color(self) -> Color32 {
        match self {
            Self::Live => kit::GREEN,
            Self::Cached | Self::Partial => kit::ORANGE,
            Self::Unavailable => kit::RED,
        }
    }
}

pub fn show_overview(
    ui: &mut egui::Ui,
    state: &mut OverviewState,
    input: &OverviewInput,
) -> Vec<OverviewAction> {
    let mut actions = Vec::new();
    let stats = PortfolioStatistics::new(input.holdings, input.history, &[]);
    let ranged = state.range.filter(input.history, input.now_ms);
    let period = PortfolioPeriodChange::new(&ranged, input.total_value, input.net_contributions);

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            egui::Frame::new()
                .inner_margin(Margin::same(PAGE_INSET))
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing = vec2(GRID_GAP, SECTION_GAP);
                    let wide = ui.available_width() >= WIDE_MIN;
                    hero(ui, state, input, &stats, period);
                    summary_cards(ui, input.formatter, &stats);
                    if wide {
                        let width = ui.available_width();
                        let widths = [width - SIDE_WIDTH - SECTION_GAP, SIDE_WIDTH];
                        split(
                            ui,
                            widths,
                            SECTION_GAP,
                            state,
                            |ui, state| history_card(ui, state, input, WIDE_CHART_HEIGHT),
                            |ui, state| allocation_panel(ui, state, input, WIDE_CHART_HEIGHT),
                        );
                        split(
                            ui,
                            widths,
                            SECTION_GAP,
                            &mut actions,
                            |ui, actions| top_holdings_panel(ui, input, actions),
                            |ui, _| {
                                ui.spacing_mut().item_spacing.y = GRID_GAP;
                                let each = (panel_height(PANEL_CONTENT_HEIGHT) - GRID_GAP) / 2.0;
                                performer_card(
                                    ui,
                                    "Best Performer",
                                    stats.best.as_ref(),
                                    input,
                                    each,
                                );
                                performer_card(
                                    ui,
                                    "Worst Performer",
                                    stats.worst.as_ref(),
                                    input,
                                    each,
                                );
                            },
                        );
                    } else {
                        ui.columns(2, |columns| {
                            performer_card(
                                &mut columns[0],
                                "Best Performer",
                                stats.best.as_ref(),
                                input,
                                CARD_HEIGHT,
                            );
                            performer_card(
                                &mut columns[1],
                                "Worst Performer",
                                stats.worst.as_ref(),
                                input,
                                CARD_HEIGHT,
                            );
                        });
                        history_card(ui, state, input, HISTORY_CHART_HEIGHT);
                        ui.spacing_mut().item_spacing.x = SECTION_GAP;
                        ui.columns(2, |columns| {
                            allocation_panel(&mut columns[0], state, input, PANEL_CONTENT_HEIGHT);
                            top_holdings_panel(&mut columns[1], input, &mut actions);
                        });
                    }
                });
        });
    actions
}

fn allocation_panel(
    ui: &mut Ui,
    state: &mut OverviewState,
    input: &OverviewInput,
    content_height: f32,
) {
    panel(ui, content_height, Icon::PieChart, "Allocation", |_, ui| {
        charts::show_allocation(
            ui,
            input.holdings,
            input.formatter,
            &mut state.hovered_slice,
            content_height,
        );
    });
}

#[cfg(test)]
mod tests;
