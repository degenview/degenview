//! Taker flow: who crossed the spread. Every trade has a taker, the side that hit the book, and the
//! volume the buyers took minus the volume the sellers took says which way the aggression leaned.
//! Pure: the trade sockets that fill it are in `service::taker_flow`.

use crate::model::DataSource;
use crate::model::liquidation::Exchange;
use std::collections::VecDeque;

#[cfg(test)]
mod tests;

/// The longest span the totals reach back.
pub const HOUR_MS: i64 = 3_600_000;
/// How many one-minute bars of net flow a snapshot holds.
pub const MINUTES: usize = 30;

/// Spot or the perpetual contract of the same asset.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Market {
    Spot,
    Perpetual,
}

/// One market of an asset on one exchange.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Venue {
    pub exchange: Exchange,
    pub market: Market,
}

impl Venue {
    /// Every venue, in the order a panel lists them.
    pub const ALL: [Self; 4] = [
        Self::new(Exchange::Binance, Market::Spot),
        Self::new(Exchange::Binance, Market::Perpetual),
        Self::new(Exchange::Bybit, Market::Spot),
        Self::new(Exchange::Bybit, Market::Perpetual),
    ];

    const fn new(exchange: Exchange, market: Market) -> Self {
        Self { exchange, market }
    }

    pub fn label(self) -> String {
        let market = match self.market {
            Market::Spot => "spot",
            Market::Perpetual => "perp",
        };
        format!("{} {market}", self.exchange.name())
    }
}

/// The side that took liquidity: the buyer who lifted an offer or the seller who hit a bid.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Taker {
    Buy,
    Sell,
}

/// One trade, as far as the flow cares.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Trade {
    pub venue: Venue,
    /// In the quote currency (USDT, taken as dollars).
    pub notional: f64,
    pub taker: Taker,
    /// Exchange time, in milliseconds since the epoch.
    pub time_ms: i64,
}

/// The asset whose flow `symbol` on `source` is a market of (`BTC` for Binance's `BTCUSDT` or
/// Coinbase's `BTC-USD`), when `source` is a crypto exchange and the name is one a venue could
/// list: letters and digits only. `None` for stocks, prediction markets and aggregators.
pub fn asset_of(source: DataSource, symbol: &str) -> Option<String> {
    use DataSource::{Binance, Bybit, Coinbase, Gate, Kraken, Mexc, Okx};
    if !matches!(
        source,
        Binance | Bybit | Okx | Kraken | Gate | Mexc | Coinbase
    ) {
        return None;
    }
    let base = source.base_asset(symbol);
    // Kraken's names for Bitcoin and Dogecoin.
    let asset = match base.as_str() {
        "XBT" => "BTC".to_owned(),
        "XDG" => "DOGE".to_owned(),
        _ => base,
    };
    let valid = (2..=15).contains(&asset.len())
        && asset
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit());
    valid.then_some(asset)
}

/// What takers bought and sold over a span, in dollars.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Volumes {
    pub buy: f64,
    pub sell: f64,
}

impl Volumes {
    /// Bought minus sold: positive when buyers were the aggressors.
    pub fn net(self) -> f64 {
        self.buy - self.sell
    }

    pub fn total(self) -> f64 {
        self.buy + self.sell
    }

    fn merge(&mut self, other: Self) {
        self.buy += other.buy;
        self.sell += other.sell;
    }
}

/// The last minute, five minutes and hour.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Spans {
    pub minute: Volumes,
    pub five_minutes: Volumes,
    pub hour: Volumes,
}

impl Spans {
    fn merge(&mut self, other: Self) {
        self.minute.merge(other.minute);
        self.five_minutes.merge(other.five_minutes);
        self.hour.merge(other.hour);
    }
}

/// What a panel draws: each venue's flow, all of them added up, and the net flow of the last
/// half hour a minute at a time.
#[derive(Clone, Debug, PartialEq)]
pub struct Snapshot {
    pub venues: Vec<(Venue, Spans)>,
    pub all: Spans,
    /// Oldest first; the last is the minute just past. All venues together.
    pub minutes: [Volumes; MINUTES],
    /// Whether any trade has arrived in the last hour.
    pub has_trades: bool,
}

impl Default for Snapshot {
    fn default() -> Self {
        Self {
            venues: Venue::ALL
                .iter()
                .map(|venue| (*venue, Spans::default()))
                .collect(),
            all: Spans::default(),
            minutes: [Volumes::default(); MINUTES],
            has_trades: false,
        }
    }
}

/// What the takers of one venue did within one second of exchange time.
#[derive(Clone, Copy, Debug)]
struct Second {
    second: i64,
    volumes: Volumes,
}

impl Second {
    fn end_ms(&self) -> i64 {
        self.second * 1_000 + 999
    }
}

/// The trades of the last hour, kept as one total per venue and second: memory stays the same
/// however busy the market is.
#[derive(Debug, Default)]
pub struct Flow {
    venues: [VecDeque<Second>; 4],
    revision: u64,
}

impl Flow {
    /// Counts up with every trade added or the record cleared.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Adds `trade` to its second, and drops what is older than an hour before `now_ms`. A trade
    /// that old to begin with is not added.
    pub fn push(&mut self, trade: Trade, now_ms: i64) {
        let cutoff = now_ms - HOUR_MS;
        let index = Venue::ALL.iter().position(|venue| *venue == trade.venue);
        if let (Some(index), true) = (index, trade.time_ms >= cutoff) {
            let seconds = &mut self.venues[index];
            let second = trade.time_ms.div_euclid(1_000);
            let at = seconds.partition_point(|bucket| bucket.second < second);
            if seconds.get(at).is_none_or(|bucket| bucket.second != second) {
                seconds.insert(
                    at,
                    Second {
                        second,
                        volumes: Volumes::default(),
                    },
                );
            }
            match trade.taker {
                Taker::Buy => seconds[at].volumes.buy += trade.notional,
                Taker::Sell => seconds[at].volumes.sell += trade.notional,
            }
        }
        for seconds in &mut self.venues {
            while seconds
                .front()
                .is_some_and(|oldest| oldest.end_ms() < cutoff)
            {
                seconds.pop_front();
            }
        }
        self.revision += 1;
    }

    /// Forgets everything: a record with a gap in it would add up wrongly.
    pub fn clear(&mut self) {
        self.venues.iter_mut().for_each(VecDeque::clear);
        self.revision += 1;
    }

    /// The totals as of `now_ms`. They reach back by whole seconds, so a span may hold up to a
    /// second more than its name says.
    pub fn snapshot(&self, now_ms: i64) -> Snapshot {
        let mut snapshot = Snapshot::default();
        for ((_, spans), seconds) in snapshot.venues.iter_mut().zip(&self.venues) {
            for bucket in seconds.iter().rev() {
                let age = now_ms - bucket.end_ms();
                if age > HOUR_MS {
                    break;
                }
                snapshot.has_trades = true;
                spans.hour.merge(bucket.volumes);
                if age <= 5 * 60_000 {
                    spans.five_minutes.merge(bucket.volumes);
                }
                if age <= 60_000 {
                    spans.minute.merge(bucket.volumes);
                }
                let slot = (age.max(0) / 60_000) as usize;
                if slot < MINUTES {
                    snapshot.minutes[MINUTES - 1 - slot].merge(bucket.volumes);
                }
            }
        }
        for (_, spans) in &snapshot.venues {
            snapshot.all.merge(*spans);
        }
        snapshot
    }
}
