//! What an exchange's socket says, in the terms the model understands. A parser turns a message
//! into these; nothing downstream knows the exchange's wire format.

use super::{Delta, Snapshot, Trade};

/// A change to the book.
#[derive(Clone, Debug, PartialEq)]
pub enum BookEvent {
    /// The whole book, replacing whatever was held.
    Snapshot(Snapshot),
    /// Changes to apply to the book held.
    Delta(Delta),
}

/// One message's worth of news.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    Book(BookEvent),
    Trade(Trade),
}
