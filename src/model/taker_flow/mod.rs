//! Taker flow: who crossed the spread. Every trade has a taker, the side that hit the book, and the
//! volume the buyers took minus the volume the sellers took says which way the aggression leaned.
//! Pure: the trade sockets that fill it are in `service::taker_flow`.

use crate::model::DataSource;
use crate::model::exchange::Venue;
use crate::model::market::profile::Class;
use std::collections::VecDeque;

#[cfg(test)]
mod tests;

/// The longest span the totals reach back.
pub const HOUR_MS: i64 = 3_600_000;
/// How many one-minute bars of net flow a snapshot holds.
pub const MINUTES: usize = 30;

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
    let profile = source.profile();
    if profile.class != Class::Exchange {
        return None;
    }
    let base = source.coin_asset(symbol);
    // Kraken's names for Bitcoin and Dogecoin.
    let asset = match base.as_str() {
        "XBT" => "BTC".to_owned(),
        "XDG" => "DOGE".to_owned(),
        _ => base,
    };
    let valid = (1..=15).contains(&asset.len())
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

/// How much of each span the sockets were connected for, in milliseconds. A trade that happened
/// while they were not is missing, so a span with less than all of it observed adds up to less
/// than the market did: it says nothing of a quiet market.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Observed {
    pub minute: i64,
    pub five_minutes: i64,
    pub hour: i64,
}

impl Observed {
    /// The whole of a span, give or take what whole seconds and a clock that differs by a moment
    /// from the exchange's say.
    const SLACK_MS: i64 = 2_000;

    pub fn minute_full(self) -> bool {
        full(self.minute, MINUTE_MS)
    }

    pub fn five_minutes_full(self) -> bool {
        full(self.five_minutes, 5 * MINUTE_MS)
    }

    pub fn hour_full(self) -> bool {
        full(self.hour, HOUR_MS)
    }

    fn least(self, other: Self) -> Self {
        Self {
            minute: self.minute.min(other.minute),
            five_minutes: self.five_minutes.min(other.five_minutes),
            hour: self.hour.min(other.hour),
        }
    }
}

/// Whether `observed` milliseconds are all of a span of `span` milliseconds.
pub fn full(observed: i64, span: i64) -> bool {
    observed >= span - Observed::SLACK_MS
}

/// How many venues the flow follows.
pub const VENUES: i64 = Venue::ALL.len() as i64;

/// One minute, in milliseconds.
pub const MINUTE_MS: i64 = 60_000;

/// The last minute, five minutes and hour.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Spans {
    pub minute: Volumes,
    pub five_minutes: Volumes,
    pub hour: Volumes,
    pub observed: Observed,
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
    /// How long of each of those minutes was observed, in milliseconds added up over the venues:
    /// nothing when no socket was connected, all of `VENUES * MINUTE_MS` when every one was.
    pub minutes_observed: [i64; MINUTES],
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
            minutes_observed: [0; MINUTES],
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

/// A stretch of time a venue's socket was connected for: still is while `until` is `None`.
#[derive(Clone, Copy, Debug)]
struct Link {
    from: i64,
    until: Option<i64>,
}

impl Link {
    /// How much of `from..to` it covers, with `now_ms` as the end of one still open.
    fn overlap(&self, from: i64, to: i64, now_ms: i64) -> i64 {
        let until = self.until.unwrap_or(now_ms);
        (until.min(to) - self.from.max(from)).max(0)
    }
}

/// The trades of the last hour, kept as one total per venue and second: memory stays the same
/// however busy the market is. Next to them, when each venue's socket was connected: a quiet
/// market and a socket that was down look alike in the trades.
#[derive(Debug, Default)]
pub struct Flow {
    venues: [VecDeque<Second>; 4],
    links: [VecDeque<Link>; 4],
    revision: u64,
}

impl Flow {
    /// Counts up with every trade added or link noted.
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

    /// Notes that `venue`'s socket connected (`up`) or was lost at `now_ms`.
    pub fn link(&mut self, venue: Venue, up: bool, now_ms: i64) {
        let Some(index) = Venue::ALL.iter().position(|candidate| *candidate == venue) else {
            return;
        };
        let links = &mut self.links[index];
        if let Some(open) = links.back_mut().filter(|link| link.until.is_none()) {
            open.until = Some(now_ms);
        }
        if up {
            links.push_back(Link {
                from: now_ms,
                until: None,
            });
        }
        let cutoff = now_ms - HOUR_MS;
        while links
            .front()
            .is_some_and(|oldest| oldest.until.is_some_and(|until| until < cutoff))
        {
            links.pop_front();
        }
        self.revision += 1;
    }

    /// How much of `from..to` `venue`'s socket was connected for.
    fn observed(&self, index: usize, from: i64, to: i64, now_ms: i64) -> i64 {
        self.links[index]
            .iter()
            .map(|link| link.overlap(from, to, now_ms))
            .sum()
    }

    /// The totals as of `now_ms`. They reach back by whole seconds, so a span may hold up to a
    /// second more than its name says.
    pub fn snapshot(&self, now_ms: i64) -> Snapshot {
        let mut snapshot = Snapshot::default();
        for (index, ((_, spans), seconds)) in
            snapshot.venues.iter_mut().zip(&self.venues).enumerate()
        {
            spans.observed = Observed {
                minute: self.observed(index, now_ms - MINUTE_MS, now_ms, now_ms),
                five_minutes: self.observed(index, now_ms - 5 * MINUTE_MS, now_ms, now_ms),
                hour: self.observed(index, now_ms - HOUR_MS, now_ms, now_ms),
            };
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
        let mut observed: Option<Observed> = None;
        for (_, spans) in &snapshot.venues {
            snapshot.all.merge(*spans);
            observed = Some(observed.map_or(spans.observed, |least| least.least(spans.observed)));
        }
        snapshot.all.observed = observed.unwrap_or_default();
        for (index, slot) in snapshot.minutes_observed.iter_mut().enumerate() {
            let ago = (MINUTES - 1 - index) as i64;
            let (from, to) = (now_ms - (ago + 1) * MINUTE_MS, now_ms - ago * MINUTE_MS);
            *slot = (0..self.links.len())
                .map(|venue| self.observed(venue, from, to, now_ms))
                .sum();
        }
        snapshot
    }
}
