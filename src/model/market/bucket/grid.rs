//! Where a market cuts its buckets: the UTC grid every crypto source shares, or the regular US
//! equity session. A stock's day is not a UTC day: its intraday candles open at the session open
//! (09:30 New York), every session restarts them, the last one is cut at the close (16:00), and
//! the hours around the session hold none (TradingView, `NASDAQ:AAPL`: 4h is `09:30` and `13:30`,
//! 1h is `09:30 10:30 … 15:30`). Days, weeks and months are the same on both grids; only the
//! instant a bucket stops trading (`session_end_ms`) differs, a stock's day ending at the close.

use super::{
    DAY_MS, bucket_start_ms as utc_bucket_start_ms, nests_in,
    next_bucket_start_ms as utc_next_bucket_start_ms,
};
use crate::model::market::session;
use crate::model::{Kline, Timeframe};

/// How many times more base candles than the session holds a request asks for, see
/// [`Grid::fetch_slack`].
const EXTENDED_HOURS_SLACK: usize = 3;

/// How a market's buckets are cut.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum Grid {
    /// Midnight UTC: seconds, minutes and hours restart there (see the module docs of `bucket`).
    #[default]
    Utc,
    /// The regular US equity session, [`session`]: intraday buckets anchor at its open and end at
    /// its close, and only candles inside it exist.
    UsEquities,
}

impl Grid {
    /// Whether `timeframe` is cut on the session rather than the UTC day on this grid.
    fn anchors(self, timeframe: Timeframe) -> bool {
        self == Self::UsEquities && timeframe.is_intraday()
    }

    /// Open time of the bucket of `timeframe` that contains `t_ms`.
    ///
    /// On the session grid an intraday bucket starts at the open plus a whole number of lengths,
    /// counted from the session of the New York day that holds `t_ms`. A moment outside the
    /// session lies on that grid too (no candle does: see [`Grid::is_traded`]).
    pub fn bucket_start_ms(self, timeframe: Timeframe, t_ms: i64) -> i64 {
        if !self.anchors(timeframe) {
            return utc_bucket_start_ms(timeframe, t_ms);
        }
        let open = session::open_ms(session::local_day(t_ms));
        let length = timeframe.nominal_ms();
        open + (t_ms - open).div_euclid(length) * length
    }

    /// Open time of the bucket after the one that contains `t_ms`; on the session grid the last
    /// bucket of a session ends at the close, not a length after it opens.
    pub fn next_bucket_start_ms(self, timeframe: Timeframe, t_ms: i64) -> i64 {
        if !self.anchors(timeframe) {
            return utc_next_bucket_start_ms(timeframe, t_ms);
        }
        let start = self.bucket_start_ms(timeframe, t_ms);
        let close = session::close_ms(session::local_day(start));
        let next = start + timeframe.nominal_ms();
        if start < close { next.min(close) } else { next }
    }

    /// Where trading in the bucket that opens at `start_ms` stops: the next bucket's open, except
    /// that a stock's day, week or month ends at the close of its last weekday.
    pub fn session_end_ms(self, timeframe: Timeframe, start_ms: i64) -> i64 {
        let next = self.next_bucket_start_ms(timeframe, start_ms);
        if self != Self::UsEquities || timeframe.is_intraday() {
            return next;
        }
        let mut day = (next - 1).div_euclid(DAY_MS);
        while !session::is_weekday(day) {
            day -= 1;
        }
        let close = session::close_ms(day);
        if close > start_ms { close } else { next }
    }

    /// Whether a candle of `timeframe` can open at `t_ms`: on the session grid an intraday candle
    /// opens inside a session of a weekday, never before the open or from the close on.
    pub fn is_traded(self, timeframe: Timeframe, t_ms: i64) -> bool {
        !self.anchors(timeframe) || session::is_open(t_ms)
    }

    /// How many base candles to ask a provider for per candle that will be shown: the session grid
    /// leaves out the hours around the session, which a provider's answer holds, so the newest `n`
    /// base candles can be all pre or post market. A New York day has about 2.5 times as many half
    /// hours of trading in the extended hours as in the session (04:00 to 20:00 against 09:30 to
    /// 16:00); a request for three times as many leaves enough of the session in the answer.
    pub fn fetch_slack(self, base: Timeframe) -> usize {
        if self.anchors(base) {
            EXTENDED_HOURS_SLACK
        } else {
            1
        }
    }

    /// Whether every bucket of `base` candles lies in the session grid of `target`: a base length
    /// that divides the open's offset into the day and the session's length, so a base bucket never
    /// straddles the open, a target bucket's start or the close. Always true on the UTC grid.
    fn aligns(self, base: Timeframe) -> bool {
        !self.anchors(base)
            || (session::OPEN_MS % base.nominal_ms() == 0
                && (session::CLOSE_MS - session::OPEN_MS) % base.nominal_ms() == 0)
    }

    /// The coarsest of the `natives` whose buckets nest exactly in `target`'s, so `target` can be
    /// folded from it on this grid: `target` itself when it is native and aligned. None when no
    /// native does.
    ///
    /// Intraday natives must divide the day (their buckets restart at midnight like every intraday
    /// bucket on the UTC grid; one that does not has an anchoring a provider chooses, not us) and,
    /// for an intraday target, divide the target. Days nest in days of a multiple of their count,
    /// and in weeks and months as single days; weeks in weeks, months in months.
    pub fn fold_base(self, target: Timeframe, natives: &[Timeframe]) -> Option<Timeframe> {
        if natives.contains(&target) && self.aligns(target) {
            return Some(target);
        }
        natives
            .iter()
            .copied()
            .filter(|native| nests_in(*native, target) && self.aligns(*native))
            .max()
    }

    /// Fold ascending candles into buckets of `timeframe`, leaving out the candles that open
    /// outside the session on the session grid. Each bucket opens at its start and closes a
    /// millisecond before the next; the forming last bucket is kept.
    ///
    /// `truncated` says the series starts where a request stopped rather than where the market
    /// began: its first bucket is then a fragment and is dropped, so no bar shows a range the data
    /// did not cover.
    pub fn fold(self, candles: &[Kline], timeframe: Timeframe, truncated: bool) -> Vec<Kline> {
        let traded: Vec<&Kline> = candles
            .iter()
            .filter(|candle| self.is_traded(timeframe, candle.open_time))
            .collect();
        let mut out: Vec<Kline> = Vec::new();
        for candle in &traded {
            let bucket = self.bucket_start_ms(timeframe, candle.open_time);
            match out.last_mut() {
                Some(last) if last.open_time == bucket => last.absorb(candle),
                _ => out.push(Kline {
                    open_time: bucket,
                    close_time: self.next_bucket_start_ms(timeframe, bucket) - 1,
                    ..(*candle).clone()
                }),
            }
        }
        if truncated
            && let (Some(first), Some(bucket)) = (traded.first(), out.first())
            && first.open_time > bucket.open_time
            && out.len() > 1
        {
            out.remove(0);
        }
        out
    }

    /// `candles` without the ones that open outside the session, on the session grid, for a
    /// series that is already of `timeframe`.
    pub fn retain_traded(self, mut candles: Vec<Kline>, timeframe: Timeframe) -> Vec<Kline> {
        if self.anchors(timeframe) {
            candles.retain(|candle| session::is_open(candle.open_time));
        }
        candles
    }
}
