//! The taker-flow feeds' owner, shared by every window: it opens the trade sockets of each asset a
//! panel asks about while one is drawn, closes them when none is, and hands each frame the latest
//! [`Snapshot`] without blocking or recomputing it when nothing changed. The feeds' lifetimes are
//! a [`LeaseMap`], one entry per asset.

use crate::clock::now_ms;
use crate::host::lease_map::{LeaseMap, Running};
use crate::host::memo::Memo;
use crate::host::repaint_of;
use crate::lock::lock;
use crate::model::taker_flow::{Flow, Snapshot};
use crate::service::taker_flow::TakerFlowFeed;
use eframe::egui;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// The totals roll off as time passes, so the panel looks again this often even when quiet.
const REFRESH: Duration = Duration::from_secs(1);

/// What a snapshot was made for: the flow's revision and the second.
type Made = (u64, i64);

type Feeds<F> = LeaseMap<String, Flow, F, Memo<Made, Snapshot>>;

/// A cheap handle: clones share the feeds.
#[derive(Clone)]
pub struct TakerFlowHost {
    feeds: Arc<Mutex<Feeds<TakerFlowFeed>>>,
}

impl Running for TakerFlowFeed {
    fn is_running(&self) -> bool {
        TakerFlowFeed::is_running(self)
    }
}

impl Default for TakerFlowHost {
    fn default() -> Self {
        Self {
            feeds: Arc::new(Mutex::new(LeaseMap::default())),
        }
    }
}

impl TakerFlowHost {
    /// Called by a panel each frame it is drawn: keeps the sockets of `asset` open (opening them,
    /// with an empty record, if they are not) and returns what they have traded.
    pub fn watch(&self, ctx: &egui::Context, asset: &str) -> Arc<Snapshot> {
        let mut feeds = lock(&self.feeds);
        let now = now_ms();
        let entry = feeds.entry_for(&asset.to_owned(), now, Flow::default, |flow, lease| {
            TakerFlowFeed::start(asset, Arc::clone(flow), lease.clone(), repaint_of(ctx))
        });
        ctx.request_repaint_after(REFRESH);
        snapshot(&entry.record, &mut entry.cache, now)
    }
}

/// The flow as of `now_ms`, reused while its revision and the second hold.
fn snapshot(record: &Mutex<Flow>, cache: &mut Memo<Made, Snapshot>, now_ms: i64) -> Arc<Snapshot> {
    let flow = lock(record);
    cache.get_or((flow.revision(), now_ms / 1_000), || flow.snapshot(now_ms))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::exchange::{Exchange, Market, Venue};
    use crate::model::taker_flow::{Taker, Trade};

    const NOW: i64 = 5_000_000_000;

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
        let flow = Mutex::new(Flow::default());
        let mut cache = Memo::default();
        let first = snapshot(&flow, &mut cache, NOW);
        assert!(Arc::ptr_eq(&first, &snapshot(&flow, &mut cache, NOW + 500)));

        let later = snapshot(&flow, &mut cache, NOW + 1_000);
        assert!(!Arc::ptr_eq(&first, &later));

        lock(&flow).push(trade(50.0), NOW);
        let added = snapshot(&flow, &mut cache, NOW + 1_000);
        assert!(!Arc::ptr_eq(&later, &added));
        assert!(added.has_trades);
    }

    #[test]
    fn each_asset_is_snapshotted_from_its_own_flow() {
        let (btc, eth) = (Mutex::new(Flow::default()), Mutex::new(Flow::default()));
        lock(&btc).push(trade(1_000.0), NOW);
        let (mut btc_cache, mut eth_cache) = (Memo::default(), Memo::default());
        let seen = snapshot(&btc, &mut btc_cache, NOW);
        assert_eq!(seen.all.minute.sell, 1_000.0);
        assert!(!snapshot(&eth, &mut eth_cache, NOW).has_trades);
    }
}
