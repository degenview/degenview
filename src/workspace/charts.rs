//! The workspace's charts as a collection: adding, finding, replacing, removing and describing them.

use super::ChartWorkspace;
use super::chart::{Chart, displayed_candles};
use crate::model::portfolio_card::PortfolioCard;
use crate::model::price_scale::PriceScaleMode;
use crate::model::watchlist::WatchlistInstrument;
use crate::model::{DataSource, Drawing, IndicatorConfig, InstrumentId, Kline};
use crate::portfolio_card_host::PortfolioCardEnv;
use crate::service::SearchResult;
use eframe::egui;

/// What a remote caller (the MCP server) may see of one chart: identity and state, never candles.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChartSummary {
    pub id: u64,
    pub instrument: InstrumentId,
    pub display_symbol: String,
    pub is_line_chart: bool,
    /// How the chart draws its bars (candles for a market that draws a line).
    pub chart_type: crate::model::ChartType,
    pub loading: bool,
    pub error: Option<String>,
    pub focused: bool,
    pub indicators: IndicatorConfig,
    /// `(instance id, script name)` of each applied Pine script.
    pub pine_instances: Vec<(u64, String)>,
}

/// What the MCP candle and drawing tools read off one market chart. Borrowed from the workspace,
/// so nothing is cloned until a tool decides what it returns.
pub struct ChartData<'a> {
    pub instrument: InstrumentId,
    pub chart_type: crate::model::ChartType,
    /// The market's own candles, cut at the replay position as the chart shows them.
    pub candles: &'a [Kline],
    /// The replay position, when replay hides the bars after it.
    pub replay_ms: Option<i64>,
    pub drawings: &'a [Drawing],
    /// The id the chart's next new drawing takes.
    pub next_drawing_id: u64,
}

/// The result of [`ChartWorkspace::add_chart_for`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AddChartOutcome {
    pub chart_id: u64,
    /// False when the market was already charted here and that chart was reused.
    pub created: bool,
}

impl ChartWorkspace {
    /// How many charts show a market: the ones that can be linked (cards show no time axis).
    pub(super) fn market_charts(&self) -> usize {
        self.charts.iter().filter(|chart| chart.is_market()).count()
    }

    pub fn chart_count(&self) -> usize {
        self.charts.len()
    }

    /// Every chart's identity and state, in layout order (column by column). No market data.
    pub fn chart_summaries(&self) -> Vec<ChartSummary> {
        let focused = self.resolved_focused_id();
        let ordered = self
            .columns
            .iter()
            .flat_map(|column| column.chart_ids.iter())
            .filter_map(|id| self.charts.iter().find(|chart| chart.id == *id))
            .filter(|chart| chart.is_market());
        ordered
            .map(|chart| ChartSummary {
                id: chart.id,
                instrument: InstrumentId::new(chart.source, &chart.symbol),
                display_symbol: chart.source.display_symbol(&chart.symbol),
                is_line_chart: chart.source.is_line_chart(),
                chart_type: chart.chart.effective_chart_type(chart.source),
                loading: chart.loading,
                error: chart.error.clone(),
                focused: focused == Some(chart.id),
                indicators: chart.indicators,
                pine_instances: chart
                    .chart
                    .pine_instances()
                    .iter()
                    .map(|instance| (instance.id, instance.name.clone()))
                    .collect(),
            })
            .collect()
    }

    /// A market chart's candles and drawings, for the MCP tools. `None` for an unknown chart or a
    /// portfolio card.
    pub fn chart_data(&self, chart_id: u64) -> Option<ChartData<'_>> {
        let chart = self
            .charts
            .iter()
            .find(|chart| chart.id == chart_id && chart.is_market())?;
        let replay_ms = if self.replay.is_active() {
            self.replay.current_ms()
        } else {
            None
        };
        Some(ChartData {
            instrument: InstrumentId::new(chart.source, &chart.symbol),
            chart_type: chart.chart.effective_chart_type(chart.source),
            candles: displayed_candles(&chart.candles, replay_ms),
            replay_ms,
            drawings: &chart.drawings,
            next_drawing_id: chart.next_drawing_id,
        })
    }

    /// Adds a chart for `instrument` through the same path as the Add Chart sheet: an existing
    /// chart of the same market is reused (and focused) instead of duplicated, otherwise a new
    /// chart is placed in the grid, focused, and starts fetching.
    pub fn add_chart_for(
        &mut self,
        instrument: &InstrumentId,
        ctx: &egui::Context,
    ) -> AddChartOutcome {
        if let Some(existing) = self
            .charts
            .iter()
            .find(|chart| InstrumentId::new(chart.source, &chart.symbol).is_same_market(instrument))
        {
            let chart_id = existing.id;
            self.focus_chart(chart_id);
            return AddChartOutcome {
                chart_id,
                created: false,
            };
        }
        self.add_chart_with_symbol(&instrument.symbol, instrument.source, ctx);
        let chart_id = self.charts.last().map_or(0, |chart| chart.id);
        AddChartOutcome {
            chart_id,
            created: true,
        }
    }

    /// Adds a portfolio card to the grid, or focuses the one already showing the same kind for
    /// the same portfolio. Cards have no market, so nothing is fetched.
    pub fn add_card(&mut self, card: PortfolioCard) -> AddChartOutcome {
        if let Some(existing) = self
            .charts
            .iter()
            .find(|chart| chart.card.is_some_and(|shown| shown.same_card(&card)))
        {
            let chart_id = existing.id;
            self.focus_chart(chart_id);
            return AddChartOutcome {
                chart_id,
                created: false,
            };
        }
        let id = self.fresh_id();
        self.charts.push(Chart::portfolio_card(id, card));
        self.place_in_grid(id);
        self.focused = Some(id);
        AddChartOutcome {
            chart_id: id,
            created: true,
        }
    }

    pub(super) fn fresh_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    /// Sets a chart's price-scale mode: the one place the corner menu, the shortcuts and the
    /// price-axis menu all end up. A view, not data: nothing is refetched, rerun or redrawn
    /// beyond the next frame, and the horizontal position and the drawings stay as they are.
    pub(crate) fn set_scale_mode(&mut self, chart_id: u64, mode: PriceScaleMode) {
        if let Some(chart) = self
            .charts
            .iter_mut()
            .find(|chart| chart.id == chart_id && chart.is_market())
        {
            chart.scale_mode = mode;
        }
    }

    pub(super) fn resolved_focused_id(&self) -> Option<u64> {
        if let Some(id) = self.focused
            && self.charts.iter().any(|chart| chart.id == id)
        {
            return Some(id);
        }
        self.charts.first().map(|chart| chart.id)
    }

    fn add_chart_with_symbol(&mut self, symbol: &str, source: DataSource, ctx: &egui::Context) {
        let id = self.fresh_id();
        self.charts
            .push(Chart::new(id, symbol, source, self.timeframe, ctx));
        self.place_in_grid(id);
        self.focused = Some(id);
    }

    pub(super) fn add_from_search(&mut self, result: &SearchResult, ctx: &egui::Context) {
        let instrument = InstrumentId::new(result.source, &result.full_symbol);
        let outcome = self.add_chart_for(&instrument, ctx);
        if result.source == DataSource::DexScreener
            && let Some(chart) = self
                .charts
                .iter_mut()
                .find(|chart| chart.id == outcome.chart_id)
        {
            // DEXScreener fetches by pair address, but that opaque id should never become the
            // card title. Keep the search result's BASE/QUOTE label alongside it.
            chart.chart.set_title(Some(result.symbol.clone()));
        }
        if result.source.is_prediction_market()
            && let Some(chart) = self
                .charts
                .iter_mut()
                .find(|chart| chart.id == outcome.chart_id)
        {
            chart.image_url = result
                .event
                .as_ref()
                .and_then(|event| event.image_url.clone());
        }
        if !outcome.created {
            return;
        }
        if result.source.is_prediction_market() {
            self.title_prediction(result, ctx);
        }
    }

    /// Gives the chart just added for a prediction-market pick its event title and, when several
    /// choices were ticked, their lines; the choices need a second fetch.
    fn title_prediction(&mut self, result: &SearchResult, ctx: &egui::Context) {
        let Some(chart) = self.charts.last_mut() else {
            return;
        };
        let (title, series) = match &result.event {
            Some(event) if !event.title.is_empty() => (event.title.clone(), event.series.clone()),
            Some(event) => (result.symbol.clone(), event.series.clone()),
            None => (result.symbol.clone(), Vec::new()),
        };
        chart.set_prediction(title, series);
        chart.fetch(self.timeframe, ctx);
    }

    /// The macOS-style alert behind a header ✕.
    pub(super) fn confirm_chart_removal(&mut self, ctx: &egui::Context, chart_id: u64) {
        let Some(chart) = self.charts.iter().find(|chart| chart.id == chart_id) else {
            self.removing_chart = None;
            return;
        };
        let title = format!("Remove {}?", chart.display_title());
        let result = crate::ui::kit::confirm(
            ctx,
            egui::Id::new("remove-chart"),
            &crate::ui::kit::Confirm {
                icon: crate::ui::icons::Icon::Trash,
                title: &title,
                message: "The chart leaves this tab. You can add it again from Add Chart.",
                destructive: "Remove",
            },
        );
        if result == Some(crate::ui::kit::ConfirmOutcome::Confirmed) {
            self.remove_chart(chart_id);
        }
        if result.is_some() {
            self.removing_chart = None;
        }
    }

    pub(super) fn remove_chart(&mut self, chart_id: u64) {
        self.sync_hub.forget(chart_id);
        self.charts.retain(|chart| chart.id != chart_id);
        if self.focused == Some(chart_id) {
            self.focused = None;
        }
        for column in &mut self.columns {
            column.chart_ids.retain(|id| *id != chart_id);
        }
        self.columns.retain(|column| !column.chart_ids.is_empty());
    }

    pub(super) fn focus_chart(&mut self, chart_id: u64) {
        if self.focused == Some(chart_id) {
            return;
        }
        if self.charts.iter().any(|chart| chart.id == chart_id) {
            self.focused = Some(chart_id);
        }
    }

    pub(super) fn switch_chart(
        &mut self,
        chart_id: u64,
        item: &WatchlistInstrument,
        ctx: &egui::Context,
    ) {
        let Some(chart) = self.charts.iter_mut().find(|chart| chart.id == chart_id) else {
            return;
        };
        let instrument = &item.instrument;
        let already_showing = chart.source == instrument.source
            && InstrumentId::new(chart.source, &chart.symbol).is_same_market(instrument);
        if already_showing {
            self.focus_chart(chart_id);
            return;
        }
        chart.replace_market(item);
        chart.chart.reset_view();
        chart.fetch(self.timeframe, ctx);
        self.focus_chart(chart_id);
    }

    /// Gives the cards on this tab their portfolios. Cheap to call every frame.
    pub fn set_card_env(&mut self, env: &PortfolioCardEnv) {
        if self.card_env.is_none() {
            self.card_env = Some(env.clone());
        }
    }

    /// The identity a chart has in a watchlist: its market, or its card's placeholder.
    pub(super) fn chart_instrument(chart: &Chart) -> InstrumentId {
        chart.card.as_ref().map_or_else(
            || InstrumentId::new(chart.source, &chart.symbol),
            PortfolioCard::instrument_id,
        )
    }

    /// The watchlist row that stands for `chart`.
    pub(super) fn favorite_item(&self, chart: &Chart) -> WatchlistInstrument {
        if let Some(card) = chart.card {
            let scope = self
                .card_env
                .as_ref()
                .map_or_else(Vec::new, PortfolioCardEnv::portfolios);
            return WatchlistInstrument::for_card(card, &card.scope_name(&scope));
        }
        let mut item =
            WatchlistInstrument::from_market(chart.source, &chart.display_title(), &chart.symbol);
        item.image_url = chart.image_url.clone();
        item
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::watchlist_quotes::{WatchlistQuoteBook, WatchlistQuoteCoordinator};
    use crate::store::SharedStore;
    use crate::store::watchlist::WatchlistStore;
    use crate::workspace::PersistedWorkspace;
    use crate::workspace::test_support::empty_workspace;
    use std::sync::{Arc, Mutex};

    #[test]
    fn add_chart_for_places_focuses_and_reuses() {
        let ctx = egui::Context::default();
        let mut workspace = empty_workspace(7);
        assert!(workspace.chart_summaries().is_empty());

        let btc = InstrumentId::new(DataSource::Binance, "BTCUSDT");
        let first = workspace.add_chart_for(&btc, &ctx);
        assert!(first.created);
        assert!(workspace.is_dirty());
        assert_eq!(workspace.chart_count(), 1);

        // Adding the same market again (any spelling the provider treats alike) reuses it.
        let again =
            workspace.add_chart_for(&InstrumentId::new(DataSource::Binance, "btcusdt"), &ctx);
        assert_eq!(
            again,
            AddChartOutcome {
                chart_id: first.chart_id,
                created: false
            }
        );
        assert_eq!(workspace.chart_count(), 1);

        let eth =
            workspace.add_chart_for(&InstrumentId::new(DataSource::Coinbase, "ETH-USD"), &ctx);
        assert!(eth.created);
        assert_ne!(eth.chart_id, first.chart_id);

        let summaries = workspace.chart_summaries();
        assert_eq!(summaries.len(), 2);
        let ids: Vec<u64> = summaries.iter().map(|chart| chart.id).collect();
        assert!(ids.contains(&first.chart_id) && ids.contains(&eth.chart_id));
        let focused: Vec<_> = summaries.iter().filter(|chart| chart.focused).collect();
        assert_eq!(focused.len(), 1);
        assert_eq!(focused[0].id, eth.chart_id);
        assert_eq!(focused[0].display_symbol, "ETH/USD");
        assert!(!focused[0].is_line_chart);
    }

    #[test]
    fn a_portfolio_card_is_added_once_and_is_not_a_market() {
        use crate::model::portfolio_card::{CardKind, PortfolioCard};
        let ctx = egui::Context::default();
        let mut workspace = empty_workspace(9);
        let btc = workspace.add_chart_for(&InstrumentId::new(DataSource::Binance, "BTCUSDT"), &ctx);
        workspace.mark_clean();

        let allocation = PortfolioCard::new(CardKind::Allocation, None);
        let first = workspace.add_card(allocation);
        assert!(first.created);
        assert!(workspace.is_dirty(), "adding a card changes the layout");
        assert_eq!(workspace.chart_count(), 2);

        // The same kind for the same portfolio is focused, not duplicated; another scope is new.
        let again = workspace.add_card(PortfolioCard::new(CardKind::Allocation, None));
        assert_eq!((again.chart_id, again.created), (first.chart_id, false));
        let scoped = workspace.add_card(PortfolioCard::new(CardKind::Allocation, Some(3)));
        assert!(scoped.created);

        // Cards stay out of everything that reads a market: MCP, scripts, paper, live feeds.
        let summaries = workspace.chart_summaries();
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].id, btc.chart_id);
        assert_eq!(workspace.pine_datasets().len(), 1);
        assert_eq!(workspace.charts_candles().len(), 1);
        let card_chart = workspace
            .charts
            .iter()
            .find(|chart| chart.id == first.chart_id)
            .unwrap();
        assert!(!card_chart.is_market());
        assert!(
            card_chart.inbox.is_none() && !card_chart.loading,
            "a card never fetches"
        );
    }

    #[test]
    fn a_card_survives_a_save_and_changing_its_range_marks_the_layout() {
        use crate::model::portfolio_card::{CardKind, PortfolioCard};
        use crate::model::portfolio_statistics::PortfolioHistoryRange;
        let ctx = egui::Context::default();
        let mut workspace = empty_workspace(10);
        workspace.add_card(PortfolioCard::new(CardKind::TopPerformers, Some(2)));
        workspace.mark_clean();

        let saved = workspace.persisted();
        assert_eq!(
            saved.charts[0]
                .card
                .map(|card| (card.kind, card.portfolio_id)),
            Some((CardKind::TopPerformers, Some(2)))
        );

        workspace.charts[0].card.as_mut().unwrap().range = PortfolioHistoryRange::OneYear;
        assert!(workspace.is_dirty(), "a range is part of the layout");

        let store: SharedStore = Arc::new(Mutex::new(
            crate::store::Store::open_in_memory().expect("in-memory store"),
        ));
        let book = WatchlistQuoteBook::default();
        let restored = ChartWorkspace::restored(
            workspace.persisted(),
            Arc::new(Mutex::new(WatchlistStore::new(store.clone()))),
            book.clone(),
            WatchlistQuoteCoordinator::new(book, Arc::new(|| {})),
            store,
            &ctx,
        );
        let chart = &restored.charts[0];
        assert!(
            !chart.is_market() && chart.inbox.is_none(),
            "a restored card does not fetch"
        );
        assert_eq!(chart.card.unwrap().range, PortfolioHistoryRange::OneYear);
        assert_eq!(chart.display_title(), "Top Performers");
    }

    #[test]
    fn a_dex_search_result_keeps_its_pair_address_and_shows_its_ticker() {
        let ctx = egui::Context::default();
        let mut workspace = empty_workspace(8);
        let result = SearchResult {
            symbol: "MCADE/USDC".into(),
            full_symbol: "7pWfX2LongPairAddress".into(),
            source: DataSource::DexScreener,
            price: Some(0.01),
            event: None,
        };

        workspace.add_from_search(&result, &ctx);

        assert_eq!(workspace.charts[0].symbol, "7pWfX2LongPairAddress");
        assert_eq!(workspace.charts[0].display_title(), "MCADE/USDC");
        assert_eq!(
            workspace.persisted().charts[0].title.as_deref(),
            Some("MCADE/USDC")
        );
    }

    #[test]
    fn added_charts_persist_and_restore() {
        let ctx = egui::Context::default();
        let mut workspace = empty_workspace(3);
        let outcome =
            workspace.add_chart_for(&InstrumentId::new(DataSource::Binance, "SOLUSDT"), &ctx);
        let persisted = workspace.persisted();
        assert_eq!(persisted.charts.len(), 1);
        assert_eq!(persisted.charts[0].id, outcome.chart_id);
        assert_eq!(persisted.charts[0].symbol, "SOLUSDT");
    }

    #[test]
    fn each_chart_keeps_its_own_scale_mode_through_a_restart_and_marks_the_layout_changed() {
        let ctx = egui::Context::default();
        let mut workspace = empty_workspace(6);
        let ids: Vec<u64> = ["BTCUSDT", "ETHUSDT", "SOLUSDT", "XRPUSDT"]
            .into_iter()
            .map(|symbol| {
                workspace
                    .add_chart_for(&InstrumentId::new(DataSource::Binance, symbol), &ctx)
                    .chart_id
            })
            .collect();
        workspace.mark_clean();
        for (id, mode) in ids.iter().zip(PriceScaleMode::ALL) {
            workspace.set_scale_mode(*id, mode);
        }
        assert!(workspace.is_dirty(), "a scale mode is part of the layout");

        let persisted = workspace.persisted();
        for (saved, mode) in persisted.charts.iter().zip(PriceScaleMode::ALL) {
            assert_eq!(saved.scale_mode, Some(mode));
            assert_eq!(
                saved.log_scale,
                mode == PriceScaleMode::Logarithmic,
                "the switch older builds read says the same"
            );
        }
        let decoded: PersistedWorkspace =
            serde_json::from_str(&serde_json::to_string(&persisted).expect("encode"))
                .expect("decode");
        let restored = ChartWorkspace::restored(
            decoded,
            workspace.watchlist.clone(),
            workspace.quote_book.clone(),
            workspace.quote_coordinator.clone(),
            workspace.store.clone(),
            &ctx,
        );
        let modes: Vec<PriceScaleMode> = restored.charts.iter().map(|c| c.scale_mode).collect();
        assert_eq!(modes, PriceScaleMode::ALL);

        // Changing one chart leaves the others as they were; the same mode again changes nothing.
        workspace.mark_clean();
        workspace.set_scale_mode(ids[0], PriceScaleMode::Regular);
        assert!(!workspace.is_dirty());
        workspace.set_scale_mode(ids[0], PriceScaleMode::Percent);
        assert_eq!(workspace.charts[0].scale_mode, PriceScaleMode::Percent);
        assert_eq!(workspace.charts[1].scale_mode, PriceScaleMode::Percent);
        assert_eq!(workspace.charts[3].scale_mode, PriceScaleMode::Logarithmic);
    }

    #[test]
    fn a_layout_saved_before_the_scale_modes_opens_with_the_log_switch_it_had() {
        let ctx = egui::Context::default();
        let mut workspace = empty_workspace(6);
        for symbol in ["BTCUSDT", "ETHUSDT"] {
            workspace.add_chart_for(&InstrumentId::new(DataSource::Binance, symbol), &ctx);
        }
        // What v0.5 wrote: the on/off switch, and no mode.
        let mut row = serde_json::to_value(workspace.persisted()).expect("encode");
        for (chart, log) in row["charts"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .zip([true, false])
        {
            let chart = chart.as_object_mut().unwrap();
            chart.remove("scale_mode");
            chart.insert("log_scale".to_owned(), log.into());
        }
        let decoded: PersistedWorkspace = serde_json::from_value(row).expect("decode");
        let restored = ChartWorkspace::restored(
            decoded,
            workspace.watchlist.clone(),
            workspace.quote_book.clone(),
            workspace.quote_coordinator.clone(),
            workspace.store.clone(),
            &ctx,
        );
        let modes: Vec<PriceScaleMode> = restored.charts.iter().map(|c| c.scale_mode).collect();
        assert_eq!(
            modes,
            [PriceScaleMode::Logarithmic, PriceScaleMode::Regular]
        );
    }

    #[test]
    fn a_portfolio_card_has_no_price_scale_to_set() {
        let ctx = egui::Context::default();
        let mut workspace = empty_workspace(6);
        let id = workspace
            .add_chart_for(&InstrumentId::new(DataSource::Binance, "BTCUSDT"), &ctx)
            .chart_id;
        workspace.charts[0].card = Some(PortfolioCard::new(
            crate::model::portfolio_card::CardKind::Allocation,
            None,
        ));
        workspace.set_scale_mode(id, PriceScaleMode::Percent);
        assert_eq!(workspace.charts[0].scale_mode, PriceScaleMode::Regular);
    }
}
