//! Where a higher-timeframe bar opens and closes, for a bar length in seconds (as Pine states
//! timeframes, a month being 30 days). The cutting is `model::market::bucket`'s, on the market's
//! [`Grid`]: a length that is a whole timeframe (`30S`, `5` minutes, `1D`, `2W`, `3M`, `12M`) gets
//! its buckets, quarters and years included. Any other length (90.5 seconds) floors onto the epoch.

use crate::model::market::bucket::Grid;
use crate::model::{TimeUnit, Timeframe};

const MINUTE: i64 = 60;
const DAY: i64 = 86_400;

/// The timeframe a length in seconds is, with Pine's month of 30 days: the largest unit that
/// divides it evenly, as `timeframe_from_seconds` writes it. None for a length that is no
/// timeframe: not whole seconds, or more than a unit allows.
pub fn timeframe_of(seconds: f64) -> Option<Timeframe> {
    let whole = seconds.round() as i64;
    if whole < 1 || (seconds - whole as f64).abs() > 1e-6 {
        return None;
    }
    if whole < MINUTE {
        return Timeframe::new(u16::try_from(whole).ok()?, TimeUnit::Second).ok();
    }
    let (count, unit) = [
        (30 * DAY, TimeUnit::Month),
        (7 * DAY, TimeUnit::Week),
        (DAY, TimeUnit::Day),
        (MINUTE, TimeUnit::Minute),
    ]
    .into_iter()
    .find(|(length, _)| whole % length == 0)
    .map(|(length, unit)| (whole / length, unit))?;
    Timeframe::new(u16::try_from(count).ok()?, unit).ok()
}

/// The length of `timeframe` in seconds as Pine states it, a month being 30 days (`3M` is 90 days
/// and `1Y` 360): what `timeframe.in_seconds()` answers on a chart of that timeframe, and the
/// inverse of [`timeframe_of`].
pub fn seconds_of(timeframe: Timeframe) -> f64 {
    let count = f64::from(timeframe.count());
    match timeframe.unit() {
        TimeUnit::Second => count,
        TimeUnit::Minute => count * MINUTE as f64,
        TimeUnit::Hour => count * 3_600.0,
        TimeUnit::Day => count * DAY as f64,
        TimeUnit::Week => count * 7.0 * DAY as f64,
        TimeUnit::Month => count * 30.0 * DAY as f64,
    }
}

/// Length of a bar that is no timeframe, in milliseconds.
fn epoch_grid_ms(seconds: f64) -> i64 {
    ((seconds * 1_000.0).round() as i64).max(1)
}

/// Pine's length of a calendar bar (`timeframe.in_seconds()`: a month is 30 days) for a bar of
/// `seconds` that opens at `open_time`, as measured between bars: a 31-day month is 2,678,400
/// seconds, but a monthly chart (or a quarterly or yearly one) is `2,592,000`, so a `"M"` request
/// is the chart's own timeframe and not a finer one. Any other length is returned as it is.
pub fn nominal_length(open_time: i64, seconds: f64) -> f64 {
    let months = (seconds / (30 * DAY) as f64).round();
    if months < 1.0 || seconds < (28 * DAY) as f64 {
        return seconds;
    }
    let nominal = months * (30 * DAY) as f64;
    if timeframe_of(nominal).is_none() {
        return seconds;
    }
    let bucket_start = start(Grid::Utc, open_time, nominal);
    let measured = (end(Grid::Utc, bucket_start, nominal) - bucket_start) as f64 / 1_000.0;
    if bucket_start == open_time && (measured - seconds).abs() < 1.0 {
        nominal
    } else {
        seconds
    }
}

/// The start of the bucket that contains `open_time`, for a bar length in seconds.
pub fn start(grid: Grid, open_time: i64, seconds: f64) -> i64 {
    match timeframe_of(seconds) {
        Some(timeframe) => grid.bucket_start_ms(timeframe, open_time),
        None => {
            let grid = epoch_grid_ms(seconds);
            open_time.div_euclid(grid) * grid
        }
    }
}

/// End of the bucket that opens at `start`: where the next one opens, or where trading in it stops
/// on a market whose day, week or month ends at its close (`Grid::session_end_ms`).
pub fn end(grid: Grid, start: i64, seconds: f64) -> i64 {
    match timeframe_of(seconds) {
        Some(timeframe) => grid.session_end_ms(timeframe, start),
        None => start + epoch_grid_ms(seconds),
    }
}

/// End of the bar after the one that opens at `start`: on a market with closing hours the next bar
/// does not open where this one ends (`Grid::following_end_ms`).
pub fn following_end(grid: Grid, start: i64, seconds: f64) -> i64 {
    match timeframe_of(seconds) {
        Some(timeframe) => grid.following_end_ms(timeframe, start),
        None => start + 2 * epoch_grid_ms(seconds),
    }
}

/// Where the bar that opens at `open_time` closes (exclusive), for a bar length in seconds: the end of
/// its bucket. A month, quarter or year is the calendar's, not `seconds` (Pine's nominal 30 days)
/// long, and a length that does not divide the day is cut at midnight.
pub fn bar_end(grid: Grid, open_time: i64, seconds: f64) -> i64 {
    end(grid, start(grid, open_time, seconds), seconds)
}

/// `time_close` of the bar that opens at `open_time`, for a bar length in seconds. A month, quarter
/// or year closes where the calendar's does (TradingView, BTCUSDT 1M: October's bar is 31 days and
/// February's 28, and the next bar opens at the close), and so do several weeks, whose last period
/// of a year the next year's count cuts short (TradingView, `"3W"`: the one that opens on 29
/// December 2025 closes on 5 January). Any other bar closes `seconds` after it opens, as a stock's
/// :30 hourly bar does off the UTC grid its bucket is cut on.
pub fn close_time(grid: Grid, open_time: i64, seconds: f64) -> i64 {
    let month = (30 * DAY) as f64;
    let whole_months = seconds >= month && (seconds / month).fract() == 0.0;
    let several_weeks = grid == Grid::Utc
        && timeframe_of(seconds).is_some_and(|tf| tf.unit() == TimeUnit::Week && tf.count() > 1);
    if whole_months || several_weeks {
        bar_end(grid, open_time, seconds)
    } else {
        open_time + (seconds * 1_000.0) as i64
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pine::time::calendar;

    // The UTC grid, which most of these cases are about.
    fn start(open_time: i64, seconds: f64) -> i64 {
        super::start(Grid::Utc, open_time, seconds)
    }

    fn end(start: i64, seconds: f64) -> i64 {
        super::end(Grid::Utc, start, seconds)
    }

    fn close_time(open_time: i64, seconds: f64) -> i64 {
        super::close_time(Grid::Utc, open_time, seconds)
    }

    fn ms(year: i64, month: i64, day: i64, hour: i64) -> i64 {
        calendar::make(year, month, day, hour, 0, 0, 0).unwrap()
    }

    #[test]
    fn lengths_are_read_the_way_pine_writes_timeframes() {
        let cases = [
            (60.0, Some(Timeframe::MINUTE_1)),
            (300.0, Some(Timeframe::MINUTE_5)),
            (3_600.0, Some(Timeframe::HOUR_1)),
            (14_400.0, Some(Timeframe::HOUR_4)),
            (86_400.0, Some(Timeframe::DAY_1)),
            (172_800.0, Timeframe::new(2, TimeUnit::Day).ok()),
            (604_800.0, Some(Timeframe::WEEK_1)),
            (2_592_000.0, Some(Timeframe::MONTH_1)),
            (7_776_000.0, Some(Timeframe::MONTH_3)),
            (31_104_000.0, Some(Timeframe::YEAR_1)),
            (1_320.0, Timeframe::new(22, TimeUnit::Minute).ok()),
            (1.0, Some(Timeframe::SECOND_1)),
            (30.0, Some(Timeframe::SECOND_30)),
            (59.0, Timeframe::new(59, TimeUnit::Second).ok()),
            (90.5, None),
            (0.5, None),
            // Thirteen months are not a timeframe.
            (13.0 * 30.0 * 86_400.0, None),
            (0.0, None),
        ];
        for (seconds, expected) in cases {
            assert_eq!(timeframe_of(seconds), expected, "{seconds}");
        }
    }

    #[test]
    fn a_timeframes_length_is_the_one_timeframe_of_reads_back() {
        let timeframes = Timeframe::PRESETS.into_iter().chain([
            Timeframe::MONTH_3,
            Timeframe::new(59, TimeUnit::Second).unwrap(),
            Timeframe::new(22, TimeUnit::Minute).unwrap(),
            Timeframe::new(2, TimeUnit::Day).unwrap(),
        ]);
        for timeframe in timeframes {
            let seconds = seconds_of(timeframe);
            assert_eq!(timeframe_of(seconds), Some(timeframe), "{timeframe}");
        }
        assert_eq!(seconds_of(Timeframe::MONTH_3), 7_776_000.0);
        assert_eq!(seconds_of(Timeframe::YEAR_1), 31_104_000.0);
    }

    #[test]
    fn sub_day_buckets_floor_to_the_grid_and_close_at_the_next() {
        assert_eq!(start(7_300_000, 3600.0), 7_200_000);
        assert_eq!(end(3_600_000, 3600.0), 7_200_000);
        // 4h buckets floor to the 4h grid.
        assert_eq!(start(7_200_000, 14_400.0), 0);
    }

    #[test]
    fn an_intraday_bucket_that_does_not_divide_the_day_is_cut_at_midnight() {
        // Five hours: the fifth bucket of a day is its last hour.
        let last = ms(2024, 1, 11, 20);
        assert_eq!(start(ms(2024, 1, 11, 23), 18_000.0), last);
        assert_eq!(end(last, 18_000.0), ms(2024, 1, 12, 0));
    }

    #[test]
    fn a_calendar_bar_closes_where_the_calendar_does() {
        let day = 86_400_000;
        let month = 2_592_000.0;
        // TradingView (BTCUSDT 1M): October 31 days, February 28 (2026), the next bar opens at the close.
        assert_eq!(close_time(ms(2026, 10, 1, 0), month), ms(2026, 11, 1, 0));
        assert_eq!(
            close_time(ms(2026, 2, 1, 0), month),
            ms(2026, 2, 1, 0) + 28 * day
        );
        assert_eq!(
            close_time(ms(2026, 1, 1, 0), 3.0 * month),
            ms(2026, 4, 1, 0)
        );
        // Any other bar: its length after it opens, also off the UTC grid.
        assert_eq!(
            close_time(ms(2026, 10, 1, 20), 5.0 * 3600.0),
            ms(2026, 10, 2, 1)
        );
        assert_eq!(
            close_time(ms(2026, 10, 1, 0) + 1_800_000, 3600.0),
            ms(2026, 10, 1, 1) + 1_800_000
        );
        assert_eq!(
            close_time(ms(2026, 10, 5, 0), 604_800.0),
            ms(2026, 10, 12, 0)
        );
    }

    #[test]
    fn a_calendar_bars_length_is_pines_nominal_one() {
        let day = 86_400.0;
        // January (31 days), February (28) and a quarter and a year of 2024 (leap).
        assert_eq!(nominal_length(ms(2024, 1, 1, 0), 31.0 * day), 30.0 * day);
        assert_eq!(nominal_length(ms(2024, 2, 1, 0), 29.0 * day), 30.0 * day);
        assert_eq!(nominal_length(ms(2024, 1, 1, 0), 91.0 * day), 90.0 * day);
        assert_eq!(nominal_length(ms(2024, 1, 1, 0), 366.0 * day), 360.0 * day);
        // A length that is not a calendar bucket stays: four weeks, a month that opens mid-month.
        assert_eq!(nominal_length(ms(2024, 1, 1, 0), 28.0 * day), 28.0 * day);
        assert_eq!(nominal_length(ms(2024, 1, 10, 0), 31.0 * day), 31.0 * day);
        assert_eq!(nominal_length(ms(2024, 1, 8, 0), 7.0 * day), 7.0 * day);
    }

    #[test]
    fn week_and_month_buckets_use_the_calendar() {
        // A Thursday: 2024-01-11 12:00:00 UTC.
        let thursday = ms(2024, 1, 11, 12);
        // The week (Mon) containing it starts Monday 2024-01-08.
        assert_eq!(start(thursday, 604_800.0), ms(2024, 1, 8, 0));
        // A month bucket is the first of the month, and ends at the next one's.
        let first = ms(2024, 1, 1, 0);
        assert_eq!(start(thursday, 2_592_000.0), first);
        assert_eq!(end(first, 2_592_000.0), ms(2024, 2, 1, 0));
    }

    #[test]
    fn quarters_and_years_are_not_monthly_buckets() {
        let may = ms(2024, 5, 20, 12);
        // 3M is the calendar quarter, 12M the calendar year; both used to cut monthly buckets.
        assert_eq!(start(may, 7_776_000.0), ms(2024, 4, 1, 0));
        assert_eq!(end(ms(2024, 4, 1, 0), 7_776_000.0), ms(2024, 7, 1, 0));
        assert_eq!(start(may, 31_104_000.0), ms(2024, 1, 1, 0));
        assert_eq!(end(ms(2024, 1, 1, 0), 31_104_000.0), ms(2025, 1, 1, 0));
    }

    #[test]
    fn a_length_that_is_no_timeframe_floors_onto_the_epoch() {
        assert_eq!(start(95_000, 90.5), 90_500);
        assert_eq!(end(90_500, 90.5), 181_000);
    }

    #[test]
    fn second_buckets_are_cut_from_midnight() {
        assert_eq!(start(95_000, 30.0), 90_000);
        assert_eq!(end(90_000, 30.0), 120_000);
        // 7 s does not divide the day: the last bucket of a day is cut at midnight.
        let midnight = 86_400_000;
        assert_eq!(start(midnight - 1, 7.0), midnight - 6_000);
        assert_eq!(end(midnight - 6_000, 7.0), midnight);
    }
}
