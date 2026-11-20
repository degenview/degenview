//! What a ladder draws: rows of price, folded from the book, the trades and the pulling and
//! stacking, around the market. A plain value; nothing in it is live.

use super::pull_stack::PullStack;
use super::{Book, Price, Side, Tape};
use crate::model::taker_flow::Taker;

/// What to compute a ladder for. Part of the viewer's choices, not of the market.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct View {
    /// The price span of a row; `None` picks one that fits the market.
    pub group: Option<Price>,
    /// The price the rows are centred on; `None` follows the middle of the spread.
    pub centre: Option<Price>,
    /// How many price rows the panel shows; a ladder has at least one.
    pub rows: usize,
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

/// The largest figures among the rows, for scaling their bars.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Scale {
    pub size: f64,
    pub traded: f64,
}

/// Where a ladder sits: the price span of a row, the price in its middle and how many rows.
pub(super) struct Window {
    pub group: Price,
    pub centre: Price,
    pub rows: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Ladder {
    /// The price span of a row.
    pub group: Price,
    /// Every row size on offer for the market, smallest first.
    pub group_choices: Vec<Price>,
    /// Highest price first, as many as the [`View`] asked for; empty while there is no book.
    pub rows: Vec<Row>,
    pub best_bid: Option<Price>,
    pub best_ask: Option<Price>,
    /// The row the last trade belongs to, when it is among the rows.
    last_trade_row: Option<usize>,
}

impl Ladder {
    /// A ladder with no rows, for a market whose book is not there to show.
    pub fn empty(group: Price) -> Self {
        Self {
            group,
            group_choices: Vec::new(),
            rows: Vec::new(),
            best_bid: None,
            best_ask: None,
            last_trade_row: None,
        }
    }

    /// The price in the middle of the rows.
    pub fn centre(&self) -> Option<Price> {
        self.rows.get(self.rows.len() / 2).map(|row| row.price)
    }

    /// The gap between the best bid and the best ask.
    pub fn spread(&self) -> Option<Price> {
        Some(self.best_ask? - self.best_bid?)
    }

    /// The index of the row the best bid rests in.
    pub fn best_bid_row(&self) -> Option<usize> {
        self.row_index(Side::Bid.row_of(self.best_bid?, self.group))
    }

    /// The index of the row the last trade belongs to.
    pub fn last_trade_row(&self) -> Option<usize> {
        self.last_trade_row
    }

    /// What the bars of the rows are scaled against: the largest bid or ask and the most traded
    /// among them.
    pub fn scale(&self) -> Scale {
        let largest = |size: fn(&Row) -> f64| self.rows.iter().map(size).fold(0.0, f64::max);
        Scale {
            size: largest(|row| row.bid.max(row.ask)),
            traded: largest(|row| row.bought + row.sold),
        }
    }

    /// Folds the book into the rows of `window`, each holding what [`Side::row_of`] places in it.
    /// A trade goes where its taker hit ([`Side::of_taker`]), and so do the pulls and stacks.
    pub(super) fn build(
        book: &Book,
        tape: &Tape,
        pull_stack: &PullStack,
        window: Window,
        group_choices: Vec<Price>,
        now_ms: i64,
    ) -> Self {
        let Window {
            group,
            centre,
            rows,
        } = window;
        let count = rows.max(1);
        let top = centre.floor_to(group) + group.times((count / 2) as i64);
        let bottom = top - group.times(count as i64 - 1);
        let mut ladder = Self {
            group,
            group_choices,
            rows: (0..count)
                .map(|index| Row {
                    price: top - group.times(index as i64),
                    ..Row::default()
                })
                .collect(),
            best_bid: book.best_bid(),
            best_ask: book.best_ask(),
            last_trade_row: None,
        };
        for (price, size) in book.bids_between(bottom, top + group) {
            if let Some(row) = ladder.row_mut(Side::Bid, price) {
                row.bid += size;
            }
        }
        for (price, size) in book.asks_between(bottom - group, top) {
            if let Some(row) = ladder.row_mut(Side::Ask, price) {
                row.ask += size;
            }
        }
        for (price, traded) in tape.traded() {
            if let Some(row) = ladder.row_mut(Side::of_taker(Taker::Buy), price) {
                row.bought += traded.bought;
            }
            if let Some(row) = ladder.row_mut(Side::of_taker(Taker::Sell), price) {
                row.sold += traded.sold;
            }
        }
        for ((side, price), change) in pull_stack.net(now_ms) {
            if let Some(row) = ladder.row_mut(side, price) {
                row.pull_stack += change;
            }
        }
        ladder.last_trade_row = tape.last().and_then(|trade| {
            ladder.row_index(Side::of_taker(trade.taker).row_of(trade.price, group))
        });
        ladder
    }

    /// The row a price on `side` belongs to, when it is among the rows.
    fn row_mut(&mut self, side: Side, price: Price) -> Option<&mut Row> {
        let index = self.row_index(side.row_of(price, self.group))?;
        self.rows.get_mut(index)
    }

    /// The index of the row at `price`, a multiple of the group.
    fn row_index(&self, price: Price) -> Option<usize> {
        let offset = self.rows.first()?.price - price;
        (offset >= Price::ZERO)
            .then(|| offset.steps_of(self.group) as usize)
            .filter(|index| *index < self.rows.len())
    }
}
