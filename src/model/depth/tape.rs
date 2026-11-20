//! The trades of one market: the latest few for a tape, and how much traded at each price since
//! the panel opened.

use super::Price;
use crate::model::taker_flow::Taker;
use std::collections::{HashMap, VecDeque};

/// How many trades the tape lists.
pub const TAPE_ROWS: usize = 300;
/// How many distinct prices keep their traded volume: those nearest the market.
pub const PRICES_KEPT: usize = 2_000;

/// One trade.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Trade {
    /// Exchange time, in milliseconds since the epoch.
    pub time_ms: i64,
    pub price: Price,
    /// In the base coin.
    pub size: f64,
    pub taker: Taker,
}

impl Trade {
    /// What it was worth, in the quote currency.
    pub fn notional(&self) -> f64 {
        self.price.as_f64() * self.size
    }
}

/// What was bought and sold at one price, in the base coin.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Traded {
    pub bought: f64,
    pub sold: f64,
}

/// What the tape of trades shows.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TapeView {
    /// The latest trades, newest first.
    pub trades: Vec<Trade>,
    /// When the first trade counted in the ladder's rows happened.
    pub since_ms: Option<i64>,
}

impl TapeView {
    pub(super) fn of(tape: &Tape) -> Self {
        Self {
            trades: tape.recent().copied().collect(),
            since_ms: tape.since_ms(),
        }
    }

    pub fn last_trade(&self) -> Option<&Trade> {
        self.trades.first()
    }
}

#[derive(Debug, Default)]
pub struct Tape {
    recent: VecDeque<Trade>,
    traded: HashMap<Price, Traded>,
    since_ms: Option<i64>,
}

impl Tape {
    pub fn push(&mut self, trade: Trade) {
        self.since_ms.get_or_insert(trade.time_ms);
        self.recent.push_front(trade);
        self.recent.truncate(TAPE_ROWS);
        let traded = self.traded.entry(trade.price).or_default();
        match trade.taker {
            Taker::Buy => traded.bought += trade.size,
            Taker::Sell => traded.sold += trade.size,
        }
    }

    /// The latest trades, newest first.
    pub fn recent(&self) -> impl ExactSizeIterator<Item = &Trade> {
        self.recent.iter()
    }

    pub fn last(&self) -> Option<&Trade> {
        self.recent.front()
    }

    /// Everything traded, by price.
    pub fn traded(&self) -> impl Iterator<Item = (Price, Traded)> + '_ {
        self.traded.iter().map(|(price, traded)| (*price, *traded))
    }

    /// When the first trade counted here happened.
    pub fn since_ms(&self) -> Option<i64> {
        self.since_ms
    }

    /// Lets go of the prices farthest from `mid` once there are twice as many as are kept, so the
    /// map stays bounded while the market wanders.
    pub fn prune(&mut self, mid: Price) {
        if self.traded.len() <= 2 * PRICES_KEPT {
            return;
        }
        let mut by_distance: Vec<Price> = self.traded.keys().copied().collect();
        by_distance.sort_by_key(|price| (price.units() - mid.units()).abs());
        for price in by_distance.split_off(PRICES_KEPT) {
            self.traded.remove(&price);
        }
    }
}
