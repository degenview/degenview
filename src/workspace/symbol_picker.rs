//! The symbol picker on a workspace: opening it on the focused chart, keeping the highlighted
//! market priced while it is open, and what a pick does to the charts.

use super::{ChartWorkspace, WorkspaceAction};
use crate::model::InstrumentId;
use crate::model::watchlist::WatchlistInstrument;
use crate::service::SearchResult;
use crate::ui::symbol_picker::{self, Env, Output};
use eframe::egui;

impl ChartWorkspace {
    /// Opens the picker searching for the focused chart's market (nothing with no chart).
    pub(super) fn open_symbol_picker(&mut self) {
        let current = self
            .focused_market_chart()
            .map(super::chart::Chart::picker_query)
            .unwrap_or_default();
        self.symbol_picker.open(&current);
    }

    /// Opens the picker to compare markets on the focused chart. Does nothing for a chart that
    /// cannot take another market (a line-only source, a price-based chart type).
    pub(super) fn open_compare_picker(&mut self) {
        if let Some(chart) = self.focused_market_chart()
            && self.can_compare(chart.id)
        {
            self.symbol_picker.open_compare();
        }
    }

    /// Draws the picker while it is open and applies what it picked.
    pub(super) fn show_symbol_picker(&mut self, ctx: &egui::Context) -> Vec<WorkspaceAction> {
        let owner = symbol_picker::quote_owner(self.id);
        if !self.symbol_picker.is_open() {
            return Vec::new();
        }
        if self.symbol_picker.wants_watchlists()
            && let Ok(store) = self.watchlist.lock()
        {
            self.symbol_picker
                .set_watchlists(symbol_picker::watchlists::groups(store.lists()));
        }
        // Only the market under the highlight is priced.
        let highlighted = self.symbol_picker.highlighted();
        self.quote_coordinator
            .set_demand(owner, highlighted.iter().cloned().collect(), true);
        let quote = highlighted
            .as_ref()
            .and_then(|market| self.quote_book.get(market));
        let on_tab: Vec<InstrumentId> = self
            .charts
            .iter()
            .filter(|chart| chart.is_market())
            .map(Self::chart_instrument)
            .collect();
        let replacing = self
            .focused_market_chart()
            .map(super::chart::Chart::display_title);
        // The focused chart's own market and what is compared on it, which cannot be drawn again.
        let overlaid: Vec<InstrumentId> = self
            .focused_market_chart()
            .map(|chart| {
                std::iter::once(Self::chart_instrument(chart))
                    .chain(chart.overlays.instruments().cloned())
                    .collect()
            })
            .unwrap_or_default();
        let can_overlay = self
            .focused_market_chart()
            .is_some_and(|chart| self.can_compare(chart.id));
        let output = self.symbol_picker.show(
            ctx,
            &Env {
                on_tab: &on_tab,
                quote: quote.as_ref(),
                replacing: replacing.as_deref(),
                overlaid: &overlaid,
                can_overlay,
            },
        );
        let mut actions = Vec::new();
        match output {
            Some(Output::Replace(result)) => self.replace_focused_market(&result, ctx),
            Some(Output::Add(result)) => self.add_from_search(&result, ctx),
            Some(Output::Overlay(result)) => self.overlay_from_search(&result),
            Some(Output::OpenAlpacaSettings) => actions.push(WorkspaceAction::OpenAlpacaSettings),
            None => {}
        }
        if !self.symbol_picker.is_open() {
            self.quote_coordinator.release(owner);
        }
        actions
    }

    /// Draws `result` over the focused chart.
    pub(super) fn overlay_from_search(&mut self, result: &SearchResult) {
        let Some(chart_id) = self.focused_market_chart().map(|chart| chart.id) else {
            return;
        };
        let instrument = InstrumentId::new(result.source, &result.full_symbol);
        // A DEX pair is named by its pair, not the address it is fetched by.
        let title =
            (result.source == crate::model::DataSource::DexScreener).then(|| result.symbol.clone());
        self.add_overlay(chart_id, instrument, title);
    }

    /// Shows `result` on the focused market chart, keeping the chart's settings, indicators and
    /// drawings; with no market chart focused it adds one instead.
    pub(super) fn replace_focused_market(&mut self, result: &SearchResult, ctx: &egui::Context) {
        let Some(chart_id) = self.focused_market_chart().map(|chart| chart.id) else {
            self.add_from_search(result, ctx);
            return;
        };
        let item =
            WatchlistInstrument::from_market(result.source, &result.symbol, &result.full_symbol);
        self.switch_chart(chart_id, &item, ctx);
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::{chart_with_drawings, empty_workspace};
    use crate::model::DataSource;
    use crate::service::SearchResult;
    use eframe::egui;

    fn pair(source: DataSource, symbol: &str, base: &str, quote: &str) -> SearchResult {
        SearchResult::new(source, symbol, symbol).pair(base, quote)
    }

    #[test]
    fn the_picker_opens_searching_for_the_focused_charts_market() {
        let mut workspace = empty_workspace(1);
        workspace.open_symbol_picker();
        assert!(workspace.symbol_picker.is_open());
        assert_eq!(workspace.symbol_picker.query(), "");

        let btc = chart_with_drawings(&mut workspace, "BTCUSDT", &[]);
        let eth = chart_with_drawings(&mut workspace, "ETHUSDT", &[]);
        workspace.focused = Some(btc);
        workspace.open_symbol_picker();
        assert_eq!(workspace.symbol_picker.query(), "BTCUSDT");
        workspace.focused = Some(eth);
        workspace.open_symbol_picker();
        assert_eq!(workspace.symbol_picker.query(), "ETHUSDT");
    }

    #[test]
    fn a_pick_replaces_the_focused_chart_alone() {
        let ctx = egui::Context::default();
        let mut workspace = empty_workspace(1);
        let first = chart_with_drawings(&mut workspace, "BTCUSDT", &[]);
        let second = chart_with_drawings(&mut workspace, "ETHUSDT", &[]);
        workspace.focused = Some(second);
        workspace
            .replace_focused_market(&pair(DataSource::Coinbase, "SOL-USD", "SOL", "USD"), &ctx);

        let market = |id: u64| {
            let chart = workspace
                .charts
                .iter()
                .find(|chart| chart.id == id)
                .unwrap();
            (chart.source, chart.symbol.clone())
        };
        assert_eq!(market(second), (DataSource::Coinbase, "SOL-USD".to_owned()));
        assert_eq!(market(first), (DataSource::Binance, "BTCUSDT".to_owned()));
        assert_eq!(workspace.chart_count(), 2);
        assert_eq!(workspace.focused, Some(second));
    }

    #[test]
    fn a_pick_with_no_chart_adds_one() {
        let ctx = egui::Context::default();
        let mut workspace = empty_workspace(1);
        workspace
            .replace_focused_market(&pair(DataSource::Binance, "BTCUSDT", "BTC", "USDT"), &ctx);
        assert_eq!(workspace.chart_count(), 1);
    }

    #[test]
    fn a_dex_pick_keeps_its_pair_name_as_the_title() {
        let ctx = egui::Context::default();
        let mut workspace = empty_workspace(1);
        let chart = chart_with_drawings(&mut workspace, "BTCUSDT", &[]);
        workspace.focused = Some(chart);
        let dex =
            SearchResult::new(DataSource::DexScreener, "PEPE/WETH", "0xpair").pair("PEPE", "WETH");
        workspace.replace_focused_market(&dex, &ctx);
        let chart = workspace.charts.iter().find(|c| c.id == chart).unwrap();
        assert_eq!(chart.display_title(), "PEPE/WETH");
        assert_eq!(chart.symbol, "0xpair");
    }
}
