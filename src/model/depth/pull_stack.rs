//! Pulling and stacking: how much resting size appeared at a price (stacked) or disappeared from
//! it without trading (pulled) over the last few seconds. Size that left because a trade took it
//! is not a pull, so each trade is counted back in.

use super::{Price, Side};
use std::collections::{HashMap, VecDeque};

/// How far back the figures reach, in seconds.
pub const WINDOW_SECONDS: i64 = 10;

type Changes = HashMap<(Side, Price), f64>;

#[derive(Debug, Default)]
pub struct PullStack {
    /// Per second, newest last: the change in resting size, less what traded there.
    seconds: VecDeque<(i64, Changes)>,
}

impl PullStack {
    /// The resting size at `price` went from `before` to `after`.
    pub fn changed(&mut self, side: Side, price: Price, before: f64, after: f64, now_ms: i64) {
        self.add(side, price, after - before, now_ms);
    }

    /// `size` traded at `price`, taking it from the `side` that rested there: that is no pull.
    pub fn traded(&mut self, side: Side, price: Price, size: f64, now_ms: i64) {
        self.add(side, price, size, now_ms);
    }

    /// Forgets everything: the book it described is gone.
    pub fn clear(&mut self) {
        self.seconds.clear();
    }

    /// The net change at each price over the last [`WINDOW_SECONDS`]: positive is stacked,
    /// negative pulled.
    pub fn net(&self, now_ms: i64) -> Changes {
        let oldest = now_ms.div_euclid(1_000) - WINDOW_SECONDS;
        let mut net = Changes::new();
        for (_, changes) in self.seconds.iter().filter(|(second, _)| *second > oldest) {
            for (&key, &change) in changes {
                *net.entry(key).or_default() += change;
            }
        }
        net.retain(|_, change| change.abs() > f64::EPSILON);
        net
    }

    fn add(&mut self, side: Side, price: Price, change: f64, now_ms: i64) {
        let second = now_ms.div_euclid(1_000);
        while self
            .seconds
            .front()
            .is_some_and(|(oldest, _)| *oldest <= second - WINDOW_SECONDS)
        {
            self.seconds.pop_front();
        }
        if self
            .seconds
            .back()
            .is_none_or(|(latest, _)| *latest != second)
        {
            self.seconds.push_back((second, Changes::new()));
        }
        if let Some((_, changes)) = self.seconds.back_mut() {
            *changes.entry((side, price)).or_default() += change;
        }
    }
}
