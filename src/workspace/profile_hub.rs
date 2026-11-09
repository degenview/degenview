//! The finer data behind the order-flow chart types, fetched once per market and shared.
//!
//! A chart that draws a footprint, a TPO profile or a session volume profile **needs** more than
//! its own candles: lower-timeframe candles (with Binance's taker-buy volume) and, on Binance, the
//! trades. It says so with a [`ProfileNeed`]; this hub holds the data, the chart holds nothing but
//! the question. Like the overlay hub, one entry exists per market however many charts name it, it
//! lives only while some chart holds it, and nothing here blocks: every request runs on the work
//! queue and answers over a channel (candles) or into the shared book (trades), polled once a frame.
//!
//! What is bounded, and by what:
//!
//! - **Candles** per series: [`MAX_CANDLES`] (the same cap Pine's intrabars use), the oldest dropped.
//! - **Trades** per market: [`TRADE_WINDOW_MS`] of history and [`TRADE_BUDGET`] trades, newest
//!   first, so a slow or refused request still leaves the recent part complete. The book keeps at
//!   most [`TRADE_CAPACITY`] and says how far back it is gap free.
//! - **Requests**: [`MAX_IN_FLIGHT`] candle requests at once, trades behind one rate limiter.
//!
//! The computation (footprints, profiles) does not live here: it reads the shared [`CandleData`] and
//! [`TradeBook`] on its own worker and compares [`CandleShared::revision`] to know when to redo.
//! Replay is the computation's job too: it asks for data **up to** the replay position, so nothing
//! here ever holds back or drops what the market already did.

use crate::model::market_profile::flow::{self, CandleBook, FlowKline};
use crate::model::market_profile::trades::{Trade, TradeBook};
use crate::model::{DataSource, InstrumentId};
use crate::service::binance_trades::{HistoryEnd, TradeInbox};
use eframe::egui;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

/// Candle requests that may run at once.
pub const MAX_IN_FLIGHT: usize = 2;
/// Most candles one series holds (and one request asks for).
pub const MAX_CANDLES: usize = flow::DEFAULT_CAPACITY;
/// How far back trades are fetched at most.
pub const TRADE_WINDOW_MS: i64 = 6 * 3_600_000;
/// Most trades one history fetch spends (a busy market has more in the window than this).
pub const TRADE_BUDGET: usize = 1_000_000;
/// Most trades a market's book keeps.
pub const TRADE_CAPACITY: usize = 1_000_000;
/// Missed live ids a reconnect backfills at most; a larger gap is left as a gap.
pub const MAX_BACKFILL: u64 = 200_000;
/// How often the newest candles are asked for again.
const TAIL_REFRESH: Duration = Duration::from_secs(15);
const SLOW_TAIL_REFRESH: Duration = Duration::from_secs(45);
const RETRY_AFTER: Duration = Duration::from_secs(20);
/// Candles a refresh asks for: the forming one and the last few, in case one was revised.
const TAIL_CANDLES: i64 = 3;

/// What a chart asks the hub for.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProfileNeed {
    pub instrument: InstrumentId,
    /// The provider's interval token of the lower-timeframe candles (`1m`, `5m`, `30m`, `1h`).
    pub interval: &'static str,
    pub interval_ms: i64,
    /// The oldest time the chart needs data for.
    pub from_ms: i64,
    /// The end of what it needs: `i64::MAX` for a live chart (everything up to now), the end of the
    /// last displayed bar in replay, where the candles it needs are not the newest ones.
    pub to_ms: i64,
    /// Whether trades are wanted too (honoured where the source has them: Binance).
    pub trades: bool,
}

/// Where a series stands, for the chart's status line.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FlowStatus {
    Queued,
    Loading,
    Ready,
    /// The last request failed. Candles from before it, if any, are kept.
    Error(Arc<str>),
}

/// How the trades of a market stand.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TradeStatus {
    /// History is loading (the stream is already collecting).
    Loading,
    /// History is in and the stream is the source of new trades.
    Live,
    /// History ended early: `HistoryEnd` says why. What arrived is kept and is gap free.
    Partial(HistoryEnd),
}

// MARK: Backend

/// A live trade stream held open for a market; dropping the guard closes it.
pub struct LiveTrades {
    pub inbox: Arc<TradeInbox>,
    /// Dropping it closes the stream; never read.
    #[allow(dead_code)]
    pub guard: Box<dyn Send>,
}

/// How the hub reaches the services. The workspace uses the real ones; tests replace it.
pub trait ProfileBackend: Send + Sync + 'static {
    /// Runs `job` off the UI thread, on `source`'s lane of the work queue.
    fn submit(&self, source: DataSource, job: Box<dyn FnOnce() + Send>);
    fn candles(
        &self,
        source: DataSource,
        symbol: &str,
        interval: &str,
        start_ms: i64,
        end_ms: i64,
        max: usize,
    ) -> Result<Vec<FlowKline>, String>;
    fn tick_size(&self, source: DataSource, symbol: &str) -> Option<f64>;
    /// Whether `source` serves trades at all.
    fn has_trades(&self, source: DataSource) -> bool;
    fn trade_history(
        &self,
        symbol: &str,
        window_start_ms: i64,
        budget: usize,
        cancel: &AtomicBool,
        sink: &mut dyn FnMut(Vec<Trade>),
    ) -> HistoryEnd;
    fn trade_range(
        &self,
        symbol: &str,
        from_id: u64,
        to_id: u64,
        cancel: &AtomicBool,
        sink: &mut dyn FnMut(Vec<Trade>),
    ) -> HistoryEnd;
    fn open_trades(&self, symbol: &str, ctx: &egui::Context) -> Option<LiveTrades>;
}

struct ServiceBackend;

impl ProfileBackend for ServiceBackend {
    fn submit(&self, source: DataSource, job: Box<dyn FnOnce() + Send>) {
        crate::service::work_queue::submit_for(source, job);
    }

    fn candles(
        &self,
        source: DataSource,
        symbol: &str,
        interval: &str,
        start_ms: i64,
        end_ms: i64,
        max: usize,
    ) -> Result<Vec<FlowKline>, String> {
        crate::service::fetch_flow_range(source, symbol, interval, start_ms, end_ms, max)
    }

    fn tick_size(&self, source: DataSource, symbol: &str) -> Option<f64> {
        crate::service::tick_size(source, symbol)
    }

    fn has_trades(&self, source: DataSource) -> bool {
        source == DataSource::Binance
    }

    fn trade_history(
        &self,
        symbol: &str,
        window_start_ms: i64,
        budget: usize,
        cancel: &AtomicBool,
        sink: &mut dyn FnMut(Vec<Trade>),
    ) -> HistoryEnd {
        crate::service::binance_trades::fetch_history(
            &crate::service::binance_trades::BinanceRest,
            symbol,
            window_start_ms,
            budget,
            cancel,
            &|wait| std::thread::sleep(wait),
            sink,
        )
    }

    fn trade_range(
        &self,
        symbol: &str,
        from_id: u64,
        to_id: u64,
        cancel: &AtomicBool,
        sink: &mut dyn FnMut(Vec<Trade>),
    ) -> HistoryEnd {
        crate::service::binance_trades::fetch_range(
            &crate::service::binance_trades::BinanceRest,
            symbol,
            from_id,
            to_id,
            cancel,
            &|wait| std::thread::sleep(wait),
            sink,
        )
    }

    fn open_trades(&self, symbol: &str, ctx: &egui::Context) -> Option<LiveTrades> {
        let feed = crate::service::binance_trades::BinanceTradeFeed::connect(symbol, ctx.clone());
        Some(LiveTrades {
            inbox: Arc::clone(&feed.inbox),
            guard: Box::new(feed),
        })
    }
}

// MARK: Shared data

/// One candle series' data, read by the computation under the lock.
pub struct CandleData {
    pub book: CandleBook,
    /// The exchange's price step where it says; the computation infers one (and says so) otherwise.
    pub tick_size: Option<f64>,
}

/// A candle series shared with the computation: the data and a counter that changes whenever the
/// data does.
pub struct CandleShared {
    pub data: Mutex<CandleData>,
    pub revision: AtomicU64,
}

/// A market's trades shared with the computation.
pub struct TradeShared {
    pub book: Mutex<TradeBook>,
    pub revision: AtomicU64,
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The lock if nobody holds it right now. The UI thread uses this: a computation reads the shared
/// data under the lock for as long as it runs, and a frame must never wait for it. What could not be
/// stored is kept and stored on a later frame.
fn try_lock<T>(mutex: &Mutex<T>) -> Option<std::sync::MutexGuard<'_, T>> {
    match mutex.try_lock() {
        Ok(guard) => Some(guard),
        Err(std::sync::TryLockError::Poisoned(poisoned)) => Some(poisoned.into_inner()),
        Err(std::sync::TryLockError::WouldBlock) => None,
    }
}

// MARK: Series

/// What a candle request brings back.
struct Loaded {
    candles: Vec<FlowKline>,
    tick_size: Option<f64>,
    /// The start the request asked for.
    from_ms: i64,
    kind: Kind,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Kind {
    /// The first load: from the need's start to its end (or now).
    First,
    /// Older candles: reaches back to `from_ms`.
    Older,
    /// Candles after the newest held, up to the need's end (replay moving forward).
    Newer,
    /// The newest few.
    Tail,
}

struct CandleSeries {
    instrument: InstrumentId,
    interval: &'static str,
    interval_ms: i64,
    /// Holder chart id → the oldest time it needs.
    holders: HashMap<u64, (i64, i64)>,
    shared: Arc<CandleShared>,
    status: FlowStatus,
    generation: Arc<AtomicU64>,
    inbox: Option<Receiver<Result<Loaded, String>>>,
    /// How far back the held candles are known to reach: what was asked for, or the oldest the
    /// provider gave when it gave less (then asking again would be asking for what it cannot).
    reached_ms: Option<i64>,
    /// The provider gave less than was asked for (its depth limit, or the market is younger): no
    /// older start is asked for again.
    exhausted: bool,
    last_tail: Option<Instant>,
    retry_at: Option<Instant>,
    loaded_once: bool,
    /// An answer that could not be stored because a computation held the data; tried again next frame.
    pending: Option<Loaded>,
    /// The oldest candle held, kept here so asking for older ones never needs the lock.
    oldest_held: Option<i64>,
    /// And the newest.
    newest_held: Option<i64>,
    /// The end a newer-candles request was last made for, so an answer with nothing newer is not
    /// asked for again until the need moves.
    newer_asked: Option<i64>,
}

impl CandleSeries {
    fn new(need: &ProfileNeed) -> Self {
        Self {
            instrument: need.instrument.clone(),
            interval: need.interval,
            interval_ms: need.interval_ms,
            holders: HashMap::new(),
            shared: Arc::new(CandleShared {
                data: Mutex::new(CandleData {
                    book: CandleBook::new(need.interval_ms, MAX_CANDLES),
                    tick_size: None,
                }),
                revision: AtomicU64::new(0),
            }),
            status: FlowStatus::Queued,
            generation: Arc::new(AtomicU64::new(0)),
            inbox: None,
            reached_ms: None,
            exhausted: false,
            last_tail: None,
            retry_at: None,
            loaded_once: false,
            pending: None,
            oldest_held: None,
            newest_held: None,
            newer_asked: None,
        }
    }

    fn wanted_from(&self) -> Option<i64> {
        self.holders.values().map(|(from, _)| *from).min()
    }

    fn wanted_to(&self) -> Option<i64> {
        self.holders.values().map(|(_, to)| *to).max()
    }

    fn busy(&self) -> bool {
        self.inbox.is_some() || self.pending.is_some()
    }
}

struct TradeSeries {
    instrument: InstrumentId,
    holders: HashMap<u64, i64>,
    shared: Arc<TradeShared>,
    status: Arc<Mutex<TradeStatus>>,
    /// Set to stop the history and backfill jobs when the market is dropped.
    cancel: Arc<AtomicBool>,
    live: Option<LiveTrades>,
    /// Live trades that could not be stored yet because a computation held the book.
    stash: Vec<Trade>,
    reconnects_seen: u64,
    history_started: bool,
    /// Whether a backfill job runs, so two never overlap.
    backfilling: Arc<AtomicBool>,
    /// Id ranges `(from, to)` (`to` exclusive) the stream skipped and no backfill has asked for yet:
    /// a gap found while a backfill runs waits here instead of being forgotten.
    gaps: Vec<(u64, u64)>,
}

fn key_of(instrument: &InstrumentId) -> String {
    instrument.key()
}

fn candle_key(instrument: &InstrumentId, interval: &str) -> String {
    format!("{}|{interval}", instrument.key())
}

fn tail_refresh(source: DataSource) -> Duration {
    match source {
        DataSource::CoinGecko | DataSource::DexScreener => SLOW_TAIL_REFRESH,
        _ => TAIL_REFRESH,
    }
}

// MARK: Hub

pub struct ProfileHub {
    backend: Arc<dyn ProfileBackend>,
    candles: HashMap<String, CandleSeries>,
    trades: HashMap<String, TradeSeries>,
    /// Chart id → what it holds, to tell a changed need from an unchanged one without allocating.
    held: HashMap<u64, Vec<ProfileNeed>>,
    version: u64,
}

impl ProfileHub {
    pub fn new() -> Self {
        Self::with_backend(Arc::new(ServiceBackend))
    }

    pub fn with_backend(backend: Arc<dyn ProfileBackend>) -> Self {
        Self {
            backend,
            candles: HashMap::new(),
            trades: HashMap::new(),
            held: HashMap::new(),
            version: 0,
        }
    }

    /// Changes whenever data or state a chart shows may differ from the last frame's.
    #[cfg(test)]
    pub fn version(&self) -> u64 {
        self.version
    }

    /// Whether nothing is held for any chart.
    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.candles.is_empty() && self.trades.is_empty()
    }

    pub fn in_flight(&self) -> usize {
        self.candles.values().filter(|series| series.busy()).count()
    }

    /// The candles of a market at an interval, with the series' state, if some chart holds it.
    pub fn candles(
        &self,
        instrument: &InstrumentId,
        interval: &str,
    ) -> Option<(Arc<CandleShared>, FlowStatus)> {
        self.candles
            .get(&candle_key(instrument, interval))
            .map(|series| (Arc::clone(&series.shared), series.status.clone()))
    }

    /// The trades of a market, if some chart holds them.
    pub fn trades(&self, instrument: &InstrumentId) -> Option<(Arc<TradeShared>, TradeStatus)> {
        self.trades
            .get(&key_of(instrument))
            .map(|series| (Arc::clone(&series.shared), lock(&series.status).clone()))
    }

    /// Makes the hub hold exactly what `needs` asks for on behalf of `chart_id`. Cheap when
    /// nothing changed: a comparison, no allocation.
    pub fn sync_chart(&mut self, chart_id: u64, needs: &[ProfileNeed]) {
        if self.held.get(&chart_id).is_some_and(|held| held == needs) {
            return;
        }
        let old = self.held.remove(&chart_id).unwrap_or_default();
        for need in &old {
            if let Some(series) = self
                .candles
                .get_mut(&candle_key(&need.instrument, need.interval))
            {
                series.holders.remove(&chart_id);
            }
            if let Some(series) = self.trades.get_mut(&key_of(&need.instrument)) {
                series.holders.remove(&chart_id);
            }
        }
        for need in needs {
            let series = self
                .candles
                .entry(candle_key(&need.instrument, need.interval))
                .or_insert_with(|| CandleSeries::new(need));
            series.holders.insert(chart_id, (need.from_ms, need.to_ms));
            if need.trades && self.backend.has_trades(need.instrument.source) {
                let trades = self
                    .trades
                    .entry(key_of(&need.instrument))
                    .or_insert_with(|| TradeSeries {
                        instrument: need.instrument.clone(),
                        holders: HashMap::new(),
                        shared: Arc::new(TradeShared {
                            book: Mutex::new(TradeBook::with_capacity(TRADE_CAPACITY)),
                            revision: AtomicU64::new(0),
                        }),
                        status: Arc::new(Mutex::new(TradeStatus::Loading)),
                        cancel: Arc::new(AtomicBool::new(false)),
                        live: None,
                        stash: Vec::new(),
                        reconnects_seen: 0,
                        history_started: false,
                        backfilling: Arc::new(AtomicBool::new(false)),
                        gaps: Vec::new(),
                    });
                trades.holders.insert(chart_id, need.from_ms);
            }
        }
        self.held.insert(chart_id, needs.to_vec());
        self.prune();
        self.version += 1;
    }

    /// A chart went away: drops what only it held.
    pub fn forget_chart(&mut self, chart_id: u64) {
        if self.held.contains_key(&chart_id) {
            self.sync_chart(chart_id, &[]);
            self.held.remove(&chart_id);
        }
    }

    /// Drops the series nothing holds. Bumping a generation turns a job not yet started into a
    /// no-op; a running one answers a receiver that is gone; trade jobs see `cancel`.
    fn prune(&mut self) {
        self.candles.retain(|_, series| {
            let keep = !series.holders.is_empty();
            if !keep {
                series.generation.fetch_add(1, Ordering::AcqRel);
            }
            keep
        });
        self.trades.retain(|_, series| {
            let keep = !series.holders.is_empty();
            if !keep {
                series.cancel.store(true, Ordering::Release);
            }
            keep
        });
    }

    /// Takes in what arrived and starts what is due. Once a frame.
    pub fn poll(&mut self, ctx: &egui::Context, now: Instant) {
        let mut changed = false;
        for series in self.candles.values_mut() {
            changed |= receive(series, now);
        }
        for series in self.trades.values_mut() {
            changed |= drain_live(series, &self.backend, ctx);
        }
        let mut free = MAX_IN_FLIGHT.saturating_sub(self.in_flight());
        let backend = Arc::clone(&self.backend);
        let mut due: Vec<(u8, Option<Instant>, String)> = Vec::new();
        for (key, series) in &self.candles {
            if series.busy() || series.retry_at.is_some_and(|at| now < at) {
                continue;
            }
            let Some(wanted) = series.wanted_from() else {
                continue;
            };
            let target = series.wanted_to().map_or(i64::MIN, |to| to.min(now_ms()));
            let class = if !series.loaded_once {
                0
            } else if !series.exhausted && series.reached_ms.is_none_or(|reached| wanted < reached)
            {
                1
            } else if series.wanted_to().is_some_and(|to| to < i64::MAX)
                && series
                    .newest_held
                    .is_some_and(|newest| target > newest + 2 * series.interval_ms)
                && series
                    .newer_asked
                    .is_none_or(|asked| target > asked + series.interval_ms)
            {
                2
            } else if series
                .last_tail
                .is_none_or(|at| now.duration_since(at) >= tail_refresh(series.instrument.source))
            {
                3
            } else {
                continue;
            };
            due.push((class, series.last_tail, key.clone()));
        }
        due.sort();
        for (class, _, key) in due {
            if free == 0 {
                break;
            }
            let Some(series) = self.candles.get_mut(&key) else {
                continue;
            };
            let kind = match class {
                0 => Kind::First,
                1 => Kind::Older,
                2 => Kind::Newer,
                _ => Kind::Tail,
            };
            request_candles(series, &backend, ctx, now, kind);
            if class == 0 {
                series.status = FlowStatus::Loading;
            }
            free -= 1;
            changed = true;
        }
        for series in self.trades.values_mut() {
            if !series.history_started {
                series.history_started = true;
                start_trades(series, &backend, ctx, now_ms());
                changed = true;
            }
        }
        if changed {
            self.version += 1;
        }
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as i64)
}

// MARK: Candles

fn request_candles(
    series: &mut CandleSeries,
    backend: &Arc<dyn ProfileBackend>,
    ctx: &egui::Context,
    now: Instant,
    kind: Kind,
) {
    let (sender, receiver) = mpsc::channel();
    let instrument = series.instrument.clone();
    let (interval, step) = (series.interval, series.interval_ms);
    let generation = series.generation.fetch_add(1, Ordering::AcqRel) + 1;
    let current = Arc::downgrade(&series.generation);
    let (worker, repaint) = (Arc::clone(backend), ctx.clone());
    let wanted = series.wanted_from().unwrap_or(0);
    let wanted_to = series.wanted_to().unwrap_or(i64::MAX);
    let end_for_range = series.oldest_held.map(|open| open + step);
    let newest_held = series.newest_held;
    let first_load = !series.loaded_once;
    if kind == Kind::Newer {
        series.newer_asked = Some(wanted_to.min(now_ms()));
    }
    backend.submit(
        instrument.source,
        Box::new(move || {
            if !current
                .upgrade()
                .is_some_and(|state| state.load(Ordering::Acquire) == generation)
            {
                return;
            }
            let now_ms = now_ms();
            let result = if kind == Kind::Tail {
                worker
                    .candles(
                        instrument.source,
                        &instrument.symbol,
                        interval,
                        now_ms - TAIL_CANDLES * step,
                        now_ms + step,
                        TAIL_CANDLES as usize + 1,
                    )
                    .map(|candles| Loaded {
                        candles,
                        tick_size: None,
                        from_ms: now_ms - TAIL_CANDLES * step,
                        kind: Kind::Tail,
                    })
            } else if kind == Kind::Newer {
                // After what is held, up to where the need ends (the candle of overlap is
                // deduplicated by open time).
                let start = newest_held.map_or(wanted, |open| open - step);
                let end = wanted_to.min(now_ms) + step;
                worker
                    .candles(
                        instrument.source,
                        &instrument.symbol,
                        interval,
                        start,
                        end,
                        MAX_CANDLES,
                    )
                    .map(|candles| Loaded {
                        candles,
                        tick_size: None,
                        from_ms: start,
                        kind: Kind::Newer,
                    })
            } else {
                // A first load reaches from the need's start to its end (now for a live chart, the
                // replay position in replay: the newest candles are not what replay needs); later
                // ones fill the part before what is held (the one candle of overlap is deduplicated
                // by open time).
                let end = match (kind, end_for_range) {
                    (Kind::Older, Some(end)) => end,
                    _ => wanted_to.min(now_ms) + step,
                };
                worker
                    .candles(
                        instrument.source,
                        &instrument.symbol,
                        interval,
                        wanted,
                        end,
                        MAX_CANDLES,
                    )
                    .map(|candles| Loaded {
                        candles,
                        tick_size: first_load
                            .then(|| worker.tick_size(instrument.source, &instrument.symbol))
                            .flatten(),
                        from_ms: wanted,
                        kind: if kind == Kind::First {
                            Kind::First
                        } else {
                            Kind::Older
                        },
                    })
            };
            let _ = sender.send(result);
            repaint.request_repaint();
        }),
    );
    series.inbox = Some(receiver);
    series.last_tail = Some(now);
}

fn receive(series: &mut CandleSeries, now: Instant) -> bool {
    if let Some(loaded) = series.pending.take() {
        return store(series, loaded);
    }
    let Some(receiver) = &series.inbox else {
        return false;
    };
    let result = match receiver.try_recv() {
        Ok(result) => result,
        Err(mpsc::TryRecvError::Empty) => return false,
        Err(mpsc::TryRecvError::Disconnected) => {
            Err("The request stopped before it answered".to_owned())
        }
    };
    series.inbox = None;
    match result {
        Ok(loaded) => store(series, loaded),
        Err(message) => {
            series.status = FlowStatus::Error(message.into());
            series.retry_at = Some(now + RETRY_AFTER);
            // A failed first load is retried as a first load.
            true
        }
    }
}

/// Puts an answer into the shared data, or keeps it for the next frame when a computation holds it.
fn store(series: &mut CandleSeries, loaded: Loaded) -> bool {
    let Some(mut data) = try_lock(&series.shared.data) else {
        series.pending = Some(loaded);
        return false;
    };
    let oldest_returned = loaded.candles.iter().map(|c| c.kline.open_time).min();
    if loaded.kind == Kind::Older {
        // Older history into a full book: the candles just fetched are the oldest, so it is the
        // newest that makes room.
        data.book.insert_evicting_newest(loaded.candles);
    } else {
        data.book.insert(loaded.candles);
    }
    if loaded.tick_size.is_some() {
        data.tick_size = loaded.tick_size;
    }
    series.oldest_held = data.book.oldest_open();
    series.newest_held = data.book.newest_open();
    drop(data);
    series.shared.revision.fetch_add(1, Ordering::AcqRel);
    if matches!(loaded.kind, Kind::First | Kind::Older) {
        series.loaded_once = true;
        // Reached what was asked for, or as far as the provider goes (and then no further request
        // for the same start: it would be answered the same way).
        let reached = match oldest_returned {
            Some(oldest) if oldest > loaded.from_ms + 2 * series.interval_ms => {
                series.exhausted = true;
                oldest
            }
            _ => loaded.from_ms,
        };
        series.reached_ms = Some(series.reached_ms.map_or(reached, |held| held.min(reached)));
    }
    series.status = FlowStatus::Ready;
    series.retry_at = None;
    true
}

// MARK: Trades

fn start_trades(
    series: &mut TradeSeries,
    backend: &Arc<dyn ProfileBackend>,
    ctx: &egui::Context,
    now_ms: i64,
) {
    series.live = backend.open_trades(&series.instrument.symbol, ctx);
    if let Some(live) = &series.live {
        series.reconnects_seen = live.inbox.reconnects();
    }
    let window_start = series
        .holders
        .values()
        .copied()
        .min()
        .unwrap_or(now_ms)
        .max(now_ms - TRADE_WINDOW_MS);
    let (shared, status, cancel) = (
        Arc::clone(&series.shared),
        Arc::clone(&series.status),
        Arc::clone(&series.cancel),
    );
    let (worker, repaint) = (Arc::clone(backend), ctx.clone());
    let symbol = series.instrument.symbol.clone();
    let source = series.instrument.source;
    backend.submit(
        source,
        Box::new(move || {
            let end =
                worker.trade_history(&symbol, window_start, TRADE_BUDGET, &cancel, &mut |page| {
                    lock(&shared.book).insert(&page);
                    shared.revision.fetch_add(1, Ordering::AcqRel);
                    repaint.request_repaint();
                });
            *lock(&status) = match end {
                HistoryEnd::WindowStart | HistoryEnd::Exhausted => TradeStatus::Live,
                HistoryEnd::Cancelled => return,
                other => TradeStatus::Partial(other),
            };
            shared.revision.fetch_add(1, Ordering::AcqRel);
            repaint.request_repaint();
        }),
    );
}

/// Moves what the stream delivered into the book; when ids were skipped (a reconnect, or a full
/// inbox), starts a backfill of exactly the missing ids.
fn drain_live(
    series: &mut TradeSeries,
    backend: &Arc<dyn ProfileBackend>,
    ctx: &egui::Context,
) -> bool {
    let Some(inbox) = series.live.as_ref().map(|live| Arc::clone(&live.inbox)) else {
        return false;
    };
    let mut incoming = std::mem::take(&mut series.stash);
    incoming.extend(inbox.drain());
    series.reconnects_seen = inbox.reconnects();
    let mut changed = false;
    if !incoming.is_empty() {
        incoming.sort_by_key(|trade| trade.id);
        let Some(mut book) = try_lock(&series.shared.book) else {
            series.stash = incoming;
            return false;
        };
        let newest_held = book.newest().map(|trade| trade.id);
        book.insert(&incoming);
        drop(book);
        series.shared.revision.fetch_add(1, Ordering::AcqRel);
        changed = true;
        // Every skipped id range: between what was held and the batch, and inside the batch (a
        // reconnect between two drains that were stashed together).
        let mut previous = newest_held;
        for trade in &incoming {
            if let Some(before) = previous
                && trade.id > before + 1
            {
                let gap = (before + 1, trade.id);
                if gap.1 - gap.0 <= MAX_BACKFILL && !series.gaps.contains(&gap) {
                    series.gaps.push(gap);
                }
            }
            previous = Some(previous.map_or(trade.id, |before| before.max(trade.id)));
        }
    }
    // One backfill at a time; the next waiting gap starts when the last ends.
    if !series.gaps.is_empty() && !series.backfilling.swap(true, Ordering::AcqRel) {
        let (from, to) = series.gaps.remove(0);
        let (shared, cancel, flag) = (
            Arc::clone(&series.shared),
            Arc::clone(&series.cancel),
            Arc::clone(&series.backfilling),
        );
        let (worker, repaint) = (Arc::clone(backend), ctx.clone());
        let symbol = series.instrument.symbol.clone();
        backend.submit(
            series.instrument.source,
            Box::new(move || {
                worker.trade_range(&symbol, from, to, &cancel, &mut |page| {
                    lock(&shared.book).insert(&page);
                    shared.revision.fetch_add(1, Ordering::AcqRel);
                    repaint.request_repaint();
                });
                flag.store(false, Ordering::Release);
            }),
        );
        changed = true;
    }
    changed
}

/// A scripted backend the hub's and the workspace's tests share: jobs run inline, candles come from
/// a table, trades from a count, and every request is logged.
#[cfg(test)]
pub(in crate::workspace) mod testing {
    use super::*;
    use crate::model::Kline;
    use crate::model::market_profile::trades::Trade;
    use std::sync::atomic::AtomicUsize;

    pub(in crate::workspace) const MINUTE: i64 = 60_000;

    pub(in crate::workspace) fn flow(open_time: i64) -> FlowKline {
        FlowKline {
            kline: Kline {
                open_time,
                close_time: open_time + MINUTE - 1,
                open: 100.0,
                high: 101.0,
                low: 99.0,
                close: 100.5,
                volume: 10.0,
            },
            taker_buy: Some(4.0),
        }
    }

    pub(in crate::workspace) fn trade(id: u64) -> Trade {
        Trade {
            id,
            time_ms: 1_000 + id as i64,
            price: 100.0,
            quantity: 1.0,
            buyer_is_maker: false,
        }
    }

    /// Runs jobs inline; answers candles over a fixed range; keeps a log of what was asked.
    #[derive(Default)]
    pub(in crate::workspace) struct Fake {
        /// Candles the "exchange" holds, by open time.
        pub(in crate::workspace) market: Mutex<Vec<i64>>,
        pub(in crate::workspace) candle_requests: Mutex<Vec<(i64, i64, usize)>>,
        pub(in crate::workspace) tick: Option<f64>,
        pub(in crate::workspace) trades: AtomicBool,
        /// Trades the REST endpoint holds: ids 1..=count.
        pub(in crate::workspace) trade_count: AtomicU64,
        pub(in crate::workspace) history_requests: AtomicUsize,
        pub(in crate::workspace) range_requests: Mutex<Vec<(u64, u64)>>,
        pub(in crate::workspace) inbox: Mutex<Option<Arc<TradeInbox>>>,
        pub(in crate::workspace) fail_candles: AtomicBool,
    }

    impl ProfileBackend for Fake {
        fn submit(&self, _: DataSource, job: Box<dyn FnOnce() + Send>) {
            job();
        }

        fn candles(
            &self,
            _: DataSource,
            _: &str,
            _: &str,
            start_ms: i64,
            end_ms: i64,
            max: usize,
        ) -> Result<Vec<FlowKline>, String> {
            self.candle_requests
                .lock()
                .unwrap()
                .push((start_ms, end_ms, max));
            if self.fail_candles.load(Ordering::Acquire) {
                return Err("offline".to_owned());
            }
            let held = self.market.lock().unwrap();
            let mut found: Vec<i64> = held
                .iter()
                .copied()
                .filter(|open| *open >= start_ms && *open < end_ms)
                .collect();
            let skip = found.len().saturating_sub(max);
            found.drain(..skip);
            Ok(found.into_iter().map(flow).collect())
        }

        fn tick_size(&self, _: DataSource, _: &str) -> Option<f64> {
            self.tick
        }

        fn has_trades(&self, _: DataSource) -> bool {
            self.trades.load(Ordering::Acquire)
        }

        fn trade_history(
            &self,
            _: &str,
            _: i64,
            _: usize,
            _: &AtomicBool,
            sink: &mut dyn FnMut(Vec<Trade>),
        ) -> HistoryEnd {
            self.history_requests.fetch_add(1, Ordering::AcqRel);
            let count = self.trade_count.load(Ordering::Acquire);
            sink((1..=count).map(trade).collect());
            HistoryEnd::WindowStart
        }

        fn trade_range(
            &self,
            _: &str,
            from_id: u64,
            to_id: u64,
            _: &AtomicBool,
            sink: &mut dyn FnMut(Vec<Trade>),
        ) -> HistoryEnd {
            self.range_requests.lock().unwrap().push((from_id, to_id));
            sink((from_id..to_id).map(trade).collect());
            HistoryEnd::WindowStart
        }

        fn open_trades(&self, _: &str, _: &egui::Context) -> Option<LiveTrades> {
            let inbox = Arc::new(TradeInbox::default());
            *self.inbox.lock().unwrap() = Some(Arc::clone(&inbox));
            Some(LiveTrades {
                inbox,
                guard: Box::new(()),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testing::*;
    use super::*;
    fn need(from_ms: i64, trades: bool) -> ProfileNeed {
        ProfileNeed {
            instrument: InstrumentId::new(DataSource::Binance, "BTCUSDT"),
            interval: "1m",
            interval_ms: MINUTE,
            from_ms,
            to_ms: i64::MAX,
            trades,
        }
    }

    fn hub(fake: &Arc<Fake>) -> ProfileHub {
        ProfileHub::with_backend(fake.clone())
    }

    fn open_times(shared: &CandleShared) -> Vec<i64> {
        lock(&shared.data)
            .book
            .between(i64::MIN, i64::MAX)
            .map(|c| c.kline.open_time)
            .collect()
    }

    /// Far in the past so the fake's candles are "history" and the tail request finds nothing new.
    const T0: i64 = 1_000 * MINUTE;

    fn fake_with_candles(count: i64) -> Arc<Fake> {
        let fake = Arc::new(Fake::default());
        *fake.market.lock().unwrap() = (0..count).map(|i| T0 + i * MINUTE).collect();
        fake
    }

    #[test]
    fn a_need_loads_candles_with_their_taker_buy_split() {
        let fake = fake_with_candles(5);
        let mut hub = hub(&fake);
        let ctx = egui::Context::default();
        hub.sync_chart(1, &[need(T0, false)]);
        assert_eq!(
            hub.candles(&need(T0, false).instrument, "1m").unwrap().1,
            FlowStatus::Queued
        );
        hub.poll(&ctx, Instant::now()); // submits (and, inline, answers)
        hub.poll(&ctx, Instant::now()); // receives
        let (shared, status) = hub.candles(&need(T0, false).instrument, "1m").unwrap();
        assert_eq!(status, FlowStatus::Ready);
        assert_eq!(open_times(&shared).len(), 5);
        assert_eq!(
            lock(&shared.data)
                .book
                .between(T0, T0 + MINUTE)
                .next()
                .unwrap()
                .taker_buy,
            Some(4.0)
        );
        assert!(shared.revision.load(Ordering::Acquire) > 0);
    }

    #[test]
    fn the_exchange_tick_size_is_kept_when_there_is_one() {
        let fake = Arc::new(Fake {
            tick: Some(0.01),
            ..Fake::default()
        });
        let mut hub = hub(&fake);
        let ctx = egui::Context::default();
        hub.sync_chart(1, &[need(T0, false)]);
        hub.poll(&ctx, Instant::now());
        hub.poll(&ctx, Instant::now());
        let (shared, _) = hub.candles(&need(T0, false).instrument, "1m").unwrap();
        assert_eq!(lock(&shared.data).tick_size, Some(0.01));
    }

    #[test]
    fn two_charts_on_one_market_share_one_series() {
        let fake = fake_with_candles(3);
        let mut hub = hub(&fake);
        hub.sync_chart(1, &[need(T0, false)]);
        hub.sync_chart(2, &[need(T0, false)]);
        assert_eq!(hub.candles.len(), 1);
        let ctx = egui::Context::default();
        hub.poll(&ctx, Instant::now());
        hub.poll(&ctx, Instant::now());
        assert_eq!(
            fake.candle_requests.lock().unwrap().len(),
            1,
            "fetched once"
        );
    }

    #[test]
    fn a_series_is_dropped_when_its_last_chart_goes() {
        let fake = fake_with_candles(3);
        let mut hub = hub(&fake);
        hub.sync_chart(1, &[need(T0, false)]);
        hub.sync_chart(2, &[need(T0, false)]);
        hub.forget_chart(1);
        assert_eq!(hub.candles.len(), 1);
        hub.forget_chart(2);
        assert!(hub.candles.is_empty());
        assert!(hub.held.is_empty());
    }

    #[test]
    fn an_unchanged_need_changes_nothing() {
        let fake = fake_with_candles(3);
        let mut hub = hub(&fake);
        hub.sync_chart(1, &[need(T0, false)]);
        let version = hub.version();
        hub.sync_chart(1, &[need(T0, false)]);
        assert_eq!(hub.version(), version);
    }

    #[test]
    fn a_need_that_reaches_further_back_fetches_only_the_part_before_what_is_held() {
        let fake = fake_with_candles(20);
        let mut hub = hub(&fake);
        let ctx = egui::Context::default();
        hub.sync_chart(1, &[need(T0 + 10 * MINUTE, false)]);
        hub.poll(&ctx, Instant::now());
        hub.poll(&ctx, Instant::now());
        let (shared, _) = hub.candles(&need(0, false).instrument, "1m").unwrap();
        assert_eq!(open_times(&shared).len(), 10);
        hub.sync_chart(1, &[need(T0, false)]);
        hub.poll(&ctx, Instant::now());
        hub.poll(&ctx, Instant::now());
        assert_eq!(open_times(&shared).len(), 20);
        let requests = fake.candle_requests.lock().unwrap().clone();
        let older = requests.last().unwrap();
        assert_eq!(older.0, T0);
        assert_eq!(
            older.1,
            T0 + 10 * MINUTE + MINUTE,
            "ends at the oldest held, one candle of overlap"
        );
    }

    #[test]
    fn a_provider_that_cannot_go_back_is_not_asked_again_for_the_same_start() {
        // The market only reaches back 5 candles; the chart wants 100.
        let fake = fake_with_candles(5);
        let mut hub = hub(&fake);
        let ctx = egui::Context::default();
        hub.sync_chart(1, &[need(T0 - 95 * MINUTE, false)]);
        let mut now = Instant::now();
        for _ in 0..6 {
            hub.poll(&ctx, now);
            now += Duration::from_secs(1);
        }
        let ranges = fake
            .candle_requests
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.2 == MAX_CANDLES)
            .count();
        assert_eq!(ranges, 1, "one range request, then only tail refreshes");
    }

    #[test]
    fn the_tail_is_refreshed_on_its_interval_and_not_before() {
        let fake = fake_with_candles(5);
        let mut hub = hub(&fake);
        let ctx = egui::Context::default();
        hub.sync_chart(1, &[need(T0, false)]);
        let start = Instant::now();
        hub.poll(&ctx, start);
        hub.poll(&ctx, start);
        let before = fake.candle_requests.lock().unwrap().len();
        hub.poll(&ctx, start + Duration::from_secs(5));
        assert_eq!(fake.candle_requests.lock().unwrap().len(), before);
        hub.poll(&ctx, start + TAIL_REFRESH + Duration::from_secs(1));
        assert_eq!(fake.candle_requests.lock().unwrap().len(), before + 1);
    }

    #[test]
    fn a_failed_load_reports_the_error_and_retries_later() {
        let fake = fake_with_candles(5);
        fake.fail_candles.store(true, Ordering::Release);
        let mut hub = hub(&fake);
        let ctx = egui::Context::default();
        hub.sync_chart(1, &[need(T0, false)]);
        let start = Instant::now();
        hub.poll(&ctx, start);
        hub.poll(&ctx, start);
        let (shared, status) = hub.candles(&need(T0, false).instrument, "1m").unwrap();
        assert!(matches!(status, FlowStatus::Error(_)));
        assert!(open_times(&shared).is_empty());
        let requests = fake.candle_requests.lock().unwrap().len();
        hub.poll(&ctx, start + Duration::from_secs(2));
        assert_eq!(
            fake.candle_requests.lock().unwrap().len(),
            requests,
            "waits before retrying"
        );
        fake.fail_candles.store(false, Ordering::Release);
        hub.poll(&ctx, start + RETRY_AFTER + Duration::from_secs(1));
        hub.poll(&ctx, start + RETRY_AFTER + Duration::from_secs(2));
        let (shared, status) = hub.candles(&need(T0, false).instrument, "1m").unwrap();
        assert_eq!(status, FlowStatus::Ready);
        assert_eq!(open_times(&shared).len(), 5);
    }

    #[test]
    fn at_most_a_few_candle_requests_run_at_once() {
        // The fake answers inline, so make jobs wait by not polling their receivers: count busy.
        let fake = fake_with_candles(3);
        let mut hub = hub(&fake);
        let mut needs = Vec::new();
        for n in 0..6 {
            let mut one = need(T0, false);
            one.instrument = InstrumentId::new(DataSource::Binance, format!("COIN{n}USDT"));
            needs.push(one);
        }
        hub.sync_chart(1, &needs);
        hub.poll(&egui::Context::default(), Instant::now());
        assert!(hub.in_flight() <= MAX_IN_FLIGHT);
    }

    // MARK: Trades

    #[test]
    fn trades_load_into_the_book_and_the_market_goes_live() {
        let fake = fake_with_candles(3);
        fake.trades.store(true, Ordering::Release);
        fake.trade_count.store(50, Ordering::Release);
        let mut hub = hub(&fake);
        let ctx = egui::Context::default();
        hub.sync_chart(1, &[need(T0, true)]);
        hub.poll(&ctx, Instant::now());
        let instrument = need(T0, true).instrument;
        let (shared, status) = hub.trades(&instrument).unwrap();
        assert_eq!(status, TradeStatus::Live);
        assert_eq!(lock(&shared.book).len(), 50);
        assert!(lock(&shared.book).covers_from(1_000 + 2));
        // History is fetched once, not every frame.
        hub.poll(&ctx, Instant::now());
        hub.poll(&ctx, Instant::now());
        assert_eq!(fake.history_requests.load(Ordering::Acquire), 1);
    }

    #[test]
    fn a_source_without_trades_gets_none() {
        let fake = fake_with_candles(3);
        let mut hub = hub(&fake);
        hub.sync_chart(1, &[need(T0, true)]);
        assert!(hub.trades(&need(T0, true).instrument).is_none());
        assert!(hub.trades.is_empty());
    }

    #[test]
    fn live_trades_join_the_book_without_double_counting() {
        let fake = fake_with_candles(3);
        fake.trades.store(true, Ordering::Release);
        fake.trade_count.store(10, Ordering::Release);
        let mut hub = hub(&fake);
        let ctx = egui::Context::default();
        hub.sync_chart(1, &[need(T0, true)]);
        hub.poll(&ctx, Instant::now());
        let inbox = fake.inbox.lock().unwrap().clone().unwrap();
        // The stream re-delivers 9 and 10 (an overlap with history) and brings 11, 12.
        for id in [9, 10, 11, 12] {
            inbox.push(trade(id));
        }
        hub.poll(&ctx, Instant::now());
        let (shared, _) = hub.trades(&need(T0, true).instrument).unwrap();
        let book = lock(&shared.book);
        assert_eq!(book.len(), 12);
        assert_eq!(book.newest().unwrap().id, 12);
        assert!(
            fake.range_requests.lock().unwrap().is_empty(),
            "no gap, no backfill"
        );
    }

    #[test]
    fn skipped_ids_after_a_reconnect_are_backfilled_exactly() {
        let fake = fake_with_candles(3);
        fake.trades.store(true, Ordering::Release);
        fake.trade_count.store(10, Ordering::Release);
        let mut hub = hub(&fake);
        let ctx = egui::Context::default();
        hub.sync_chart(1, &[need(T0, true)]);
        hub.poll(&ctx, Instant::now());
        let inbox = fake.inbox.lock().unwrap().clone().unwrap();
        // After a reconnect the stream resumes at 40: ids 11..=39 were missed.
        inbox.push(trade(40));
        inbox.push(trade(41));
        hub.poll(&ctx, Instant::now());
        assert_eq!(*fake.range_requests.lock().unwrap(), vec![(11, 40)]);
        let (shared, _) = hub.trades(&need(T0, true).instrument).unwrap();
        let mut book = lock(&shared.book);
        assert_eq!(book.len(), 41);
        let ids: Vec<u64> = book.trades().iter().map(|t| t.id).collect();
        assert!(ids.windows(2).all(|w| w[1] == w[0] + 1));
        assert!(book.complete_from_ms().is_some());
    }

    #[test]
    fn an_enormous_gap_is_left_as_a_gap_not_backfilled() {
        let fake = fake_with_candles(3);
        fake.trades.store(true, Ordering::Release);
        fake.trade_count.store(10, Ordering::Release);
        let mut hub = hub(&fake);
        let ctx = egui::Context::default();
        hub.sync_chart(1, &[need(T0, true)]);
        hub.poll(&ctx, Instant::now());
        let inbox = fake.inbox.lock().unwrap().clone().unwrap();
        inbox.push(trade(10 + MAX_BACKFILL + 100));
        hub.poll(&ctx, Instant::now());
        assert!(fake.range_requests.lock().unwrap().is_empty());
        let (shared, _) = hub.trades(&need(T0, true).instrument).unwrap();
        let mut book = lock(&shared.book);
        // Honest coverage: only the newest trade's run is complete.
        let from = book.complete_from_ms().unwrap();
        assert_eq!(from, trade(10 + MAX_BACKFILL + 100).time_ms + 1);
    }

    #[test]
    fn dropping_the_last_chart_cancels_the_trade_jobs() {
        let fake = fake_with_candles(3);
        fake.trades.store(true, Ordering::Release);
        let mut hub = hub(&fake);
        hub.sync_chart(1, &[need(T0, true)]);
        hub.poll(&egui::Context::default(), Instant::now());
        let cancel = Arc::clone(&hub.trades.values().next().unwrap().cancel);
        hub.forget_chart(1);
        assert!(cancel.load(Ordering::Acquire));
        assert!(hub.trades.is_empty());
    }

    // MARK: Replay

    fn open_range(shared: &CandleShared) -> (Option<i64>, Option<i64>, usize) {
        let data = lock(&shared.data);
        (
            data.book.oldest_open(),
            data.book.newest_open(),
            data.book.len(),
        )
    }

    #[test]
    fn a_chart_in_replay_far_from_now_gets_the_candles_around_the_replay_not_the_newest() {
        let fake = Arc::new(Fake::default());
        let t0 = 1_000 * MINUTE;
        // More candles than the book holds: the newest 60 000 are nowhere near the replay.
        *fake.market.lock().unwrap() = (0..100_000).map(|i| t0 + i * MINUTE).collect();
        let mut hub = hub(&fake);
        let ctx = egui::Context::default();
        let mut replay = need(t0 + 70_000 * MINUTE, false);
        replay.to_ms = t0 + 75_000 * MINUTE;
        hub.sync_chart(1, std::slice::from_ref(&replay));
        hub.poll(&ctx, Instant::now());
        hub.poll(&ctx, Instant::now());
        let (shared, _) = hub.candles(&replay.instrument, "1m").unwrap();
        let (oldest, newest, count) = open_range(&shared);
        assert_eq!(
            oldest,
            Some(t0 + 70_000 * MINUTE),
            "from the start of the need"
        );
        assert!(
            newest.unwrap() <= t0 + 75_000 * MINUTE,
            "not past the replay: {newest:?}"
        );
        assert!(count > 4_000, "{count}");
        // Replay steps forward: the candles after what is held are fetched, not the newest ones.
        replay.to_ms = t0 + 80_000 * MINUTE;
        hub.sync_chart(1, std::slice::from_ref(&replay));
        hub.poll(&ctx, Instant::now());
        hub.poll(&ctx, Instant::now());
        let (oldest, newest, _) = open_range(&shared);
        assert_eq!(oldest, Some(t0 + 70_000 * MINUTE));
        assert_eq!(newest, Some(t0 + 80_000 * MINUTE), "up to the new position");
        let opens = open_times(&shared);
        assert!(
            opens.windows(2).all(|w| w[1] - w[0] == MINUTE),
            "no hole between the two loads"
        );
        // Nothing newer exists to fetch beyond what the replay asked for: no repeated request.
        let requests = fake.candle_requests.lock().unwrap().len();
        hub.poll(&ctx, Instant::now());
        hub.poll(&ctx, Instant::now());
        assert_eq!(fake.candle_requests.lock().unwrap().len(), requests);
    }

    #[test]
    fn a_live_chart_still_loads_up_to_now_and_a_replay_ending_in_the_future_is_clamped_to_now() {
        let fake = fake_with_candles(5);
        let mut hub = hub(&fake);
        let ctx = egui::Context::default();
        hub.sync_chart(1, &[need(T0, false)]);
        hub.poll(&ctx, Instant::now());
        hub.poll(&ctx, Instant::now());
        let requests = fake.candle_requests.lock().unwrap().clone();
        let first = requests[0];
        assert!(
            first.1
                > std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_millis() as i64
                    - MINUTE * 10,
            "the end of a first load is about now, not i64::MAX: {}",
            first.1
        );
        assert!(first.1 < i64::MAX / 2);
    }

    // MARK: Gaps

    #[test]
    fn every_skipped_id_range_of_a_batch_is_backfilled_one_after_the_other() {
        let fake = fake_with_candles(3);
        fake.trades.store(true, Ordering::Release);
        fake.trade_count.store(10, Ordering::Release);
        let mut hub = hub(&fake);
        let ctx = egui::Context::default();
        hub.sync_chart(1, &[need(T0, true)]);
        hub.poll(&ctx, Instant::now());
        let inbox = fake.inbox.lock().unwrap().clone().unwrap();
        // After a reconnect: 11..=39 missed, then 40..=42, then 43..=89 missed again, then 90.
        for id in [40, 41, 42, 90] {
            inbox.push(trade(id));
        }
        for _ in 0..4 {
            hub.poll(&ctx, Instant::now());
        }
        assert_eq!(
            *fake.range_requests.lock().unwrap(),
            vec![(11, 40), (43, 90)]
        );
        let (shared, _) = hub.trades(&need(T0, true).instrument).unwrap();
        let mut book = lock(&shared.book);
        let ids: Vec<u64> = book.trades().iter().map(|t| t.id).collect();
        assert!(
            ids.windows(2).all(|w| w[1] == w[0] + 1),
            "the book is gap free: {ids:?}"
        );
        assert!(book.complete_from_ms().is_some());
    }
}
