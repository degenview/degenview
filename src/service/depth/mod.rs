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
use crate::service::stream::{Stream, Worker, retry_until_shutdown};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// How long a look-up that failed waits before the next try.
const RETRY_AFTER: Duration = Duration::from_secs(10);
/// How long a book snapshot that failed waits: the book is empty until it comes.
const SNAPSHOT_RETRY_AFTER: Duration = Duration::from_secs(2);

/// The running socket of one market; dropping it closes it.
pub struct DepthFeed {
    stream: Stream,
}

impl DepthFeed {
    /// Starts following `market`, recording what its exchange says in `record`.
    pub fn start(
        market: DepthMarket,
        record: Arc<Mutex<DepthRecord>>,
        lease: Lease,
        repaint: Repaint,
    ) -> Self {
        let venue = venue::for_exchange(market.venue.exchange);
        let stream = Stream::new(lease, repaint);
        stream.spawn(move |worker| follow(worker, venue.as_ref(), &market.symbol, &record));
        Self { stream }
    }

    /// Whether the feed still runs. It ends once the lease lapses, and then a new one is needed.
    pub fn is_running(&self) -> bool {
        self.stream.is_running()
    }
}

fn follow(worker: &Worker, venue: &dyn DepthVenue, symbol: &str, record: &Mutex<DepthRecord>) {
    let shutdown = worker.shutdown();
    let Some(tick) =
        retry_until_shutdown(shutdown, RETRY_AFTER, || learn_tick(venue, symbol, record))
    else {
        return;
    };
    lock(record).set_tick(tick);
    worker.wake();
    let session = venue.session(symbol);
    worker.run_linked(
        &session,
        |up| {
            lock(record).link(up);
            if up {
                start_book(shutdown, venue, symbol, record);
            }
            worker.wake();
        },
        |text| {
            if take(shutdown, venue, symbol, record, text) {
                worker.wake();
            }
            true
        },
    );
}

/// The market's tick size, or `None` to try again; a market the exchange does not list says so
/// on the record meanwhile.
fn learn_tick(venue: &dyn DepthVenue, symbol: &str, record: &Mutex<DepthRecord>) -> Option<Price> {
    match venue.tick_size(symbol) {
        Ok(tick) => Some(tick),
        Err(DepthError::UnknownSymbol(_)) => {
            lock(record).mark_unlisted();
            None
        }
        Err(_) => None,
    }
}

/// Gives the record what one message says; whether it changed anything. A stream that skipped
/// something is started again from a snapshot.
fn take(
    shutdown: &AtomicBool,
    venue: &dyn DepthVenue,
    symbol: &str,
    record: &Mutex<DepthRecord>,
    text: &str,
) -> bool {
    let mut changed = false;
    for event in venue.parse(text) {
        let outcome = lock(record).apply(event, now_ms());
        changed |= outcome != SyncOutcome::Stale;
        if outcome == SyncOutcome::NeedsSnapshot {
            start_book(shutdown, venue, symbol, record);
        }
    }
    changed
}

/// Fetches the book the socket's updates are to be applied to, when the exchange does not send
/// one itself, trying until it comes. The socket's own updates wait in its buffer meanwhile; it
/// must not be given up on, because that would end the feed.
fn start_book(
    shutdown: &AtomicBool,
    venue: &dyn DepthVenue,
    symbol: &str,
    record: &Mutex<DepthRecord>,
) {
    let fetched = retry_until_shutdown(shutdown, SNAPSHOT_RETRY_AFTER, || {
        match venue.snapshot(symbol) {
            None => Some(None),
            Some(result) => result.ok().map(Some),
        }
    });
    if let Some(Some(snapshot)) = fetched {
        let event = Event::Book(BookEvent::Snapshot(snapshot));
        lock(record).apply(event, now_ms());
    }
}
