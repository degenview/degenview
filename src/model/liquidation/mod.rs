//! Forced liquidations the exchanges announce: what one is, the rolling record of them, and the
//! totals and rows a panel shows. Pure: the sockets that fill it are in `service::liquidation`.

use crate::i18n::tr;
use crate::model::exchange::Exchange;
use std::collections::VecDeque;

#[cfg(test)]
mod tests;

/// The longest span the totals reach back.
pub const HOUR_MS: i64 = 3_600_000;
/// The most rows kept for the tape; the totals do not depend on it.
const MAX_EVENTS: usize = 20_000;
/// The most rows a snapshot lists.
pub const TAPE_ROWS: usize = 300;

/// Which side of the market the exchange closed by force.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Side {
    /// A long position was liquidated: a forced sale.
    Long,
    /// A short position was liquidated: a forced buy.
    Short,
}

/// One forced liquidation.
#[derive(Clone, Debug, PartialEq)]
pub struct Liquidation {
    pub exchange: Exchange,
    /// The market as the exchange names it (`BTCUSDT`).
    pub symbol: String,
    pub side: Side,
    pub price: f64,
    /// In the market's base asset.
    pub quantity: f64,
    /// Exchange time, in milliseconds since the epoch.
    pub time_ms: i64,
}

impl Liquidation {
    /// What it was worth in the quote currency (dollars, for the USDT and USDC contracts).
    pub fn notional(&self) -> f64 {
        self.price * self.quantity
    }
}

/// The smallest liquidation the tape lists. Totals always count every one.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum MinSize {
    #[default]
    Any,
    Thousand,
    TenThousand,
    HundredThousand,
    Million,
}

impl MinSize {
    pub const ALL: [Self; 5] = [
        Self::Any,
        Self::Thousand,
        Self::TenThousand,
        Self::HundredThousand,
        Self::Million,
    ];

    pub fn notional(self) -> f64 {
        match self {
            Self::Any => 0.0,
            Self::Thousand => 1_000.0,
            Self::TenThousand => 10_000.0,
            Self::HundredThousand => 100_000.0,
            Self::Million => 1_000_000.0,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Any => tr!("Any size"),
            Self::Thousand => tr!("$1K and up"),
            Self::TenThousand => tr!("$10K and up"),
            Self::HundredThousand => tr!("$100K and up"),
            Self::Million => tr!("$1M and up"),
        }
    }
}

/// Liquidated notional over a span, by the side that was closed.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Totals {
    pub long: f64,
    pub short: f64,
}

impl Totals {
    pub fn total(self) -> f64 {
        self.long + self.short
    }

    fn add(&mut self, liquidation: &Liquidation) {
        match liquidation.side {
            Side::Long => self.long += liquidation.notional(),
            Side::Short => self.short += liquidation.notional(),
        }
    }

    fn merge(&mut self, other: Self) {
        self.long += other.long;
        self.short += other.short;
    }
}

/// What a panel draws: the latest liquidations of at least the asked size, newest first, and what
/// the last minute, five minutes and hour added up to.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Snapshot {
    pub latest: Vec<Liquidation>,
    pub minute: Totals,
    pub five_minutes: Totals,
    pub hour: Totals,
    /// How many liquidations the last hour holds, of any size.
    pub count: usize,
}

/// Liquidated notional within one second of exchange time.
#[derive(Clone, Copy, Debug)]
struct Second {
    second: i64,
    totals: Totals,
    count: usize,
}

impl Second {
    /// The last millisecond of the second.
    fn end_ms(&self) -> i64 {
        self.second * 1_000 + 999
    }
}

/// The liquidations of the last hour, ordered by exchange time.
///
/// Two records are kept. The rows (for the tape) are capped, so a cascade drops the oldest of
/// them. The totals are by second and are not: they add up every liquidation of the hour however
/// many rows are kept. Liquidations are placed by the exchange's time, not by when they arrived:
/// two sockets deliver in their own order.
#[derive(Debug, Default)]
pub struct Tape {
    events: VecDeque<Liquidation>,
    seconds: VecDeque<Second>,
    revision: u64,
}

impl Tape {
    /// Counts up with every liquidation added or the record cleared, so a view knows when to look
    /// again.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Adds `liquidation` where its time puts it, and drops what is older than an hour before
    /// `now_ms`. A liquidation that old to begin with is not added.
    pub fn push(&mut self, liquidation: Liquidation, now_ms: i64) {
        let cutoff = now_ms - HOUR_MS;
        if liquidation.time_ms >= cutoff {
            self.add_to_seconds(&liquidation);
            let at = self
                .events
                .partition_point(|event| event.time_ms <= liquidation.time_ms);
            self.events.insert(at, liquidation);
        }
        while self
            .events
            .front()
            .is_some_and(|oldest| oldest.time_ms < cutoff)
            || self.events.len() > MAX_EVENTS
        {
            self.events.pop_front();
        }
        while self
            .seconds
            .front()
            .is_some_and(|oldest| oldest.end_ms() < cutoff)
        {
            self.seconds.pop_front();
        }
        self.revision += 1;
    }

    fn add_to_seconds(&mut self, liquidation: &Liquidation) {
        let second = liquidation.time_ms.div_euclid(1_000);
        let at = self
            .seconds
            .partition_point(|bucket| bucket.second < second);
        if self
            .seconds
            .get(at)
            .is_none_or(|bucket| bucket.second != second)
        {
            self.seconds.insert(
                at,
                Second {
                    second,
                    totals: Totals::default(),
                    count: 0,
                },
            );
        }
        let bucket = &mut self.seconds[at];
        bucket.totals.add(liquidation);
        bucket.count += 1;
    }

    /// Forgets everything: a record with a gap in it would add up wrongly.
    pub fn clear(&mut self) {
        self.events.clear();
        self.seconds.clear();
        self.revision += 1;
    }

    /// The rows and totals as of `now_ms`. The totals reach back by whole seconds, so a span may
    /// hold up to a second more than its name says.
    pub fn snapshot(&self, now_ms: i64, minimum: MinSize) -> Snapshot {
        let mut snapshot = Snapshot::default();
        for bucket in self.seconds.iter().rev() {
            let age = now_ms - bucket.end_ms();
            if age > HOUR_MS {
                break;
            }
            snapshot.count += bucket.count;
            snapshot.hour.merge(bucket.totals);
            if age <= 5 * 60_000 {
                snapshot.five_minutes.merge(bucket.totals);
            }
            if age <= 60_000 {
                snapshot.minute.merge(bucket.totals);
            }
        }
        let floor = minimum.notional();
        snapshot.latest = self
            .events
            .iter()
            .rev()
            .take_while(|event| now_ms - event.time_ms <= HOUR_MS)
            .filter(|event| event.notional() >= floor)
            .take(TAPE_ROWS)
            .cloned()
            .collect();
        snapshot
    }
}
