//! The symbol picker on a workspace: opening it on the focused chart, keeping the highlighted
//! market priced while it is open, and what a pick does to the charts.

use crate::host::portfolio::card::PortfolioCardEnv;
use crate::model::InstrumentId;
use crate::model::portfolio::card::PortfolioCard;
use crate::model::watchlist::WatchlistInstrument;
use crate::service::SearchResult;
use crate::ui::chart::symbol_picker::{self, Env, Output};
use crate::workspace::{ChartWorkspace, WorkspaceAction};
use eframe::egui;

impl ChartWorkspace {
    /// Opens the picker searching for the focused chart's market (nothing with no chart).
    pub(in crate::workspace) fn open_symbol_picker(&mut self) {
        let current = self
            .focused_market_chart()
            .map(super::Chart::picker_query)
            .unwrap_or_default();
        self.symbol_picker.open(&current);
    }

    /// Opens the picker to add a chart (the new-tab page's search).
    pub(in crate::workspace) fn open_add_picker(&mut self) {
        self.symbol_picker.open_add();
    }

    /// Opens the picker to compare markets on the focused chart. Does nothing for a chart that
    /// cannot take another market (a line-only source, a price-based chart type).
    pub(in crate::workspace) fn open_compare_picker(&mut self) {
        if let Some(chart) = self.focused_market_chart()
            && self.can_compare(chart.id)
        {
            self.symbol_picker.open_compare();
        }
    }

    /// Draws the picker while it is open and applies what it picked.
    pub(in crate::workspace) fn show_symbol_picker(
        &mut self,
        ctx: &egui::Context,
    ) -> Vec<WorkspaceAction> {
        let owner = symbol_picker::quote_owner(self.id);
        if !self.symbol_picker.is_open() {
            // A watchlist the picker was opened for is only the target while it is open.
            self.add_to_watchlist = None;
            return Vec::new();
        }
        if self.symbol_picker.wants_watchlists()
            && let Ok(store) = self.watchlist.lock()
        {
            self.symbol_picker
                .set_watchlists(symbol_picker::watchlists::groups(store.lists()));
        }
        if self.symbol_picker.wants_portfolios()
            && let Some(env) = &self.card_env
        {
            self.symbol_picker.set_portfolios(env.portfolios());
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
        let cards_on_tab: Vec<PortfolioCard> =
            self.charts.iter().filter_map(|chart| chart.card).collect();
        let replacing = self.focused_market_chart().map(super::Chart::display_title);
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
                cards_on_tab: &cards_on_tab,
            },
        );
        let mut actions = Vec::new();
        match output {
            Some(Output::Replace(result)) => self.replace_focused_market(&result, ctx),
            Some(Output::Add(result)) => self.add_from_search(&result, ctx),
            Some(Output::Overlay(result)) => self.overlay_from_search(&result),
            Some(Output::AddCard(card)) => {
                self.add_card(card);
            }
            Some(Output::Watchlist(result)) => self.add_pick_to_watchlist(&result),
            Some(Output::WatchlistCard(card)) => {
                let scope = self
                    .card_env
                    .as_ref()
                    .map_or_else(Vec::new, PortfolioCardEnv::portfolios);
                self.add_to_picker_watchlist(WatchlistInstrument::for_card(
                    card,
                    &card.scope_name(&scope),
                ));
            }
            Some(Output::OpenAlpacaSettings) => actions.push(WorkspaceAction::OpenAlpacaSettings),
            None => {}
        }
        if !self.symbol_picker.is_open() {
            self.quote_coordinator.release(owner);
        }
        actions
    }

    /// Follows `result` in the watchlist the picker was opened for.
    fn add_pick_to_watchlist(&mut self, result: &SearchResult) {
        let mut item =
            WatchlistInstrument::from_market(result.source, &result.symbol, &result.full_symbol);
        item.image_url = result
            .event
            .as_ref()
            .and_then(|event| event.image_url.clone());
        self.add_to_picker_watchlist(item);
    }

    fn add_to_picker_watchlist(&mut self, item: WatchlistInstrument) {
        let Some(list_id) = self.add_to_watchlist else {
            return;
        };
        if let Ok(mut store) = self.watchlist.lock() {
            let result = store.add_to_list(list_id, item).map(|_| ());
            crate::workspace::watchlist::report_watchlist(&mut self.watchlist_error, result);
        }
    }

    /// Draws `result` over the focused chart.
    pub(in crate::workspace) fn overlay_from_search(&mut self, result: &SearchResult) {
        let Some(chart_id) = self.focused_market_chart().map(|chart| chart.id) else {
            return;
        };
        let instrument = InstrumentId::new(result.source, &result.full_symbol);
        // A DEX pair is named by its pair, not the address it is fetched by.
        let title =
            (result.source == crate::model::DataSource::DexScreener).then(|| result.symbol.clone());
        self.add_overlay(chart_id, instrument, title);
    }

    /// Shows `result` on the focused market chart as one undo step, keeping the chart's settings
    /// and drawings and, on a market that takes them, its indicators; with no market chart
    /// focused it adds one instead. A prediction market takes its event's title and artwork, and
    /// the lines of the choices ticked to chart together.
    pub(in crate::workspace) fn replace_focused_market(
        &mut self,
        result: &SearchResult,
        ctx: &egui::Context,
    ) {
        let Some(chart_id) = self.focused_market_chart().map(|chart| chart.id) else {
            self.add_from_search(result, ctx);
            return;
        };
        let mut item =
            WatchlistInstrument::from_market(result.source, &result.symbol, &result.full_symbol);
        if result.source.is_prediction_market()
            && let Some(event) = &result.event
        {
            if !event.title.is_empty() {
                item.display_name = Some(event.title.clone());
            }
            item.image_url = event.image_url.clone();
        }
        self.replace_chart_market(chart_id, &item, ctx);
        if result
            .event
            .as_ref()
            .is_some_and(|event| event.series.len() > 1)
        {
            self.title_prediction(chart_id, result, ctx);
        }
        self.announce_market(&item);
    }
}

#[cfg(test)]
mod tests {
    use crate::model::DataSource;
    use crate::model::portfolio::card::PortfolioCard;
    use crate::model::watchlist::WatchlistInstrument;
    use crate::service::SearchResult;
    use crate::workspace::ChartWorkspace;
    use crate::workspace::test_support::{chart_with_drawings, empty_workspace};
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

    fn choice(event: &str, label: &str, chance: f64) -> SearchResult {
        SearchResult {
            price: Some(chance),
            event: Some(crate::service::EventInfo {
                title: event.into(),
                image_url: Some("https://example.com/fed.png".into()),
                ..crate::service::EventInfo::default()
            }),
            ..SearchResult::new(DataSource::Polymarket, label, format!("{event}/{label}"))
        }
    }

    fn chart(workspace: &ChartWorkspace, id: u64) -> &crate::workspace::Chart {
        workspace.charts.iter().find(|c| c.id == id).unwrap()
    }

    #[test]
    fn replacing_with_a_prediction_market_keeps_the_events_title_and_artwork() {
        let ctx = egui::Context::default();
        let mut workspace = empty_workspace(1);
        let id = chart_with_drawings(&mut workspace, "BTCUSDT", &[]);
        workspace.focused = Some(id);
        workspace.replace_focused_market(&choice("Fed decision", "Hold", 0.7), &ctx);
        let replaced = chart(&workspace, id);
        assert_eq!(replaced.source, DataSource::Polymarket);
        assert_eq!(replaced.display_title(), "Fed decision");
        assert_eq!(
            replaced.image_url.as_deref(),
            Some("https://example.com/fed.png")
        );
        assert!(replaced.pm_series.is_empty());
    }

    #[test]
    fn replacing_with_ticked_choices_draws_them_as_the_lines_of_the_chart() {
        let ctx = egui::Context::default();
        let mut workspace = empty_workspace(1);
        let id = chart_with_drawings(&mut workspace, "BTCUSDT", &[]);
        workspace.focused = Some(id);
        let mut pick = choice("Fed decision", "Hold", 0.7);
        pick.event.as_mut().unwrap().series = vec![
            crate::model::PmSeries {
                token_id: "Fed decision/Hold".into(),
                label: "Hold".into(),
            },
            crate::model::PmSeries {
                token_id: "Fed decision/Cut".into(),
                label: "Cut".into(),
            },
        ];
        workspace.replace_focused_market(&pick, &ctx);
        let replaced = chart(&workspace, id);
        assert_eq!(replaced.pm_series.len(), 2);
        assert_eq!(replaced.display_title(), "Fed decision");
        assert_eq!(
            workspace.chart_count(),
            1,
            "the chart was replaced, not added"
        );
    }

    #[test]
    fn a_new_prediction_chart_is_titled_on_the_chart_it_made_not_the_last_one() {
        let ctx = egui::Context::default();
        let mut workspace = empty_workspace(1);
        workspace.add_from_search(&choice("Fed decision", "Hold", 0.7), &ctx);
        let first = workspace.charts[0].id;
        // Another chart is added after; titling the first must not land on it.
        let other = chart_with_drawings(&mut workspace, "BTCUSDT", &[]);
        workspace.title_prediction(first, &choice("Rain", "Yes", 0.4), &ctx);
        assert_eq!(chart(&workspace, first).display_title(), "Rain");
        assert_eq!(chart(&workspace, other).display_title(), "BTCUSDT");
    }

    #[test]
    fn a_picked_portfolio_view_is_added_once_and_never_replaces_a_chart() {
        let mut workspace = empty_workspace(1);
        let card = PortfolioCard::new(crate::model::portfolio::card::CardKind::Allocation, None);
        let market = chart_with_drawings(&mut workspace, "BTCUSDT", &[]);
        workspace.focused = Some(market);
        workspace.add_card(card);
        workspace.add_card(card);
        assert_eq!(
            workspace.chart_count(),
            2,
            "the same view is focused, not added twice"
        );
        assert_eq!(chart(&workspace, market).symbol, "BTCUSDT");
        // With the card focused there is no market chart to replace.
        assert!(workspace.focused_market_chart().is_none());
    }

    #[test]
    fn a_pick_for_a_watchlist_lands_in_that_list_and_not_on_the_grid() {
        let mut workspace = empty_workspace(1);
        let id = {
            let mut store = workspace.watchlist.lock().unwrap();
            store.create_list("Bets", None).unwrap()
        };
        workspace.add_to_watchlist = Some(id);
        workspace.add_pick_to_watchlist(&choice("Fed decision", "Hold", 0.7));
        workspace.add_to_picker_watchlist(WatchlistInstrument::for_card(
            PortfolioCard::new(crate::model::portfolio::card::CardKind::Value, None),
            "All Portfolios",
        ));
        let store = workspace.watchlist.lock().unwrap();
        let list = store.lists().iter().find(|list| list.id == id).unwrap();
        let items: Vec<_> = list.instruments().collect();
        assert_eq!(items.len(), 2);
        assert_eq!(
            items[0].image_url.as_deref(),
            Some("https://example.com/fed.png")
        );
        assert!(items[1].card.is_some());
        drop(store);
        assert_eq!(workspace.chart_count(), 0);
    }

    #[test]
    fn the_watchlist_target_lasts_only_while_the_picker_is_open() {
        let ctx = egui::Context::default();
        let mut workspace = empty_workspace(1);
        workspace.add_to_watchlist = Some(7);
        workspace.symbol_picker.open_for_watchlist("Bets");
        let frame = |workspace: &mut ChartWorkspace| {
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                workspace.show_symbol_picker(ui.ctx());
            });
            output.textures_delta.clear();
        };
        frame(&mut workspace);
        assert_eq!(workspace.add_to_watchlist, Some(7));
        workspace.symbol_picker.close();
        frame(&mut workspace);
        assert_eq!(workspace.add_to_watchlist, None);
    }

    #[test]
    fn the_add_button_opens_the_picker_to_add_not_to_replace() {
        let mut workspace = empty_workspace(1);
        workspace.open_add_picker();
        assert!(workspace.symbol_picker.is_open());
        assert_eq!(
            workspace.symbol_picker.mode(),
            crate::ui::chart::symbol_picker::PickMode::Add
        );
    }
}
