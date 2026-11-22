//! Where a higher-timeframe bar opens and closes, for a bar length in seconds (as Pine states
//! timeframes, a month being 30 days). The cutting is `model::market::bucket`'s: a length that is
//! a whole timeframe (`5` minutes, `1D`, `2W`, `3M`, `12M`) gets its buckets, quarters and years
//! included. Any other length (`30S`, 90 seconds) floors onto the epoch.

use crate::model::market::bucket;
use crate::model::{TimeUnit, Timeframe};

const MINUTE: i64 = 60;
const DAY: i64 = 86_400;

/// The timeframe a length in seconds is, with Pine's month of 30 days: the largest unit that
/// divides it evenly, as `timeframe_from_seconds` writes it. None for a length that is no
/// timeframe: under a minute, not whole seconds, or more than a unit allows.
pub fn timeframe_of(seconds: f64) -> Option<Timeframe> {
    let whole = seconds.round() as i64;
    if whole < MINUTE || (seconds - whole as f64).abs() > 1e-6 {
        return None;
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

/// Length of a bar that is no timeframe, in milliseconds.
fn epoch_grid_ms(seconds: f64) -> i64 {
    ((seconds * 1_000.0).round() as i64).max(1)
}

/// The start of the bucket that contains `open_time`, for a bar length in seconds.
pub fn start(open_time: i64, seconds: f64) -> i64 {
    match timeframe_of(seconds) {
        Some(timeframe) => bucket::bucket_start_ms(timeframe, open_time),
        None => {
            let grid = epoch_grid_ms(seconds);
            open_time.div_euclid(grid) * grid
        }
    }
}

/// End of the bucket that opens at `start`: where the next one opens.
pub fn end(start: i64, seconds: f64) -> i64 {
    match timeframe_of(seconds) {
        Some(timeframe) => bucket::next_bucket_start_ms(timeframe, start),
        None => start + epoch_grid_ms(seconds),
    }
}

/// Whether every bucket of `coarser` seconds opens where a bucket of `finer` seconds does, so bars
/// of the finer length never straddle a boundary of the coarser one and fold into it whole. A 45
/// minute bucket does not nest in 30 minute bars (the bar at 00:30 spans 00:45); a daily one nests
/// in any intraday bars, as every bar is cut at midnight.
pub fn nests(coarser: f64, finer: f64) -> bool {
    use TimeUnit::{Day, Hour, Minute, Month, Week};
    let (Some(coarse), Some(fine)) = (timeframe_of(coarser), timeframe_of(finer)) else {
        let (coarse, fine) = (epoch_grid_ms(coarser), epoch_grid_ms(finer));
        return coarse % fine == 0;
    };
    let count_nests = || coarse.count() % fine.count() == 0;
    match (coarse.unit(), fine.unit()) {
        (Minute | Hour, Minute | Hour) => coarse.nominal_ms() % fine.nominal_ms() == 0,
        (Day | Week | Month, Minute | Hour) => true,
        (Day, Day) | (Week, Week) | (Month, Month) => count_nests(),
        // Weeks start on Monday and months on the 1st: only single days begin both.
        (Week | Month, Day) => fine.count() == 1,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pine::time::calendar;

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
            (30.0, None),
            (90.5, None),
            // Thirteen months are not a timeframe.
            (13.0 * 30.0 * 86_400.0, None),
            (0.0, None),
        ];
        for (seconds, expected) in cases {
            assert_eq!(timeframe_of(seconds), expected, "{seconds}");
        }
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
    fn a_bucket_nests_in_bars_when_every_boundary_is_a_bar_boundary() {
        let nests = |coarser: f64, finer: f64| super::nests(coarser, finer);
        assert!(nests(14_400.0, 3_600.0), "4h in 1h");
        assert!(nests(14_400.0, 900.0), "4h in 15m");
        assert!(!nests(2_700.0, 1_800.0), "45m in 30m");
        assert!(!nests(10_800.0, 18_000.0 / 2.0), "3h in 2.5h");
        assert!(
            nests(86_400.0, 18_000.0),
            "a day in 5h bars cut at midnight"
        );
        assert!(nests(36_000.0, 18_000.0), "10h in 5h");
        assert!(!nests(10_800.0, 18_000.0), "3h in 5h");
        assert!(nests(604_800.0, 86_400.0), "a week in days");
        assert!(!nests(604_800.0, 172_800.0), "a week in 2D bars");
        assert!(nests(172_800.0, 172_800.0), "2D in 2D");
        assert!(!nests(172_800.0, 259_200.0), "2D in 3D");
        assert!(!nests(2_592_000.0, 604_800.0), "a month in weeks");
        assert!(nests(2_592_000.0, 86_400.0), "a month in days");
        assert!(nests(7_776_000.0, 2_592_000.0), "3M in 1M");
        assert!(!nests(2_592_000.0, 7_776_000.0), "1M in 3M");
        assert!(nests(90.0, 30.0), "90s in 30s");
        assert!(!nests(90.0, 60.0), "90s in 60s");
    }

    #[test]
    fn a_length_that_is_no_timeframe_floors_onto_the_epoch() {
        assert_eq!(start(95_000, 30.0), 90_000);
        assert_eq!(end(90_000, 30.0), 120_000);
    }
}
