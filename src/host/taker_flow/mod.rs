//! The taker-flow feed's owner, shared by every window: it opens the trade sockets of the asset a
//! panel asks about while one is drawn, closes them when none is, and hands each frame the latest
//! [`Snapshot`] without blocking or recomputing it when nothing changed.

use crate::lock::lock;
use crate::model::taker_flow::{Flow, Snapshot};
use crate::service::now_ms;
use crate::service::taker_flow::{Lease, TakerFlowFeed};
use eframe::egui;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// How long the feed runs after a panel last asked: longer than a frame, so a frame that is late
/// does not close the sockets, and short enough that a tab in the background stops them.
const LEASE: Duration = Duration::from_secs(3);
/// The totals roll off as time passes, so the panel looks again this often even when quiet.
const REFRESH: Duration = Duration::from_secs(1);

/// A cheap handle: clones share one feed.
#[derive(Clone)]
pub struct TakerFlowHost {
    inner: Arc<Mutex<Inner>>,
}

struct Inner {
    flow: Arc<Mutex<Flow>>,
    lease: Lease,
    feed: Option<TakerFlowFeed>,
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
                flow: Arc::default(),
                lease: Lease::default(),
                feed: None,
                cached: None,
            })),
        }
    }
}

impl TakerFlowHost {
    /// Called by a panel each frame it is drawn: keeps the sockets of `asset` open (opening them,
    /// with an empty record, if they are not or if another asset's are) and returns what they
    /// have traded.
    pub fn watch(&self, ctx: &egui::Context, asset: &str) -> Arc<Snapshot> {
        let mut inner = lock(&self.inner);
        inner.lease.renew(LEASE);
        let wanted = inner
            .feed
            .as_ref()
            .is_some_and(|feed| feed.asset() == asset && feed.is_running());
        if !wanted {
            // The old feed's sockets close as it is dropped, before the record is cleared.
            inner.feed = None;
            lock(&inner.flow).clear();
            let repaint = {
                let ctx = ctx.clone();
                Arc::new(move || ctx.request_repaint())
            };
            let feed =
                TakerFlowFeed::start(asset, Arc::clone(&inner.flow), inner.lease.clone(), repaint);
            inner.feed = Some(feed);
        }
        ctx.request_repaint_after(REFRESH);
        snapshot(&mut inner, now_ms())
    }
}

/// The flow as of `now_ms`, reused while its revision and the second hold.
fn snapshot(inner: &mut Inner, now_ms: i64) -> Arc<Snapshot> {
    let flow = lock(&inner.flow);
    let (revision, second) = (flow.revision(), now_ms / 1_000);
    if let Some(cached) = &inner.cached
        && cached.revision == revision
        && cached.second == second
    {
        return Arc::clone(&cached.snapshot);
    }
    let snapshot = Arc::new(flow.snapshot(now_ms));
    drop(flow);
    inner.cached = Some(Cached {
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

    const NOW: i64 = 5_000_000_000;

    fn inner() -> Inner {
        Inner {
            flow: Arc::default(),
            lease: Lease::default(),
            feed: None,
            cached: None,
        }
    }

    #[test]
    fn a_snapshot_is_reused_until_the_flow_or_the_second_changes() {
        let mut inner = inner();
        let first = snapshot(&mut inner, NOW);
        assert!(Arc::ptr_eq(&first, &snapshot(&mut inner, NOW + 500)));

        let later = snapshot(&mut inner, NOW + 1_000);
        assert!(!Arc::ptr_eq(&first, &later));

        lock(&inner.flow).push(
            Trade {
                venue: Venue {
                    exchange: Exchange::Bybit,
                    market: Market::Spot,
                },
                notional: 50.0,
                taker: Taker::Sell,
                time_ms: NOW,
            },
            NOW,
        );
        let added = snapshot(&mut inner, NOW + 1_000);
        assert!(!Arc::ptr_eq(&later, &added));
        assert!(added.has_trades);
    }
}
