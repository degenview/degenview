//! The resting orders of one market: how much is bid and offered at each price.

use super::Price;
use std::collections::BTreeMap;

/// The most levels kept on each side, nearest the market first. Binance sends 5,000 a side, far
/// more than a ladder shows.
pub const MAX_LEVELS: usize = 2_000;

/// A side of the book.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Side {
    Bid,
    Ask,
}

/// Sizes at prices, in the base coin; a size of zero means the level is gone.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Levels {
    pub bids: Vec<(Price, f64)>,
    pub asks: Vec<(Price, f64)>,
}

/// The whole book as of an update id.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Snapshot {
    pub id: u64,
    pub levels: Levels,
}

/// Changes to the book, covering the update ids `first..=last`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Delta {
    pub first: u64,
    pub last: u64,
    pub levels: Levels,
}

/// A level whose resting size changed, as the book placed an update.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Change {
    pub side: Side,
    pub price: Price,
    pub before: f64,
    pub after: f64,
}

/// Both sides of the book. Prices are exact keys, so the best bid is the last of one map and the
/// best ask the first of the other.
#[derive(Clone, Debug, Default)]
pub struct Book {
    bids: BTreeMap<Price, f64>,
    asks: BTreeMap<Price, f64>,
}

impl Book {
    pub fn clear(&mut self) {
        self.bids.clear();
        self.asks.clear();
    }

    /// Replaces everything with `levels`.
    pub fn replace(&mut self, levels: &Levels) {
        self.clear();
        self.place(levels, |_| {});
    }

    /// Sets each level to its new size, removing those that reach zero, then lets go of the levels
    /// beyond [`MAX_LEVELS`]. Returns the levels whose size changed, in the order placed.
    pub fn apply(&mut self, levels: &Levels) -> Vec<Change> {
        let mut changes = Vec::new();
        self.place(levels, |change| changes.push(change));
        changes
    }

    /// Places `levels` and hands each level whose size changed to `changed`.
    fn place(&mut self, levels: &Levels, mut changed: impl FnMut(Change)) {
        for (side, map, rows) in [
            (Side::Bid, &mut self.bids, &levels.bids),
            (Side::Ask, &mut self.asks, &levels.asks),
        ] {
            for &(price, after) in rows {
                let before = set(map, price, after);
                if before != after {
                    changed(Change {
                        side,
                        price,
                        before,
                        after,
                    });
                }
            }
        }
        while self.bids.len() > MAX_LEVELS {
            self.bids.pop_first();
        }
        while self.asks.len() > MAX_LEVELS {
            self.asks.pop_last();
        }
    }

    pub fn best_bid(&self) -> Option<Price> {
        self.bids.last_key_value().map(|(price, _)| *price)
    }

    pub fn best_ask(&self) -> Option<Price> {
        self.asks.first_key_value().map(|(price, _)| *price)
    }

    /// Halfway between the best bid and the best ask.
    pub fn mid(&self) -> Option<Price> {
        let (bid, ask) = (self.best_bid()?, self.best_ask()?);
        Some(Price::from_units((bid.units() + ask.units()) / 2))
    }

    /// The bids from `low` up to but not including `high`: the levels a row of that span folds.
    pub fn bids_between(&self, low: Price, high: Price) -> impl Iterator<Item = (Price, f64)> + '_ {
        self.bids
            .range(low..high)
            .map(|(price, size)| (*price, *size))
    }

    /// The asks above `low` up to and including `high`.
    pub fn asks_between(&self, low: Price, high: Price) -> impl Iterator<Item = (Price, f64)> + '_ {
        use std::ops::Bound::{Excluded, Included};
        self.asks
            .range((Excluded(low), Included(high)))
            .map(|(price, size)| (*price, *size))
    }
}

/// Sets the level at `price` to `size` (none when it is zero); the size it had, zero if none.
fn set(side: &mut BTreeMap<Price, f64>, price: Price, size: f64) -> f64 {
    let before = if size > 0.0 {
        side.insert(price, size)
    } else {
        side.remove(&price)
    };
    before.unwrap_or(0.0)
}
