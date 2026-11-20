//! Keeping a [`Book`] right as an exchange's updates arrive. Each exchange has its own rules for
//! what makes a stream of updates complete; one [`BookSync`] holds them, and nothing else in the
//! model knows which exchange it is following.

mod binance;
mod bybit;
#[cfg(test)]
mod tests;

use super::{Book, BookEvent};
use crate::model::exchange::Exchange;

/// What an update did to the book.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SyncOutcome {
    /// The book changed.
    Applied,
    /// The update was older than the book, or came before it could be used: ignored.
    Stale,
    /// The stream skipped something, or never started from a snapshot: the book has been emptied
    /// and cannot be trusted again until a snapshot arrives.
    NeedsSnapshot,
}

/// One exchange's rules for building a book from its updates.
pub trait BookSync: Send {
    /// Applies `event` to `book` if the rules allow it.
    fn apply(&mut self, book: &mut Book, event: BookEvent) -> SyncOutcome;

    /// Whether the book is built on a snapshot, so that it may be shown.
    fn is_synced(&self) -> bool;

    /// The connection dropped: whatever the book held no longer follows the stream.
    fn reset(&mut self, book: &mut Book);
}

/// The rules of `exchange`.
pub fn for_exchange(exchange: Exchange) -> Box<dyn BookSync> {
    match exchange {
        Exchange::Binance => Box::new(binance::DiffStream::default()),
        Exchange::Bybit => Box::new(bybit::SnapshotAndDeltas::default()),
    }
}
