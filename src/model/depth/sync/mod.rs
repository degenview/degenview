//! Judging an exchange's book updates. Each exchange has its own rules for what makes a stream of
//! updates complete; one [`BookSync`] holds them, and nothing else in the model knows which
//! exchange it is following. A sync only judges: the [`DepthRecord`](super::DepthRecord) places
//! the levels in the book.

mod binance;
mod bybit;
#[cfg(test)]
mod tests;

pub use binance::DiffStream;
pub use bybit::SnapshotAndDeltas;

use super::BookEvent;

/// What to do with an update, as judged by a [`BookSync`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Verdict {
    /// Replace the book with the update's levels.
    Replace,
    /// Place the update's levels in the book.
    Apply,
    /// The update is older than the book, or came before it could be used: ignore it.
    Stale,
    /// The stream skipped something, or never started from a snapshot: empty the book, which
    /// cannot be trusted again until a snapshot arrives.
    Gap,
}

/// One exchange's rules for building a book from its updates.
pub trait BookSync: Send {
    /// What to do with `event`, given what came before; updates only the sync's own state.
    fn judge(&mut self, event: &BookEvent) -> Verdict;

    /// Whether the book must be started from a fetched snapshot before updates can be placed.
    /// False for an exchange whose socket sends its own snapshots.
    fn needs_snapshot(&self) -> bool;

    /// Whether the book is built on a snapshot, so that it may be shown.
    fn is_synced(&self) -> bool;

    /// The connection dropped: whatever the book held no longer follows the stream.
    fn reset(&mut self);
}
