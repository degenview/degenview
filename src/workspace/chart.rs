//! One chart's market data: what it holds, how it fetches, and how older history is paged in.

use super::PersistedChart;
use crate::model::history::{self, OlderHistory};
use crate::model::portfolio_card::PortfolioCard;
use crate::model::price_scale::PriceScaleMode;
use crate::model::watchlist::WatchlistInstrument;
use crate::model::{DataSource, Drawing, IndicatorConfig, Kline, PmSeries, Timeframe};
use crate::portfolio_card_host::CardRuntime;
use crate::service;
use crate::ui::{ChartCard, SeriesLine};
use eframe::egui;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

const CANDLE_LIMIT: u32 = 500;
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
    pub(super) error: Option<String>,
    pub(super) last_fetch: Instant,
    pub(super) inbox: Option<Receiver<Result<Fetched, String>>>,
    /// What is known of the history behind the oldest candle, and the request for it, if any.
    pub(super) older: OlderHistory,
    pub(super) older_inbox: Option<OlderRequest>,
    pub(super) fetch_generation: Arc<AtomicU64>,
    pub(super) chart: ChartCard,
    pub(super) scale_mode: PriceScaleMode,
    pub(super) indicators: IndicatorConfig,
    pub(super) drawings: Vec<Drawing>,
    pub(super) next_drawing_id: u64,
    pub(super) last_paper_quote: Option<(f64, Instant)>,
    pub(super) last_live_open_time: Option<i64>,
    /// Prediction markets: the choices charted together as lines, when there are several.
    pub(super) pm_series: Vec<PmSeries>,
    /// Provider artwork carried into watchlists without another metadata search.
    pub(super) image_url: Option<String>,
    /// A portfolio card instead of a market: `symbol` is the card's key and `source` a
    /// placeholder nothing reads. Cards never fetch, stream, trade or run scripts.
    pub(super) card: Option<PortfolioCard>,
    /// The card's own price demand and feeds, made the first frame it is drawn with an app that
    /// has portfolios (`ChartWorkspace::set_card_env`). Dropping it releases the demand.
    pub(super) card_runtime: Option<CardRuntime>,
}

/// What one fetch brought back: the candles the chart reads, and for a multi-choice prediction
/// market each choice's own history.
pub(super) struct Fetched {
    candles: Vec<Kline>,
    series: Vec<(String, Vec<Kline>)>,
}

impl Fetched {
    /// The candles come from the first choice that has any; the rest only draw their lines.
    fn from_choices(histories: Vec<(String, Vec<Kline>)>) -> Result<Self, String> {
        let candles = histories
            .iter()
            .find(|(_, history)| !history.is_empty())
            .map(|(_, history)| history.clone())
            .ok_or_else(|| "No price history for these choices".to_owned())?;
        Ok(Self {
            candles,
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
            next_drawing_id: 1,
            last_paper_quote: None,
            last_live_open_time: None,
            pm_series: Vec::new(),
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

    /// The name shown for the chart: an explicit display title, else its provider symbol.
    pub(super) fn display_title(&self) -> String {
        if let Some(card) = &self.card {
            return card.kind.title().to_owned();
        }
        self.chart
            .title()
            .map_or_else(|| self.source.display_symbol(&self.symbol), str::to_owned)
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
    pub(super) fn replace_market(&mut self, item: &WatchlistInstrument) {
        self.symbol = item.instrument.symbol.clone();
        self.source = item.instrument.source;
        self.apply_watchlist_metadata(item);
        self.chart.set_series(Vec::new());
        self.pm_series.clear();
        self.last_paper_quote = None;
        self.last_live_open_time = None;
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
        chart.image_url = saved.image_url;
        chart.fetch(timeframe, ctx);
        chart.scale_mode = scale_mode;
        chart.chart.set_y_zoom(saved.y_zoom);
        chart.chart.set_appearance(saved.appearance);
        chart.chart.set_pine_instances(instances);
        chart.chart.set_visible_candles(saved.candle_count as usize);
        chart.chart.restore_viewport(saved.viewport);
        chart.indicators = saved.indicators;
        chart.drawings = saved.drawings;
        chart.next_drawing_id = chart
            .drawings
            .iter()
            .map(Drawing::stored_id)
            .max()
            .map_or(1, |max| max + 1);
        chart
    }

    pub(super) fn fetch(&mut self, timeframe: Timeframe, ctx: &egui::Context) {
        if !self.is_market() {
            return;
        }
        let (sender, receiver) = mpsc::channel();
        let symbol = self.symbol.clone();
        let source = self.source;
        let interval = timeframe.interval();
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
                service::fetch_klines(source, &symbol, interval, CANDLE_LIMIT).map(|candles| {
                    Fetched {
                        candles,
                        series: Vec::new(),
                    }
                })
            } else {
                Fetched::from_choices(service::fetch_choices(
                    source,
                    &choices,
                    interval,
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
            DataSource::CoinGecko | DataSource::DexScreener if self.error.is_some() => {
                SLOW_RETRY_INTERVAL
            }
            DataSource::CoinGecko | DataSource::DexScreener => SLOW_REFRESH_INTERVAL,
            _ => REFRESH_INTERVAL,
        }
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
        let Some(end) = history::page_end(&self.candles) else {
            return;
        };
        let wanted = self.older.should_request(
            self.source.supports_older_history(timeframe),
            self.chart.near_oldest_bar() || self.older.untouched(),
            self.candles.len(),
            Instant::now(),
        );
        if !wanted {
            return;
        }
        let (sender, receiver) = mpsc::channel();
        let (source, symbol) = (self.source, self.symbol.clone());
        let interval = timeframe.interval();
        let repaint = ctx.clone();
        let asked = symbol.clone();
        service::work_queue::submit_for(source, move || {
            let page = service::fetch_klines_range(source, &asked, interval, 0, end, history::PAGE);
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
                Err("Older candles stopped loading".to_owned())
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
                let added = history::prepend(&mut self.candles, page);
                self.older.arrived(added);
            }
            Err(_) => self.older.failed(Instant::now()),
        }
    }

    pub(super) fn receive_fetch(&mut self) {
        let Some(receiver) = &self.inbox else {
            return;
        };
        let result = match receiver.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => {
                Err("Chart fetch stopped before returning data".to_owned())
            }
        };

        self.loading = false;
        self.inbox = None;
        match result {
            Ok(fetched) => {
                let fresh =
                    merge_fetched_candles(fetched.candles, &self.candles, self.last_live_open_time);
                self.candles = history::with_older(&self.candles, fresh, self.older.keeps_older());
                if !self.pm_series.is_empty() {
                    self.chart
                        .set_series(SeriesLine::build(&self.pm_series, fetched.series));
                }
                self.error = None;
            }
            Err(message) => self.error = Some(message),
        }
    }
}

/// A request for the candles before the oldest one, with what it was asked for: an answer for a
/// market or timeframe the chart has left is dropped.
pub(super) struct OlderRequest {
    receiver: Receiver<Result<Vec<Kline>, String>>,
    source: DataSource,
    symbol: String,
    timeframe: Timeframe,
}

/// A websocket update for the current or a later bar wins over an older REST snapshot.
fn merge_fetched_candles(
    mut fetched: Vec<Kline>,
    current: &[Kline],
    last_live_open_time: Option<i64>,
) -> Vec<Kline> {
    if let Some(live_open) = last_live_open_time
        && let Some(last_fetched) = fetched.last()
    {
        let newer = current
            .partition_point(|candle| candle.open_time < live_open.max(last_fetched.open_time));
        if newer < current.len() {
            let first_newer = current[newer].open_time;
            fetched.retain(|candle| candle.open_time < first_newer);
            fetched.extend_from_slice(&current[newer..]);
        }
    }
    if fetched.len() > CANDLE_LIMIT as usize {
        fetched.drain(..fetched.len() - CANDLE_LIMIT as usize);
    }
    fetched
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

    #[test]
    fn live_candle_wins_over_an_older_rest_snapshot() {
        let fetched = vec![candle(1, 10.0), candle(2, 20.0)];
        let live = vec![candle(1, 10.0), candle(2, 25.0), candle(3, 30.0)];
        assert_eq!(merge_fetched_candles(fetched.clone(), &live, Some(2)), live);
        assert_eq!(merge_fetched_candles(fetched.clone(), &live, None), fetched);
    }

    /// A chart holding `times`, with a request for older candles outstanding that answers `page`.
    fn chart_awaiting_older(times: &[i64], page: Result<Vec<Kline>, String>) -> Chart {
        let mut chart = Chart::blank(1, "BTCUSDT", DataSource::Binance);
        chart.candles = times.iter().map(|time| candle(*time, 1.0)).collect();
        let (sender, receiver) = mpsc::channel();
        sender.send(page).unwrap();
        chart.older.started();
        chart.older_inbox = Some(OlderRequest {
            receiver,
            source: DataSource::Binance,
            symbol: "BTCUSDT".into(),
            timeframe: Timeframe::Hour,
        });
        chart
    }

    #[test]
    fn rate_limited_sources_refresh_less_often_and_back_off_after_an_error() {
        let mut binance = Chart::blank(1, "BTCUSDT", DataSource::Binance);
        let mut dex = Chart::blank(2, "0xpool", DataSource::DexScreener);
        assert_eq!(binance.refresh_after(), REFRESH_INTERVAL);
        assert_eq!(dex.refresh_after(), SLOW_REFRESH_INTERVAL);
        binance.error = Some("down".into());
        dex.error = Some("429".into());
        assert_eq!(binance.refresh_after(), REFRESH_INTERVAL);
        assert_eq!(dex.refresh_after(), SLOW_RETRY_INTERVAL);
    }

    fn open_times(chart: &Chart) -> Vec<i64> {
        chart.candles.iter().map(|bar| bar.open_time).collect()
    }

    #[test]
    fn an_older_page_goes_in_front_and_a_refresh_does_not_lose_it() {
        let mut chart =
            chart_awaiting_older(&[10, 11, 12], Ok(vec![candle(7, 1.0), candle(8, 1.0)]));
        chart.receive_older(Timeframe::Hour);
        assert_eq!(open_times(&chart), vec![7, 8, 10, 11, 12]);
        assert!(chart.older_inbox.is_none());

        // The next refresh brings only the newest candles.
        let (sender, receiver) = mpsc::channel();
        sender
            .send(Ok(Fetched {
                candles: vec![candle(11, 2.0), candle(12, 2.0), candle(13, 2.0)],
                series: Vec::new(),
            }))
            .unwrap();
        chart.inbox = Some(receiver);
        chart.receive_fetch();
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
                series: Vec::new(),
            }))
            .unwrap();
        chart.inbox = Some(receiver);
        chart.receive_fetch();
        assert_eq!(open_times(&chart), vec![2, 3]);
    }

    #[test]
    fn an_older_page_for_a_timeframe_the_chart_left_is_dropped() {
        let mut chart = chart_awaiting_older(&[10, 11], Ok(vec![candle(7, 1.0)]));
        chart.receive_older(Timeframe::Day);
        assert_eq!(open_times(&chart), vec![10, 11]);
        assert!(chart.older_inbox.is_none());
    }

    #[test]
    fn a_failed_older_page_is_not_retried_at_once() {
        let mut chart = chart_awaiting_older(&[10, 11], Err("offline".into()));
        chart.receive_older(Timeframe::Hour);
        assert_eq!(open_times(&chart), vec![10, 11]);
        assert!(
            !chart.older.should_request(true, true, 2, Instant::now()),
            "it waits before asking again"
        );
    }

    #[test]
    fn an_empty_older_page_is_the_start_of_history() {
        let mut chart = chart_awaiting_older(&[10, 11], Ok(Vec::new()));
        chart.receive_older(Timeframe::Hour);
        assert!(!chart.older.should_request(true, true, 2, Instant::now()));
    }

    #[test]
    fn changing_the_market_forgets_the_history_behind_the_old_one() {
        let mut chart = chart_awaiting_older(&[10, 11], Ok(Vec::new()));
        chart.receive_older(Timeframe::Hour);
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
        assert_eq!(chart.display_title(), "BTC/USDT");
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
            drawings: Vec::new(),
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
