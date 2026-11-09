//! The order-flow chart types' data, per chart: what a chart asks the profile hub for, the set
//! computed from it on a worker, and the finished set handed to the card.
//!
//! The card draws and the hub fetches; this is the part between. Once a frame, for every market chart
//! on an order-flow type (the volume footprint, the TPO):
//!
//! 1. the finer data it needs is asked of the [`ProfileHub`](super::profile_hub::ProfileHub) (a
//!    comparison when nothing changed): the footprint wants the finest candles that cut its bar (and
//!    trades), the TPO 5-minute candles back to its oldest period;
//! 2. when anything the set depends on changed (the bars, the finer data, the settings that shape it,
//!    the splits and merges, the replay position) a computation is started on the work queue, one at a
//!    time per chart, newest request wins;
//! 3. a finished set is handed to the card; what was built for completed bars or periods is cached
//!    across computations, so a live candle or trade rebuilds only what it touches.
//!
//! A chart on another type drops its hold on the hub. Replay hands the computation the bars up to the
//! replay position and tells the TPO not to read candles past the end of the last of them, so nothing
//! after it is ever read.

use super::chart::displayed_candles;
use super::profile_hub::{CandleShared, FlowStatus, MAX_CANDLES, ProfileNeed, TradeShared};
use crate::model::chart_type::{ChartType, FootprintShape, SvpShape, TpoShape};
use crate::model::market_profile::feed::ProfileFeed;
use crate::model::market_profile::footprint::{
    self, Cache, FootprintSet, MAX_BARS, SetInput, SetStatus,
};
use crate::model::market_profile::quality::Unsupported;
use crate::model::market_profile::svp::{self, SvpSet};
use crate::model::market_profile::tpo::{self, TpoEdit, TpoSet};
use crate::model::{InstrumentId, Kline};
use eframe::egui;
use std::hash::{Hash, Hasher};
use std::sync::atomic::Ordering;
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, PoisonError};

/// The candles a TPO is built from: every block size TradingView offers is a multiple of five minutes.
const TPO_INTERVAL: &str = "5m";
const TPO_INTERVAL_MS: i64 = 300_000;

/// The longest chart bar a session volume profile is offered on: TradingView supports intraday and
/// daily charts, not weekly or longer.
const SVP_LONGEST_BAR_MS: i64 = 7 * 86_400_000;

/// One chart's computation state.
#[derive(Default)]
pub(super) struct ProfileRuntime {
    /// The type the state below belongs to.
    kind: Option<ChartType>,
    /// What the last computation was started for (or the card was last given).
    key: Option<u64>,
    /// The running computation's answer.
    inbox: Option<Receiver<Computed>>,
    footprint_cache: Option<Cache>,
    tpo_cache: Option<tpo::Cache>,
    svp_cache: Option<svp::Cache>,
    /// What the next computation is for, when one is already running.
    waiting: Option<Box<Job>>,
    /// Whether the card holds a feed of this runtime's.
    fed: bool,
}

struct Computed {
    key: u64,
    done: Done,
}

enum Done {
    Footprint(FootprintSet, Cache),
    Tpo(TpoSet, tpo::Cache),
    Svp(SvpSet, svp::Cache),
}

enum Job {
    Footprint(Box<FootprintJob>),
    Tpo(Box<TpoJob>),
    Svp(Box<SvpJob>),
}

impl Job {
    fn key(&self) -> u64 {
        match self {
            Self::Footprint(job) => job.key,
            Self::Tpo(job) => job.key,
            Self::Svp(job) => job.key,
        }
    }
}

struct FootprintJob {
    key: u64,
    bars: Vec<Kline>,
    bar_ms: i64,
    candles: Arc<CandleShared>,
    trades: Option<Arc<TradeShared>>,
    shape: FootprintShape,
}

struct TpoJob {
    key: u64,
    bars: Vec<Kline>,
    bar_ms: i64,
    candles: Arc<CandleShared>,
    shape: TpoShape,
    edits: Vec<TpoEdit>,
    until_ms: i64,
}

struct SvpJob {
    key: u64,
    bars: Vec<Kline>,
    bar_ms: i64,
    candles: Arc<CandleShared>,
    shape: SvpShape,
    until_ms: i64,
}

fn hash_of(parts: impl Hash) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    parts.hash(&mut hasher);
    hasher.finish()
}

fn last_bar_signature(bars: &[Kline]) -> (usize, Option<i64>, Option<[u64; 6]>) {
    (
        bars.len(),
        bars.first().map(|bar| bar.open_time),
        bars.last().map(|bar| {
            [
                bar.open_time as u64,
                bar.open.to_bits(),
                bar.high.to_bits(),
                bar.low.to_bits(),
                bar.close.to_bits(),
                bar.volume.to_bits(),
            ]
        }),
    )
}

/// What the footprint of `bars` depends on, hashed: change any and it is built again.
fn footprint_key(
    bars: &[Kline],
    shape: FootprintShape,
    candle_revision: u64,
    trade_revision: Option<u64>,
    interval: &str,
) -> u64 {
    hash_of((
        last_bar_signature(bars),
        shape,
        candle_revision,
        trade_revision,
        interval,
    ))
}

fn svp_key(
    bars: &[Kline],
    shape: SvpShape,
    candle_revision: u64,
    until_ms: i64,
    interval: &str,
) -> u64 {
    hash_of((
        last_bar_signature(bars),
        shape,
        candle_revision,
        until_ms,
        interval,
    ))
}

fn tpo_key(
    bars: &[Kline],
    shape: TpoShape,
    edits: &[TpoEdit],
    candle_revision: u64,
    until_ms: i64,
) -> u64 {
    hash_of((
        last_bar_signature(bars),
        shape,
        edits.iter().map(|e| format!("{e:?}")).collect::<Vec<_>>(),
        candle_revision,
        until_ms,
    ))
}

impl ProfileRuntime {
    /// The chart left the order-flow types: forget everything.
    fn clear(&mut self) {
        *self = Self::default();
    }

    fn run(&mut self, job: Job, ctx: &egui::Context) {
        let (sender, receiver) = mpsc::channel();
        self.key = Some(job.key());
        self.inbox = Some(receiver);
        let repaint = ctx.clone();
        match job {
            Job::Footprint(job) => {
                let mut cache = self.footprint_cache.take().unwrap_or_default();
                crate::service::work_queue::submit(move || {
                    let candle_guard = job
                        .candles
                        .data
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner);
                    let tick = candle_guard.tick_size;
                    let mut trade_guard = job
                        .trades
                        .as_ref()
                        .map(|trades| trades.book.lock().unwrap_or_else(PoisonError::into_inner));
                    let set = footprint::build_set(
                        SetInput {
                            bars: &job.bars,
                            bar_ms: job.bar_ms,
                            candles: Some(&candle_guard.book),
                            trades: trade_guard.as_deref_mut(),
                            exchange_tick: tick,
                            shape: job.shape,
                        },
                        &mut cache,
                    );
                    drop(trade_guard);
                    drop(candle_guard);
                    let _ = sender.send(Computed {
                        key: job.key,
                        done: Done::Footprint(set, cache),
                    });
                    repaint.request_repaint();
                });
            }
            Job::Tpo(job) => {
                let mut cache = self.tpo_cache.take().unwrap_or_default();
                crate::service::work_queue::submit(move || {
                    let candle_guard = job
                        .candles
                        .data
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner);
                    let set = tpo::build_set(
                        tpo::SetInput {
                            bars: &job.bars,
                            bar_ms: job.bar_ms,
                            candles: &candle_guard.book,
                            exchange_tick: candle_guard.tick_size,
                            shape: job.shape,
                            edits: &job.edits,
                            until_ms: job.until_ms,
                            now_ms: now_millis(),
                        },
                        &mut cache,
                    );
                    drop(candle_guard);
                    let _ = sender.send(Computed {
                        key: job.key,
                        done: Done::Tpo(set, cache),
                    });
                    repaint.request_repaint();
                });
            }
            Job::Svp(job) => {
                let mut cache = self.svp_cache.take().unwrap_or_default();
                crate::service::work_queue::submit(move || {
                    let candle_guard = job
                        .candles
                        .data
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner);
                    let set = svp::build_set(
                        svp::SetInput {
                            bars: &job.bars,
                            bar_ms: job.bar_ms,
                            candles: &candle_guard.book,
                            exchange_tick: candle_guard.tick_size,
                            shape: job.shape,
                            until_ms: job.until_ms,
                            now_ms: now_millis(),
                        },
                        &mut cache,
                    );
                    drop(candle_guard);
                    let _ = sender.send(Computed {
                        key: job.key,
                        done: Done::Svp(set, cache),
                    });
                    repaint.request_repaint();
                });
            }
        }
    }
}

fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as i64)
}

/// The end of what a chart in replay needs: the end of its last displayed bar. A live chart needs
/// everything up to now (`i64::MAX`).
fn replay_end(bars: &[Kline], bar_ms: i64, replay_ms: Option<i64>) -> i64 {
    match (replay_ms, bars.last()) {
        (Some(_), Some(last)) => last.open_time.saturating_add(bar_ms),
        _ => i64::MAX,
    }
}

/// What the replay position lets a TPO read: candles opening before the end of the last displayed bar.
fn tpo_until(bars: &[Kline], bar_ms: i64, replay_ms: Option<i64>) -> i64 {
    match (replay_ms, bars.last()) {
        (Some(_), Some(last)) => last.open_time.saturating_add(bar_ms),
        _ => i64::MAX,
    }
}

/// The oldest instant the candle budget lets a TPO of `spans` reach: the newest spans whose 5-minute
/// candles together fit, so a long history does not ask for more than the hub holds.
fn tpo_from(spans: &[crate::model::market_profile::sessions::Session], until_ms: i64) -> i64 {
    let mut candles = 0i64;
    let mut from = spans.last().map_or(0, |span| span.start_ms);
    for span in spans.iter().rev() {
        let end = span.end_ms.min(until_ms);
        candles += (end - span.start_ms).max(0) / TPO_INTERVAL_MS + 1;
        if candles > MAX_CANDLES as i64 {
            break;
        }
        from = span.start_ms;
    }
    from
}

impl super::ChartWorkspace {
    /// Once a frame: asks the hub for what each order-flow chart needs, takes in finished
    /// computations and starts the next, and hands the cards their feeds.
    pub(super) fn tick_profiles(&mut self, ctx: &egui::Context) {
        let replay_ms = self.replay_cut();
        let bar_ms = self.timeframe.nominal_ms();
        for chart in &mut self.charts {
            let kind = chart.chart.effective_chart_type(chart.source);
            let order_flow = chart.is_market()
                && matches!(
                    kind,
                    ChartType::VolumeFootprint
                        | ChartType::TimePriceOpportunity
                        | ChartType::SessionVolumeProfile
                );
            if !order_flow {
                if chart.profile.fed {
                    chart.chart.set_profile_feed(None);
                }
                if chart.profile.kind.is_some() || chart.profile.fed {
                    self.profile_hub.forget_chart(chart.id);
                    chart.profile.clear();
                }
                continue;
            }
            if chart.profile.kind != Some(kind) {
                // Another order-flow type: nothing built for the last one means anything here.
                chart.chart.set_profile_feed(None);
                chart.profile.clear();
                chart.profile.kind = Some(kind);
            }
            let style = chart.chart.appearance().style;
            let bars = displayed_candles(&chart.candles, replay_ms);
            let instrument = InstrumentId::new(chart.source, chart.symbol.clone());
            let need = if kind == ChartType::VolumeFootprint {
                let shape = style.footprint.shape();
                // The footprint is built for the newest bars the candle budget covers.
                match footprint::intrabar_plan(bar_ms, bars.len(), MAX_CANDLES) {
                    Ok(plan) => {
                        let first = bars.len().saturating_sub(plan.bars.min(MAX_BARS));
                        Ok(ProfileNeed {
                            instrument,
                            interval: plan.interval,
                            interval_ms: plan.interval_ms,
                            from_ms: bars.get(first).map_or(0, |bar| bar.open_time),
                            to_ms: replay_end(bars, bar_ms, replay_ms),
                            trades: shape.tick_based,
                        })
                    }
                    Err(reason) => Err(reason),
                }
            } else if kind == ChartType::SessionVolumeProfile {
                // TradingView: intraday and daily charts only.
                if bar_ms >= SVP_LONGEST_BAR_MS {
                    Err(Unsupported::BarTooLong)
                } else {
                    let shape = style.svp.shape();
                    let until = tpo_until(bars, bar_ms, replay_ms);
                    let spans = svp::spans_for(bars, bar_ms, until, &shape);
                    let (interval, interval_ms, from_ms) =
                        svp::candle_plan(&spans, until, MAX_CANDLES);
                    Ok(ProfileNeed {
                        instrument,
                        interval,
                        interval_ms,
                        from_ms,
                        to_ms: replay_end(bars, bar_ms, replay_ms),
                        trades: false,
                    })
                }
            } else {
                let shape = style.tpo.shape();
                let until = tpo_until(bars, bar_ms, replay_ms);
                let spans = tpo::spans_for(bars, bar_ms, until, &shape);
                Ok(ProfileNeed {
                    instrument,
                    interval: TPO_INTERVAL,
                    interval_ms: TPO_INTERVAL_MS,
                    from_ms: tpo_from(&spans, until),
                    to_ms: replay_end(bars, bar_ms, replay_ms),
                    trades: false,
                })
            };
            match need {
                Ok(need) => self
                    .profile_hub
                    .sync_chart(chart.id, std::slice::from_ref(&need)),
                Err(reason) => {
                    self.profile_hub.forget_chart(chart.id);
                    let status = SetStatus::Unsupported(reason);
                    let feed = match kind {
                        ChartType::VolumeFootprint => ProfileFeed {
                            footprint: Some(Arc::new(FootprintSet::empty(status))),
                            ..ProfileFeed::default()
                        },
                        ChartType::SessionVolumeProfile => ProfileFeed {
                            svp: Some(Arc::new(SvpSet::empty(status))),
                            ..ProfileFeed::default()
                        },
                        _ => ProfileFeed {
                            tpo: Some(Arc::new(TpoSet::empty(status))),
                            ..ProfileFeed::default()
                        },
                    };
                    chart.chart.set_profile_feed(Some(feed));
                    chart.profile.fed = true;
                    chart.profile.key = None;
                }
            }
            // Splits and merges mean something only for the periods and block size they were made
            // under: changing either drops them (TradingView's rule). The first look keeps what a
            // saved layout brought.
            let edit_key = style.tpo.shape().edit_key();
            if kind == ChartType::TimePriceOpportunity {
                if chart.tpo_edit_key.is_some_and(|key| key != edit_key) {
                    chart.tpo_edits.clear();
                }
                chart.tpo_edit_key = Some(edit_key);
            }
        }
        self.profile_hub.poll(ctx, std::time::Instant::now());
        let ids: Vec<u64> = self
            .charts
            .iter()
            .filter(|chart| {
                chart.is_market()
                    && matches!(
                        chart.chart.effective_chart_type(chart.source),
                        ChartType::VolumeFootprint
                            | ChartType::TimePriceOpportunity
                            | ChartType::SessionVolumeProfile
                    )
            })
            .map(|chart| chart.id)
            .collect();
        for id in ids {
            self.advance_profile(id, ctx, replay_ms, bar_ms);
        }
    }

    fn advance_profile(
        &mut self,
        chart_id: u64,
        ctx: &egui::Context,
        replay_ms: Option<i64>,
        bar_ms: i64,
    ) {
        let Some(index) = self.chart_index(chart_id) else {
            return;
        };
        let chart = &mut self.charts[index];
        let kind = chart.chart.effective_chart_type(chart.source);
        // Take in a finished computation.
        if let Some(receiver) = &chart.profile.inbox {
            match receiver.try_recv() {
                Ok(done) => {
                    chart.profile.inbox = None;
                    let feed = match done.done {
                        Done::Footprint(set, cache) => {
                            chart.profile.footprint_cache = Some(cache);
                            ProfileFeed {
                                footprint: Some(Arc::new(set)),
                                ..ProfileFeed::default()
                            }
                        }
                        Done::Tpo(set, cache) => {
                            chart.profile.tpo_cache = Some(cache);
                            ProfileFeed {
                                tpo: Some(Arc::new(set)),
                                ..ProfileFeed::default()
                            }
                        }
                        Done::Svp(set, cache) => {
                            chart.profile.svp_cache = Some(cache);
                            ProfileFeed {
                                svp: Some(Arc::new(set)),
                                ..ProfileFeed::default()
                            }
                        }
                    };
                    chart.chart.set_profile_feed(Some(feed));
                    chart.profile.fed = true;
                    chart.profile.key = Some(done.key);
                }
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => chart.profile.inbox = None,
            }
        }
        let style = chart.chart.appearance().style;
        let bars = displayed_candles(&chart.candles, replay_ms);
        let instrument = InstrumentId::new(chart.source, chart.symbol.clone());
        let (job, status) = if kind == ChartType::VolumeFootprint {
            let shape = style.footprint.shape();
            let Ok(plan) = footprint::intrabar_plan(bar_ms, bars.len(), MAX_CANDLES) else {
                return;
            };
            let Some((candles, status)) = self.profile_hub.candles(&instrument, plan.interval)
            else {
                return;
            };
            let trades = shape
                .tick_based
                .then(|| self.profile_hub.trades(&instrument))
                .flatten();
            let first = bars.len().saturating_sub(plan.bars.min(MAX_BARS));
            let window = &bars[first.min(bars.len())..];
            let key = footprint_key(
                window,
                shape,
                candles.revision.load(Ordering::Acquire),
                trades
                    .as_ref()
                    .map(|(shared, _)| shared.revision.load(Ordering::Acquire)),
                plan.interval,
            );
            (
                Job::Footprint(Box::new(FootprintJob {
                    key,
                    bars: window.to_vec(),
                    bar_ms,
                    candles,
                    trades: trades.map(|(shared, _)| shared),
                    shape,
                })),
                status,
            )
        } else if kind == ChartType::SessionVolumeProfile {
            let shape = style.svp.shape();
            if bar_ms >= SVP_LONGEST_BAR_MS {
                return;
            }
            let until = tpo_until(bars, bar_ms, replay_ms);
            let spans = svp::spans_for(bars, bar_ms, until, &shape);
            let (interval, _, from) = svp::candle_plan(&spans, until, MAX_CANDLES);
            let Some((candles, status)) = self.profile_hub.candles(&instrument, interval) else {
                return;
            };
            let first = bars.partition_point(|bar| bar.open_time < from);
            let window = &bars[first.min(bars.len())..];
            let key = svp_key(
                window,
                shape,
                candles.revision.load(Ordering::Acquire),
                until,
                interval,
            );
            (
                Job::Svp(Box::new(SvpJob {
                    key,
                    bars: window.to_vec(),
                    bar_ms,
                    candles,
                    shape,
                    until_ms: until,
                })),
                status,
            )
        } else {
            let shape = style.tpo.shape();
            let Some((candles, status)) = self.profile_hub.candles(&instrument, TPO_INTERVAL)
            else {
                return;
            };
            let until = tpo_until(bars, bar_ms, replay_ms);
            // Only the bars from the oldest period the budget reaches: the rest cannot have a profile.
            let spans = tpo::spans_for(bars, bar_ms, until, &shape);
            let from = tpo_from(&spans, until);
            let first = bars.partition_point(|bar| bar.open_time < from);
            let window = &bars[first.min(bars.len())..];
            let key = tpo_key(
                window,
                shape,
                &chart.tpo_edits,
                candles.revision.load(Ordering::Acquire),
                until,
            );
            (
                Job::Tpo(Box::new(TpoJob {
                    key,
                    bars: window.to_vec(),
                    bar_ms,
                    candles,
                    shape,
                    edits: chart.tpo_edits.clone(),
                    until_ms: until,
                })),
                status,
            )
        };
        // What is wanted now is what is running (or already shown): a job that waited for a state the
        // chart has since left would, once it ran, show a superseded set.
        if chart.profile.key == Some(job.key()) {
            chart.profile.waiting = None;
        }
        if chart.profile.key != Some(job.key()) {
            if chart.profile.inbox.is_some() {
                chart.profile.waiting = Some(Box::new(job));
            } else {
                chart.profile.run(job, ctx);
            }
        } else if chart.profile.inbox.is_none()
            && let Some(waiting) = chart.profile.waiting.take()
            && chart.profile.key != Some(waiting.key())
        {
            chart.profile.run(*waiting, ctx);
        }
        // A computation that finished while a newer one was waiting: start the waiting one.
        if chart.profile.inbox.is_none()
            && let Some(waiting) = chart.profile.waiting.take()
        {
            chart.profile.run(*waiting, ctx);
        }
        if let FlowStatus::Error(message) = status
            && !chart.profile.fed
        {
            chart.chart.set_profile_feed(Some(ProfileFeed {
                error: Some(message),
                ..ProfileFeed::default()
            }));
            chart.profile.fed = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::DataSource;
    use crate::model::market_profile::footprint::FootprintSet;
    use crate::model::market_profile::quality::DataQuality;
    use crate::workspace::profile_hub::ProfileHub;
    use crate::workspace::profile_hub::testing::{Fake, MINUTE};
    use crate::workspace::test_support::{chart_with_drawings, empty_workspace};
    use std::sync::atomic::Ordering;
    use std::time::Duration;

    const DAY: i64 = 86_400_000;

    fn day_bar(open_time: i64, low: f64, high: f64) -> Kline {
        Kline {
            open_time,
            close_time: open_time + DAY - 1,
            open: low + 1.0,
            high,
            low,
            close: high - 1.0,
            volume: 50.0,
        }
    }

    /// A workspace with one Binance chart of two daily bars on the volume footprint, and a hub that
    /// answers from `fake`.
    fn footprint_workspace(
        fake: &Arc<Fake>,
        first_open: i64,
    ) -> (super::super::ChartWorkspace, u64) {
        let mut workspace = empty_workspace(1);
        workspace.profile_hub = ProfileHub::with_backend(fake.clone());
        let id = chart_with_drawings(&mut workspace, "BTCUSDT", &[]);
        let at = workspace.chart_index(id).unwrap();
        workspace.charts[at].candles = vec![
            day_bar(first_open, 98.0, 103.0),
            day_bar(first_open + DAY, 99.0, 104.0),
        ];
        workspace.charts[at]
            .chart
            .set_chart_type(ChartType::VolumeFootprint);
        (workspace, id)
    }

    fn feed_of(workspace: &super::super::ChartWorkspace, id: u64) -> Option<Arc<FootprintSet>> {
        let at = workspace.chart_index(id)?;
        workspace.charts[at]
            .chart
            .profile_feed()
            .and_then(|feed| feed.footprint.clone())
    }

    /// Ticks until the card has a footprint (the computation runs on the work queue).
    fn wait_for_set(
        workspace: &mut super::super::ChartWorkspace,
        id: u64,
        ready: impl Fn(&FootprintSet) -> bool,
    ) -> Arc<FootprintSet> {
        let ctx = egui::Context::default();
        for _ in 0..400 {
            workspace.tick_profiles(&ctx);
            if let Some(set) = feed_of(workspace, id)
                && ready(&set)
            {
                return set;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("no footprint arrived");
    }

    fn market_for(fake: &Fake, from: i64, minutes: i64) {
        *fake.market.lock().unwrap() = (0..minutes).map(|i| from + i * MINUTE).collect();
    }

    #[test]
    fn a_footprint_chart_asks_for_finer_candles_and_the_card_is_handed_the_set() {
        let fake = Arc::new(Fake::default());
        let first = 10 * DAY;
        market_for(&fake, first, 2 * 24 * 60);
        let (mut workspace, id) = footprint_workspace(&fake, first);
        let set = wait_for_set(&mut workspace, id, |set| set.counts.split == 2);
        assert_eq!(set.bars.len(), 2);
        assert!(set.bars.iter().all(|bar| {
            bar.as_ref()
                .is_some_and(|bar| bar.quality == DataQuality::ExchangeSplit)
        }));
        // Daily bars are cut into minutes while the budget allows it.
        let requests = fake.candle_requests.lock().unwrap();
        assert!(requests.iter().any(|(start, _, _)| *start <= first));
        // The tick is inferred from the prices (no exchange tick from the fake), and says so.
        assert_eq!(
            set.tick.map(|tick| tick.source),
            Some(crate::model::market_profile::tick::TickSource::Inferred)
        );
    }

    #[test]
    fn trades_build_the_bars_they_cover_and_candles_the_rest() {
        let fake = Arc::new(Fake::default());
        fake.trades.store(true, Ordering::Release);
        fake.trade_count.store(50, Ordering::Release); // ids 1..=50 at times 1001..=1050
        // The second (last, still forming) bar opens after the first trade, so the book is complete
        // from it; the first bar opened before the book starts, so candles build it.
        let first = 1_010 - DAY;
        market_for(&fake, first, 2 * 24 * 60);
        let (mut workspace, id) = footprint_workspace(&fake, first);
        let set = wait_for_set(&mut workspace, id, |set| {
            set.counts.trades == 1 && set.counts.split == 1
        });
        assert_eq!(
            set.bars[1].as_ref().unwrap().quality,
            DataQuality::TradeDerived
        );
        assert_eq!(
            set.bars[0].as_ref().unwrap().quality,
            DataQuality::ExchangeSplit
        );
    }

    #[test]
    fn leaving_the_footprint_releases_the_hub_and_clears_the_card() {
        let fake = Arc::new(Fake::default());
        let first = 10 * DAY;
        market_for(&fake, first, 2 * 24 * 60);
        let (mut workspace, id) = footprint_workspace(&fake, first);
        wait_for_set(&mut workspace, id, |set| set.counts.split == 2);
        assert!(!workspace.profile_hub.is_empty());
        let at = workspace.chart_index(id).unwrap();
        workspace.charts[at]
            .chart
            .set_chart_type(ChartType::Candles);
        workspace.tick_profiles(&egui::Context::default());
        assert!(
            workspace.profile_hub.is_empty(),
            "nothing held for a candle chart"
        );
        assert!(feed_of(&workspace, id).is_none());
    }

    #[test]
    fn a_source_with_no_intraday_candles_keeps_drawing_candles_and_fetches_nothing() {
        let fake = Arc::new(Fake::default());
        let mut workspace = empty_workspace(1);
        workspace.profile_hub = ProfileHub::with_backend(fake.clone());
        let ctx = egui::Context::default();
        let id = workspace
            .add_chart_for(
                &crate::model::InstrumentId::new(DataSource::Alpaca, "AAPL"),
                &ctx,
            )
            .chart_id;
        let at = workspace.chart_index(id).unwrap();
        workspace.charts[at].candles = vec![day_bar(DAY, 98.0, 103.0)];
        workspace.charts[at]
            .chart
            .set_chart_type(ChartType::VolumeFootprint);
        // The type is kept (the user's choice survives) but the chart draws candles.
        assert_eq!(
            workspace.charts[at]
                .chart
                .effective_chart_type(DataSource::Alpaca),
            ChartType::Candles
        );
        workspace.tick_profiles(&ctx);
        assert!(workspace.profile_hub.is_empty());
        assert!(fake.candle_requests.lock().unwrap().is_empty());
    }

    #[test]
    fn a_changed_row_size_builds_the_set_again() {
        let fake = Arc::new(Fake::default());
        let first = 10 * DAY;
        market_for(&fake, first, 2 * 24 * 60);
        let (mut workspace, id) = footprint_workspace(&fake, first);
        let one = wait_for_set(&mut workspace, id, |set| set.counts.split == 2);
        let at = workspace.chart_index(id).unwrap();
        {
            let footprint = &mut workspace.charts[at].chart.chart_style_mut().footprint;
            footprint.row_mode = crate::model::chart_type::RowMode::Manual;
            footprint.ticks_per_row = 2;
        }
        let two = wait_for_set(&mut workspace, id, |set| {
            set.grid.is_some_and(|grid| grid.ticks_per_row() == 2)
        });
        assert!(one.grid.unwrap().ticks_per_row() != two.grid.unwrap().ticks_per_row());
    }

    // MARK: TPO

    use crate::model::chart_type::PeriodKind;
    use crate::model::market_profile::tpo::TpoSet;

    /// Five-minute candles from `first` for `days` days: what the hub's fake serves a TPO.
    fn five_minute_market(fake: &Fake, first: i64, days: i64) {
        *fake.market.lock().unwrap() = (0..days * 288).map(|i| first + i * 300_000).collect();
    }

    fn tpo_workspace(fake: &Arc<Fake>, first_open: i64) -> (super::super::ChartWorkspace, u64) {
        let (mut workspace, id) = footprint_workspace(fake, first_open);
        let at = workspace.chart_index(id).unwrap();
        workspace.charts[at]
            .chart
            .set_chart_type(ChartType::TimePriceOpportunity);
        (workspace, id)
    }

    fn tpo_of(workspace: &super::super::ChartWorkspace, id: u64) -> Option<Arc<TpoSet>> {
        let at = workspace.chart_index(id)?;
        workspace.charts[at]
            .chart
            .profile_feed()
            .and_then(|feed| feed.tpo.clone())
    }

    fn wait_for_tpo(
        workspace: &mut super::super::ChartWorkspace,
        id: u64,
        ready: impl Fn(&TpoSet) -> bool,
    ) -> Arc<TpoSet> {
        let ctx = egui::Context::default();
        for _ in 0..400 {
            workspace.tick_profiles(&ctx);
            if let Some(set) = tpo_of(workspace, id)
                && ready(&set)
            {
                return set;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("no TPO arrived");
    }

    #[test]
    fn a_tpo_chart_asks_for_five_minute_candles_and_the_card_is_handed_profiles() {
        let fake = Arc::new(Fake::default());
        let first = 10 * DAY;
        five_minute_market(&fake, first, 2);
        let (mut workspace, id) = tpo_workspace(&fake, first);
        let set = wait_for_tpo(&mut workspace, id, |set| set.profiles.len() == 2);
        assert_eq!(set.profiles[0].start_ms, first);
        assert_eq!(set.profiles[0].blocks, 48);
        assert_eq!(set.block_ms, 1_800_000);
        // No trades, no footprint: only the TPO's feed.
        let at = workspace.chart_index(id).unwrap();
        assert!(
            workspace.charts[at]
                .chart
                .profile_feed()
                .unwrap()
                .footprint
                .is_none()
        );
        let requests = fake.candle_requests.lock().unwrap();
        assert!(requests.iter().any(|(start, _, _)| *start <= first));
        assert!(
            fake.history_requests.load(Ordering::Acquire) == 0,
            "a TPO has no use for trades"
        );
    }

    #[test]
    fn switching_between_the_order_flow_types_rebuilds_for_the_new_one() {
        let fake = Arc::new(Fake::default());
        let first = 10 * DAY;
        five_minute_market(&fake, first, 2);
        let (mut workspace, id) = footprint_workspace(&fake, first);
        // Footprint first (the fake serves 5-minute spacing as its minutes: still candles).
        wait_for_set(&mut workspace, id, |set| !set.bars.is_empty());
        let at = workspace.chart_index(id).unwrap();
        workspace.charts[at]
            .chart
            .set_chart_type(ChartType::TimePriceOpportunity);
        let set = wait_for_tpo(&mut workspace, id, |set| !set.profiles.is_empty());
        assert!(!set.profiles.is_empty());
        assert!(
            workspace.charts[at]
                .chart
                .profile_feed()
                .unwrap()
                .footprint
                .is_none(),
            "the footprint's set is gone"
        );
    }

    #[test]
    fn splits_and_merges_change_the_profiles_and_changing_the_block_size_drops_them() {
        let fake = Arc::new(Fake::default());
        let first = 10 * DAY;
        five_minute_market(&fake, first, 2);
        let (mut workspace, id) = tpo_workspace(&fake, first);
        wait_for_tpo(&mut workspace, id, |set| set.profiles.len() == 2);
        let at = workspace.chart_index(id).unwrap();
        workspace.charts[at].tpo_edits = vec![crate::model::market_profile::tpo::TpoEdit::Split {
            at_ms: first + 12 * 3_600_000,
        }];
        let split = wait_for_tpo(&mut workspace, id, |set| set.profiles.len() == 3);
        assert_eq!(split.profiles[1].start_ms, first + 12 * 3_600_000);
        // The user changes the block size: the splits no longer mean anything.
        workspace.charts[at]
            .chart
            .chart_style_mut()
            .tpo
            .block_minutes = 60;
        let rebuilt = wait_for_tpo(&mut workspace, id, |set| set.block_ms == 3_600_000);
        assert_eq!(rebuilt.profiles.len(), 2);
        assert!(workspace.charts[at].tpo_edits.is_empty());
    }

    #[test]
    fn a_layouts_saved_splits_survive_the_first_look() {
        let fake = Arc::new(Fake::default());
        let first = 10 * DAY;
        five_minute_market(&fake, first, 2);
        let (mut workspace, id) = tpo_workspace(&fake, first);
        let at = workspace.chart_index(id).unwrap();
        // As if restored from a layout: edits present, never seen with a key before.
        workspace.charts[at].tpo_edits = vec![crate::model::market_profile::tpo::TpoEdit::Split {
            at_ms: first + 6 * 3_600_000,
        }];
        let set = wait_for_tpo(&mut workspace, id, |set| set.profiles.len() == 3);
        assert_eq!(set.profiles[1].start_ms, first + 6 * 3_600_000);
        assert_eq!(workspace.charts[at].tpo_edits.len(), 1);
    }

    #[test]
    fn weekly_periods_group_the_days_and_leaving_the_tpo_clears_the_card() {
        let fake = Arc::new(Fake::default());
        let first = 10 * DAY;
        five_minute_market(&fake, first, 2);
        let (mut workspace, id) = tpo_workspace(&fake, first);
        let at = workspace.chart_index(id).unwrap();
        workspace.charts[at].chart.chart_style_mut().tpo.period = PeriodKind::Week;
        wait_for_tpo(&mut workspace, id, |set| {
            set.profiles.len() == 1 || set.profiles.len() == 2
        });
        workspace.charts[at]
            .chart
            .set_chart_type(ChartType::Candles);
        workspace.tick_profiles(&egui::Context::default());
        assert!(workspace.profile_hub.is_empty());
        assert!(workspace.charts[at].chart.profile_feed().is_none());
    }

    /// The whole path against real Binance: klines with the taker-buy split, the exchange tick size,
    /// trade history and the live stream through the real hub, the real computation, and the real
    /// painter into a picture. `DEGENVIEW_GALLERY_DIR=/tmp/g cargo test --lib live_binance_footprint
    /// -- --ignored --nocapture`.
    #[test]
    #[ignore = "needs the network"]
    fn live_binance_footprint_end_to_end() {
        use crate::model::Timeframe;
        let ctx = egui::Context::default();
        let mut workspace = empty_workspace(1);
        workspace.timeframe = Timeframe::Hour;
        let id = workspace
            .add_chart_for(&InstrumentId::new(DataSource::Binance, "BTCUSDT"), &ctx)
            .chart_id;
        let at = workspace.chart_index(id).unwrap();
        workspace.charts[at]
            .chart
            .set_chart_type(ChartType::VolumeFootprint);
        let started = std::time::Instant::now();
        // Wait for the chart's own candles.
        while workspace.charts[at].candles.is_empty() && started.elapsed() < Duration::from_secs(30)
        {
            workspace.charts[at].receive_fetch();
            std::thread::sleep(Duration::from_millis(100));
        }
        assert!(
            !workspace.charts[at].candles.is_empty(),
            "no candles from Binance"
        );
        println!(
            "{} chart candles after {:?}",
            workspace.charts[at].candles.len(),
            started.elapsed()
        );
        // Keep only the newest few so the hub fetches little and the picture is readable.
        let keep = workspace.charts[at].candles.len() - 8;
        workspace.charts[at].candles.drain(..keep);
        let set = wait_for_set(&mut workspace, id, |set| {
            set.counts.split + set.counts.trades >= 7
        });
        println!(
            "{:?} after {:?}; tick {:?}; grid rows of {} tick(s)",
            set.counts,
            started.elapsed(),
            set.tick,
            set.grid.map_or(0, |grid| grid.ticks_per_row())
        );
        // Let the trade history and the live stream run a while, then look again.
        let deadline = std::time::Instant::now() + Duration::from_secs(60);
        let mut latest = set;
        while std::time::Instant::now() < deadline {
            workspace.tick_profiles(&ctx);
            if let Some(next) = feed_of(&workspace, id) {
                latest = next;
                if latest.counts.trades >= 1 {
                    break;
                }
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        println!("{:?} after {:?}", latest.counts, started.elapsed());
        for (index, bar) in latest.bars.iter().enumerate() {
            if let Some(bar) = bar {
                println!(
                    "bar {index}: {:?} {} rows, buy {:.2} sell {:.2}, poc row {}, stacks {}",
                    bar.quality,
                    bar.cells.len(),
                    bar.buy,
                    bar.sell,
                    bar.poc_row(),
                    bar.stacks.len()
                );
            }
        }
        assert!(
            latest.counts.split + latest.counts.trades >= 7,
            "{:?}",
            latest.counts
        );
        assert!(latest.counts.refused == 0, "{:?}", latest.first_refusal);
        // History is a one-off fetch, so only the live stream can make the book's newest trade
        // newer: watch it move, with no id gap behind it.
        let instrument = InstrumentId::new(DataSource::Binance, "BTCUSDT");
        let (shared, _) = workspace
            .profile_hub
            .trades(&instrument)
            .expect("trades held");
        let newest = |workspace: &mut super::super::ChartWorkspace| {
            for _ in 0..50 {
                workspace.tick_profiles(&ctx);
                std::thread::sleep(Duration::from_millis(100));
            }
            let book = shared.book.lock().unwrap();
            (book.newest().map(|t| t.id), book.len())
        };
        let (before, held_before) = newest(&mut workspace);
        let (after, held_after) = newest(&mut workspace);
        println!(
            "live stream: newest trade id {before:?} -> {after:?}, trades held {held_before} -> {held_after}"
        );
        assert!(after > before, "the live stream added no trades");
        let mut book = shared.book.lock().unwrap();
        let from = book.complete_from_ms().expect("a complete run");
        println!("book complete from {from}");
        // A picture of the real thing through the real painter.
        let bars: Vec<Kline> = workspace.charts[at].candles.clone();
        crate::ui::paint_gallery_for_tests("footprint-live-binance", &bars, &latest);
    }

    // MARK: Session volume profile

    fn svp_workspace(fake: &Arc<Fake>, first_open: i64) -> (super::super::ChartWorkspace, u64) {
        let (mut workspace, id) = footprint_workspace(fake, first_open);
        let at = workspace.chart_index(id).unwrap();
        workspace.charts[at]
            .chart
            .set_chart_type(ChartType::SessionVolumeProfile);
        (workspace, id)
    }

    fn svp_of(workspace: &super::super::ChartWorkspace, id: u64) -> Option<Arc<SvpSet>> {
        let at = workspace.chart_index(id)?;
        workspace.charts[at]
            .chart
            .profile_feed()
            .and_then(|feed| feed.svp.clone())
    }

    fn wait_for_svp(
        workspace: &mut super::super::ChartWorkspace,
        id: u64,
        ready: impl Fn(&SvpSet) -> bool,
    ) -> Arc<SvpSet> {
        let ctx = egui::Context::default();
        for _ in 0..400 {
            workspace.tick_profiles(&ctx);
            if let Some(set) = svp_of(workspace, id)
                && ready(&set)
            {
                return set;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("no session profile arrived");
    }

    #[test]
    fn a_session_profile_asks_for_the_finest_candles_that_fit_and_the_card_is_handed_sessions() {
        let fake = Arc::new(Fake::default());
        let first = 10 * DAY;
        market_for(&fake, first, 2 * 24 * 60);
        let (mut workspace, id) = svp_workspace(&fake, first);
        let set = wait_for_svp(&mut workspace, id, |set| set.profiles.len() == 2);
        assert_eq!(set.candle_minutes, 1, "two days of minutes fit the budget");
        assert_eq!(set.profiles[0].start_ms, first);
        assert!(set.profiles.iter().all(|p| p.total() > 0.0));
        // The fake's candles carry a taker-buy split, so the profile says it is the exchange's.
        assert_eq!(
            set.profiles[0].quality,
            crate::model::market_profile::quality::DataQuality::ExchangeSplit
        );
        let at = workspace.chart_index(id).unwrap();
        let feed = workspace.charts[at].chart.profile_feed().unwrap();
        assert!(feed.footprint.is_none() && feed.tpo.is_none());
    }

    #[test]
    fn weekly_and_longer_charts_are_refused_for_a_session_profile_and_fetch_nothing() {
        use crate::model::Timeframe;
        let fake = Arc::new(Fake::default());
        let first = 10 * DAY;
        market_for(&fake, first, 2 * 24 * 60);
        let (mut workspace, id) = svp_workspace(&fake, first);
        workspace.timeframe = Timeframe::Week;
        let ctx = egui::Context::default();
        workspace.tick_profiles(&ctx);
        let set = svp_of(&workspace, id).expect("a status");
        assert_eq!(set.status, SetStatus::Unsupported(Unsupported::BarTooLong));
        assert!(workspace.profile_hub.is_empty());
        assert!(fake.candle_requests.lock().unwrap().is_empty());
    }

    #[test]
    fn a_changed_row_size_rebuilds_the_session_profile_and_leaving_the_type_clears_it() {
        let fake = Arc::new(Fake::default());
        let first = 10 * DAY;
        market_for(&fake, first, 2 * 24 * 60);
        let (mut workspace, id) = svp_workspace(&fake, first);
        let a = wait_for_svp(&mut workspace, id, |set| set.profiles.len() == 2);
        let at = workspace.chart_index(id).unwrap();
        {
            // The fake's candles span two ticks: rows of two ticks make fewer of them.
            let svp = &mut workspace.charts[at].chart.chart_style_mut().svp;
            svp.layout = crate::model::chart_type::RowsLayout::TicksPerRow;
            svp.row_size = 2;
        }
        let b = wait_for_svp(&mut workspace, id, |set| {
            set.profiles
                .first()
                .is_some_and(|p| p.rows.len() != a.profiles[0].rows.len())
        });
        assert!(b.profiles[0].rows.len() < a.profiles[0].rows.len());
        workspace.charts[at]
            .chart
            .set_chart_type(ChartType::Candles);
        workspace.tick_profiles(&egui::Context::default());
        assert!(workspace.profile_hub.is_empty());
        assert!(workspace.charts[at].chart.profile_feed().is_none());
    }

    #[test]
    fn the_developing_levels_are_only_computed_when_a_setting_asks_for_them() {
        let fake = Arc::new(Fake::default());
        let first = 10 * DAY;
        market_for(&fake, first, 2 * 24 * 60);
        let (mut workspace, id) = svp_workspace(&fake, first);
        let plain = wait_for_svp(&mut workspace, id, |set| set.profiles.len() == 2);
        assert!(plain.profiles[0].developing.is_empty());
        let at = workspace.chart_index(id).unwrap();
        workspace.charts[at]
            .chart
            .chart_style_mut()
            .svp
            .developing_poc = true;
        let with = wait_for_svp(&mut workspace, id, |set| {
            set.profiles
                .first()
                .is_some_and(|p| !p.developing.is_empty())
        });
        assert!(with.profiles[0].developing.len() <= svp::MAX_DEVELOPING + 1);
    }

    /// Real Binance candles through the hub, the computation and the painter. The chosen candle size
    /// is whatever fits the budget for the days asked for.
    /// `DEGENVIEW_GALLERY_DIR=/tmp/g cargo test --lib live_binance_svp -- --ignored --nocapture`.
    #[test]
    #[ignore = "needs the network"]
    fn live_binance_svp_end_to_end() {
        use crate::model::Timeframe;
        let ctx = egui::Context::default();
        let mut workspace = empty_workspace(1);
        workspace.timeframe = Timeframe::Hour;
        let id = workspace
            .add_chart_for(&InstrumentId::new(DataSource::Binance, "BTCUSDT"), &ctx)
            .chart_id;
        let at = workspace.chart_index(id).unwrap();
        workspace.charts[at]
            .chart
            .set_chart_type(ChartType::SessionVolumeProfile);
        workspace.charts[at]
            .chart
            .chart_style_mut()
            .svp
            .developing_poc = true;
        workspace.charts[at]
            .chart
            .chart_style_mut()
            .svp
            .developing_value_area = true;
        let started = std::time::Instant::now();
        while workspace.charts[at].candles.is_empty() && started.elapsed() < Duration::from_secs(30)
        {
            workspace.charts[at].receive_fetch();
            std::thread::sleep(Duration::from_millis(100));
        }
        assert!(
            !workspace.charts[at].candles.is_empty(),
            "no candles from Binance"
        );
        let keep = workspace.charts[at].candles.len() - 72;
        workspace.charts[at].candles.drain(..keep);
        let set = wait_for_svp(&mut workspace, id, |set| set.profiles.len() >= 3);
        println!(
            "{} sessions after {:?} from {}-minute candles; tick {:?}; refused {}",
            set.profiles.len(),
            started.elapsed(),
            set.candle_minutes,
            set.tick,
            set.refused
        );
        for profile in &set.profiles {
            println!(
                "{}..{}: {} rows of {} ticks, volume {:.0} (up {:.0}, down {:.0}), {:?}, poc {:.2}, va {:.2}..{:.2}, developing {} points, forming {}",
                profile.start_ms,
                profile.end_ms,
                profile.rows.len(),
                profile.grid.ticks_per_row(),
                profile.total(),
                profile.up(),
                profile.down(),
                profile.quality,
                profile.poc_price(),
                profile.val(),
                profile.vah(),
                profile.developing.len(),
                profile.forming,
            );
        }
        assert_eq!(set.refused, 0, "{:?}", set.first_refusal);
        for profile in &set.profiles {
            assert!(profile.total() > 0.0);
            // Volume is conserved: what the rows hold is what the candles reported.
            if let Some(last) = profile.developing.last() {
                assert!((last.poc - profile.poc_price()).abs() < 1e-6);
            }
        }
        let bars = workspace.charts[at].candles.clone();
        crate::ui::svp_gallery_picture("svp-live-binance", &bars, &set);
    }

    /// A TPO of real Binance 5-minute candles through the real hub, computation and painter.
    /// `DEGENVIEW_GALLERY_DIR=/tmp/g cargo test --lib live_binance_tpo -- --ignored --nocapture`.
    #[test]
    #[ignore = "needs the network"]
    fn live_binance_tpo_end_to_end() {
        use crate::model::Timeframe;
        let ctx = egui::Context::default();
        let mut workspace = empty_workspace(1);
        workspace.timeframe = Timeframe::Hour;
        let id = workspace
            .add_chart_for(&InstrumentId::new(DataSource::Binance, "BTCUSDT"), &ctx)
            .chart_id;
        let at = workspace.chart_index(id).unwrap();
        workspace.charts[at]
            .chart
            .set_chart_type(ChartType::TimePriceOpportunity);
        let started = std::time::Instant::now();
        while workspace.charts[at].candles.is_empty() && started.elapsed() < Duration::from_secs(30)
        {
            workspace.charts[at].receive_fetch();
            std::thread::sleep(Duration::from_millis(100));
        }
        assert!(
            !workspace.charts[at].candles.is_empty(),
            "no candles from Binance"
        );
        // The newest three days of hourly bars.
        let keep = workspace.charts[at].candles.len() - 72;
        workspace.charts[at].candles.drain(..keep);
        let set = wait_for_tpo(&mut workspace, id, |set| {
            set.profiles.len() >= 3 && set.profiles.iter().all(|p| p.blocks > 0)
        });
        println!(
            "{} profiles after {:?}; tick {:?}; {} ticks a row; refused {}",
            set.profiles.len(),
            started.elapsed(),
            set.tick,
            set.grid.map_or(0, |grid| grid.ticks_per_row()),
            set.refused
        );
        for profile in &set.profiles {
            println!(
                "{}..{}: {} rows, {} blocks, width {}, poc row {} ({} blocks), value area rows {:?}, IB {:?}, poor high {} low {}, single prints {}, rotation {}, volume {:.0}, forming {}",
                profile.start_ms,
                profile.end_ms,
                profile.rows.len(),
                profile.blocks,
                profile.width(),
                profile.poc_row(),
                profile.rows[profile.poc].count(),
                profile.value_area,
                profile.initial_balance,
                profile.poor_high,
                profile.poor_low,
                profile.single_prints.len(),
                profile.rotation,
                profile.volume_total(),
                profile.forming,
            );
        }
        assert_eq!(set.refused, 0, "{:?}", set.first_refusal);
        // Every finished day of 5-minute candles has all 48 half-hour blocks.
        for profile in set.profiles.iter().filter(|p| !p.forming) {
            assert_eq!(profile.blocks, 48);
        }
        let bars = workspace.charts[at].candles.clone();
        crate::ui::tpo_gallery_picture("tpo-live-binance", &bars, &set);
    }
}
