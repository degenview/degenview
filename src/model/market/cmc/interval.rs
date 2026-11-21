//! How CoinMarketCap's global-metrics history is asked for: the spacing of its readings, and the
//! windows one request may cover.
//!
//! The history comes at three spacings, and a request answers at most 2,200 readings. Windows sit
//! on a fixed grid (a whole number of days from the epoch), so a window that is over is the same
//! request for ever and can be kept.

use crate::model::Timeframe;
use crate::model::market::samples;

const DAY_MS: i64 = 86_400_000;

/// The first reading of the history, 2013-04-29 00:00 UTC.
pub const HISTORY_START_MS: i64 = 1_367_193_600_000;

/// Readings in one window: below the 2,200 a request is allowed.
pub const WINDOW_POINTS: i64 = 2_000;

/// The most windows one chart asks for.
pub const MAX_WINDOWS: usize = 4;

/// How long after a window ends its readings can still be revised.
const SETTLE_MS: i64 = DAY_MS;

/// The spacing of the readings.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Interval {
    Minutes5,
    Hour1,
    Day1,
}

impl Interval {
    /// Coarsest first, which is the order a candle length is matched in.
    pub const ALL: [Self; 3] = [Self::Day1, Self::Hour1, Self::Minutes5];

    /// The coarsest spacing fine enough for candles of `timeframe`, or `None` when none divides
    /// it (one minute, and any length that is not a whole number of five minutes).
    pub fn for_timeframe(timeframe: Timeframe) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|interval| samples::divides(timeframe, interval.spacing_ms()))
    }

    /// The `interval` query parameter.
    pub const fn query(self) -> &'static str {
        match self {
            Self::Minutes5 => "5m",
            Self::Hour1 => "1h",
            Self::Day1 => "1d",
        }
    }

    pub const fn spacing_ms(self) -> i64 {
        match self {
            Self::Minutes5 => 300_000,
            Self::Hour1 => 3_600_000,
            Self::Day1 => DAY_MS,
        }
    }

    /// The windows covering `[from_ms, to_ms]`, oldest first: never earlier than the history
    /// starts, and only the newest [`MAX_WINDOWS`] of them.
    pub fn windows(self, from_ms: i64, to_ms: i64) -> Vec<Window> {
        let length = self.spacing_ms() * WINDOW_POINTS;
        let first = from_ms.max(HISTORY_START_MS).div_euclid(length);
        let last = to_ms.div_euclid(length);
        let first = first.max(last - MAX_WINDOWS as i64 + 1);
        (first..=last)
            .map(|index| Window {
                index,
                start_ms: index * length,
                end_ms: (index + 1) * length,
            })
            .collect()
    }
}

/// One request's range, `[start_ms, end_ms)`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Window {
    /// Windows from the epoch, which names it in the cache.
    pub index: i64,
    pub start_ms: i64,
    pub end_ms: i64,
}

impl Window {
    /// Whether the window was over and settled at `at_ms`, so what it held then never changes.
    pub fn is_settled(&self, at_ms: i64) -> bool {
        self.end_ms + SETTLE_MS <= at_ms
    }

    /// The query's `timeStart` and `timeEnd`, in seconds. The range is inclusive at both ends, so
    /// the end is the last second of the window and neighbouring windows never share a reading.
    pub fn query_seconds(&self) -> (i64, i64) {
        (self.start_ms / 1_000, self.end_ms / 1_000 - 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn interval(timeframe: &str) -> Option<Interval> {
        Interval::for_timeframe(Timeframe::parse(timeframe).unwrap())
    }

    #[test]
    fn a_candle_takes_the_coarsest_spacing_that_divides_it() {
        for timeframe in ["1D", "2D", "1W", "1M", "1Y"] {
            assert_eq!(interval(timeframe), Some(Interval::Day1), "{timeframe}");
        }
        for timeframe in ["1h", "2h", "4h", "12h"] {
            assert_eq!(interval(timeframe), Some(Interval::Hour1), "{timeframe}");
        }
        for timeframe in ["5m", "10m", "15m", "30m", "90m"] {
            assert_eq!(interval(timeframe), Some(Interval::Minutes5), "{timeframe}");
        }
    }

    #[test]
    fn a_length_no_spacing_divides_has_no_interval() {
        for timeframe in ["1m", "2m", "7m", "13m", "22m"] {
            assert_eq!(interval(timeframe), None, "{timeframe}");
        }
    }

    #[test]
    fn a_window_never_holds_more_readings_than_a_request_allows() {
        for interval in Interval::ALL {
            let window = interval.windows(HISTORY_START_MS, HISTORY_START_MS)[0];
            let (from, to) = window.query_seconds();
            let readings = (to - from) / (interval.spacing_ms() / 1_000) + 1;
            assert!(readings <= 2_000, "{}", interval.query());
        }
    }

    #[test]
    fn windows_tile_the_range_without_overlap() {
        let interval = Interval::Minutes5;
        let length = interval.spacing_ms() * WINDOW_POINTS;
        let now = 1_795_245_000_000;
        let windows = interval.windows(now - 3 * length, now);
        assert_eq!(windows.len(), 4);
        for pair in windows.windows(2) {
            assert_eq!(pair[0].end_ms, pair[1].start_ms);
            assert_eq!(pair[0].query_seconds().1 + 1, pair[1].query_seconds().0);
        }
        assert!(windows[0].start_ms <= now - 3 * length);
        assert!(windows[3].end_ms > now);
    }

    #[test]
    fn only_the_newest_windows_are_asked_for() {
        let now = 1_795_245_000_000;
        let windows = Interval::Minutes5.windows(HISTORY_START_MS, now);
        assert_eq!(windows.len(), MAX_WINDOWS);
        assert!(windows[MAX_WINDOWS - 1].end_ms > now);
    }

    #[test]
    fn nothing_is_asked_for_before_the_history_starts() {
        let windows = Interval::Day1.windows(0, 1_795_245_000_000);
        // 4,590 days of history lie across four windows of 2,000 on the grid.
        assert_eq!(windows.len(), 4);
        assert!(windows[0].start_ms <= HISTORY_START_MS);
        assert!(windows[0].end_ms > HISTORY_START_MS);
    }

    #[test]
    fn a_window_settles_a_day_after_it_ends() {
        let window = Interval::Hour1.windows(1_795_245_000_000, 1_795_245_000_000)[0];
        assert!(!window.is_settled(window.end_ms));
        assert!(!window.is_settled(window.end_ms + DAY_MS - 1));
        assert!(window.is_settled(window.end_ms + DAY_MS));
    }

    #[test]
    fn the_query_range_is_whole_seconds_inside_the_window() {
        let window = Window {
            index: 1,
            start_ms: 600_000_000,
            end_ms: 1_200_000_000,
        };
        assert_eq!(window.query_seconds(), (600_000, 1_199_999));
    }
}
