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
use crate::model::chart::countdown::{Countdown, CountdownInput};
use crate::model::chart::price_scale::PriceScaleMode;
use crate::model::drawings::market::MarketDrawings;
use crate::model::market;
use crate::model::market::history::OlderHistory;
use crate::model::market::live::{LiveFold, LiveMark};
use crate::model::overlay::OverlayList;
use crate::model::pine::bundled;
use crate::model::portfolio::card::PortfolioCard;
use crate::model::watchlist::WatchlistInstrument;
use crate::model::{DataSource, Drawing, IndicatorConfig, Kline, PmSeries, Timeframe};
use crate::service::market::error::ErrorKind;
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
    /// The timeframe the candles held were fetched for. A timeframe chosen since is not theirs
    /// until its answer arrives, and nothing counts down a bar of a length the candles are not.
    data_timeframe: Option<Timeframe>,
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
    fn from_choices(
        source: DataSource,
        histories: Vec<(String, Vec<Kline>)>,
    ) -> Result<Self, ProviderError> {
        let candles = histories
            .iter()
            .find(|(_, history)| !history.is_empty())
            .map(|(_, history)| history.clone())
            .ok_or_else(|| ProviderError::new(source, ErrorKind::NoData))?;
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
            data_timeframe: None,
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
            return card.kind.display_title().to_owned();
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
            | DataSource::Alpaca
            | DataSource::Oanda => self.source.display_symbol(&self.symbol),
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
        self.data_timeframe = None;
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
        chart.adopt_legacy_rsi();
        if !chart.source.takes_indicators() {
            // A layout saved before these charts stopped taking indicators.
            chart.indicators = IndicatorConfig::default();
            chart.chart.set_pine_instances(Vec::new());
        }
        chart.set_stored_drawings(saved.drawings);
        chart
    }

    /// A layout saved before RSI had a pane of its own holds it as a flag of the built-in
    /// indicators, drawn as a band of the price plot. It becomes the bundled RSI, hidden if its eye
    /// was off, so the chart keeps its RSI where an RSI belongs. A chart that already has the
    /// bundled one (another build wrote the flag back) keeps just that.
    fn adopt_legacy_rsi(&mut self) {
        let Some(hidden) = self.indicators.take_legacy_rsi() else {
            return;
        };
        let Some(rsi) = bundled::find("rsi") else {
            return;
        };
        let script_id = bundled::script_id(rsi);
        let has_it = self
            .chart
            .pine_instances()
            .iter()
            .any(|instance| instance.script_id.as_deref() == Some(script_id.as_str()));
        if !has_it {
            let id = self
                .chart
                .add_pine_instance(Some(script_id), rsi.name.text(), rsi.source);
            self.chart.set_pine_visible(id, !hidden);
        }
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
                Fetched::from_choices(
                    source,
                    service::fetch_choices(source, &choices, timeframe, CANDLE_LIMIT),
                )
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
        match self.refresh_cadence() {
            SLOW_REFRESH_INTERVAL if self.error.is_some() => SLOW_RETRY_INTERVAL,
            cadence => cadence,
        }
    }

    /// Records that candles for `timeframe` have just arrived, from a refresh or the live feed.
    /// The newest candle is what a countdown reads, so a live bar accepted for the timeframe is
    /// a bar of it even while the history behind it is still on its way.
    pub(super) fn note_arrival(&mut self, timeframe: Timeframe, now_ms: i64) {
        self.last_data_ms = Some(now_ms);
        self.data_timeframe = Some(timeframe);
    }

    /// How often the source is asked for candles while it answers.
    fn refresh_cadence(&self) -> Duration {
        match self.source {
            DataSource::CoinGecko | DataSource::DexScreener | DataSource::Kraken => {
                SLOW_REFRESH_INTERVAL
            }
            _ => REFRESH_INTERVAL,
        }
    }

    /// The time the forming bar has left, for a chart of `timeframe` at `now_ms`, or `None` where
    /// the chart has nothing to promise: the setting is off, a replay cuts it, or the data is not
    /// a live bar of this timeframe (see [`Countdown::resolve`]).
    pub(super) fn bar_countdown(
        &self,
        timeframe: Timeframe,
        replay: bool,
        now_ms: i64,
    ) -> Option<Countdown> {
        let appearance = self.chart.appearance();
        if !appearance.canvas.bar_countdown || self.data_timeframe != Some(timeframe) {
            return None;
        }
        Countdown::resolve(&CountdownInput {
            chart_type: appearance.style.effective_type(self.source),
            source: self.source,
            timeframe,
            newest: self.candles.last(),
            now_ms,
            data_age_ms: self.last_data_ms.map(|at| now_ms - at),
            refresh_ms: self.refresh_cadence().as_millis() as i64,
            replay,
        })
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
                Err(ProviderError::interrupted(request.source))
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
            Err(mpsc::TryRecvError::Disconnected) => Err(ProviderError::interrupted(self.source)),
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
                let refreshed = market::history::refreshed(self.source, &self.candles, fresh);
                if refreshed.restarted {
                    self.reset_history();
                }
                self.candles = refreshed.candles;
                if !self.pm_series.is_empty() {
                    self.chart
                        .set_series(SeriesLine::build(&self.pm_series, fetched.series));
                }
                self.error = None;
                self.note_arrival(timeframe, crate::clock::now_ms());
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
    use crate::pine::compiler::NoLibraries;
    use crate::pine::runtime::execution::ExecutionController;
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

    const NOW: i64 = 1_800_000_000_000;

    /// A Binance chart of 5-minute bars whose newest candle is the bar of `NOW`, answered a
    /// second ago.
    fn live_chart() -> Chart {
        let mut chart = Chart::blank(1, "BTCUSDT", DataSource::Binance);
        let open = crate::model::market::bucket::bucket_start_ms(Timeframe::MINUTE_5, NOW);
        let mut newest = candle(open, 1.0);
        newest.close_time = open + 5 * 60_000 - 1;
        chart.candles = vec![newest];
        chart.last_data_ms = Some(NOW - 1_000);
        chart.data_timeframe = Some(Timeframe::MINUTE_5);
        chart
    }

    fn seconds_left(chart: &Chart, timeframe: Timeframe, replay: bool) -> Option<i64> {
        chart
            .bar_countdown(timeframe, replay, NOW)
            .map(|countdown| countdown.remaining_ms / 1_000)
    }

    #[test]
    fn a_live_chart_counts_down_to_the_close_of_its_bar() {
        let chart = live_chart();
        let left = seconds_left(&chart, Timeframe::MINUTE_5, false).expect("counts");
        assert!((0..=300).contains(&left), "{left}");
    }

    #[test]
    fn a_countdown_belongs_to_the_timeframe_the_candles_were_fetched_for() {
        let mut chart = live_chart();
        assert_eq!(seconds_left(&chart, Timeframe::MINUTE_1, false), None);
        // The answer for the new timeframe arrives: candles of one minute.
        chart.data_timeframe = Some(Timeframe::MINUTE_1);
        let open = crate::model::market::bucket::bucket_start_ms(Timeframe::MINUTE_1, NOW);
        let mut minute = candle(open, 1.0);
        minute.close_time = open + 60_000 - 1;
        chart.candles = vec![minute];
        assert!(seconds_left(&chart, Timeframe::MINUTE_1, false).is_some());
        chart.clear_market_data();
        assert_eq!(seconds_left(&chart, Timeframe::MINUTE_1, false), None);
    }

    #[test]
    fn a_countdown_is_off_in_a_replay_and_when_the_setting_is_off() {
        let mut chart = live_chart();
        assert_eq!(seconds_left(&chart, Timeframe::MINUTE_5, true), None);
        chart.chart.appearance_mut().canvas.bar_countdown = false;
        assert_eq!(seconds_left(&chart, Timeframe::MINUTE_5, false), None);
    }

    #[test]
    fn a_failed_refresh_alone_does_not_hide_a_countdown_but_silence_does() {
        let mut chart = live_chart();
        chart.error = Some(ProviderError::refused(
            crate::model::DataSource::Binance,
            "down",
        ));
        assert!(seconds_left(&chart, Timeframe::MINUTE_5, false).is_some());
        chart.last_data_ms = Some(NOW - 16_000);
        assert_eq!(
            seconds_left(&chart, Timeframe::MINUTE_5, false),
            None,
            "three refreshes without an answer"
        );
    }

    #[test]
    fn a_chart_of_a_session_market_has_no_countdown() {
        let mut chart = live_chart();
        chart.source = DataSource::Alpaca;
        assert_eq!(seconds_left(&chart, Timeframe::MINUTE_5, false), None);
    }

    #[test]
    fn rate_limited_sources_refresh_less_often_and_back_off_after_an_error() {
        let mut binance = Chart::blank(1, "BTCUSDT", DataSource::Binance);
        let mut dex = Chart::blank(2, "0xpool", DataSource::DexScreener);
        assert_eq!(binance.refresh_after(), REFRESH_INTERVAL);
        assert_eq!(dex.refresh_after(), SLOW_REFRESH_INTERVAL);
        binance.error = Some(ProviderError::refused(
            crate::model::DataSource::Binance,
            "down",
        ));
        dex.error = Some(ProviderError::http(
            DataSource::DexScreener,
            crate::service::net::http::Error::Status(429),
        ));
        assert_eq!(binance.refresh_after(), REFRESH_INTERVAL);
        assert_eq!(dex.refresh_after(), SLOW_RETRY_INTERVAL);
    }

    #[test]
    fn kraken_charts_refresh_slowly_because_the_socket_keeps_them_live() {
        let mut kraken = Chart::blank(1, "BTC/USD", DataSource::Kraken);
        assert_eq!(kraken.refresh_after(), SLOW_REFRESH_INTERVAL);
        kraken.error = Some(ProviderError::http(
            DataSource::Kraken,
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
            chart
                .error
                .as_ref()
                .map(crate::service::provider_message::message)
                .as_deref(),
            Some("CoinGecko has no 1m history")
        );
        chart.last_fetch = Instant::now() - Duration::from_secs(3_600);
        assert!(
            !chart.refresh_due(),
            "asking again cannot change the answer"
        );

        // Any other failure is retried as before, and a fetch for another timeframe or market
        // (which does not wait for this check) replaces the error with its own answer.
        chart.error = Some(ProviderError::refused(
            crate::model::DataSource::Binance,
            "down",
        ));
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
        refresh(
            &mut chart,
            vec![candle(11, 2.0), candle(12, 2.0), candle(13, 2.0)],
        );
        assert_eq!(open_times(&chart), vec![7, 8, 10, 11, 12, 13]);
    }

    /// Delivers `candles` to `chart` as the answer to a refresh.
    fn refresh(chart: &mut Chart, candles: Vec<Kline>) {
        let (sender, receiver) = mpsc::channel();
        sender
            .send(Ok(Fetched {
                candles,
                bars: Vec::new(),
                series: Vec::new(),
            }))
            .unwrap();
        chart.inbox = Some(receiver);
        chart.receive_fetch(Timeframe::HOUR_1);
    }

    #[test]
    fn a_refresh_keeps_the_candles_it_did_not_bring() {
        let mut chart = Chart::blank(1, "bitcoin", DataSource::CoinGecko);
        refresh(&mut chart, vec![candle(1, 1.0), candle(2, 1.0)]);
        refresh(&mut chart, vec![candle(2, 2.0), candle(3, 2.0)]);
        assert_eq!(open_times(&chart), vec![1, 2, 3]);
    }

    #[test]
    fn a_refresh_that_leaves_a_gap_starts_the_history_again() {
        let mut chart = chart_awaiting_older(&[10, 11], Ok(Vec::new()));
        chart.receive_older(Timeframe::HOUR_1);
        assert!(!chart.older.should_request(true, true, 2, Instant::now()));
        refresh(&mut chart, vec![candle(20, 1.0), candle(21, 1.0)]);
        assert_eq!(open_times(&chart), vec![20, 21]);
        assert!(
            chart.older.should_request(true, true, 2, Instant::now()),
            "what was behind the old candles says nothing about these"
        );
    }

    /// A source that answers a window of its newest candles leaves one out for each that opens.
    /// The scripts must go on running over the same history: a replay raises no alert, and it
    /// takes the new bar's `alert()`, once per bar unless the script says otherwise, with it.
    #[test]
    fn a_new_bar_raises_its_alert_while_the_source_slides_its_window() {
        const HOUR: i64 = 3_600_000;
        let window = |from: i64| -> Vec<Kline> {
            (from..from + 5)
                .map(|hour| Kline {
                    close_time: (hour + 1) * HOUR - 1,
                    ..candle(hour * HOUR, 1.0)
                })
                .collect()
        };
        let mut script = ExecutionController::new(
            "//@version=6\nindicator(\"S\")\nalert(\"hi\")",
            Default::default(),
            Default::default(),
            None,
            Default::default(),
            &NoLibraries,
            None,
            None,
        )
        .expect("compiles")
        .with_alert_mode();
        let mut chart = Chart::blank(1, "bitcoin", DataSource::CoinGecko);
        refresh(&mut chart, window(0));
        script.update(&chart.candles, None, 4 * HOUR + 1);
        // The app ticks several times a second: one passes the close before the next candle comes.
        script.update(&chart.candles, None, 5 * HOUR);

        refresh(&mut chart, window(1));
        let update = script.update(&chart.candles, None, 5 * HOUR + 5_000);
        assert!(!update.rebuilt, "the same history, one candle longer");
        assert_eq!(update.alerts.len(), 1);
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
        let mut chart = chart_awaiting_older(
            &[10, 11],
            Err(ProviderError::refused(
                crate::model::DataSource::Binance,
                "offline",
            )),
        );
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
        let fetched = Fetched::from_choices(
            DataSource::Polymarket,
            vec![
                ("a".into(), Vec::new()),
                ("b".into(), vec![bar(1), bar(2)]),
                ("c".into(), vec![bar(3)]),
            ],
        )
        .expect("some history");
        assert_eq!(fetched.candles.len(), 2);
        assert_eq!(fetched.series.len(), 3);
        let none = Fetched::from_choices(DataSource::Polymarket, vec![("a".into(), Vec::new())]);
        assert_eq!(
            none.err(),
            Some(ProviderError::new(
                DataSource::Polymarket,
                ErrorKind::NoData
            ))
        );
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
