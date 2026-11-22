//! The order book and trades of one market, fed into a [`DepthRecord`] while somebody is looking.
//!
//! One thread (`service::stream`) on the lease the viewer renews each frame it draws. It asks the
//! exchange for the market's tick size, opens the socket, and gives everything the socket says to
//! the record, which knows what it means. What differs between exchanges is behind
//! [`DepthVenue`]; nothing here knows which one it is.

mod binance;
mod bybit;
mod error;
mod levels;
#[cfg(test)]
mod tests;
mod venue;

pub use crate::service::stream::{Lease, Repaint};
pub use error::DepthError;
pub use venue::DepthVenue;

use crate::clock::now_ms;
use crate::lock::lock;
use crate::model::depth::{BookEvent, DepthMarket, DepthRecord, Event, Price, SyncOutcome};
use crate::service::stream::{Stream, Worker};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// How long a look-up that failed waits before the next try.
const RETRY_AFTER: Duration = Duration::from_secs(10);
/// How long a book snapshot that failed waits: the book is empty until it comes.
const SNAPSHOT_RETRY_AFTER: Duration = Duration::from_secs(2);

/// An empty record of `market`, judging its exchange's book updates by that exchange's rules.
pub fn new_record(market: &DepthMarket) -> DepthRecord {
    DepthRecord::new(venue::for_exchange(market.venue.exchange).book_sync())
}

/// The running socket of one market; dropping it closes it.
pub struct DepthFeed {
    stream: Stream,
}

impl DepthFeed {
    /// Starts following `market`, recording what its exchange says in `record`, which should come
    /// from [`new_record`].
    pub fn start(
        market: DepthMarket,
        record: Arc<Mutex<DepthRecord>>,
        lease: Lease,
        repaint: Repaint,
    ) -> Self {
        let venue = venue::for_exchange(market.venue.exchange);
        let stream = Stream::new(lease, repaint);
        stream.spawn(move |worker| {
            Follower {
                worker,
                venue: venue.as_ref(),
                symbol: &market.symbol,
                record: &record,
            }
            .run();
        });
        Self { stream }
    }

    /// Whether the feed still runs. It ends once the lease lapses, and then a new one is needed.
    pub fn is_running(&self) -> bool {
        self.stream.is_running()
    }
}

/// What one feed's thread works on: a market of a venue, and the record its news goes into.
struct Follower<'a> {
    worker: &'a Worker,
    venue: &'a dyn DepthVenue,
    symbol: &'a str,
    record: &'a Mutex<DepthRecord>,
}

impl Follower<'_> {
    /// Learns the tick size, then follows the socket until the feed is shut down.
    fn run(&self) {
        let Some(tick) = self.worker.retry(RETRY_AFTER, || self.learn_tick()) else {
            return;
        };
        lock(self.record).set_tick(tick);
        self.worker.wake();
        let session = self.venue.session(self.symbol);
        self.worker.run_linked(
            &session,
            |up| {
                lock(self.record).link(up);
                if up {
                    self.start_book();
                }
                self.worker.wake();
            },
            |text| {
                if self.take(text) {
                    self.worker.wake();
                }
                true
            },
        );
    }

    /// The market's tick size, or `None` to try again; a market the exchange does not list says
    /// so on the record meanwhile.
    fn learn_tick(&self) -> Option<Price> {
        match self.venue.tick_size(self.symbol) {
            Ok(tick) => Some(tick),
            Err(DepthError::UnknownSymbol(_)) => {
                lock(self.record).mark_unlisted();
                None
            }
            Err(_) => None,
        }
    }

    /// Gives the record what one message says; whether it changed anything. A stream that skipped
    /// something is started again from a snapshot.
    fn take(&self, text: &str) -> bool {
        let mut changed = false;
        for event in self.venue.parse(text) {
            let outcome = lock(self.record).apply(event, now_ms());
            changed |= outcome != SyncOutcome::Stale;
            if outcome == SyncOutcome::NeedsSnapshot {
                self.start_book();
            }
        }
        changed
    }

    /// Fetches the book the socket's updates are to be applied to, when the record says it needs
    /// one, trying until it comes. The socket's own updates wait in its buffer meanwhile; it must
    /// not be given up on, because that would end the feed.
    fn start_book(&self) {
        if !lock(self.record).needs_snapshot() {
            return;
        }
        let fetched = self.worker.retry(SNAPSHOT_RETRY_AFTER, || {
            self.venue.snapshot(self.symbol).ok()
        });
        if let Some(snapshot) = fetched {
            let event = Event::Book(BookEvent::Snapshot(snapshot));
            lock(self.record).apply(event, now_ms());
        }
    }
}
