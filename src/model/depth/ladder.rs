//! What a ladder draws: rows of price, folded from the book, the trades and the pulling and
//! stacking, for a window around the market. A plain value; nothing in it is live.

use super::pull_stack::PullStack;
use super::{Book, Price, Side, Tape, Trade};
use std::collections::HashMap;

/// How many rows a ladder holds: enough for the tallest panel, [`CENTRE_ROW`] being the middle.
pub const WINDOW_ROWS: usize = 201;
pub const CENTRE_ROW: usize = WINDOW_ROWS / 2;

/// What to compute a ladder for. Part of the viewer's choices, not of the market.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct View {
    /// The price span of a row; `None` picks one that fits the market.
    pub group: Option<Price>,
    /// The price the window is centred on; `None` follows the middle of the spread.
    pub centre: Option<Price>,
}

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

/// One price of the ladder, with everything at it, in the base coin.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Row {
    pub price: Price,
    pub bid: f64,
    pub ask: f64,
    pub bought: f64,
    pub sold: f64,
    /// Size stacked (positive) or pulled (negative) here over the last few seconds.
    pub pull_stack: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Ladder {
    pub link: Link,
    /// The market's smallest price step; zero until it is known.
    pub tick: Price,
    /// The price span of a row.
    pub group: Price,
    /// Highest price first; empty unless the link is live.
    pub rows: Vec<Row>,
    pub best_bid: Option<Price>,
    pub best_ask: Option<Price>,
    pub last_trade: Option<Trade>,
    /// The latest trades, newest first.
    pub trades: Vec<Trade>,
    /// When the first trade counted in the rows happened.
    pub since_ms: Option<i64>,
}

/// The largest figures among the rows on show, for scaling their bars.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Scale {
    pub size: f64,
    pub traded: f64,
}

impl Ladder {
    /// A ladder with nothing to show yet.
    pub fn waiting(link: Link, tick: Option<Price>) -> Self {
        Self {
            link,
            tick: tick.unwrap_or_default(),
            group: tick.unwrap_or_default(),
            rows: Vec::new(),
            best_bid: None,
            best_ask: None,
            last_trade: None,
            trades: Vec::new(),
            since_ms: None,
        }
    }

    /// The price in the middle of the window.
    pub fn centre(&self) -> Option<Price> {
        self.rows.get(CENTRE_ROW).map(|row| row.price)
    }

    /// The gap between the best bid and the best ask.
    pub fn spread(&self) -> Option<Price> {
        Some(self.best_ask? - self.best_bid?)
    }

    /// What the bars of the rows in `shown` are scaled against: the largest bid or ask and the most
    /// traded among those rows only, so the rows on screen fill their columns whatever lies
    /// beyond them in the window.
    pub fn scale(&self, shown: std::ops::Range<usize>) -> Scale {
        let rows = self.rows.get(shown).unwrap_or_default();
        let largest = |size: fn(&Row) -> f64| rows.iter().map(size).fold(0.0, f64::max);
        Scale {
            size: largest(|row| row.bid.max(row.ask)),
            traded: largest(|row| row.bought + row.sold),
        }
    }

    /// Folds the book into rows of `group` around `centre`: a bid rests in the row at or below
    /// its price and an ask in the one at or above, so a row is either bid or offered and never
    /// both. A trade goes where its taker hit: a buyer lifts an ask, a seller hits a bid.
    pub(super) fn build(
        book: &Book,
        tape: &Tape,
        pull_stack: &PullStack,
        tick: Price,
        group: Price,
        centre: Price,
        now_ms: i64,
    ) -> Self {
        let top = centre.floor_to(group) + group.times(CENTRE_ROW as i64);
        let price_of = |index: usize| top - group.times(index as i64);
        let mut rows: Vec<Row> = (0..WINDOW_ROWS)
            .map(|index| {
                let price = price_of(index);
                Row {
                    price,
                    bid: book
                        .bids_between(price, price + group)
                        .map(|(_, size)| size)
                        .sum(),
                    ask: book
                        .asks_between(price - group, price)
                        .map(|(_, size)| size)
                        .sum(),
                    ..Row::default()
                }
            })
            .collect();
        let index_of = |price: Price| -> Option<usize> {
            let offset = top - price;
            (offset >= Price::ZERO)
                .then(|| offset.steps_of(group) as usize)
                .filter(|index| *index < WINDOW_ROWS)
        };
        for (price, traded) in tape.traded() {
            if let Some(index) = index_of(price.ceil_to(group)) {
                rows[index].bought += traded.bought;
            }
            if let Some(index) = index_of(price.floor_to(group)) {
                rows[index].sold += traded.sold;
            }
        }
        let mut net: HashMap<usize, f64> = HashMap::new();
        for ((side, price), change) in pull_stack.net(now_ms) {
            let row = match side {
                Side::Bid => price.floor_to(group),
                Side::Ask => price.ceil_to(group),
            };
            if let Some(index) = index_of(row) {
                *net.entry(index).or_default() += change;
            }
        }
        for (index, change) in net {
            rows[index].pull_stack = change;
        }
        Self {
            link: Link::Live,
            tick,
            group,
            best_bid: book.best_bid(),
            best_ask: book.best_ask(),
            last_trade: tape.last().copied(),
            trades: tape.recent().copied().collect(),
            since_ms: tape.since_ms(),
            rows,
        }
    }
}
