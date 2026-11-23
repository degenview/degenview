//! The candle lengths each source serves itself (listed in its `profile`), and from them which
//! timeframes it can chart.
//!
//! Every provider plans its requests from these lists (`bucket::FoldPlan`), and the symbol picker
//! asks `DataSource::serves` before offering a timeframe, so the two agree by construction. A
//! source folds the coarsest finer native candle for a timeframe it lacks and never substitutes a
//! nearby length (`bucket::fold_base` says what nests).

use super::bucket::FoldPlan;
use super::profile::{OlderHistory, Series};
use super::samples;
use super::source::DataSource;
use crate::model::Timeframe;

/// The candles Kraken's OHLC answers with: always the newest 720, whatever `since` says, so a
/// chart reaches back 12 hours of 1-minute bars, 30 days of hours and 2 years of days.
pub const KRAKEN_BARS: usize = 720;

/// The most candles one GeckoTerminal request returns.
pub const GECKOTERMINAL_PAGE: usize = 1_000;
/// How far back GeckoTerminal's free API reaches, in days.
pub const GECKOTERMINAL_HISTORY_DAYS: i64 = 180;

impl DataSource {
    /// The candle lengths the source serves itself, shortest first. Empty for a source that has no
    /// candles to ask for (CoinGecko builds them from prices, the line charts are not candles).
    pub fn natives(self) -> &'static [Timeframe] {
        self.profile().natives
    }

    /// How `timeframe` is made from the source's own candles; `None` for a source that has none
    /// or whose candles no fold of reaches it.
    pub fn plan(self, timeframe: Timeframe) -> Option<FoldPlan> {
        FoldPlan::new(timeframe, self.natives())
    }

    /// Whether the source can chart `timeframe`: its candles fold to it, CoinGecko's price
    /// samples are fine enough, or the source draws a line that follows any timeframe.
    pub fn serves(self, timeframe: Timeframe) -> bool {
        match self.profile().series {
            Series::Candles => self.plan(timeframe).is_some(),
            Series::Samples => samples::chart_days(timeframe, 1).is_some(),
            Series::Line => true,
        }
    }

    /// Whether this source can serve the candles before a given time at `timeframe`, which
    /// scrolling back past what a refresh brings needs. GeckoTerminal's free API reaches back only
    /// 180 days: a page of its base candles that already spans that has no older one.
    pub fn supports_older_history(self, timeframe: Timeframe) -> bool {
        match self.profile().older_history {
            OlderHistory::Never => false,
            OlderHistory::Always => true,
            OlderHistory::Within { page, days } => self
                .plan(timeframe)
                .is_some_and(|plan| plan.base().nominal_ms() * page / 86_400_000 < days),
        }
    }
}

#[cfg(test)]
mod tests;
