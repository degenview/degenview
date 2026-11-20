//! What a panel draws of one market's depth: whether the book is there, its ladder and its tape.

use super::{Ladder, Price, TapeView};

/// Whether a book is there to show.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Link {
    /// Not connected yet, or waiting for the book to arrive.
    Connecting,
    Live,
    /// The connection dropped; the book was let go rather than shown out of date.
    Reconnecting,
    /// The exchange does not list the market.
    Unlisted,
}

/// One market's depth as of a moment: a plain value, nothing in it is live.
#[derive(Clone, Debug, PartialEq)]
pub struct DepthSnapshot {
    pub link: Link,
    /// The market's smallest price step; zero until it is known.
    pub tick: Price,
    pub ladder: Ladder,
    pub tape: TapeView,
}

impl DepthSnapshot {
    /// A snapshot with nothing to show yet: no rows and no trades.
    pub fn waiting(link: Link, tick: Option<Price>) -> Self {
        let tick = tick.unwrap_or_default();
        Self {
            link,
            tick,
            ladder: Ladder::empty(tick),
            tape: TapeView::default(),
        }
    }
}
