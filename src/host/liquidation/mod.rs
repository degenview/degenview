//! The liquidation feed's owner, shared by every window: it starts the exchanges' sockets while a
//! panel is drawn and closes them when none is, and hands each frame the latest [`Snapshot`]
//! without blocking or recomputing it when nothing changed.

use crate::clock::now_ms;
use crate::host::repaint_of;
use crate::lock::lock;
use crate::model::liquidation::{MinSize, Snapshot, Tape};
use crate::service::liquidation::{Lease, LiquidationFeed};
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
pub struct LiquidationHost {
    inner: Arc<Mutex<Inner>>,
}

struct Inner {
    tape: Arc<Mutex<Tape>>,
    lease: Lease,
    feed: Option<LiquidationFeed>,
    cached: Option<Cached>,
}

/// A snapshot and what it was made for.
struct Cached {
    revision: u64,
    minimum: MinSize,
    second: i64,
    snapshot: Arc<Snapshot>,
}

impl Default for LiquidationHost {
    fn default() -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner {
                tape: Arc::default(),
                lease: Lease::default(),
                feed: None,
                cached: None,
            })),
        }
    }
}

impl LiquidationHost {
    /// Called by a panel each frame it is drawn: keeps the sockets open (opening them, with an
    /// empty tape, if they are not) and returns what the tape holds of at least `minimum`.
    pub fn watch(&self, ctx: &egui::Context, minimum: MinSize) -> Arc<Snapshot> {
        let mut inner = lock(&self.inner);
        inner.lease.renew(LEASE);
        if inner.feed.as_ref().is_none_or(|feed| !feed.is_running()) {
            lock(&inner.tape).clear();
            let feed = LiquidationFeed::start(
                Arc::clone(&inner.tape),
                inner.lease.clone(),
                repaint_of(ctx),
            );
            inner.feed = Some(feed);
        }
        ctx.request_repaint_after(REFRESH);
        snapshot(&mut inner, now_ms(), minimum)
    }
}

/// The tape as of `now_ms`, reused while its revision, the size asked for and the second hold.
fn snapshot(inner: &mut Inner, now_ms: i64, minimum: MinSize) -> Arc<Snapshot> {
    let tape = lock(&inner.tape);
    let (revision, second) = (tape.revision(), now_ms / 1_000);
    if let Some(cached) = &inner.cached
        && cached.revision == revision
        && cached.minimum == minimum
        && cached.second == second
    {
        return Arc::clone(&cached.snapshot);
    }
    let snapshot = Arc::new(tape.snapshot(now_ms, minimum));
    drop(tape);
    inner.cached = Some(Cached {
        revision,
        minimum,
        second,
        snapshot: Arc::clone(&snapshot),
    });
    snapshot
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::exchange::Exchange;
    use crate::model::liquidation::{Liquidation, Side};

    const NOW: i64 = 5_000_000_000;

    fn liquidation(notional: f64) -> Liquidation {
        Liquidation {
            exchange: Exchange::Bybit,
            symbol: "BTCUSDT".to_owned(),
            side: Side::Short,
            price: 1.0,
            quantity: notional,
            time_ms: NOW - 1_000,
        }
    }

    fn inner() -> Inner {
        Inner {
            tape: Arc::default(),
            lease: Lease::default(),
            feed: None,
            cached: None,
        }
    }

    #[test]
    fn a_snapshot_is_reused_until_the_tape_the_size_or_the_second_changes() {
        let mut inner = inner();
        let first = snapshot(&mut inner, NOW, MinSize::Any);
        assert!(Arc::ptr_eq(
            &first,
            &snapshot(&mut inner, NOW + 500, MinSize::Any)
        ));

        let size = snapshot(&mut inner, NOW, MinSize::Thousand);
        assert!(!Arc::ptr_eq(&first, &size));

        let later = snapshot(&mut inner, NOW + 1_000, MinSize::Thousand);
        assert!(!Arc::ptr_eq(&size, &later));

        lock(&inner.tape).push(liquidation(5_000.0), NOW);
        let added = snapshot(&mut inner, NOW + 1_000, MinSize::Thousand);
        assert!(!Arc::ptr_eq(&later, &added));
        assert_eq!(added.latest.len(), 1);
    }
}
