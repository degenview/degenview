//! The candle lengths each source serves itself, and from them which timeframes it can chart.
//!
//! Every provider plans its requests from these lists (`bucket::FoldPlan`), and the symbol picker
//! asks `DataSource::serves` before offering a timeframe, so the two agree by construction. A
//! source folds the coarsest finer native candle for a timeframe it lacks and never substitutes a
//! nearby length (`bucket::fold_base` says what nests).

use super::bucket::FoldPlan;
use super::samples;
use super::source::DataSource;
use crate::model::{TimeUnit, Timeframe};

const fn minutes(count: u16) -> Timeframe {
    native(count, TimeUnit::Minute)
}

const fn hours(count: u16) -> Timeframe {
    native(count, TimeUnit::Hour)
}

/// A native length, checked when the list is built.
const fn native(count: u16, unit: TimeUnit) -> Timeframe {
    match Timeframe::new(count, unit) {
        Ok(timeframe) => timeframe,
        Err(_) => panic!("a native timeframe is out of range"),
    }
}

/// Binance's kline intervals. Its `3d` is left out: Binance counts those from a day of its own
/// (not a multiple of three since 1970-01-01), so they would not match every other source's.
/// Weeks start on Monday, as here.
const BINANCE: [Timeframe; 14] = [
    minutes(1),
    minutes(3),
    minutes(5),
    minutes(15),
    minutes(30),
    hours(1),
    hours(2),
    hours(4),
    hours(6),
    hours(8),
    hours(12),
    Timeframe::DAY_1,
    Timeframe::WEEK_1,
    Timeframe::MONTH_1,
];

/// OKX's candle bars. Its plain `6H`, `12H`, `1D`, `1W`, `1M` and `3M` are cut at UTC+8
/// midnight, so the `utc` bars are the ones used: they are cut as everything else is. Left out:
/// `8H` (OKX has no such bar), `2D` and `3D` (counted from a day that is not a multiple of the
/// length since 1970-01-01, as Binance's `3d` is: they fold from days) and the year (it folds from
/// the quarter).
const OKX: [Timeframe; 14] = [
    minutes(1),
    minutes(3),
    minutes(5),
    minutes(15),
    minutes(30),
    hours(1),
    hours(2),
    hours(4),
    hours(6),
    hours(12),
    Timeframe::DAY_1,
    Timeframe::WEEK_1,
    Timeframe::MONTH_1,
    Timeframe::MONTH_3,
];

/// Kraken's OHLC intervals in minutes: 1, 5, 15, 30, 60, 240 and 1440. Left out: its week
/// (`10080`) starts on a Thursday and its `21600` is a 15-day length counted from 1970-01-01, so
/// weeks, months and years fold from days.
const KRAKEN: [Timeframe; 7] = [
    minutes(1),
    minutes(5),
    minutes(15),
    minutes(30),
    hours(1),
    hours(4),
    Timeframe::DAY_1,
];

/// The candles Kraken's OHLC answers with: always the newest 720, whatever `since` says, so a
/// chart reaches back 12 hours of 1-minute bars, 30 days of hours and 2 years of days.
pub const KRAKEN_BARS: usize = 720;

/// Coinbase's granularities: 60, 300, 900, 3600, 21600 and 86400 seconds.
const COINBASE: [Timeframe; 6] = [
    minutes(1),
    minutes(5),
    minutes(15),
    hours(1),
    hours(6),
    Timeframe::DAY_1,
];

/// Alpaca serves any minute count to 59 and hour count to 23, but only the lengths that divide
/// the hour are used: it is not documented where its other bars start, and one that is not on our
/// midnight grid would cut buckets the other sources do not. Days, weeks and months are as it
/// stamps them (at New York midnight).
const ALPACA: [Timeframe; 8] = [
    minutes(1),
    minutes(5),
    minutes(15),
    minutes(30),
    hours(1),
    Timeframe::DAY_1,
    Timeframe::WEEK_1,
    Timeframe::MONTH_1,
];

/// GeckoTerminal's OHLCV aggregates (it indexes DEXScreener's pools): minute 1, 5 and 15, hour 1,
/// 4 and 12, and the day.
const GECKOTERMINAL: [Timeframe; 7] = [
    minutes(1),
    minutes(5),
    minutes(15),
    hours(1),
    hours(4),
    hours(12),
    Timeframe::DAY_1,
];

/// The most candles one GeckoTerminal request returns.
pub const GECKOTERMINAL_PAGE: usize = 1_000;
/// How far back GeckoTerminal's free API reaches, in days.
pub const GECKOTERMINAL_HISTORY_DAYS: i64 = 180;

impl DataSource {
    /// The candle lengths the source serves itself, shortest first. Empty for a source that has no
    /// candles to ask for (CoinGecko builds them from prices, the line charts are not candles).
    pub fn natives(self) -> &'static [Timeframe] {
        match self {
            Self::Binance => &BINANCE,
            Self::Coinbase => &COINBASE,
            Self::Okx => &OKX,
            Self::Kraken => &KRAKEN,
            Self::Alpaca => &ALPACA,
            Self::DexScreener => &GECKOTERMINAL,
            Self::CoinGecko | Self::Polymarket | Self::Kalshi | Self::CoinMarketCap => &[],
        }
    }

    /// How `timeframe` is made from the source's own candles; `None` for a source that has none
    /// or whose candles no fold of reaches it.
    pub fn plan(self, timeframe: Timeframe) -> Option<FoldPlan> {
        FoldPlan::new(timeframe, self.natives())
    }

    /// Whether the source can chart `timeframe`: its candles fold to it, CoinGecko's price
    /// samples are fine enough, or the source draws a line that follows any timeframe.
    pub fn serves(self, timeframe: Timeframe) -> bool {
        match self {
            Self::Binance
            | Self::Coinbase
            | Self::Okx
            | Self::Kraken
            | Self::Alpaca
            | Self::DexScreener => self.plan(timeframe).is_some(),
            Self::CoinGecko => samples::chart_days(timeframe, 1).is_some(),
            Self::Polymarket | Self::Kalshi | Self::CoinMarketCap => true,
        }
    }

    /// Whether this source can serve the candles before a given time at `timeframe`, which
    /// scrolling back past what a refresh brings needs. GeckoTerminal's free API reaches back only
    /// 180 days: a page of its base candles that already spans that has no older one.
    pub fn supports_older_history(self, timeframe: Timeframe) -> bool {
        match self {
            // Kraken's OHLC answers the newest 720 candles whatever is asked: the chart's first
            // 500 leave the rest for the first older page, and a second page is empty.
            Self::Binance | Self::Coinbase | Self::Okx | Self::Kraken | Self::Alpaca => true,
            Self::DexScreener => self.plan(timeframe).is_some_and(|plan| {
                let page_days = plan.base().nominal_ms() * GECKOTERMINAL_PAGE as i64 / 86_400_000;
                page_days < GECKOTERMINAL_HISTORY_DAYS
            }),
            Self::CoinGecko | Self::Polymarket | Self::Kalshi | Self::CoinMarketCap => false,
        }
    }
}

#[cfg(test)]
mod tests;
