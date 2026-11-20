//! The taker-flow feeds' owner, shared by every window: it opens the trade sockets of each asset a
//! panel asks about while one is drawn, closes them when none is, and hands each frame the latest
//! [`Snapshot`] without blocking or recomputing it when nothing changed.
//!
//! Every watched asset has a feed of its own, with its own record and lease, so two windows
//! looking at two assets do not disturb each other, and a socket of one asset that is slow to close
//! can never write into the record of another.

use crate::clock::now_ms;
use crate::lock::lock;
use crate::model::taker_flow::{Flow, Snapshot};
use crate::service::taker_flow::{Lease, TakerFlowFeed};
use eframe::egui;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// How long a feed runs after a panel last asked: longer than a frame, so a frame that is late
/// does not close the sockets, and short enough that a tab in the background stops them.
const LEASE: Duration = Duration::from_secs(3);
/// The totals roll off as time passes, so the panel looks again this often even when quiet.
const REFRESH: Duration = Duration::from_secs(1);

/// A cheap handle: clones share the feeds.
#[derive(Clone)]
pub struct TakerFlowHost {
    inner: Arc<Mutex<Inner<TakerFlowFeed>>>,
}

/// What a feed must say for the host to keep it: whether it still serves all its sockets.
trait Running {
    fn is_running(&self) -> bool;
}

impl Running for TakerFlowFeed {
    fn is_running(&self) -> bool {
        TakerFlowFeed::is_running(self)
    }
}

struct Inner<F> {
    feeds: HashMap<String, Entry<F>>,
}

/// One asset's feed, and everything that belongs to it: dropping the entry closes its sockets,
/// and what it recorded goes with it.
struct Entry<F> {
    flow: Arc<Mutex<Flow>>,
    lease: Lease,
    feed: F,
    /// When a panel last asked, in milliseconds since the epoch.
    asked_ms: i64,
    cached: Option<Cached>,
}

/// A snapshot and what it was made for.
struct Cached {
    revision: u64,
    second: i64,
    snapshot: Arc<Snapshot>,
}

impl Default for TakerFlowHost {
    fn default() -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner {
                feeds: HashMap::new(),
            })),
        }
    }
}

impl TakerFlowHost {
    /// Called by a panel each frame it is drawn: keeps the sockets of `asset` open (opening them,
    /// with an empty record, if they are not) and returns what they have traded.
    pub fn watch(&self, ctx: &egui::Context, asset: &str) -> Arc<Snapshot> {
        let mut inner = lock(&self.inner);
        let now = now_ms();
        let entry = inner.entry_for(asset, now, |flow, lease| {
            let repaint = {
                let ctx = ctx.clone();
                Arc::new(move || ctx.request_repaint())
            };
            TakerFlowFeed::start(asset, Arc::clone(flow), lease.clone(), repaint)
        });
        ctx.request_repaint_after(REFRESH);
        snapshot(entry, now)
    }
}

impl<F: Running> Inner<F> {
    /// The entry of `asset`, asked for at `now`. A new one, with a record of its own, replaces one
    /// that does not serve all its sockets any more; and the feeds nobody has asked about for a
    /// lease are let go, their sockets closing as they are dropped.
    fn entry_for(
        &mut self,
        asset: &str,
        now: i64,
        start: impl FnOnce(&Arc<Mutex<Flow>>, &Lease) -> F,
    ) -> &mut Entry<F> {
        let lapsed = LEASE.as_millis() as i64;
        self.feeds
            .retain(|name, entry| name == asset || now - entry.asked_ms <= lapsed);
        if self
            .feeds
            .get(asset)
            .is_some_and(|entry| !entry.feed.is_running())
        {
            self.feeds.remove(asset);
        }
        let entry = self.feeds.entry(asset.to_owned()).or_insert_with(|| {
            let flow = Arc::new(Mutex::new(Flow::default()));
            let lease = Lease::default();
            // Held before the feed starts, or its watchdog ends it at once.
            lease.renew(LEASE);
            let feed = start(&flow, &lease);
            Entry {
                flow,
                lease,
                feed,
                asked_ms: now,
                cached: None,
            }
        });
        entry.lease.renew(LEASE);
        entry.asked_ms = now;
        entry
    }
}

/// The flow as of `now_ms`, reused while its revision and the second hold.
fn snapshot<F>(entry: &mut Entry<F>, now_ms: i64) -> Arc<Snapshot> {
    let flow = lock(&entry.flow);
    let (revision, second) = (flow.revision(), now_ms / 1_000);
    if let Some(cached) = &entry.cached
        && cached.revision == revision
        && cached.second == second
    {
        return Arc::clone(&cached.snapshot);
    }
    let snapshot = Arc::new(flow.snapshot(now_ms));
    drop(flow);
    entry.cached = Some(Cached {
        revision,
        second,
        snapshot: Arc::clone(&snapshot),
    });
    snapshot
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::liquidation::Exchange;
    use crate::model::taker_flow::{Market, Taker, Trade, Venue};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    const NOW: i64 = 5_000_000_000;

    /// A feed that is running until told it is not, counting how many were started.
    struct Fake {
        alive: Arc<AtomicBool>,
    }

    impl Running for Fake {
        fn is_running(&self) -> bool {
            self.alive.load(Ordering::Acquire)
        }
    }

    struct Rig {
        inner: Inner<Fake>,
        started: AtomicUsize,
    }

    impl Rig {
        fn new() -> Self {
            Self {
                inner: Inner {
                    feeds: HashMap::new(),
                },
                started: AtomicUsize::new(0),
            }
        }

        fn ask(&mut self, asset: &str, now: i64) -> (Arc<Mutex<Flow>>, Arc<AtomicBool>) {
            let started = &self.started;
            let mut alive = None;
            let entry = self.inner.entry_for(asset, now, |_, _| {
                started.fetch_add(1, Ordering::AcqRel);
                let flag = Arc::new(AtomicBool::new(true));
                alive = Some(Arc::clone(&flag));
                Fake { alive: flag }
            });
            let flag = alive.unwrap_or_else(|| Arc::clone(&entry.feed.alive));
            (Arc::clone(&entry.flow), flag)
        }

        fn started(&self) -> usize {
            self.started.load(Ordering::Acquire)
        }
    }

    fn trade(notional: f64) -> Trade {
        Trade {
            venue: Venue {
                exchange: Exchange::Bybit,
                market: Market::Spot,
            },
            notional,
            taker: Taker::Sell,
            time_ms: NOW,
        }
    }

    #[test]
    fn a_snapshot_is_reused_until_the_flow_or_the_second_changes() {
        let mut rig = Rig::new();
        rig.ask("BTC", NOW);
        let entry = rig.inner.feeds.get_mut("BTC").unwrap();
        let first = snapshot(entry, NOW);
        assert!(Arc::ptr_eq(&first, &snapshot(entry, NOW + 500)));

        let later = snapshot(entry, NOW + 1_000);
        assert!(!Arc::ptr_eq(&first, &later));

        lock(&entry.flow).push(trade(50.0), NOW);
        let added = snapshot(entry, NOW + 1_000);
        assert!(!Arc::ptr_eq(&later, &added));
        assert!(added.has_trades);
    }

    #[test]
    fn two_windows_on_two_assets_each_keep_their_feed_and_their_record() {
        let mut rig = Rig::new();
        let (btc, _) = rig.ask("BTC", NOW);
        let (eth, _) = rig.ask("ETH", NOW);
        lock(&btc).push(trade(1_000.0), NOW);
        // Both windows draw, frame after frame.
        for frame in 1..20 {
            let at = NOW + frame * 16;
            let (again_btc, _) = rig.ask("BTC", at);
            let (again_eth, _) = rig.ask("ETH", at);
            assert!(Arc::ptr_eq(&btc, &again_btc));
            assert!(Arc::ptr_eq(&eth, &again_eth));
        }
        assert_eq!(rig.started(), 2, "a feed was restarted by the other window");
        let snapshot = lock(&btc).snapshot(NOW);
        assert_eq!(snapshot.all.minute.sell, 1_000.0, "BTC kept its history");
        assert!(!lock(&eth).snapshot(NOW).has_trades);
    }

    #[test]
    fn a_late_write_of_an_old_feed_never_reaches_another_assets_record() {
        let mut rig = Rig::new();
        let (btc, _) = rig.ask("BTC", NOW);
        // Nobody asks about BTC for longer than a lease; ETH is asked, and BTC's feed is let go.
        let later = NOW + 10_000;
        let (eth, _) = rig.ask("ETH", later);
        assert!(!rig.inner.feeds.contains_key("BTC"));
        // A BTC socket still reading a frame writes into the record it was given.
        lock(&btc).push(trade(9_000.0), later);
        assert!(!lock(&eth).snapshot(later).has_trades);
        // And asking about BTC again starts over, with a record of its own.
        let (new_btc, _) = rig.ask("BTC", later + 100);
        assert!(!Arc::ptr_eq(&btc, &new_btc));
        assert!(!lock(&new_btc).snapshot(later + 100).has_trades);
        let entry = rig.inner.feeds.get_mut("BTC").unwrap();
        assert!(!snapshot(entry, later + 100).has_trades);
    }

    #[test]
    fn a_feed_that_stopped_is_replaced_with_a_fresh_record() {
        let mut rig = Rig::new();
        let (flow, alive) = rig.ask("BTC", NOW);
        lock(&flow).push(trade(5.0), NOW);
        alive.store(false, Ordering::Release);
        let (replaced, _) = rig.ask("BTC", NOW + 100);
        assert_eq!(rig.started(), 2);
        assert!(!Arc::ptr_eq(&flow, &replaced));
        assert!(!lock(&replaced).snapshot(NOW + 100).has_trades);
    }

    #[test]
    fn an_asset_a_window_is_still_drawing_is_never_let_go_by_another_window() {
        let mut rig = Rig::new();
        rig.ask("BTC", NOW);
        rig.ask("ETH", NOW + 2_000);
        // BTC was asked 2 s ago: well within the lease.
        assert!(rig.inner.feeds.contains_key("BTC"));
        rig.ask("ETH", NOW + 3_500);
        assert!(!rig.inner.feeds.contains_key("BTC"));
    }
}
