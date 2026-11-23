//! One chart's market data: what it holds, how it fetches, and how older history is paged in.

pub(in crate::workspace) mod collection;
pub(in crate::workspace) mod context;
mod drawing_sets;
pub(in crate::workspace) mod live;
pub(in crate::workspace) mod settings;
pub(in crate::workspace) mod snapshot;
pub(in crate::workspace) mod symbol_picker;

use super::PersistedChart;
use crate::host::portfolio::card::CardRuntime;
use crate::model::chart::price_scale::PriceScaleMode;
use crate::model::drawings::market::MarketDrawings;
use crate::model::market;
use crate::model::market::history::OlderHistory;
use crate::model::market::live::{LiveFold, LiveMark};
use crate::model::overlay::OverlayList;
use crate::model::portfolio::card::PortfolioCard;
use crate::model::watchlist::WatchlistInstrument;
use crate::model::{DataSource, Drawing, IndicatorConfig, Kline, PmSeries, Timeframe};
use crate::service::{self, ProviderError};
use crate::ui::{ChartCard, SeriesLine};
use eframe::egui;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

pub(super) const CANDLE_LIMIT: u32 = 500;
pub(super) const REFRESH_INTERVAL: Duration = Duration::from_secs(5);
/// How often a rate-limited provider (one request every few seconds) is asked again, and how long
/// it is left alone after an error rather than spending the limiter on retries.
const SLOW_REFRESH_INTERVAL: Duration = Duration::from_secs(15);
const SLOW_RETRY_INTERVAL: Duration = Duration::from_secs(45);

/// One chart's runtime state: market data, fetch result, and view.
pub(super) struct Chart {
    pub(super) id: u64,
    pub(super) symbol: String,
    pub(super) source: DataSource,
    pub(super) candles: Vec<Kline>,
    pub(super) loading: bool,
    pub(super) error: Option<ProviderError>,
    pub(super) last_fetch: Instant,
    pub(super) inbox: Option<Receiver<Result<Fetched, ProviderError>>>,
    /// What is known of the history behind the oldest candle, and the request for it, if any.
    pub(super) older: OlderHistory,
    pub(super) older_inbox: Option<OlderRequest>,
    pub(super) fetch_generation: Arc<AtomicU64>,
    pub(super) chart: ChartCard,
    pub(super) scale_mode: PriceScaleMode,
    pub(super) indicators: IndicatorConfig,
    /// The drawings of the market the chart shows ([`drawing_sets`]).
    pub(super) drawings: Vec<Drawing>,
    /// The drawings of the markets the chart showed before, by market key.
    pub(super) parked_drawings: MarketDrawings,
    /// The next id a drawing takes: one counter for every market, ids being unique per chart.
    pub(super) next_drawing_id: u64,
    pub(super) last_paper_quote: Option<(f64, Instant)>,
    /// Wall-clock time (epoch ms) the candles last arrived from the provider or its live feed.
    /// Paper trading stamps its quotes with this, never with the time they are read.
    pub(super) last_data_ms: Option<i64>,
    /// The newest bar the live feed delivered: a refresh begun before it keeps it.
    pub(super) live: Option<LiveMark>,
    /// The live klines of the bucket forming, when the timeframe is folded from finer ones.
    pub(super) live_fold: LiveFold,
    /// Prediction markets: the choices charted together as lines, when there are several.
    pub(super) pm_series: Vec<PmSeries>,
    /// Markets compared on this chart, in visual order. Configuration only: their candles live in
    /// the workspace's `OverlayHub`. Nothing a script, alert, drawing or order reads looks here.
    pub(super) overlays: OverlayList,
    /// Provider artwork carried into watchlists without another metadata search.
    pub(super) image_url: Option<String>,
    /// A portfolio card instead of a market: `symbol` is the card's key and `source` a
    /// placeholder nothing reads. Cards never fetch, stream, trade or run scripts.
    pub(super) card: Option<PortfolioCard>,
    /// The card's own price demand and feeds, made the first frame it is drawn with an app that
    /// has portfolios (`ChartWorkspace::set_card_env`). Dropping it releases the demand.
    pub(super) card_runtime: Option<CardRuntime>,
}

/// The market a chart shows, as undo puts it back: what names it and how it is titled, never the
/// candles, which are fetched again.
pub(super) struct MarketState {
    symbol: String,
    source: DataSource,
    title: Option<String>,
    image_url: Option<String>,
    pm_series: Vec<PmSeries>,
}

/// What one fetch brought back: the candles the chart reads, and for a multi-choice prediction
/// market each choice's own history.
pub(super) struct Fetched {
    candles: Vec<Kline>,
    /// The base candles of the newest candle's bucket (see [`service::LiveCandles`]).
    bars: Vec<Kline>,
    series: Vec<(String, Vec<Kline>)>,
}

impl Fetched {
    /// The candles come from the first choice that has any; the rest only draw their lines.
    fn from_choices(histories: Vec<(String, Vec<Kline>)>) -> Result<Self, ProviderError> {
        let candles = histories
            .iter()
            .find(|(_, history)| !history.is_empty())
            .map(|(_, history)| history.clone())
            .ok_or_else(|| ProviderError::other("No price history for these choices"))?;
        Ok(Self {
            candles,
            bars: Vec::new(),
            series: histories,
        })
    }
}

impl Chart {
    pub(super) fn new(
        id: u64,
        symbol: impl Into<String>,
        source: DataSource,
        timeframe: Timeframe,
        ctx: &egui::Context,
    ) -> Self {
        let mut chart = Self::blank(id, symbol, source);
        chart.fetch(timeframe, ctx);
        chart
    }

    /// A chart that has not fetched yet, for callers that set up its prediction choices first.
    fn blank(id: u64, symbol: impl Into<String>, source: DataSource) -> Self {
        Self {
            id,
            symbol: symbol.into(),
            source,
            candles: Vec::new(),
            loading: false,
            error: None,
            last_fetch: Instant::now(),
            inbox: None,
            older: OlderHistory::default(),
            older_inbox: None,
            fetch_generation: Arc::new(AtomicU64::new(0)),
            chart: ChartCard::default(),
            scale_mode: PriceScaleMode::default(),
            indicators: IndicatorConfig::default(),
            drawings: Vec::new(),
            parked_drawings: MarketDrawings::default(),
            next_drawing_id: 1,
            last_paper_quote: None,
            last_data_ms: None,
            live: None,
            live_fold: LiveFold::default(),
            pm_series: Vec::new(),
            overlays: OverlayList::new(),
            image_url: None,
            card: None,
            card_runtime: None,
        }
    }

    /// A portfolio card. It has no market data, so it never fetches.
    pub(super) fn portfolio_card(id: u64, card: PortfolioCard) -> Self {
        let mut chart = Self::blank(id, card.key(), DataSource::Binance);
        chart.card = Some(card);
        chart
    }

    /// Whether this is a market chart (candles, indicators, alerts, scripts) rather than a
    /// portfolio card. Every loop that reads market data skips the cards.
    pub(super) fn is_market(&self) -> bool {
        self.card.is_none()
    }

    /// Whether indicators and scripts can be put on this chart: a market chart whose source has
    /// a price to study (not a prediction market or a CoinMarketCap gauge).
    pub(super) fn takes_indicators(&self) -> bool {
        self.is_market() && self.source.takes_indicators()
    }

    /// The name shown for the chart: an explicit display title, else its provider symbol.
    pub(super) fn display_title(&self) -> String {
        if let Some(card) = &self.card {
            return card.kind.title().to_owned();
        }
        self.chart
            .title()
            .map_or_else(|| self.source.display_symbol(&self.symbol), str::to_owned)
    }

    /// What the symbol picker searches for when it opens on this chart: the market's own ticker,
    /// or for a DEX pair (whose id is an address) its base token. Nothing for a market the picker
    /// does not list.
    pub(super) fn picker_query(&self) -> String {
        match self.source {
            DataSource::Binance
            | DataSource::Coinbase
            | DataSource::Okx
            | DataSource::Kraken
            | DataSource::Bybit
            | DataSource::Gate
            | DataSource::Mexc
            | DataSource::CoinGecko
            | DataSource::Alpaca => self.source.display_symbol(&self.symbol),
            DataSource::DexScreener => self.source.base_asset(&self.display_title()),
            DataSource::Polymarket | DataSource::Kalshi | DataSource::CoinMarketCap => {
                String::new()
            }
        }
    }

    /// Titles a prediction market and sets the choices it draws as lines (none for a single one).
    pub(super) fn set_prediction(&mut self, title: String, series: Vec<PmSeries>) {
        self.chart.set_title(Some(title));
        self.pm_series = if series.len() > 1 { series } else { Vec::new() };
    }

    /// Applies the display metadata retained by a watchlist row.
    pub(super) fn apply_watchlist_metadata(&mut self, item: &WatchlistInstrument) {
        let source = item.instrument.source;
        self.chart.set_title(
            (source == DataSource::DexScreener || source.is_prediction_market()).then(|| {
                item.display_name
                    .clone()
                    .unwrap_or_else(|| item.name.clone())
            }),
        );
        self.image_url = item.image_url.clone();
    }

    /// Replaces the market-specific state while preserving this card's presentation settings.
    /// A market that takes no indicators leaves the chart without the ones it had.
    pub(super) fn replace_market(&mut self, item: &WatchlistInstrument) {
        let from = self.drawing_market();
        self.symbol = item.instrument.symbol.clone();
        self.source = item.instrument.source;
        self.apply_watchlist_metadata(item);
        self.clear_market_data();
        self.file_drawings(&from);
        if !self.source.takes_indicators() {
            self.indicators = IndicatorConfig::default();
            self.chart.set_pine_instances(Vec::new());
        }
    }

    /// What names this chart's market, to put back when a replacement is undone.
    pub(super) fn market_state(&self) -> MarketState {
        MarketState {
            symbol: self.symbol.clone(),
            source: self.source,
            title: self.chart.title().map(str::to_owned),
            image_url: self.image_url.clone(),
            pm_series: self.pm_series.clone(),
        }
    }

    /// Puts `state` on the chart, dropping the candles of the market it had, and returns what the
    /// chart showed. The caller reloads the candles.
    pub(super) fn swap_market(&mut self, state: MarketState) -> MarketState {
        let current = self.market_state();
        let from = self.drawing_market();
        self.symbol = state.symbol;
        self.source = state.source;
        self.chart.set_title(state.title);
        self.image_url = state.image_url;
        self.clear_market_data();
        self.file_drawings(&from);
        self.pm_series = state.pm_series;
        current
    }

    fn clear_market_data(&mut self) {
        self.chart.set_series(Vec::new());
        self.pm_series.clear();
        self.last_paper_quote = None;
        self.last_data_ms = None;
        self.forget_live();
        self.candles.clear();
        self.reset_history();
    }

    pub(super) fn restored(
        saved: PersistedChart,
        timeframe: Timeframe,
        ctx: &egui::Context,
    ) -> Self {
        let instances = saved.instances();
        let scale_mode = saved.price_scale_mode();
        let mut chart = Self::blank(saved.id, saved.symbol, saved.source);
        chart.card = saved.card;
        chart.chart.set_title(saved.title);
        chart.pm_series = saved.pm_series;
        chart.overlays = saved.overlays;
        chart.image_url = saved.image_url;
        chart.fetch(timeframe, ctx);
        chart.scale_mode = scale_mode;
        chart.chart.set_y_zoom(saved.y_zoom);
        chart.chart.set_appearance(saved.appearance);
        chart.chart.set_pine_instances(instances);
        chart.chart.set_visible_candles(saved.candle_count as usize);
        chart.chart.restore_viewport(saved.viewport);
        chart.indicators = saved.indicators;
        if !chart.source.takes_indicators() {
            // A layout saved before these charts stopped taking indicators.
            chart.indicators = IndicatorConfig::default();
            chart.chart.set_pine_instances(Vec::new());
        }
        chart.set_stored_drawings(saved.drawings);
        chart
    }

    pub(super) fn fetch(&mut self, timeframe: Timeframe, ctx: &egui::Context) {
        if !self.is_market() {
            return;
        }
        let (sender, receiver) = mpsc::channel();
        let symbol = self.symbol.clone();
        let source = self.source;
        let choices = self.pm_series.clone();
        let repaint = ctx.clone();

        let generation = self.fetch_generation.fetch_add(1, Ordering::AcqRel) + 1;
        let current = Arc::downgrade(&self.fetch_generation);
        service::work_queue::submit_for(source, move || {
            if !current
                .upgrade()
                .is_some_and(|state| state.load(Ordering::Acquire) == generation)
            {
                return;
            }
            let result = if choices.is_empty() {
                service::fetch_klines_live(source, &symbol, timeframe, CANDLE_LIMIT).map(|live| {
                    Fetched {
                        candles: live.candles,
                        bars: live.bars,
                        series: Vec::new(),
                    }
                })
            } else {
                Fetched::from_choices(service::fetch_choices(
                    source,
                    &choices,
                    timeframe,
                    CANDLE_LIMIT,
                ))
            };
            let _ = sender.send(result);
            repaint.request_repaint();
        });

        self.inbox = Some(receiver);
        self.loading = true;
        self.last_fetch = Instant::now();
    }

    /// How long after a fetch began the next one is due.
    pub(super) fn refresh_after(&self) -> Duration {
        match self.source {
            DataSource::CoinGecko | DataSource::DexScreener | DataSource::Kraken
                if self.error.is_some() =>
            {
                SLOW_RETRY_INTERVAL
            }
            DataSource::CoinGecko | DataSource::DexScreener | DataSource::Kraken => {
                SLOW_REFRESH_INTERVAL
            }
            _ => REFRESH_INTERVAL,
        }
    }

    /// Forgets what the live feed delivered: the candles are another market's or timeframe's.
    pub(super) fn forget_live(&mut self) {
        self.live = None;
        self.live_fold.reset();
    }

    /// Whether the refresh loop should fetch again: a request is not out, the time is up, and the
    /// last answer was not that the source has no such timeframe, which asking again cannot
    /// change. The market and the timeframe are fetched when they change, whatever this says.
    pub(super) fn refresh_due(&self) -> bool {
        self.is_market()
            && self.inbox.is_none()
            && self.last_fetch.elapsed() >= self.refresh_after()
            && !self
                .error
                .as_ref()
                .is_some_and(ProviderError::is_unsupported_interval)
    }

    /// Forgets what was learnt of the history behind the candles: they are another market's or
    /// timeframe's now.
    pub(super) fn reset_history(&mut self) {
        self.older = OlderHistory::default();
        self.older_inbox = None;
    }

    /// Asks the provider for the page before the oldest candle once the view has come near it.
    pub(super) fn request_older(&mut self, timeframe: Timeframe, ctx: &egui::Context) {
        if !self.is_market() || !self.pm_series.is_empty() {
            return;
        }
        let Some(end) = market::history::page_end(&self.candles) else {
            return;
        };
        // Near the oldest candle, ahead of the user once, or while the scripts on the chart have
        // fewer candles than they are meant to run over.
        let wanted = self.older.should_request(
            self.source.supports_older_history(timeframe),
            self.chart.near_oldest_bar()
                || self.older.untouched()
                || market::history::short_for_scripts(
                    self.chart.runs_scripts(),
                    self.candles.len(),
                ),
            self.candles.len(),
            Instant::now(),
        );
        if !wanted {
            return;
        }
        let (sender, receiver) = mpsc::channel();
        let (source, symbol) = (self.source, self.symbol.clone());
        let repaint = ctx.clone();
        let asked = symbol.clone();
        service::work_queue::submit_for(source, move || {
            let page = service::fetch_klines_range(
                source,
                &asked,
                timeframe,
                0,
                end,
                market::history::PAGE,
            );
            let _ = sender.send(page);
            repaint.request_repaint();
        });
        self.older.started();
        self.older_inbox = Some(OlderRequest {
            receiver,
            source,
            symbol,
            timeframe,
        });
    }

    pub(super) fn receive_older(&mut self, timeframe: Timeframe) {
        let Some(request) = &self.older_inbox else {
            return;
        };
        let result = match request.receiver.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => {
                Err(ProviderError::other("Older candles stopped loading"))
            }
        };
        let current = request.source == self.source
            && request.symbol == self.symbol
            && request.timeframe == timeframe;
        self.older_inbox = None;
        if !current {
            self.older.abandoned();
            return;
        }
        match result {
            Ok(page) => {
                let added = market::history::prepend(&mut self.candles, page);
                self.older.arrived(added);
            }
            Err(_) => self.older.failed(Instant::now()),
        }
    }

    pub(super) fn receive_fetch(&mut self, timeframe: Timeframe) {
        let Some(receiver) = &self.inbox else {
            return;
        };
        let result = match receiver.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => Err(ProviderError::other(
                "Chart fetch stopped before returning data",
            )),
        };

        self.loading = false;
        self.inbox = None;
        match result {
            Ok(fetched) => {
                let live_open = self.live.and_then(|mark| mark.guard_for(self.last_fetch));
                let fresh = self.live_fold.merge_fetched(
                    timeframe,
                    fetched.candles,
                    fetched.bars,
                    &self.candles,
                    live_open,
                    CANDLE_LIMIT as usize,
                );
                self.candles =
                    market::history::with_older(&self.candles, fresh, self.older.keeps_older());
                if !self.pm_series.is_empty() {
                    self.chart
                        .set_series(SeriesLine::build(&self.pm_series, fetched.series));
                }
                self.error = None;
                self.last_data_ms = Some(crate::clock::now_ms());
            }
            Err(error) => self.error = Some(error),
        }
    }
}

/// A request for the candles before the oldest one, with what it was asked for: an answer for a
/// market or timeframe the chart has left is dropped.
pub(super) struct OlderRequest {
    receiver: Receiver<Result<Vec<Kline>, ProviderError>>,
    source: DataSource,
    symbol: String,
    timeframe: Timeframe,
}

pub(super) fn displayed_candles(candles: &[Kline], replay_ms: Option<i64>) -> &[Kline] {
    let Some(replay_ms) = replay_ms else {
        return candles;
    };
    let end = candles.partition_point(|candle| candle.open_time <= replay_ms);
    &candles[..end]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::test_support::candle;

    /// A chart holding `times`, with a request for older candles outstanding that answers `page`.
    fn chart_awaiting_older(times: &[i64], page: Result<Vec<Kline>, ProviderError>) -> Chart {
        let mut chart = Chart::blank(1, "BTCUSDT", DataSource::Binance);
        chart.candles = times.iter().map(|time| candle(*time, 1.0)).collect();
        let (sender, receiver) = mpsc::channel();
        sender.send(page).unwrap();
        chart.older.started();
        chart.older_inbox = Some(OlderRequest {
            receiver,
            source: DataSource::Binance,
            symbol: "BTCUSDT".into(),
            timeframe: Timeframe::HOUR_1,
        });
        chart
    }

    #[test]
    fn rate_limited_sources_refresh_less_often_and_back_off_after_an_error() {
        let mut binance = Chart::blank(1, "BTCUSDT", DataSource::Binance);
        let mut dex = Chart::blank(2, "0xpool", DataSource::DexScreener);
        assert_eq!(binance.refresh_after(), REFRESH_INTERVAL);
        assert_eq!(dex.refresh_after(), SLOW_REFRESH_INTERVAL);
        binance.error = Some(ProviderError::other("down"));
        dex.error = Some(ProviderError::Network(
            crate::service::net::http::Error::Status(429),
        ));
        assert_eq!(binance.refresh_after(), REFRESH_INTERVAL);
        assert_eq!(dex.refresh_after(), SLOW_RETRY_INTERVAL);
    }

    #[test]
    fn kraken_charts_refresh_slowly_because_the_socket_keeps_them_live() {
        let mut kraken = Chart::blank(1, "BTC/USD", DataSource::Kraken);
        assert_eq!(kraken.refresh_after(), SLOW_REFRESH_INTERVAL);
        kraken.error = Some(ProviderError::Network(
            crate::service::net::http::Error::Status(429),
        ));
        assert_eq!(kraken.refresh_after(), SLOW_RETRY_INTERVAL);
    }

    #[test]
    fn a_source_with_no_such_timeframe_is_not_refreshed_until_something_changes() {
        let mut chart = Chart::blank(1, "bitcoin", DataSource::CoinGecko);
        chart.last_fetch = Instant::now() - SLOW_REFRESH_INTERVAL - Duration::from_secs(1);
        assert!(chart.refresh_due());

        let (sender, receiver) = mpsc::channel();
        let unsupported = ProviderError::unsupported(DataSource::CoinGecko, Timeframe::MINUTE_1);
        sender.send(Err(unsupported.clone())).unwrap();
        chart.inbox = Some(receiver);
        chart.receive_fetch(Timeframe::HOUR_1);
        // The chart says why, in the slot every error uses.
        assert_eq!(chart.error, Some(unsupported));
        assert_eq!(
            chart.error.as_ref().map(ToString::to_string).as_deref(),
            Some("CoinGecko has no 1m history")
        );
        chart.last_fetch = Instant::now() - Duration::from_secs(3_600);
        assert!(
            !chart.refresh_due(),
            "asking again cannot change the answer"
        );

        // Any other failure is retried as before, and a fetch for another timeframe or market
        // (which does not wait for this check) replaces the error with its own answer.
        chart.error = Some(ProviderError::other("down"));
        assert!(chart.refresh_due());
    }

    fn open_times(chart: &Chart) -> Vec<i64> {
        chart.candles.iter().map(|bar| bar.open_time).collect()
    }

    #[test]
    fn an_older_page_goes_in_front_and_a_refresh_does_not_lose_it() {
        let mut chart =
            chart_awaiting_older(&[10, 11, 12], Ok(vec![candle(7, 1.0), candle(8, 1.0)]));
        chart.receive_older(Timeframe::HOUR_1);
        assert_eq!(open_times(&chart), vec![7, 8, 10, 11, 12]);
        assert!(chart.older_inbox.is_none());

        // The next refresh brings only the newest candles.
        let (sender, receiver) = mpsc::channel();
        sender
            .send(Ok(Fetched {
                candles: vec![candle(11, 2.0), candle(12, 2.0), candle(13, 2.0)],
                bars: Vec::new(),
                series: Vec::new(),
            }))
            .unwrap();
        chart.inbox = Some(receiver);
        chart.receive_fetch(Timeframe::HOUR_1);
        assert_eq!(open_times(&chart), vec![7, 8, 10, 11, 12, 13]);
    }

    #[test]
    fn a_refresh_replaces_the_candles_while_no_older_page_was_loaded() {
        let mut chart = Chart::blank(1, "BTCUSDT", DataSource::Binance);
        chart.candles = vec![candle(1, 1.0), candle(2, 1.0)];
        let (sender, receiver) = mpsc::channel();
        sender
            .send(Ok(Fetched {
                candles: vec![candle(2, 2.0), candle(3, 2.0)],
                bars: Vec::new(),
                series: Vec::new(),
            }))
            .unwrap();
        chart.inbox = Some(receiver);
        chart.receive_fetch(Timeframe::HOUR_1);
        assert_eq!(open_times(&chart), vec![2, 3]);
    }

    /// A chart holding live bar 2 (closing 9.0), refreshing since `begun` with a fetch that
    /// answers bar 2 closing 2.0; the live bar arrived `live_after` after the fetch began.
    fn refreshed_over_live_bar(live_after: Duration, quiet: bool) -> Chart {
        let begun = Instant::now();
        let mut chart = Chart::blank(1, "BTCUSDT", DataSource::Binance);
        chart.candles = vec![candle(1, 1.0), candle(2, 9.0)];
        chart.last_fetch = begun;
        let arrived = if quiet {
            begun - live_after
        } else {
            begun + live_after
        };
        chart.live = Some(LiveMark::new(2, arrived));
        let (sender, receiver) = mpsc::channel();
        sender
            .send(Ok(Fetched {
                candles: vec![candle(1, 1.0), candle(2, 2.0)],
                bars: Vec::new(),
                series: Vec::new(),
            }))
            .unwrap();
        chart.inbox = Some(receiver);
        chart.receive_fetch(Timeframe::HOUR_1);
        chart
    }

    #[test]
    fn a_refresh_keeps_a_live_bar_that_arrived_while_it_was_out() {
        let chart = refreshed_over_live_bar(Duration::from_millis(200), false);
        assert_eq!(chart.candles[1].close, 9.0);
    }

    #[test]
    fn a_refresh_updates_the_forming_bar_once_the_socket_went_quiet() {
        let chart = refreshed_over_live_bar(Duration::from_secs(30), true);
        assert_eq!(chart.candles[1].close, 2.0);
    }

    #[test]
    fn an_older_page_for_a_timeframe_the_chart_left_is_dropped() {
        let mut chart = chart_awaiting_older(&[10, 11], Ok(vec![candle(7, 1.0)]));
        chart.receive_older(Timeframe::DAY_1);
        assert_eq!(open_times(&chart), vec![10, 11]);
        assert!(chart.older_inbox.is_none());
    }

    #[test]
    fn a_failed_older_page_is_not_retried_at_once() {
        let mut chart = chart_awaiting_older(&[10, 11], Err(ProviderError::other("offline")));
        chart.receive_older(Timeframe::HOUR_1);
        assert_eq!(open_times(&chart), vec![10, 11]);
        assert!(
            !chart.older.should_request(true, true, 2, Instant::now()),
            "it waits before asking again"
        );
    }

    #[test]
    fn an_empty_older_page_is_the_start_of_history() {
        let mut chart = chart_awaiting_older(&[10, 11], Ok(Vec::new()));
        chart.receive_older(Timeframe::HOUR_1);
        assert!(!chart.older.should_request(true, true, 2, Instant::now()));
    }

    #[test]
    fn changing_the_market_forgets_the_history_behind_the_old_one() {
        let mut chart = chart_awaiting_older(&[10, 11], Ok(Vec::new()));
        chart.receive_older(Timeframe::HOUR_1);
        chart.reset_history();
        assert!(chart.older.should_request(true, true, 2, Instant::now()));
    }

    #[test]
    fn replacing_a_chart_from_a_watchlist_updates_its_market_metadata() {
        let mut chart = Chart::blank(1, "OldPairAddress", DataSource::DexScreener);
        chart.chart.set_title(Some("OLD/USDC".into()));
        chart.pm_series.push(PmSeries {
            token_id: "old-choice".into(),
            label: "Old choice".into(),
        });

        let mut prediction = WatchlistInstrument::from_market(
            DataSource::Polymarket,
            "Will it rain?",
            "CaseSensitiveToken",
        );
        prediction.image_url = Some("https://img/rain.png".into());
        chart.replace_market(&prediction);

        assert_eq!(chart.symbol, "CaseSensitiveToken");
        assert_eq!(chart.source, DataSource::Polymarket);
        assert_eq!(chart.display_title(), "Will it rain?");
        assert_eq!(chart.image_url, prediction.image_url);
        assert!(chart.pm_series.is_empty());

        let bitcoin = WatchlistInstrument::from_market(DataSource::Binance, "BTCUSDT", "BTCUSDT");
        chart.replace_market(&bitcoin);
        assert_eq!(chart.symbol, "BTCUSDT");
        assert_eq!(chart.display_title(), "BTCUSDT");
        assert_eq!(chart.chart.title(), None);
        assert_eq!(chart.image_url, None);
    }

    #[test]
    fn a_multi_choice_fetch_reads_candles_from_the_first_choice_with_history() {
        let bar = |time: i64| Kline {
            open_time: time,
            close_time: time + 1,
            open: 0.5,
            high: 0.5,
            low: 0.5,
            close: 0.5,
            volume: 0.0,
        };
        let fetched = Fetched::from_choices(vec![
            ("a".into(), Vec::new()),
            ("b".into(), vec![bar(1), bar(2)]),
            ("c".into(), vec![bar(3)]),
        ])
        .expect("some history");
        assert_eq!(fetched.candles.len(), 2);
        assert_eq!(fetched.series.len(), 3);
        let none = Fetched::from_choices(vec![("a".into(), Vec::new())]);
        assert!(none.is_err());
    }

    #[test]
    fn a_prediction_chart_keeps_its_title_and_choices_through_a_save() {
        let mut saved = PersistedChart {
            id: 1,
            symbol: "111".into(),
            source: DataSource::Polymarket,
            log_scale: false,
            scale_mode: None,
            y_zoom: 1.0,
            indicators: IndicatorConfig::default(),
            appearance: crate::model::ChartAppearance::default(),
            drawings: Default::default(),
            pine_instances: Vec::new(),
            candle_count: 0,
            viewport: Default::default(),
            title: Some("Fed decision".into()),
            card: None,
            pm_series: vec![
                PmSeries {
                    token_id: "111".into(),
                    label: "Cut".into(),
                },
                PmSeries {
                    token_id: "222".into(),
                    label: "Hold".into(),
                },
            ],
            image_url: Some("https://img/fed.png".into()),
            overlays: Default::default(),
        };
        let encoded = serde_json::to_value(&saved).expect("serialize");
        assert_eq!(encoded["pm_series"][0]["tokenID"], "111");
        let decoded: PersistedChart = serde_json::from_value(encoded.clone()).expect("round trip");
        assert_eq!(decoded.title.as_deref(), Some("Fed decision"));
        assert_eq!(decoded.pm_series, saved.pm_series);
        // A row from before prediction titles decodes with neither.
        let mut old = encoded;
        old.as_object_mut().unwrap().remove("title");
        old.as_object_mut().unwrap().remove("pm_series");
        old.as_object_mut().unwrap().remove("image_url");
        saved.title = None;
        saved.pm_series.clear();
        saved.image_url = None;
        let decoded: PersistedChart = serde_json::from_value(old).expect("old row");
        assert_eq!(decoded.title, None);
        assert!(decoded.pm_series.is_empty());
        assert_eq!(decoded.image_url, None);
    }
}
