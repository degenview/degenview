//! How a source serves a timeframe from the candle lengths it has: itself when native, else by
//! folding the coarsest finer native one. Every provider plans through this, so what a source can
//! chart (`DataSource::serves`) and what its fetch asks for cannot drift apart.

use super::{DAY_MS, bucket_start_ms, fold, fold_base};
use crate::model::{Kline, TimeUnit, Timeframe};

/// The most days a calendar month has, for sizing a fold that must hold a whole month.
const LONGEST_MONTH_DAYS: i64 = 31;

/// A target timeframe and the native candle length it is served from.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FoldPlan {
    target: Timeframe,
    base: Timeframe,
}

impl FoldPlan {
    /// The plan for `target` over a source's `natives`; `None` when none nests in it.
    pub fn new(target: Timeframe, natives: &[Timeframe]) -> Option<Self> {
        fold_base(target, natives).map(|base| Self { target, base })
    }

    pub fn target(self) -> Timeframe {
        self.target
    }

    /// The candle length the source is asked for.
    pub fn base(self) -> Timeframe {
        self.base
    }

    /// Whether the source has the target itself.
    pub fn is_native(self) -> bool {
        self.base == self.target
    }

    /// The most base candles one target candle holds (a month at its longest).
    pub fn per_target(self) -> usize {
        let candles = match (self.target.unit(), self.base.unit()) {
            (TimeUnit::Month, TimeUnit::Month) => {
                i64::from(self.target.count() / self.base.count())
            }
            (TimeUnit::Month, _) => {
                let longest = i64::from(self.target.count()) * LONGEST_MONTH_DAYS * DAY_MS;
                (longest + self.base.nominal_ms() - 1) / self.base.nominal_ms()
            }
            _ => self.target.nominal_ms() / self.base.nominal_ms(),
        };
        candles.max(1) as usize
    }

    /// The base candles to ask for to end with `limit` whole target candles: those, and one more
    /// target candle's worth for the fragment the start of the request cuts off. A native target
    /// is asked for as it is.
    pub fn base_count(self, limit: usize) -> usize {
        if self.is_native() {
            limit
        } else {
            (limit + 1) * self.per_target()
        }
    }

    /// `candles` (ascending, of the base length) as target candles: folded unless the source has the
    /// target itself. `truncated` says the series starts where a request stopped, not where the
    /// market began, so the fragment a fold makes of its first bucket is dropped.
    pub fn fold(self, candles: Vec<Kline>, truncated: bool) -> Vec<Kline> {
        if self.is_native() {
            candles
        } else {
            fold(&candles, self.target, truncated)
        }
    }

    /// The newest `limit` target candles of `candles`, the source's answer to a request for
    /// `requested` base candles. An answer that fills the request was cut short of the market's
    /// beginning, so its first fold is a fragment and goes.
    pub fn shape(self, candles: Vec<Kline>, requested: usize, limit: usize) -> Vec<Kline> {
        let truncated = candles.len() >= requested;
        self.trimmed(candles, truncated, limit)
    }

    /// [`shape`](Self::shape) for a source that answers a time window, not a count: the window
    /// began at `since_ms`, so even a short answer is cut off when its first candle sits in a
    /// bucket that opened before the window did. The candles before `since_ms` may exist, so that
    /// bucket is a fragment.
    pub fn shape_since(
        self,
        candles: Vec<Kline>,
        requested: usize,
        since_ms: i64,
        limit: usize,
    ) -> Vec<Kline> {
        let begins_inside = candles
            .first()
            .is_some_and(|first| since_ms > bucket_start_ms(self.target, first.open_time));
        let truncated = candles.len() >= requested || begins_inside;
        self.trimmed(candles, truncated, limit)
    }

    fn trimmed(self, candles: Vec<Kline>, truncated: bool, limit: usize) -> Vec<Kline> {
        let mut shown = self.fold(candles, truncated);
        let skip = shown.len().saturating_sub(limit);
        shown.drain(..skip);
        shown
    }
}
