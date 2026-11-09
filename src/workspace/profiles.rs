//! The order-flow chart types' data, per chart: what a chart asks the profile hub for, the footprint
//! computed from it on a worker, and the finished set handed to the card.
//!
//! The card draws and the hub fetches; this is the part between. Once a frame, for every market chart
//! on an order-flow type:
//!
//! 1. the finer data it needs is asked of the [`ProfileHub`](super::profile_hub::ProfileHub) (a
//!    comparison when nothing changed);
//! 2. when anything the footprint depends on changed (the bars, the finer candles, the trades, the
//!    settings that shape it, the replay position) a computation is started on the work queue, one at
//!    a time per chart, newest request wins;
//! 3. a finished set is handed to the card; completed bars are cached across computations, so a live
//!    trade rebuilds the forming bar only.
//!
//! A chart on another type drops its hold on the hub. Replay hands the computation the bars up to the
//! replay position, so nothing after it is ever read.

use super::chart::displayed_candles;
use super::profile_hub::{CandleShared, FlowStatus, MAX_CANDLES, ProfileNeed, TradeShared};
use crate::model::chart_type::{ChartType, FootprintShape};
use crate::model::market_profile::feed::ProfileFeed;
use crate::model::market_profile::footprint::{
    self, Cache, FootprintSet, MAX_BARS, SetInput, SetStatus,
};
use crate::model::{InstrumentId, Kline};
use eframe::egui;
use std::hash::{Hash, Hasher};
use std::sync::atomic::Ordering;
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, PoisonError};

/// One chart's footprint computation state.
#[derive(Default)]
pub(super) struct ProfileRuntime {
    /// What the last computation was started for (or the card was last given).
    key: Option<u64>,
    /// The running computation's answer.
    inbox: Option<Receiver<Computed>>,
    /// The cache of built bars, held here between computations and lent to each.
    cache: Option<Cache>,
    /// What the next computation is for, when one is already running.
    waiting: Option<Box<Job>>,
    /// Whether the card holds a feed of this runtime's.
    fed: bool,
}

struct Computed {
    key: u64,
    set: FootprintSet,
    cache: Cache,
}

struct Job {
    key: u64,
    bars: Vec<Kline>,
    bar_ms: i64,
    candles: Arc<CandleShared>,
    trades: Option<Arc<TradeShared>>,
    shape: FootprintShape,
}

fn hash_of(parts: impl Hash) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    parts.hash(&mut hasher);
    hasher.finish()
}

/// What the footprint of `bars` depends on, hashed: change any and it is built again.
fn key_of(
    bars: &[Kline],
    shape: FootprintShape,
    candle_revision: u64,
    trade_revision: Option<u64>,
    interval: &str,
) -> u64 {
    let last = bars.last();
    hash_of((
        bars.len(),
        bars.first().map(|bar| bar.open_time),
        last.map(|bar| {
            (
                bar.open_time,
                bar.open.to_bits(),
                bar.high.to_bits(),
                bar.low.to_bits(),
                bar.close.to_bits(),
                bar.volume.to_bits(),
            )
        }),
        shape,
        candle_revision,
        trade_revision,
        interval,
    ))
}

impl ProfileRuntime {
    /// The chart left the order-flow types: forget everything.
    fn clear(&mut self) {
        *self = Self::default();
    }

    fn run(&mut self, job: Job, ctx: &egui::Context) {
        let (sender, receiver) = mpsc::channel();
        let mut cache = self.cache.take().unwrap_or_default();
        self.key = Some(job.key);
        self.inbox = Some(receiver);
        let repaint = ctx.clone();
        crate::service::work_queue::submit(move || {
            let tick = job
                .candles
                .data
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .tick_size;
            let candle_guard = job
                .candles
                .data
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
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
                set,
                cache,
            });
            repaint.request_repaint();
        });
    }
}

impl super::ChartWorkspace {
    /// Once a frame: asks the hub for what each order-flow chart needs, takes in finished
    /// computations and starts the next, and hands the cards their feeds.
    pub(super) fn tick_profiles(&mut self, ctx: &egui::Context) {
        let replay_ms = self.replay_cut();
        let bar_ms = self.timeframe.nominal_ms();
        for chart in &mut self.charts {
            let order_flow = chart.is_market()
                && chart.chart.effective_chart_type(chart.source) == ChartType::VolumeFootprint;
            if !order_flow {
                if chart.profile.fed {
                    chart.chart.set_profile_feed(None);
                }
                if chart.profile.key.is_some() || chart.profile.fed {
                    self.profile_hub.forget_chart(chart.id);
                    chart.profile.clear();
                }
                continue;
            }
            let style = chart.chart.appearance().style;
            let shape = style.footprint.shape();
            let bars = displayed_candles(&chart.candles, replay_ms);
            // The footprint is built for the newest bars the candle budget covers.
            let plan = footprint::intrabar_plan(bar_ms, bars.len(), MAX_CANDLES);
            let plan = match plan {
                Ok(plan) => plan,
                Err(reason) => {
                    self.profile_hub.forget_chart(chart.id);
                    let set = FootprintSet::empty(SetStatus::Unsupported(reason));
                    chart.chart.set_profile_feed(Some(ProfileFeed {
                        footprint: Some(Arc::new(set)),
                        ..ProfileFeed::default()
                    }));
                    chart.profile.fed = true;
                    chart.profile.key = None;
                    continue;
                }
            };
            let first = bars.len().saturating_sub(plan.bars.min(MAX_BARS));
            let instrument = InstrumentId::new(chart.source, chart.symbol.clone());
            let need = ProfileNeed {
                instrument: instrument.clone(),
                interval: plan.interval,
                interval_ms: plan.interval_ms,
                from_ms: bars.get(first).map_or(0, |bar| bar.open_time),
                trades: shape.tick_based,
            };
            self.profile_hub
                .sync_chart(chart.id, std::slice::from_ref(&need));
        }
        self.profile_hub.poll(ctx, std::time::Instant::now());
        let ids: Vec<u64> = self
            .charts
            .iter()
            .filter(|chart| {
                chart.is_market()
                    && chart.chart.effective_chart_type(chart.source) == ChartType::VolumeFootprint
            })
            .map(|chart| chart.id)
            .collect();
        for id in ids {
            self.advance_footprint(id, ctx, replay_ms, bar_ms);
        }
    }

    fn advance_footprint(
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
        // Take in a finished computation.
        if let Some(receiver) = &chart.profile.inbox {
            match receiver.try_recv() {
                Ok(done) => {
                    chart.profile.inbox = None;
                    chart.profile.cache = Some(done.cache);
                    let loading = false;
                    chart.chart.set_profile_feed(Some(ProfileFeed {
                        footprint: Some(Arc::new(done.set)),
                        loading,
                        error: None,
                    }));
                    chart.profile.fed = true;
                    chart.profile.key = Some(done.key);
                }
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => chart.profile.inbox = None,
            }
        }
        let style = chart.chart.appearance().style;
        let shape = style.footprint.shape();
        let bars = displayed_candles(&chart.candles, replay_ms);
        let Ok(plan) = footprint::intrabar_plan(bar_ms, bars.len(), MAX_CANDLES) else {
            return;
        };
        let instrument = InstrumentId::new(chart.source, chart.symbol.clone());
        let Some((candles, status)) = self.profile_hub.candles(&instrument, plan.interval) else {
            return;
        };
        let trades = shape
            .tick_based
            .then(|| self.profile_hub.trades(&instrument))
            .flatten();
        let first = bars.len().saturating_sub(plan.bars.min(MAX_BARS));
        let window = &bars[first.min(bars.len())..];
        let key = key_of(
            window,
            shape,
            candles.revision.load(Ordering::Acquire),
            trades
                .as_ref()
                .map(|(shared, _)| shared.revision.load(Ordering::Acquire)),
            plan.interval,
        );
        // Until the first finer candles arrive, say so instead of drawing estimates as if final.
        if chart.profile.key != Some(key) {
            let job = Job {
                key,
                bars: window.to_vec(),
                bar_ms,
                candles,
                trades: trades.map(|(shared, _)| shared),
                shape,
            };
            if chart.profile.inbox.is_some() {
                chart.profile.waiting = Some(Box::new(job));
            } else {
                chart.profile.run(job, ctx);
            }
        } else if chart.profile.inbox.is_none()
            && let Some(job) = chart.profile.waiting.take()
            && chart.profile.key != Some(job.key)
        {
            chart.profile.run(*job, ctx);
        }
        // A computation that finished while a newer one was waiting: start the waiting one.
        if chart.profile.inbox.is_none()
            && let Some(job) = chart.profile.waiting.take()
        {
            chart.profile.run(*job, ctx);
        }
        if let FlowStatus::Error(message) = status {
            let current = chart.profile.fed;
            if !current {
                chart.chart.set_profile_feed(Some(ProfileFeed {
                    footprint: None,
                    loading: false,
                    error: Some(message),
                }));
                chart.profile.fed = true;
            }
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
        // The first bar opens after the first trade, so the book is complete from it.
        let first = 1_010;
        market_for(&fake, first, 2 * 24 * 60);
        let (mut workspace, id) = footprint_workspace(&fake, first);
        let set = wait_for_set(&mut workspace, id, |set| {
            set.counts.trades == 1 && set.counts.split == 1
        });
        assert_eq!(
            set.bars[0].as_ref().unwrap().quality,
            DataQuality::TradeDerived
        );
        assert_eq!(
            set.bars[1].as_ref().unwrap().quality,
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
}
