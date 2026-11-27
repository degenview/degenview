use super::*;

const MINUTE_MS: i64 = 60_000;
const HOUR_MS: i64 = 3_600_000;

/// The UTC grid's fold, which most of these tests are about.
fn fold(candles: &[Kline], timeframe: Timeframe, truncated: bool) -> Vec<Kline> {
    Grid::Utc.fold(candles, timeframe, truncated)
}

fn fold_base(target: Timeframe, natives: &[Timeframe]) -> Option<Timeframe> {
    Grid::Utc.fold_base(target, natives)
}

fn tf(count: u16, unit: TimeUnit) -> Timeframe {
    Timeframe::new(count, unit).unwrap()
}

fn day(year: i64, month: i64, date: i64) -> i64 {
    days_from_civil(year, month, date) * DAY_MS
}

fn at(year: i64, month: i64, date: i64, hour: i64, minute: i64) -> i64 {
    day(year, month, date) + hour * HOUR_MS + minute * MINUTE_MS
}

fn candle(open_time: i64, length: i64, price: f64) -> Kline {
    Kline {
        open_time,
        close_time: open_time + length - 1,
        open: price,
        high: price + 5.0,
        low: price - 5.0,
        close: price + 1.0,
        volume: 2.0,
    }
}

fn series(start: i64, length: i64, count: usize) -> Vec<Kline> {
    (0..count)
        .map(|index| candle(start + index as i64 * length, length, 100.0 + index as f64))
        .collect()
}

fn daily(start: i64, count: usize) -> Vec<Kline> {
    series(start, DAY_MS, count)
}

fn opens(candles: &[Kline]) -> Vec<i64> {
    candles.iter().map(|candle| candle.open_time).collect()
}

/// The functions this module replaced, kept to prove the six timeframes cut the same buckets.
mod old {
    const DAY_MS: i64 = 86_400_000;

    pub fn bucket_start_ms(epoch_ms: i64, interval_secs: i64) -> i64 {
        if interval_secs < 604_800 {
            return epoch_ms.div_euclid(interval_secs * 1_000) * interval_secs * 1_000;
        }
        let days = epoch_ms.div_euclid(DAY_MS);
        if interval_secs < 2_419_200 {
            let monday_days = (days - 4).div_euclid(7) * 7 + 4;
            return monday_days * DAY_MS;
        }
        let (year, month, _) = crate::format::civil_from_days(days);
        let (target_year, target_month) = if interval_secs < 7_000_000 {
            (year, month)
        } else if interval_secs < 30_000_000 {
            (year, (month - 1) / 3 * 3 + 1)
        } else {
            (year, 1)
        };
        crate::format::days_from_civil(target_year, target_month, 1) * DAY_MS
    }

    pub fn next_bucket_start_ms(bucket: i64, interval_secs: i64) -> i64 {
        let probe = match interval_secs {
            s if s < 604_800 => return bucket + s * 1_000,
            s if s < 2_419_200 => 8,
            s if s < 7_000_000 => 32,
            s if s < 30_000_000 => 93,
            _ => 367,
        };
        bucket_start_ms(bucket + probe * DAY_MS, interval_secs)
    }
}

#[test]
fn the_six_legacy_timeframes_cut_the_buckets_they_always_did() {
    let legacy = [
        (Timeframe::HOUR_1, 3_600),
        (Timeframe::DAY_1, 86_400),
        (Timeframe::WEEK_1, 604_800),
        (Timeframe::MONTH_1, 2_592_000),
        (Timeframe::MONTH_3, 7_776_000),
        (Timeframe::YEAR_1, 31_536_000),
    ];
    // From 1969 to 2040 in uneven steps, so every weekday, month end and leap day is visited.
    let mut time = -90_000_000_000;
    while time < 2_230_000_000_000 {
        for (timeframe, seconds) in legacy {
            let start = old::bucket_start_ms(time, seconds);
            assert_eq!(
                bucket_start_ms(timeframe, time),
                start,
                "{timeframe} {time}"
            );
            assert_eq!(
                next_bucket_start_ms(timeframe, time),
                old::next_bucket_start_ms(start, seconds),
                "{timeframe} {time}"
            );
        }
        time += 3_333_333_333;
    }
}

#[test]
fn twenty_two_minute_buckets_restart_at_midnight() {
    let timeframe = tf(22, TimeUnit::Minute);
    let midnight = day(2024, 3, 5);
    assert_eq!(bucket_start_ms(timeframe, midnight), midnight);
    assert_eq!(
        bucket_start_ms(timeframe, midnight + 21 * MINUTE_MS + 59_999),
        midnight
    );
    assert_eq!(
        bucket_start_ms(timeframe, midnight + 22 * MINUTE_MS),
        midnight + 22 * MINUTE_MS
    );
    // 65 whole buckets fill 1430 minutes; the last one is 10 minutes long and closes at midnight.
    let last = midnight + 1_430 * MINUTE_MS;
    assert_eq!(bucket_start_ms(timeframe, midnight + DAY_MS - 1), last);
    assert_eq!(next_bucket_start_ms(timeframe, last), midnight + DAY_MS);
    // The next day starts over, not 12 minutes into a bucket.
    assert_eq!(
        bucket_start_ms(timeframe, midnight + DAY_MS),
        midnight + DAY_MS
    );
}

#[test]
fn hours_and_minutes_that_divide_the_day_keep_the_epoch_grid() {
    let time = at(2024, 3, 5, 13, 47);
    assert_eq!(
        bucket_start_ms(Timeframe::HOUR_4, time),
        at(2024, 3, 5, 12, 0)
    );
    assert_eq!(
        next_bucket_start_ms(Timeframe::HOUR_4, time),
        at(2024, 3, 5, 16, 0)
    );
    assert_eq!(
        bucket_start_ms(Timeframe::MINUTE_15, time),
        at(2024, 3, 5, 13, 45)
    );
    // Five hours do not divide the day: the fifth bucket is the last hour.
    let five = tf(5, TimeUnit::Hour);
    assert_eq!(
        bucket_start_ms(five, at(2024, 3, 5, 23, 30)),
        at(2024, 3, 5, 20, 0)
    );
    assert_eq!(
        next_bucket_start_ms(five, at(2024, 3, 5, 23, 30)),
        at(2024, 3, 6, 0, 0)
    );
    // 24 hours is a day-long bucket from midnight.
    let day_long = tf(24, TimeUnit::Hour);
    assert_eq!(
        bucket_start_ms(day_long, at(2024, 3, 5, 23, 59)),
        day(2024, 3, 5)
    );
    assert_eq!(
        next_bucket_start_ms(day_long, day(2024, 3, 5)),
        day(2024, 3, 6)
    );
}

#[test]
fn multi_day_buckets_count_days_from_the_epoch() {
    let two = tf(2, TimeUnit::Day);
    // Day 0 is 1970-01-01, so even day numbers open a bucket.
    assert_eq!(bucket_start_ms(two, 0), 0);
    assert_eq!(bucket_start_ms(two, DAY_MS + 5), 0);
    assert_eq!(bucket_start_ms(two, 2 * DAY_MS), 2 * DAY_MS);
    assert_eq!(next_bucket_start_ms(two, DAY_MS), 2 * DAY_MS);
    assert_eq!(next_bucket_start_ms(two, 2 * DAY_MS), 4 * DAY_MS);
    // Before the epoch too.
    assert_eq!(bucket_start_ms(two, -1), -2 * DAY_MS);
    // 2024-01-10 is epoch day 19732, an even one.
    let tenth = day(2024, 1, 10);
    assert_eq!(bucket_start_ms(two, tenth + DAY_MS), tenth);
    assert_eq!(bucket_start_ms(two, tenth + 2 * DAY_MS), tenth + 2 * DAY_MS);
}

#[test]
fn month_counts_restart_in_january() {
    // TradingView, BTCUSDT (probe R11): five-month periods open in January, June and November.
    let five = tf(5, TimeUnit::Month);
    assert_eq!(bucket_start_ms(five, day(2024, 3, 15)), day(2024, 1, 1));
    assert_eq!(bucket_start_ms(five, day(2025, 7, 10)), day(2025, 6, 1));
    assert_eq!(bucket_start_ms(five, day(2025, 12, 31)), day(2025, 11, 1));
    assert_eq!(bucket_start_ms(five, day(2026, 1, 6)), day(2026, 1, 1));
    assert_eq!(
        next_bucket_start_ms(five, day(2025, 11, 1)),
        day(2026, 1, 1)
    );
}

#[test]
fn multi_week_buckets_count_from_the_first_monday_of_the_year() {
    // TradingView, BTCUSDT (probe R10, 2026-11-26): where the period holding each date opens.
    let weeks = |count| tf(count, TimeUnit::Week);
    let cases = [
        // date, 2W, 3W, 4W
        ((2023, 1, 2), (2023, 1, 2), (2023, 1, 2), (2023, 1, 2)),
        ((2024, 1, 1), (2024, 1, 1), (2024, 1, 1), (2024, 1, 1)),
        ((2025, 1, 6), (2025, 1, 6), (2025, 1, 6), (2025, 1, 6)),
        // Before 2026's first Monday: still counted from 2025's.
        (
            (2025, 12, 29),
            (2025, 12, 22),
            (2025, 12, 29),
            (2025, 12, 8),
        ),
        ((2026, 1, 12), (2026, 1, 5), (2026, 1, 5), (2026, 1, 5)),
        ((2026, 6, 1), (2026, 5, 25), (2026, 6, 1), (2026, 5, 25)),
        (
            (2026, 11, 23),
            (2026, 11, 23),
            (2026, 11, 16),
            (2026, 11, 9),
        ),
    ];
    for ((y, m, d), two, three, four) in cases {
        for (count, expected) in [(2, two), (3, three), (4, four)] {
            let (year, month, date) = expected;
            assert_eq!(
                bucket_start_ms(weeks(count), day(y, m, d)),
                day(year, month, date),
                "{count}W at {y}-{m}-{d}"
            );
        }
    }
    // The count restarts with the year, so its last period can be short: 2025's three-week period
    // that opens on 29 December ends when 2026's first one opens.
    assert_eq!(
        next_bucket_start_ms(weeks(3), day(2025, 12, 29)),
        day(2026, 1, 5)
    );
    assert_eq!(
        next_bucket_start_ms(weeks(2), day(2026, 1, 12)),
        day(2026, 1, 19)
    );
    // One week stays on the Mondays Binance uses.
    assert_eq!(
        bucket_start_ms(Timeframe::WEEK_1, day(2024, 1, 4)),
        day(2024, 1, 1)
    );
    assert_eq!(
        bucket_start_ms(Timeframe::WEEK_1, day(2024, 1, 7) + DAY_MS - 1),
        day(2024, 1, 1)
    );
}

#[test]
fn quarters_and_years_follow_the_calendar() {
    assert_eq!(
        bucket_start_ms(Timeframe::MONTH_3, day(2024, 5, 20)),
        day(2024, 4, 1)
    );
    assert_eq!(
        next_bucket_start_ms(Timeframe::MONTH_3, day(2024, 11, 20)),
        day(2025, 1, 1)
    );
    assert_eq!(
        bucket_start_ms(Timeframe::YEAR_1, day(2024, 11, 20)),
        day(2024, 1, 1)
    );
    assert_eq!(
        next_bucket_start_ms(Timeframe::YEAR_1, day(2024, 11, 20)),
        day(2025, 1, 1)
    );
    // Two months: January and February, then March and April.
    let two = tf(2, TimeUnit::Month);
    assert_eq!(bucket_start_ms(two, day(2024, 2, 29)), day(2024, 1, 1));
    assert_eq!(next_bucket_start_ms(two, day(2024, 2, 29)), day(2024, 3, 1));
    // Before the epoch.
    assert_eq!(
        bucket_start_ms(Timeframe::MONTH_3, day(1969, 11, 5)),
        day(1969, 10, 1)
    );
}

#[test]
fn weeks_open_on_monday_and_close_before_the_next() {
    // 2024-01-01 was a Monday.
    let weeks = fold(&daily(day(2024, 1, 1), 15), Timeframe::WEEK_1, false);
    assert_eq!(
        opens(&weeks),
        [day(2024, 1, 1), day(2024, 1, 8), day(2024, 1, 15)]
    );
    assert_eq!(weeks[0].close_time, day(2024, 1, 8) - 1);
    assert_eq!(weeks[0].open, 100.0);
    assert_eq!(weeks[0].close, 107.0);
    assert_eq!(weeks[0].high, 111.0);
    assert_eq!(weeks[0].low, 95.0);
    assert_eq!(weeks[0].volume, 14.0);
}

#[test]
fn months_close_on_their_real_last_day() {
    let months = fold(&daily(day(2024, 1, 1), 70), Timeframe::MONTH_1, false);
    assert_eq!(months[0].close_time, day(2024, 2, 1) - 1);
    assert_eq!(months[1].open_time, day(2024, 2, 1));
    // 2024 is a leap year.
    assert_eq!(months[1].close_time, day(2024, 3, 1) - 1);
}

#[test]
fn folded_quarters_and_years_follow_the_calendar() {
    let quarters = fold(&daily(day(2023, 11, 20), 200), Timeframe::MONTH_3, false);
    assert_eq!(
        opens(&quarters),
        [day(2023, 10, 1), day(2024, 1, 1), day(2024, 4, 1)]
    );
    assert_eq!(quarters[0].close_time, day(2024, 1, 1) - 1);

    let years = fold(&daily(day(2023, 11, 20), 200), Timeframe::YEAR_1, false);
    assert_eq!(opens(&years), [day(2023, 1, 1), day(2024, 1, 1)]);
    assert_eq!(years[0].close_time, day(2024, 1, 1) - 1);
}

#[test]
fn folding_days_or_months_cuts_the_same_buckets() {
    // Binance folds months, Coinbase and GeckoTerminal fold days: both must agree.
    let days = daily(day(2022, 1, 1), 900);
    let months = fold(&days, Timeframe::MONTH_1, false);
    for target in [
        Timeframe::MONTH_3,
        Timeframe::YEAR_1,
        tf(6, TimeUnit::Month),
    ] {
        let from_days = fold(&days, target, false);
        let from_months = fold(&months, target, false);
        let spans = |bars: &[Kline]| -> Vec<(i64, i64)> {
            bars.iter()
                .map(|bar| (bar.open_time, bar.close_time))
                .collect()
        };
        assert_eq!(spans(&from_days), spans(&from_months), "{target}");
    }
}

#[test]
fn minutes_fold_into_buckets_that_restart_each_midnight() {
    let timeframe = tf(22, TimeUnit::Minute);
    // The last 40 minutes of one day and the first 40 of the next.
    let start = day(2024, 3, 5) + DAY_MS - 40 * MINUTE_MS;
    let folded = fold(&series(start, MINUTE_MS, 80), timeframe, false);
    assert_eq!(
        opens(&folded),
        [
            day(2024, 3, 5) + 1_386 * MINUTE_MS,
            day(2024, 3, 5) + 1_408 * MINUTE_MS,
            day(2024, 3, 5) + 1_430 * MINUTE_MS,
            day(2024, 3, 6),
            day(2024, 3, 6) + 22 * MINUTE_MS,
        ]
    );
    let volumes: Vec<f64> = folded.iter().map(|bucket| bucket.volume).collect();
    assert_eq!(volumes, [16.0, 44.0, 20.0, 44.0, 36.0]);
    assert_eq!(
        folded[0].close_time,
        day(2024, 3, 5) + 1_408 * MINUTE_MS - 1
    );
    // The last bucket of the day is 10 minutes, and closes at midnight.
    assert_eq!(folded[2].close_time, day(2024, 3, 6) - 1);
    assert_eq!(folded[3].close_time, day(2024, 3, 6) + 22 * MINUTE_MS - 1);
}

#[test]
fn four_hours_fold_from_hours() {
    let folded = fold(
        &series(at(2024, 3, 5, 2, 0), HOUR_MS, 9),
        Timeframe::HOUR_4,
        false,
    );
    assert_eq!(
        opens(&folded),
        [
            at(2024, 3, 5, 0, 0),
            at(2024, 3, 5, 4, 0),
            at(2024, 3, 5, 8, 0)
        ]
    );
    assert_eq!(folded[0].volume, 4.0);
    assert_eq!(folded[1].volume, 8.0);
    assert_eq!(folded[2].close_time, at(2024, 3, 5, 12, 0) - 1);
}

#[test]
fn multi_day_and_multi_month_folds_use_their_own_grids() {
    let two_days = fold(&daily(day(1970, 1, 2), 5), tf(2, TimeUnit::Day), false);
    assert_eq!(opens(&two_days), [0, 2 * DAY_MS, 4 * DAY_MS]);
    assert_eq!(two_days[1].close_time, 4 * DAY_MS - 1);

    let two_weeks = fold(&daily(day(2024, 1, 1), 28), tf(2, TimeUnit::Week), false);
    assert_eq!(two_weeks.len(), 2);
    assert_eq!(two_weeks[1].open_time - two_weeks[0].open_time, 14 * DAY_MS);
}

#[test]
fn a_truncated_series_loses_its_fragment_of_a_first_bucket() {
    // Starts on a Wednesday: the first week is a fragment.
    let candles = daily(day(2024, 1, 3), 14);
    let kept = fold(&candles, Timeframe::WEEK_1, false);
    let dropped = fold(&candles, Timeframe::WEEK_1, true);
    assert_eq!(kept.len(), dropped.len() + 1);
    assert_eq!(dropped[0].open_time, day(2024, 1, 8));
    // A whole first bucket stays, and so does a lone fragment.
    assert_eq!(
        fold(&daily(day(2024, 1, 1), 7), Timeframe::WEEK_1, true).len(),
        1
    );
    assert_eq!(
        fold(&daily(day(2024, 1, 3), 3), Timeframe::WEEK_1, true).len(),
        1
    );
    // The same for minutes: starting mid-bucket drops the fragment.
    let minutes = series(at(2024, 3, 5, 0, 5), MINUTE_MS, 50);
    let folded = fold(&minutes, tf(22, TimeUnit::Minute), true);
    assert_eq!(folded[0].open_time, at(2024, 3, 5, 0, 22));
}

#[test]
fn nothing_folds_to_nothing() {
    assert!(fold(&[], Timeframe::DAY_1, true).is_empty());
}

fn base(target: Timeframe, natives: &[Timeframe]) -> Option<String> {
    fold_base(target, natives).map(Timeframe::label)
}

#[test]
fn a_native_target_is_its_own_base() {
    assert_eq!(
        fold_base(Timeframe::HOUR_4, &Timeframe::PRESETS),
        Some(Timeframe::HOUR_4)
    );
    // Even a length no other native divides.
    let odd = tf(7, TimeUnit::Minute);
    assert_eq!(fold_base(odd, &[odd, Timeframe::MINUTE_1]), Some(odd));
}

#[test]
fn an_intraday_target_folds_from_its_coarsest_dividing_native() {
    let natives = [
        Timeframe::MINUTE_1,
        Timeframe::MINUTE_5,
        Timeframe::MINUTE_15,
        Timeframe::MINUTE_30,
        Timeframe::HOUR_1,
        Timeframe::HOUR_4,
    ];
    assert_eq!(
        base(tf(10, TimeUnit::Minute), &natives).as_deref(),
        Some("5m")
    );
    assert_eq!(
        base(tf(45, TimeUnit::Minute), &natives).as_deref(),
        Some("15m")
    );
    assert_eq!(base(tf(2, TimeUnit::Hour), &natives).as_deref(), Some("1h"));
    assert_eq!(base(tf(8, TimeUnit::Hour), &natives).as_deref(), Some("4h"));
    assert_eq!(
        base(tf(90, TimeUnit::Minute), &natives).as_deref(),
        Some("30m")
    );
    // 22 minutes is only whole minutes: 5 does not divide it.
    assert_eq!(
        base(tf(22, TimeUnit::Minute), &natives).as_deref(),
        Some("1m")
    );
    // With a 2-minute native, that is the coarser one.
    let with_two = [
        Timeframe::MINUTE_1,
        tf(2, TimeUnit::Minute),
        Timeframe::MINUTE_5,
    ];
    assert_eq!(
        base(tf(22, TimeUnit::Minute), &with_two).as_deref(),
        Some("2m")
    );
    // No native that divides it.
    assert_eq!(base(tf(22, TimeUnit::Minute), &[Timeframe::MINUTE_5]), None);
    // Finer is never made from coarser.
    assert_eq!(base(Timeframe::MINUTE_5, &[Timeframe::HOUR_1]), None);
}

#[test]
fn a_native_that_does_not_divide_the_day_is_no_base() {
    // 11 minutes divides 22 but not 1440: its buckets would not restart where ours do.
    let eleven = tf(11, TimeUnit::Minute);
    assert_eq!(fold_base(tf(22, TimeUnit::Minute), &[eleven]), None);
    assert_eq!(fold_base(Timeframe::DAY_1, &[eleven]), None);
    // Seven minutes does not divide the day either.
    let seven = tf(7, TimeUnit::Minute);
    assert_eq!(fold_base(tf(21, TimeUnit::Minute), &[seven]), None);
    // 5 hours does not divide the day.
    let five_hours = tf(5, TimeUnit::Hour);
    assert_eq!(fold_base(tf(10, TimeUnit::Hour), &[five_hours]), None);
}

#[test]
fn days_and_up_fold_from_intraday_candles_or_days() {
    let natives = [Timeframe::MINUTE_1, Timeframe::HOUR_1, Timeframe::DAY_1];
    assert_eq!(base(tf(2, TimeUnit::Day), &natives).as_deref(), Some("1D"));
    assert_eq!(base(Timeframe::WEEK_1, &natives).as_deref(), Some("1D"));
    assert_eq!(base(tf(2, TimeUnit::Week), &natives).as_deref(), Some("1D"));
    assert_eq!(base(Timeframe::MONTH_3, &natives).as_deref(), Some("1D"));
    assert_eq!(base(Timeframe::YEAR_1, &natives).as_deref(), Some("1D"));
    // With no daily candles the coarsest intraday one that divides the day is the base.
    assert_eq!(
        base(Timeframe::DAY_1, &[Timeframe::MINUTE_1, Timeframe::HOUR_4]).as_deref(),
        Some("4h")
    );
    assert_eq!(
        base(Timeframe::MONTH_1, &[Timeframe::MINUTE_15]).as_deref(),
        Some("15m")
    );
    // A day is also 24 hours of candles.
    assert_eq!(
        fold_base(Timeframe::DAY_1, &[tf(24, TimeUnit::Hour)]),
        Some(tf(24, TimeUnit::Hour))
    );
}

#[test]
fn calendar_bases_must_nest_exactly() {
    let two_days = tf(2, TimeUnit::Day);
    let seven_days = tf(7, TimeUnit::Day);
    // 2 days nest in 4 days, not in 3.
    assert_eq!(fold_base(tf(4, TimeUnit::Day), &[two_days]), Some(two_days));
    assert_eq!(fold_base(tf(3, TimeUnit::Day), &[two_days]), None);
    // Weeks start on Monday; 7-day buckets from the epoch start on Thursday.
    assert_eq!(fold_base(Timeframe::WEEK_1, &[seven_days]), None);
    assert_eq!(fold_base(Timeframe::MONTH_1, &[Timeframe::WEEK_1]), None);
    assert_eq!(fold_base(Timeframe::DAY_1, &[Timeframe::WEEK_1]), None);
    // Weeks nest in multiples of weeks.
    assert_eq!(
        fold_base(
            tf(4, TimeUnit::Week),
            &[Timeframe::WEEK_1, tf(2, TimeUnit::Week)]
        ),
        Some(tf(2, TimeUnit::Week))
    );
    // Months nest in months that divide their count: 3M from 1M, 12M from 3M, never 6M from 4M.
    assert_eq!(
        base(Timeframe::YEAR_1, &[Timeframe::MONTH_1, Timeframe::MONTH_3]).as_deref(),
        Some("3M")
    );
    assert_eq!(
        base(Timeframe::MONTH_3, &[Timeframe::MONTH_1]).as_deref(),
        Some("1M")
    );
    assert_eq!(
        fold_base(tf(6, TimeUnit::Month), &[tf(4, TimeUnit::Month)]),
        None
    );
    assert_eq!(fold_base(Timeframe::MONTH_1, &[Timeframe::MONTH_3]), None);
}

#[test]
fn a_folded_base_cuts_the_same_buckets_as_the_raw_candles() {
    // Whatever fold_base picks must give the same result as folding from one-minute candles.
    let minutes = series(at(2024, 3, 4, 0, 0), MINUTE_MS, 3 * 1_440);
    let natives = [Timeframe::MINUTE_5, Timeframe::HOUR_1, Timeframe::DAY_1];
    for target in [
        tf(10, TimeUnit::Minute),
        tf(2, TimeUnit::Hour),
        tf(8, TimeUnit::Hour),
        tf(2, TimeUnit::Day),
        Timeframe::WEEK_1,
    ] {
        let native = fold_base(target, &natives).unwrap();
        let from_native = fold(&fold(&minutes, native, false), target, false);
        let direct = fold(&minutes, target, false);
        assert_eq!(from_native, direct, "{target} from {native}");
    }
}

#[test]
fn skipped_intervals_are_filled_flat_at_the_previous_close() {
    let mut candles = daily(day(2024, 1, 1), 5);
    candles.remove(2);
    let filled = fill_gaps(candles, DAY_MS, day(2024, 1, 5) + 1);
    assert_eq!(filled.len(), 5);
    assert_eq!(filled[2].open_time, day(2024, 1, 3));
    assert_eq!(filled[2].open, filled[1].close);
    assert_eq!(filled[2].high, filled[2].low);
    assert_eq!(filled[2].volume, 0.0);
}

#[test]
fn the_filled_series_reaches_the_current_interval() {
    let filled = fill_gaps(daily(day(2024, 1, 1), 2), DAY_MS, day(2024, 1, 5) + 1_000);
    assert_eq!(filled.len(), 5);
    assert_eq!(filled[4].open_time, day(2024, 1, 5));
}

#[test]
fn an_endless_silence_is_not_filled() {
    let candles = vec![daily(0, 1)[0].clone(), daily(day(2024, 1, 1), 1)[0].clone()];
    let before = candles.len();
    assert_eq!(fill_gaps(candles, HOUR_MS, day(2024, 1, 1)).len(), before);
}

fn plan(target: Timeframe, natives: &[Timeframe]) -> FoldPlan {
    FoldPlan::new(target, natives).unwrap()
}

const SOME_NATIVES: [Timeframe; 4] = [
    Timeframe::MINUTE_1,
    Timeframe::HOUR_1,
    Timeframe::DAY_1,
    Timeframe::MONTH_1,
];

#[test]
fn a_native_target_is_asked_for_as_it_is() {
    let plan = plan(Timeframe::HOUR_1, &SOME_NATIVES);
    assert!(plan.is_native());
    assert_eq!(
        (plan.target(), plan.base()),
        (Timeframe::HOUR_1, Timeframe::HOUR_1)
    );
    assert_eq!(plan.per_target(), 1);
    assert_eq!(plan.base_count(500), 500);
}

#[test]
fn a_folded_target_asks_for_one_more_buckets_worth_than_it_shows() {
    let odd = plan(tf(22, TimeUnit::Minute), &SOME_NATIVES);
    assert!(!odd.is_native());
    assert_eq!((odd.base(), odd.per_target()), (Timeframe::MINUTE_1, 22));
    assert_eq!(odd.base_count(500), 501 * 22);
    let three_hours = plan(tf(3, TimeUnit::Hour), &SOME_NATIVES);
    assert_eq!(three_hours.base_count(100), 101 * 3);
}

#[test]
fn a_calendar_bucket_is_counted_at_its_longest() {
    // A week is seven days, a month at most 31, a quarter 93 and a year 372 (so a leap year fits).
    assert_eq!(plan(Timeframe::WEEK_1, &SOME_NATIVES).per_target(), 7);
    assert_eq!(plan(tf(2, TimeUnit::Day), &SOME_NATIVES).per_target(), 2);
    let days = [Timeframe::DAY_1];
    assert_eq!(plan(Timeframe::MONTH_1, &days).per_target(), 31);
    assert_eq!(plan(Timeframe::MONTH_3, &days).per_target(), 93);
    assert_eq!(plan(Timeframe::YEAR_1, &days).per_target(), 372);
    // Months of months are exact.
    assert_eq!(
        plan(Timeframe::MONTH_3, &SOME_NATIVES).base(),
        Timeframe::MONTH_1
    );
    let months = [Timeframe::MONTH_1];
    assert_eq!(plan(Timeframe::MONTH_3, &months).per_target(), 3);
    assert_eq!(plan(Timeframe::YEAR_1, &months).per_target(), 12);
}

#[test]
fn nothing_nests_nothing_plans() {
    assert_eq!(FoldPlan::new(Timeframe::HOUR_1, &[Timeframe::DAY_1]), None);
    assert_eq!(FoldPlan::new(Timeframe::HOUR_1, &[]), None);
}

#[test]
fn a_native_answer_is_trimmed_to_the_newest_candles() {
    let hours = series(0, HOUR_MS, 10);
    let shown = plan(Timeframe::HOUR_1, &SOME_NATIVES).shape(hours, 10, 4);
    assert_eq!(
        opens(&shown),
        [6 * HOUR_MS, 7 * HOUR_MS, 8 * HOUR_MS, 9 * HOUR_MS]
    );
}

#[test]
fn a_folded_answer_that_fills_the_request_loses_the_fragment_it_cut() {
    let target = tf(3, TimeUnit::Hour);
    let plan = plan(target, &SOME_NATIVES);
    // Hours 1 to 8: the 00:00 bucket has only its last two hours, the 06:00 one its first three.
    let hourly = series(HOUR_MS, HOUR_MS, 8);
    let full = plan.shape(hourly.clone(), 8, 10);
    assert_eq!(opens(&full), [3 * HOUR_MS, 6 * HOUR_MS]);
    // An answer shorter than the request began where the market did: its first bucket stays.
    let short = plan.shape(hourly, 20, 10);
    assert_eq!(opens(&short), [0, 3 * HOUR_MS, 6 * HOUR_MS]);
}

#[test]
fn a_short_answer_whose_window_began_inside_its_first_bucket_loses_that_bucket() {
    let plan = plan(tf(3, TimeUnit::Hour), &SOME_NATIVES);
    // The window began at 01:00 and held hours 1 to 8: the 00:00 bucket may have had an hour
    // before it, whatever the count says.
    let hourly = series(HOUR_MS, HOUR_MS, 8);
    let shown = plan.shape_since(hourly, 20, HOUR_MS, 10);
    assert_eq!(opens(&shown), [3 * HOUR_MS, 6 * HOUR_MS]);
}

#[test]
fn a_short_answer_whose_window_began_on_a_bucket_boundary_keeps_its_first_bucket() {
    let plan = plan(tf(3, TimeUnit::Hour), &SOME_NATIVES);
    let hourly = series(0, HOUR_MS, 8);
    let shown = plan.shape_since(hourly, 20, 0, 10);
    assert_eq!(opens(&shown), [0, 3 * HOUR_MS, 6 * HOUR_MS]);
    // A window that began before the first candle's bucket opened held nothing of the one before.
    let later = series(3 * HOUR_MS, HOUR_MS, 5);
    let shown = plan.shape_since(later, 20, 2 * HOUR_MS, 10);
    assert_eq!(opens(&shown), [3 * HOUR_MS, 6 * HOUR_MS]);
}

#[test]
fn a_window_answer_that_fills_the_request_still_loses_the_fragment_it_cut() {
    let plan = plan(tf(3, TimeUnit::Hour), &SOME_NATIVES);
    let hourly = series(HOUR_MS, HOUR_MS, 8);
    let shown = plan.shape_since(hourly, 8, 0, 10);
    assert_eq!(opens(&shown), [3 * HOUR_MS, 6 * HOUR_MS]);
}

#[test]
fn a_window_answer_that_is_one_fragment_is_empty() {
    let plan = plan(tf(3, TimeUnit::Hour), &SOME_NATIVES);
    // The window began at 01:00 and held hours 1 and 2, which are all of the 00:00 bucket it saw.
    let hourly = series(HOUR_MS, HOUR_MS, 2);
    assert!(plan.shape_since(hourly, 20, HOUR_MS, 10).is_empty());
    // A whole bucket that follows the fragment is kept.
    let more = series(HOUR_MS, HOUR_MS, 3);
    let shown = plan.shape_since(more, 20, HOUR_MS, 10);
    assert_eq!(opens(&shown), [3 * HOUR_MS]);
}

#[test]
fn a_window_that_began_at_new_york_midnight_keeps_the_quarter_its_first_month_opens() {
    // Alpaca stamps a month at New York midnight, five hours past UTC's: a window that began
    // at 01:00 UTC on January 1 has the whole of January, whatever the UTC bucket start says.
    let plan = plan(Timeframe::MONTH_3, &SOME_NATIVES);
    let months: Vec<Kline> = [(1, 31), (2, 29), (3, 31), (4, 30)]
        .into_iter()
        .map(|(month, days)| {
            candle(
                at(2024, month, 1, 5, 0),
                days * DAY_MS,
                100.0 + month as f64,
            )
        })
        .collect();
    let since = at(2024, 1, 1, 1, 0);
    let shown = plan.shape_since(months.clone(), 20, since, 10);
    assert_eq!(opens(&shown), [day(2024, 1, 1), day(2024, 4, 1)]);
    // A window that began after the first month opened still cuts the quarter.
    let later = plan.shape_since(months, 20, at(2024, 1, 1, 6, 0), 10);
    assert_eq!(opens(&later), [day(2024, 4, 1)]);
}

#[test]
fn the_newest_bucket_is_the_base_candles_of_the_last_ones_bucket() {
    let plan = plan(tf(3, TimeUnit::Hour), &SOME_NATIVES);
    let hourly = series(0, HOUR_MS, 8);
    // Hours 6 and 7 make the forming 06:00 bucket.
    assert_eq!(
        opens(&plan.newest_bucket(&hourly)),
        [6 * HOUR_MS, 7 * HOUR_MS]
    );
    assert!(plan.newest_bucket(&[]).is_empty());
}

#[test]
fn a_native_target_has_no_newest_bucket() {
    let plan = plan(Timeframe::HOUR_1, &SOME_NATIVES);
    assert!(plan.newest_bucket(&series(0, HOUR_MS, 4)).is_empty());
}

#[test]
fn a_folded_answer_is_trimmed_after_folding() {
    let hourly = series(0, HOUR_MS, 24);
    let shown = plan(tf(3, TimeUnit::Hour), &SOME_NATIVES).shape(hourly, 100, 3);
    assert_eq!(opens(&shown), [15 * HOUR_MS, 18 * HOUR_MS, 21 * HOUR_MS]);
    assert_eq!(shown[0].close_time, 18 * HOUR_MS - 1);
}

#[test]
fn second_buckets_restart_at_midnight_and_the_last_one_is_cut_short() {
    let forty_five = tf(45, TimeUnit::Second);
    let midnight = day(2024, 1, 11);
    // 45 s divides the day (1,920 buckets), so the grid is the same every day.
    assert_eq!(bucket_start_ms(forty_five, midnight + 44_999), midnight);
    assert_eq!(
        bucket_start_ms(forty_five, midnight + 45_000),
        midnight + 45_000
    );
    assert_eq!(
        next_bucket_start_ms(forty_five, midnight + 45_000),
        midnight + 90_000
    );
    // 7 s does not: the day's last bucket holds the 6 seconds left.
    let seven = tf(7, TimeUnit::Second);
    let last = midnight + DAY_MS - 6_000;
    assert_eq!(bucket_start_ms(seven, midnight + DAY_MS - 1), last);
    assert_eq!(next_bucket_start_ms(seven, last), midnight + DAY_MS);
}

#[test]
fn seconds_fold_from_one_second_candles_that_divide_them() {
    let natives = [Timeframe::SECOND_1];
    for target in ["5s", "15s", "45s", "1m", "22m", "3h", "1D", "1W"] {
        assert_eq!(
            fold_base(tf_text(target), &natives),
            Some(Timeframe::SECOND_1),
            "{target}"
        );
    }
    // 10 s candles nest in 30 s and a minute, not in 15 s or 45 s.
    let ten = [Timeframe::SECOND_10];
    assert_eq!(fold_base(tf_text("30s"), &ten), Some(Timeframe::SECOND_10));
    assert_eq!(fold_base(tf_text("1m"), &ten), Some(Timeframe::SECOND_10));
    assert_eq!(fold_base(tf_text("15s"), &ten), None);
    assert_eq!(fold_base(tf_text("45s"), &ten), None);
    // A candle length that does not divide the day cannot be a base.
    assert_eq!(fold_base(tf_text("1m"), &[tf(7, TimeUnit::Second)]), None);
}

#[test]
fn one_second_candles_fold_into_forty_five_second_buckets() {
    let start = day(2024, 1, 11);
    let folded = fold(&series(start, 1_000, 100), tf(45, TimeUnit::Second), false);
    assert_eq!(opens(&folded), [start, start + 45_000, start + 90_000]);
    assert_eq!(folded[0].close_time, start + 44_999);
    assert_eq!(folded[2].volume, 2.0 * 10.0);
}

fn tf_text(text: &str) -> Timeframe {
    Timeframe::parse(text).unwrap()
}

/// The regular US session, as TradingView cuts a stock's candles (`NASDAQ:AAPL`, 2026-11-25).
mod session_grid {
    use super::*;

    /// 24 Nov 2026 is in standard time: the session is 14:30 to 21:00 UTC.
    fn winter(hour: i64, minute: i64) -> i64 {
        at(2026, 11, 24, hour, minute)
    }

    /// 9 Jul 2026 is in daylight time: the session is 13:30 to 20:00 UTC.
    fn summer(hour: i64, minute: i64) -> i64 {
        at(2026, 7, 9, hour, minute)
    }

    fn starts(timeframe: Timeframe, first: i64, last: i64) -> Vec<i64> {
        let mut starts = Vec::new();
        let mut open = Grid::UsEquities.bucket_start_ms(timeframe, first);
        while open <= last {
            starts.push(open);
            open = Grid::UsEquities.next_bucket_start_ms(timeframe, open);
        }
        starts
    }

    /// Open times, as hours and minutes after 14:30 UTC (09:30 New York) of a winter session.
    fn after_open(times: &[i64]) -> Vec<i64> {
        times
            .iter()
            .map(|time| (time - winter(14, 30)) / MINUTE_MS)
            .collect()
    }

    #[test]
    fn hours_start_at_half_past_nine_and_the_last_is_cut_at_the_close() {
        // TradingView: 1h 09:30 10:30 ... 15:30, 2h 09:30 11:30 13:30 15:30, 3h 09:30 12:30 15:30,
        // 4h 09:30 13:30; every session's last candle closes at 16:00.
        let (first, last) = (winter(14, 30), winter(20, 59));
        let hour = |count| starts(tf(count, TimeUnit::Hour), first, last);
        assert_eq!(after_open(&hour(1)), [0, 60, 120, 180, 240, 300, 360], "1h");
        assert_eq!(after_open(&hour(2)), [0, 120, 240, 360], "2h");
        assert_eq!(after_open(&hour(3)), [0, 180, 360], "3h");
        assert_eq!(after_open(&hour(4)), [0, 240], "4h");
        for (count, last_open) in [(1, 360), (2, 360), (3, 360), (4, 240)] {
            let timeframe = tf(count, TimeUnit::Hour);
            let open = winter(14, 30) + last_open * MINUTE_MS;
            assert_eq!(
                Grid::UsEquities.next_bucket_start_ms(timeframe, open),
                winter(21, 0),
                "{count}h closes at 16:00"
            );
        }
    }

    #[test]
    fn the_clock_change_moves_the_session_in_utc_and_nothing_else() {
        let four = tf(4, TimeUnit::Hour);
        assert_eq!(
            starts(four, summer(13, 30), summer(19, 59)),
            [summer(13, 30), summer(17, 30)]
        );
        assert_eq!(
            Grid::UsEquities.next_bucket_start_ms(four, summer(17, 30)),
            summer(20, 0)
        );
    }

    #[test]
    fn minutes_that_divide_half_an_hour_are_cut_from_the_open() {
        let grid = Grid::UsEquities;
        assert_eq!(
            grid.bucket_start_ms(tf(15, TimeUnit::Minute), winter(15, 7)),
            winter(15, 0)
        );
        assert_eq!(
            grid.bucket_start_ms(tf(30, TimeUnit::Minute), winter(14, 59)),
            winter(14, 30)
        );
        // 22 minutes from the open: 09:30, 09:52, ... the 18th ends at the close, 16:00.
        let odd = tf(22, TimeUnit::Minute);
        assert_eq!(
            grid.bucket_start_ms(odd, winter(20, 59)),
            winter(14, 30) + 17 * 22 * MINUTE_MS
        );
        assert_eq!(
            grid.next_bucket_start_ms(odd, winter(20, 59)),
            winter(21, 0)
        );
    }

    #[test]
    fn candles_outside_the_session_are_not_folded_and_each_session_starts_afresh() {
        // 30 minute candles from 08:00 to 20:00 New York on the 24th (pre and post market included)
        // and the first one of the 25th.
        let thirty = 30 * MINUTE_MS;
        let mut candles = series(winter(13, 0), thirty, 24);
        candles.push(candle(winter(14, 30) + 24 * 60 * MINUTE_MS, thirty, 100.0));
        let four = Grid::UsEquities.fold(&candles, tf(4, TimeUnit::Hour), false);
        assert_eq!(
            opens(&four),
            [
                winter(14, 30),
                winter(18, 30),
                winter(14, 30) + 24 * 60 * MINUTE_MS
            ]
        );
        assert_eq!(four[0].close_time, winter(18, 30) - 1);
        assert_eq!(four[1].close_time, winter(21, 0) - 1, "cut at the close");
        // 09:30 to 13:30 holds eight candles, 13:30 to 16:00 five: the candles at 08:00 and after
        // 16:00 are in neither.
        assert_eq!(four[0].volume, 8.0 * 2.0);
        assert_eq!(four[1].volume, 5.0 * 2.0);
    }

    #[test]
    fn a_native_intraday_series_loses_its_extended_hours_too() {
        let thirty = 30 * MINUTE_MS;
        let candles = series(winter(13, 0), thirty, 24);
        let plan = FoldPlan::in_grid(
            Grid::UsEquities,
            tf(30, TimeUnit::Minute),
            &[tf(30, TimeUnit::Minute)],
        )
        .unwrap();
        assert!(plan.is_native());
        let shown = plan.fold(candles, false);
        assert_eq!(shown.len(), 13);
        assert_eq!(shown[0].open_time, winter(14, 30));
        assert_eq!(shown[12].open_time, winter(20, 30));
    }

    #[test]
    fn the_hour_is_folded_from_half_hours_not_taken_from_the_clock_hour() {
        let natives = [
            Timeframe::MINUTE_1,
            Timeframe::MINUTE_5,
            Timeframe::MINUTE_15,
            tf(30, TimeUnit::Minute),
            Timeframe::HOUR_1,
            Timeframe::DAY_1,
            Timeframe::WEEK_1,
            Timeframe::MONTH_1,
        ];
        let grid = Grid::UsEquities;
        let half_hour = tf(30, TimeUnit::Minute);
        for target in ["1h", "2h", "4h"] {
            assert_eq!(
                grid.fold_base(tf_text(target), &natives),
                Some(half_hour),
                "{target}"
            );
        }
        assert_eq!(grid.fold_base(half_hour, &natives), Some(half_hour));
        assert_eq!(
            grid.fold_base(tf_text("45m"), &natives),
            Some(Timeframe::MINUTE_15)
        );
        assert_eq!(
            grid.fold_base(tf_text("22m"), &natives),
            Some(Timeframe::MINUTE_1)
        );
        // The same list on the UTC grid takes the clock hour.
        assert_eq!(
            Grid::Utc.fold_base(Timeframe::HOUR_1, &natives),
            Some(Timeframe::HOUR_1)
        );
        // Days and longer are not cut on the session.
        assert_eq!(
            grid.fold_base(Timeframe::WEEK_1, &natives),
            Some(Timeframe::WEEK_1)
        );
    }

    #[test]
    fn a_day_a_week_and_a_month_stop_trading_at_the_close_of_their_last_weekday() {
        let grid = Grid::UsEquities;
        // Tuesday 24 Nov 2026: the day's bucket is the UTC day, trading stops at 16:00.
        let tuesday = day(2026, 11, 24);
        assert_eq!(
            grid.session_end_ms(Timeframe::DAY_1, tuesday),
            winter(21, 0)
        );
        // The week of Monday 23 Nov ends on Friday the 27th at 16:00 (not Monday midnight).
        let monday = day(2026, 11, 23);
        assert_eq!(
            grid.session_end_ms(Timeframe::WEEK_1, monday),
            at(2026, 11, 27, 21, 0)
        );
        // November 2026 ends on Monday the 30th.
        assert_eq!(
            grid.session_end_ms(Timeframe::MONTH_1, day(2026, 11, 1)),
            at(2026, 11, 30, 21, 0)
        );
        // On the UTC grid and for intraday buckets it is where the next one opens.
        assert_eq!(
            Grid::Utc.session_end_ms(Timeframe::DAY_1, tuesday),
            tuesday + DAY_MS
        );
        assert_eq!(
            grid.session_end_ms(tf(4, TimeUnit::Hour), winter(18, 30)),
            winter(21, 0)
        );
    }

    #[test]
    fn the_candle_after_a_stocks_bucket_is_the_next_one_that_trades() {
        let grid = Grid::UsEquities;
        let following = |timeframe, start| grid.following_end_ms(timeframe, start);
        // The week of Monday 9 Nov 2026 stops on Friday; the next one on Friday the 20th.
        assert_eq!(
            following(Timeframe::WEEK_1, day(2026, 11, 9)),
            at(2026, 11, 20, 21, 0)
        );
        // Friday's day is followed by Monday's, past the weekend.
        assert_eq!(
            following(Timeframe::DAY_1, day(2026, 11, 20)),
            at(2026, 11, 23, 21, 0)
        );
        // October 2026 ends on Friday the 30th; November on Monday the 30th.
        assert_eq!(
            following(Timeframe::MONTH_1, day(2026, 10, 1)),
            at(2026, 11, 30, 21, 0)
        );
        // Intraday: the next bucket of the session, else the first of the next session.
        let four_hours = tf(4, TimeUnit::Hour);
        assert_eq!(following(four_hours, winter(14, 30)), winter(21, 0));
        assert_eq!(
            following(Timeframe::HOUR_1, at(2026, 11, 20, 20, 30)),
            at(2026, 11, 23, 15, 30)
        );
        // The UTC grid has no closing hours.
        assert_eq!(
            Grid::Utc.following_end_ms(Timeframe::WEEK_1, day(2026, 11, 9)),
            day(2026, 11, 23)
        );
    }

    /// Every minute of the weeks around both clock changes of 2026 (8 March and 1 November) and a
    /// plain one, for lengths that divide the half hour and ones that do not: the buckets tile the
    /// session exactly, whatever the offset.
    #[test]
    fn the_buckets_tile_every_session_through_the_clock_changes() {
        let lengths = [
            "1m", "5m", "15m", "22m", "30m", "45m", "1h", "2h", "3h", "4h", "6h",
        ];
        let weeks = [
            at(2026, 3, 2, 0, 0),
            at(2026, 3, 9, 0, 0),
            at(2026, 10, 26, 0, 0),
            at(2026, 11, 2, 0, 0),
            at(2026, 7, 6, 0, 0),
        ];
        let grid = Grid::UsEquities;
        for text in lengths {
            let timeframe = tf_text(text);
            for week in weeks {
                for minute in 0..7 * 24 * 60 {
                    let t = week + minute * MINUTE_MS;
                    let start = grid.bucket_start_ms(timeframe, t);
                    let next = grid.next_bucket_start_ms(timeframe, t);
                    assert!(start <= t, "{text} at {t}: starts after the moment");
                    assert!(next > start, "{text} at {t}: ends before it starts");
                    if !grid.is_traded(timeframe, t) {
                        continue;
                    }
                    assert!(
                        grid.bucket_start_ms(timeframe, start) == start,
                        "{text} at {t}: the start is not on the grid"
                    );
                    // Inside the session: the bucket stays inside it and holds the moment.
                    let day = crate::model::market::session::local_day(t);
                    let open = crate::model::market::session::open_ms(day);
                    let close = crate::model::market::session::close_ms(day);
                    assert!(
                        open <= start && next <= close,
                        "{text} at {t}: leaves the session"
                    );
                    assert!(t < next, "{text} at {t}: ends before the moment");
                    assert_eq!((start - open) % timeframe.nominal_ms(), 0, "{text} at {t}");
                    if next < close {
                        assert_eq!(
                            grid.bucket_start_ms(timeframe, next),
                            next,
                            "{text} at {t}: the next bucket opens where this one ends"
                        );
                    } else {
                        assert_eq!(
                            next, close,
                            "{text} at {t}: the last bucket ends at the close"
                        );
                    }
                }
            }
        }
    }

    /// One minute candles of five sessions across the clock change of 1 November 2026, from 08:00 to
    /// 20:00 New York (the pre and post market included), with prices and volumes that differ.
    fn minutes_around_the_clock_change() -> Vec<Kline> {
        let session = crate::model::market::session::open_ms;
        let mut minutes = Vec::new();
        let mut index = 0;
        for (month, date) in [(10, 29), (10, 30), (11, 2), (11, 3), (11, 4)] {
            let open = session(days_from_civil(2026, month, date));
            for minute in -90..(390 + 240) {
                let mut candle = candle(
                    open + minute * MINUTE_MS,
                    MINUTE_MS,
                    100.0 + (index % 37) as f64,
                );
                candle.volume = 1.0 + (index % 7) as f64;
                minutes.push(candle);
                index += 1;
            }
        }
        minutes
    }

    #[test]
    fn folding_from_a_coarser_base_gives_the_same_candles_as_folding_the_minutes() {
        let grid = Grid::UsEquities;
        let minutes = minutes_around_the_clock_change();
        for (base, targets) in [
            ("5m", vec!["15m", "30m"]),
            ("15m", vec!["30m", "1h"]),
            ("30m", vec!["1h", "2h", "3h", "4h", "6h"]),
        ] {
            let base_candles = grid.fold(&minutes, tf_text(base), false);
            for target in targets {
                let direct = grid.fold(&minutes, tf_text(target), false);
                let nested = grid.fold(&base_candles, tf_text(target), false);
                assert_eq!(direct, nested, "{base} then {target}");
            }
        }
    }

    #[test]
    fn a_fold_keeps_every_regular_hours_minute_and_nothing_else() {
        let grid = Grid::UsEquities;
        let minutes = minutes_around_the_clock_change();
        let regular: f64 = minutes
            .iter()
            .filter(|candle| crate::model::market::session::is_open(candle.open_time))
            .map(|candle| candle.volume)
            .sum();
        for target in ["1m", "5m", "22m", "30m", "1h", "4h", "6h"] {
            let folded = grid.fold(&minutes, tf_text(target), false);
            let volume: f64 = folded.iter().map(|candle| candle.volume).sum();
            assert_eq!(volume, regular, "{target}");
            assert!(
                folded
                    .iter()
                    .all(|candle| crate::model::market::session::is_open(candle.open_time)),
                "{target}: a candle opens outside the session"
            );
            // Buckets are ascending and never overlap.
            assert!(
                folded
                    .windows(2)
                    .all(|pair| pair[0].close_time < pair[1].open_time),
                "{target}"
            );
        }
    }

    #[test]
    fn a_stocks_first_bucket_of_a_cut_short_answer_is_dropped_like_any_other() {
        let thirty = 30 * MINUTE_MS;
        // The answer starts at 11:00 New York: the 09:30 to 13:30 bucket is a fragment.
        let candles = series(winter(16, 0), thirty, 12);
        let kept = Grid::UsEquities.fold(&candles, tf(4, TimeUnit::Hour), true);
        assert_eq!(opens(&kept), [winter(18, 30)]);
        let all = Grid::UsEquities.fold(&candles, tf(4, TimeUnit::Hour), false);
        assert_eq!(opens(&all), [winter(14, 30), winter(18, 30)]);
    }
}

#[test]
fn a_grid_that_drops_candles_asks_for_more_of_them() {
    let natives = [
        Timeframe::MINUTE_1,
        tf(30, TimeUnit::Minute),
        Timeframe::HOUR_1,
        Timeframe::DAY_1,
    ];
    let hour = |grid| FoldPlan::in_grid(grid, Timeframe::HOUR_1, &natives).unwrap();
    // The UTC grid takes the clock hour itself: four of them are four candles.
    assert_eq!(hour(Grid::Utc).base_count(4), 4);
    // The session grid folds half hours (two to the hour) and asks for three times as many, so a
    // newest answer made of post market candles still holds the session.
    assert_eq!(hour(Grid::UsEquities).base_count(4), 3 * 5 * 2);
    // Days are not cut on the session: nothing is left out of them.
    let day = FoldPlan::in_grid(Grid::UsEquities, Timeframe::DAY_1, &natives).unwrap();
    assert_eq!(day.base_count(4), 4);
}
