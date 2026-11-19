//! Forced liquidations the exchanges announce: what one is, the rolling record of them, and the
//! totals and rows a panel shows. Pure: the sockets that fill it are in `service::liquidation`.

use std::collections::VecDeque;

#[cfg(test)]
mod tests;

/// The longest span the totals reach back.
pub const HOUR_MS: i64 = 3_600_000;
/// The most liquidations kept; a hour of a busy market stays under this.
const MAX_EVENTS: usize = 20_000;
/// The most rows a snapshot lists.
pub const TAPE_ROWS: usize = 300;

/// An exchange that announces its liquidations.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Exchange {
    Binance,
    Bybit,
}

impl Exchange {
    pub fn name(self) -> &'static str {
        match self {
            Self::Binance => "Binance",
            Self::Bybit => "Bybit",
        }
    }
}

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
            Self::Any => "Any size",
            Self::Thousand => "$1K and up",
            Self::TenThousand => "$10K and up",
            Self::HundredThousand => "$100K and up",
            Self::Million => "$1M and up",
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

/// The liquidations of the last hour, oldest first.
#[derive(Debug, Default)]
pub struct Tape {
    events: VecDeque<Liquidation>,
    revision: u64,
}

impl Tape {
    /// Counts up with every liquidation added or the record cleared, so a view knows when to look
    /// again.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Adds `liquidation`, dropping what is older than an hour before `now_ms` and, past the cap,
    /// the oldest.
    pub fn push(&mut self, liquidation: Liquidation, now_ms: i64) {
        self.events.push_back(liquidation);
        while self
            .events
            .front()
            .is_some_and(|oldest| oldest.time_ms < now_ms - HOUR_MS)
            || self.events.len() > MAX_EVENTS
        {
            self.events.pop_front();
        }
        self.revision += 1;
    }

    /// Forgets everything: a record with a gap in it would add up wrongly.
    pub fn clear(&mut self) {
        self.events.clear();
        self.revision += 1;
    }

    /// The rows and totals as of `now_ms`.
    pub fn snapshot(&self, now_ms: i64, minimum: MinSize) -> Snapshot {
        let mut snapshot = Snapshot::default();
        let floor = minimum.notional();
        for liquidation in self.events.iter().rev() {
            let age = now_ms - liquidation.time_ms;
            if age > HOUR_MS {
                break;
            }
            snapshot.count += 1;
            snapshot.hour.add(liquidation);
            if age <= 5 * 60_000 {
                snapshot.five_minutes.add(liquidation);
            }
            if age <= 60_000 {
                snapshot.minute.add(liquidation);
            }
            if snapshot.latest.len() < TAPE_ROWS && liquidation.notional() >= floor {
                snapshot.latest.push(liquidation.clone());
            }
        }
        snapshot
    }
}
