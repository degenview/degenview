//! The candles of every comparison market on the workspace's charts, fetched once per market.
//!
//! A chart owns its overlays' *configuration* (`Chart::overlays`); this hub owns their *data*.
//! One entry exists per market (`InstrumentId::key()`), however many overlays on however many
//! charts name it, and it lives only while some overlay holds it: removing the last holder drops
//! the entry, and a request still in flight finds its answer's receiver gone and is discarded.
//!
//! There is no limit on how many markets are held. What is limited is how many requests run at
//! once (`MAX_IN_FLIGHT`) and how often each refreshes (it slows as more are active), so a chart
//! with a hundred overlays loads them in turn instead of starting a hundred requests together, and
//! a Binance chart with many overlays does not spend its rate limit on refreshes. Hidden overlays
//! are not fetched. Nothing here blocks: every request runs on the shared work queue and answers
//! over a channel polled once per frame.
//!
//! The pieces are the chart's own: `service::fetch_klines`, `fetch_klines_range`, the same merge
//! rules for a live bar, the same `OlderHistory` paging gate.

use super::chart::{CANDLE_LIMIT, merge_fetched_candles};
use crate::model::history::{self, OlderHistory};
use crate::model::overlay::{OverlayId, OverlayList};
use crate::model::overlay_feed::{OverlayFeed, OverlayStatus};
use crate::model::{DataSource, InstrumentId, Kline, Timeframe};
use crate::service;
use eframe::egui;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

/// Requests (first loads, refreshes and older pages together) that may run at once.
pub(super) const MAX_IN_FLIGHT: usize = 4;
/// The quickest a market refreshes, and the share of a second each active market adds to it:
/// with 100 active markets each one refreshes every 25 s, so the request rate stays near four a
/// second whatever the count.
const MIN_REFRESH: Duration = Duration::from_secs(5);
const REFRESH_PER_MARKET: Duration = Duration::from_millis(250);
/// Rate-limited providers (one request every few seconds) are asked less often still.
const SLOW_MIN_REFRESH: Duration = Duration::from_secs(15);
const RETRY_AFTER: Duration = Duration::from_secs(15);
const SLOW_RETRY_AFTER: Duration = Duration::from_secs(45);

/// How requests reach the providers. The workspace uses the real services; tests replace it.
pub(super) trait OverlayBackend: Send + Sync + 'static {
    /// Runs `job` off the UI thread, on `source`'s lane of the work queue.
    fn submit(&self, source: DataSource, job: Box<dyn FnOnce() + Send>);
    fn klines(
        &self,
        source: DataSource,
        symbol: &str,
        interval: &str,
        limit: u32,
    ) -> Result<Vec<Kline>, String>;
    /// A page of candles ending before `end_ms`.
    fn page(
        &self,
        source: DataSource,
        symbol: &str,
        interval: &str,
        end_ms: i64,
    ) -> Result<Vec<Kline>, String>;
}

struct ServiceBackend;

impl OverlayBackend for ServiceBackend {
    fn submit(&self, source: DataSource, job: Box<dyn FnOnce() + Send>) {
        service::work_queue::submit_for(source, job);
    }

    fn klines(
        &self,
        source: DataSource,
        symbol: &str,
        interval: &str,
        limit: u32,
    ) -> Result<Vec<Kline>, String> {
        service::fetch_klines(source, symbol, interval, limit)
    }

    fn page(
        &self,
        source: DataSource,
        symbol: &str,
        interval: &str,
        end_ms: i64,
    ) -> Result<Vec<Kline>, String> {
        service::fetch_klines_range(source, symbol, interval, 0, end_ms, history::PAGE)
    }
}

/// What one chart holds of the hub, kept to tell whether it changed without allocating.
#[derive(Default)]
struct Holding {
    entries: Vec<(OverlayId, InstrumentId, bool)>,
    /// The oldest candle the chart itself holds: overlays page back toward it, no further.
    main_first_ms: Option<i64>,
}

#[derive(Clone, Copy)]
struct Holder {
    /// Overlays of this chart on this market that are shown.
    visible: usize,
    main_first_ms: Option<i64>,
}

struct Series {
    instrument: InstrumentId,
    holders: HashMap<u64, Holder>,
    candles: Arc<Vec<Kline>>,
    revision: u64,
    status: OverlayStatus,
    /// Bumped by every request and reload; a queued job whose number is stale does nothing.
    generation: Arc<AtomicU64>,
    inbox: Option<Receiver<Result<Vec<Kline>, String>>>,
    last_fetch: Option<Instant>,
    retry_at: Option<Instant>,
    older: OlderHistory,
    older_inbox: Option<Receiver<Result<Vec<Kline>, String>>>,
    last_live_open_time: Option<i64>,
}

impl Series {
    fn new(instrument: InstrumentId) -> Self {
        Self {
            instrument,
            holders: HashMap::new(),
            candles: Arc::new(Vec::new()),
            revision: 0,
            status: OverlayStatus::Queued,
            generation: Arc::new(AtomicU64::new(0)),
            inbox: None,
            last_fetch: None,
            retry_at: None,
            older: OlderHistory::default(),
            older_inbox: None,
            last_live_open_time: None,
        }
    }

    fn shown(&self) -> bool {
        self.holders.values().any(|holder| holder.visible > 0)
    }

    fn busy(&self) -> bool {
        self.inbox.is_some() || self.older_inbox.is_some()
    }

    /// The oldest candle any holding chart holds: the history this market should reach back to.
    fn wanted_from(&self) -> Option<i64> {
        self.holders
            .values()
            .filter(|holder| holder.visible > 0)
            .filter_map(|holder| holder.main_first_ms)
            .min()
    }

    fn feed(&self) -> OverlayFeed {
        OverlayFeed {
            candles: self.candles.clone(),
            revision: self.revision,
            status: self.status.clone(),
        }
    }

    /// Takes `new` as the candles; `false` (and no revision) when they are what is held.
    fn set_candles(&mut self, new: Vec<Kline>) -> bool {
        if *self.candles == new {
            return false;
        }
        self.candles = Arc::new(new);
        self.revision += 1;
        true
    }

    /// Forgets the candles and everything learnt about the history behind them: they were
    /// another timeframe's. Anything in flight is abandoned.
    fn reset(&mut self) {
        self.generation.fetch_add(1, Ordering::AcqRel);
        self.inbox = None;
        self.older_inbox = None;
        self.older = OlderHistory::default();
        self.last_fetch = None;
        self.retry_at = None;
        self.last_live_open_time = None;
        self.status = OverlayStatus::Queued;
        if !self.candles.is_empty() {
            self.candles = Arc::new(Vec::new());
            self.revision += 1;
        }
    }
}

/// How long after a request a market is asked again, with `active` markets being refreshed.
pub(super) fn refresh_after(source: DataSource, active: usize) -> Duration {
    let floor = match source {
        DataSource::CoinGecko | DataSource::DexScreener => SLOW_MIN_REFRESH,
        _ => MIN_REFRESH,
    };
    floor.max(REFRESH_PER_MARKET * active as u32)
}

fn retry_after(source: DataSource) -> Duration {
    match source {
        DataSource::CoinGecko | DataSource::DexScreener => SLOW_RETRY_AFTER,
        _ => RETRY_AFTER,
    }
}

pub(super) struct OverlayHub {
    backend: Arc<dyn OverlayBackend>,
    series: HashMap<String, Series>,
    held: HashMap<u64, Holding>,
    timeframe: Option<Timeframe>,
    /// Bumps whenever anything a card shows changed: candles, state, or which markets are held.
    version: u64,
}

impl OverlayHub {
    pub(super) fn new() -> Self {
        Self::with_backend(Arc::new(ServiceBackend))
    }

    pub(super) fn with_backend(backend: Arc<dyn OverlayBackend>) -> Self {
        Self {
            backend,
            series: HashMap::new(),
            held: HashMap::new(),
            timeframe: None,
            version: 0,
        }
    }

    /// Changes whenever a card's view of the hub may differ from the last frame's.
    pub(super) fn version(&self) -> u64 {
        self.version
    }

    pub(super) fn series_count(&self) -> usize {
        self.series.len()
    }

    pub(super) fn in_flight(&self) -> usize {
        self.series.values().filter(|series| series.busy()).count()
    }

    /// The data of `instrument`, if any overlay holds it.
    pub(super) fn feed(&self, instrument: &InstrumentId) -> Option<OverlayFeed> {
        self.series.get(&instrument.key()).map(Series::feed)
    }

    /// Makes the hub hold exactly the markets `overlays` names for `chart_id`. Cheap when nothing
    /// changed: a comparison, no allocation.
    pub(super) fn sync_chart(
        &mut self,
        chart_id: u64,
        overlays: &OverlayList,
        main_first_ms: Option<i64>,
    ) {
        if let Some(held) = self.held.get(&chart_id)
            && held.main_first_ms == main_first_ms
            && held.entries.len() == overlays.len()
            && held.entries.iter().zip(overlays).all(|(held, now)| {
                held.0 == now.id && held.1 == now.instrument && held.2 == now.visible
            })
        {
            return;
        }
        // Take the old holdings off first, add the new, and only then drop what nothing holds, so a
        // market that stays keeps the candles it already has.
        let old = self.held.remove(&chart_id);
        for (_, instrument, _) in old.iter().flat_map(|old| &old.entries) {
            if let Some(series) = self.series.get_mut(&instrument.key()) {
                series.holders.remove(&chart_id);
            }
        }
        let mut entries = Vec::with_capacity(overlays.len());
        for overlay in overlays {
            entries.push((overlay.id, overlay.instrument.clone(), overlay.visible));
            let series = self
                .series
                .entry(overlay.instrument.key())
                .or_insert_with(|| Series::new(overlay.instrument.clone()));
            let holder = series.holders.entry(chart_id).or_insert(Holder {
                visible: 0,
                main_first_ms,
            });
            holder.visible += usize::from(overlay.visible);
        }
        self.held.insert(
            chart_id,
            Holding {
                entries,
                main_first_ms,
            },
        );
        self.prune(old);
        self.version += 1;
    }

    /// A chart went away (or has no overlays): drops every market only it held.
    pub(super) fn forget_chart(&mut self, chart_id: u64) {
        if self.release(chart_id) {
            self.version += 1;
        }
    }

    fn release(&mut self, chart_id: u64) -> bool {
        let Some(old) = self.held.remove(&chart_id) else {
            return false;
        };
        for (_, instrument, _) in &old.entries {
            if let Some(series) = self.series.get_mut(&instrument.key()) {
                series.holders.remove(&chart_id);
            }
        }
        self.prune(Some(old));
        true
    }

    /// Drops the markets among `old` that no overlay holds any more. Bumping the generation turns
    /// a job not yet started into a no-op; one that has started answers a receiver that is gone.
    fn prune(&mut self, old: Option<Holding>) {
        for (_, instrument, _) in old.iter().flat_map(|old| &old.entries) {
            let key = instrument.key();
            if self
                .series
                .get(&key)
                .is_some_and(|series| series.holders.is_empty())
            {
                if let Some(series) = self.series.remove(&key) {
                    series.generation.fetch_add(1, Ordering::AcqRel);
                }
            }
        }
    }

    /// Drains answers, then starts what is due within the in-flight budget.
    pub(super) fn poll(&mut self, timeframe: Timeframe, ctx: &egui::Context, now: Instant) {
        if self.timeframe != Some(timeframe) {
            if self.timeframe.is_some() {
                for series in self.series.values_mut() {
                    series.reset();
                }
                self.version += 1;
            }
            self.timeframe = Some(timeframe);
        }
        let mut changed = false;
        for series in self.series.values_mut() {
            changed |= receive_fetch(series, now);
            changed |= receive_older(series, now);
        }
        if changed {
            self.version += 1;
        }
        self.schedule(timeframe, ctx, now);
    }

    fn schedule(&mut self, timeframe: Timeframe, ctx: &egui::Context, now: Instant) {
        let mut free = MAX_IN_FLIGHT.saturating_sub(self.in_flight());
        if free == 0 {
            return;
        }
        let active = self.series.values().filter(|series| series.shown()).count();
        // First loads, then older pages, then refreshes; within each, the longest waiting first.
        let mut due: Vec<(u8, Option<Instant>, String)> = Vec::new();
        for (key, series) in &self.series {
            if !series.shown() || series.busy() {
                continue;
            }
            if series.retry_at.is_some_and(|at| now < at) {
                continue;
            }
            let class = match series.last_fetch {
                None => 0,
                Some(_) if wants_older(series, timeframe, now) => 1,
                Some(at)
                    if now.duration_since(at)
                        >= refresh_after(series.instrument.source, active) =>
                {
                    2
                }
                Some(_) => continue,
            };
            due.push((class, series.last_fetch, key.clone()));
        }
        due.sort();
        let backend = self.backend.clone();
        for (class, _, key) in due {
            if free == 0 {
                break;
            }
            let Some(series) = self.series.get_mut(&key) else {
                continue;
            };
            match class {
                2 | 0 => request_newest(series, &backend, timeframe, ctx, now),
                _ => request_older(series, &backend, timeframe, ctx),
            }
            if class == 0 {
                series.status = OverlayStatus::Loading;
            }
            free -= 1;
            self.version += 1;
        }
    }

    /// A live Binance candle for `symbol`: every overlay market on that symbol takes it.
    pub(super) fn apply_live(&mut self, symbol: &str, kline: &Kline) {
        for series in self.series.values_mut() {
            if series.instrument.source != DataSource::Binance
                || !series.instrument.symbol.eq_ignore_ascii_case(symbol)
            {
                continue;
            }
            let mut candles = (*series.candles).clone();
            match candles.last() {
                Some(last) if last.open_time == kline.open_time => {
                    *candles.last_mut().unwrap() = kline.clone();
                }
                Some(last) if last.open_time > kline.open_time => continue,
                _ => candles.push(kline.clone()),
            }
            series.last_live_open_time = candles.last().map(|candle| candle.open_time);
            if series.set_candles(candles) {
                self.version += 1;
            }
        }
    }

    /// The Binance symbols the live feed should also stream: shown overlay markets.
    pub(super) fn binance_symbols(&self) -> Vec<String> {
        let mut symbols: Vec<String> = self
            .series
            .values()
            .filter(|series| series.instrument.source == DataSource::Binance && series.shown())
            .map(|series| series.instrument.symbol.to_uppercase())
            .collect();
        symbols.sort();
        symbols.dedup();
        symbols
    }
}

impl super::ChartWorkspace {
    /// Once a frame: tells the hub which markets the charts' overlays name, takes in what has
    /// arrived and starts what is due.
    pub(super) fn tick_overlays(&mut self, ctx: &egui::Context) {
        for chart in &self.charts {
            if chart.is_market() {
                let first = chart.candles.first().map(|candle| candle.open_time);
                self.overlay_hub
                    .sync_chart(chart.id, &chart.overlays, first);
            }
        }
        self.overlay_hub.poll(self.timeframe, ctx, Instant::now());
        // Hand each card what changed. Charts without overlays are skipped, and a chart whose
        // overlays and hub entries are as the card last saw them costs a comparison.
        let version = self.overlay_hub.version();
        for chart in &mut self.charts {
            if !chart.is_market()
                || (chart.overlays.is_empty() && chart.chart.overlay_list().is_empty())
            {
                continue;
            }
            if !chart
                .chart
                .overlays_stale(version, chart.source, &chart.symbol, &chart.overlays)
            {
                continue;
            }
            let main = InstrumentId::new(chart.source, chart.symbol.clone()).key();
            let feeds = chart
                .overlays
                .iter()
                .map(|overlay| {
                    let mut feed = self
                        .overlay_hub
                        .feed(&overlay.instrument)
                        .unwrap_or_else(|| OverlayFeed::empty(OverlayStatus::Queued));
                    if overlay.instrument.key() == main {
                        feed.status = OverlayStatus::Blocked("Same market as the chart");
                    }
                    feed
                })
                .collect();
            chart.chart.set_overlays(
                chart.overlays.clone(),
                feeds,
                version,
                chart.source,
                &chart.symbol,
            );
        }
    }
}

fn wants_older(series: &Series, timeframe: Timeframe, now: Instant) -> bool {
    let (Some(first), Some(wanted)) = (series.candles.first(), series.wanted_from()) else {
        return false;
    };
    first.open_time > wanted.saturating_add(timeframe.nominal_ms())
        && series.older.should_request(
            series.instrument.source.supports_older_history(timeframe),
            true,
            series.candles.len(),
            now,
        )
}

fn request_newest(
    series: &mut Series,
    backend: &Arc<dyn OverlayBackend>,
    timeframe: Timeframe,
    ctx: &egui::Context,
    now: Instant,
) {
    let (sender, receiver) = mpsc::channel();
    let instrument = series.instrument.clone();
    let generation = series.generation.fetch_add(1, Ordering::AcqRel) + 1;
    let current: Weak<AtomicU64> = Arc::downgrade(&series.generation);
    let (worker, repaint) = (backend.clone(), ctx.clone());
    backend.submit(
        instrument.source,
        Box::new(move || {
            if !current
                .upgrade()
                .is_some_and(|state| state.load(Ordering::Acquire) == generation)
            {
                return;
            }
            let result = worker.klines(
                instrument.source,
                &instrument.symbol,
                timeframe.interval(),
                CANDLE_LIMIT,
            );
            let _ = sender.send(result);
            repaint.request_repaint();
        }),
    );
    series.inbox = Some(receiver);
    series.last_fetch = Some(now);
}

fn request_older(
    series: &mut Series,
    backend: &Arc<dyn OverlayBackend>,
    timeframe: Timeframe,
    ctx: &egui::Context,
) {
    let Some(end) = history::page_end(&series.candles) else {
        return;
    };
    let (sender, receiver) = mpsc::channel();
    let instrument = series.instrument.clone();
    let (worker, repaint) = (backend.clone(), ctx.clone());
    backend.submit(
        instrument.source,
        Box::new(move || {
            let page = worker.page(
                instrument.source,
                &instrument.symbol,
                timeframe.interval(),
                end,
            );
            let _ = sender.send(page);
            repaint.request_repaint();
        }),
    );
    series.older.started();
    series.older_inbox = Some(receiver);
}

fn receive_fetch(series: &mut Series, now: Instant) -> bool {
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
        Ok(fresh) => {
            let merged = merge_fetched_candles(fresh, &series.candles, series.last_live_open_time);
            let candles = history::with_older(&series.candles, merged, series.older.keeps_older());
            series.set_candles(candles);
            series.status = OverlayStatus::Ready;
            series.retry_at = None;
        }
        Err(message) => {
            series.status = OverlayStatus::Error(message.into());
            series.retry_at = Some(now + retry_after(series.instrument.source));
        }
    }
    true
}

fn receive_older(series: &mut Series, now: Instant) -> bool {
    let Some(receiver) = &series.older_inbox else {
        return false;
    };
    let result = match receiver.try_recv() {
        Ok(result) => result,
        Err(mpsc::TryRecvError::Empty) => return false,
        Err(mpsc::TryRecvError::Disconnected) => Err("Older candles stopped loading".to_owned()),
    };
    series.older_inbox = None;
    match result {
        Ok(page) => {
            let mut candles = (*series.candles).clone();
            let added = history::prepend(&mut candles, page);
            series.older.arrived(added);
            if added > 0 {
                series.candles = Arc::new(candles);
                series.revision += 1;
            }
        }
        Err(_) => series.older.failed(now),
    }
    true
}

#[cfg(test)]
mod tests;
