use super::*;
use crate::pine::calendar::make;

const HOUR: i64 = 3_600_000;

fn ms(year: i64, month: i64, day: i64, hour: i64, minute: i64) -> i64 {
    make(year, month, day, hour, minute, 0, 0).expect("a date")
}

fn bar(open_time: i64, length: i64, o: f64, h: f64, l: f64, c: f64) -> Kline {
    Kline {
        open_time,
        close_time: open_time + length - 1,
        open: o,
        high: h,
        low: l,
        close: c,
        volume: 1.0,
    }
}

/// Daily bars from Monday 2024-01-01, one `(open, high, low, close)` per consecutive day.
fn daily(rows: &[(f64, f64, f64, f64)]) -> Vec<Kline> {
    rows.iter()
        .enumerate()
        .map(|(i, &(o, h, l, c))| bar(ms(2024, 1, 1 + i as i64, 0, 0), DAY_MS, o, h, l, c))
        .collect()
}

fn run(bars: &[Kline], outcome: Outcome, filter: &Filter) -> Stats {
    compute(bars, DAY_MS, MarketHours::Continuous, outcome, filter).expect("an answer")
}

#[test]
fn wilson_interval_matches_the_published_example() {
    // edge-stats' README: 24 of 30 is 80.0%, 95% CI [62.7%, 90.5%].
    let (low, high) = Estimate { hits: 24, n: 30 }.interval().unwrap();
    assert!((low - 0.627).abs() < 0.001, "{low}");
    assert!((high - 0.905).abs() < 0.001, "{high}");
}

#[test]
fn an_empty_sample_has_no_estimate_and_a_full_one_stays_inside_zero_and_one() {
    assert_eq!(Estimate { hits: 0, n: 0 }.proportion(), None);
    assert_eq!(Estimate { hits: 0, n: 0 }.interval(), None);
    let (low, high) = Estimate { hits: 5, n: 5 }.interval().unwrap();
    assert!(low > 0.0 && high <= 1.0);
    let (low, high) = Estimate { hits: 0, n: 5 }.interval().unwrap();
    assert!(low >= 0.0 && high < 1.0);
}

#[test]
fn gap_fill_counts_days_whose_range_touches_the_previous_close() {
    let bars = daily(&[
        (100.0, 101.0, 99.0, 100.0),
        // Gap up 2: the low reaches 100, filled.
        (102.0, 103.0, 99.5, 101.0),
        // Gap up 4: the low stays at 103, not filled.
        (105.0, 106.0, 103.0, 105.5),
        // Gap down 5.5: the high reaches 105.5, filled.
        (100.0, 105.5, 99.0, 101.0),
        // No gap: left out.
        (101.0, 102.0, 100.0, 101.5),
    ]);
    let stats = run(&bars, Outcome::GapFill, &Filter::default());
    assert_eq!(stats.estimate, Estimate { hits: 2, n: 3 });
    assert!(!stats.intraday);
    assert_eq!(stats.sessions, 5);
    assert_eq!(stats.timing, None, "daily bars cannot say when");
}

#[test]
fn gap_filters_narrow_by_side_and_size() {
    let bars = daily(&[
        (100.0, 101.0, 99.0, 100.0),
        (102.0, 103.0, 99.5, 101.0),   // +2.0%, filled
        (101.0, 102.0, 100.0, 101.0),  // 0.0% from 101: left out
        (103.02, 104.0, 102.0, 103.0), // +2.0%, not filled (low 102 > 101)
        (100.0, 104.0, 99.0, 100.0),   // -2.9%, filled
    ]);
    let up = Filter {
        gap: Side::Up,
        ..Filter::default()
    };
    assert_eq!(
        run(&bars, Outcome::GapFill, &up).estimate,
        Estimate { hits: 1, n: 2 }
    );
    let down = Filter {
        gap: Side::Down,
        ..Filter::default()
    };
    assert_eq!(
        run(&bars, Outcome::GapFill, &down).estimate,
        Estimate { hits: 1, n: 1 }
    );
    let small = Filter {
        gap_min_pct: Some(1.0),
        gap_max_pct: Some(2.5),
        ..Filter::default()
    };
    assert_eq!(
        run(&bars, Outcome::GapFill, &small).estimate,
        Estimate { hits: 1, n: 2 }
    );
}

#[test]
fn weekday_filter_uses_the_session_date() {
    // 2024-01-01 is a Monday; five days, Monday to Friday, alternate up and down.
    let bars = daily(&[
        (1.0, 2.0, 0.5, 1.5),
        (1.5, 2.0, 0.5, 1.0),
        (1.0, 2.0, 0.5, 1.5),
        (1.5, 2.0, 0.5, 1.0),
        (1.0, 2.0, 0.5, 1.5),
    ]);
    assert_eq!(
        run(&bars, Outcome::DayUp, &Filter::default()).estimate,
        Estimate { hits: 3, n: 5 }
    );
    let tuesdays = Filter {
        weekdays: WeekdaySet::ALL
            .with(0, false)
            .with(2, false)
            .with(3, false)
            .with(4, false)
            .with(5, false)
            .with(6, false),
        ..Filter::default()
    };
    assert_eq!(tuesdays.weekdays.names(), ["Tue"]);
    assert_eq!(
        run(&bars, Outcome::DayUp, &tuesdays).estimate,
        Estimate { hits: 0, n: 1 }
    );
}

#[test]
fn a_flat_day_is_not_up() {
    let bars = daily(&[(1.0, 2.0, 0.5, 1.0)]);
    assert_eq!(
        run(&bars, Outcome::DayUp, &Filter::default()).estimate,
        Estimate { hits: 0, n: 1 }
    );
}

#[test]
fn prior_day_breaks_compare_with_the_day_before_and_need_one() {
    let bars = daily(&[
        (10.0, 11.0, 9.0, 10.5),
        (10.5, 11.5, 9.5, 11.0), // high above 11 and low above 9: high break only
        (11.0, 11.2, 8.0, 9.0),  // low below 9.5: low break only
    ]);
    let high = run(&bars, Outcome::PriorHighBreak, &Filter::default());
    assert_eq!(high.estimate, Estimate { hits: 1, n: 2 });
    let low = run(&bars, Outcome::PriorLowBreak, &Filter::default());
    assert_eq!(low.estimate, Estimate { hits: 1, n: 2 });
}

#[test]
fn prior_day_condition_reads_the_previous_sessions_direction() {
    let bars = daily(&[
        (10.0, 11.0, 9.0, 10.5), // up
        (10.5, 11.5, 9.5, 10.0), // down; previous up
        (10.0, 12.0, 9.0, 11.0), // up; previous down
    ]);
    let after_up = Filter {
        prior_day: Side::Up,
        ..Filter::default()
    };
    let stats = run(&bars, Outcome::DayUp, &after_up);
    assert_eq!(stats.estimate, Estimate { hits: 0, n: 1 });
    let after_down = Filter {
        prior_day: Side::Down,
        ..Filter::default()
    };
    assert_eq!(
        run(&bars, Outcome::DayUp, &after_down).estimate,
        Estimate { hits: 1, n: 1 }
    );
}

#[test]
fn a_hole_in_the_data_is_not_compared_across() {
    let mut bars = daily(&[(10.0, 11.0, 9.0, 10.5)]);
    // Ten days later.
    bars.push(bar(ms(2024, 1, 12, 0, 0), DAY_MS, 10.5, 12.0, 10.0, 11.0));
    let stats = run(&bars, Outcome::PriorHighBreak, &Filter::default());
    assert_eq!(stats.estimate.n, 0);
}

#[test]
fn bars_longer_than_a_day_and_empty_input_are_refused() {
    let bars = daily(&[(1.0, 2.0, 0.5, 1.5)]);
    assert_eq!(
        compute(
            &bars,
            7 * DAY_MS,
            MarketHours::Continuous,
            Outcome::DayUp,
            &Filter::default()
        ),
        Err(Unavailable::TooCoarse)
    );
    assert_eq!(
        compute(
            &[],
            DAY_MS,
            MarketHours::Continuous,
            Outcome::DayUp,
            &Filter::default()
        ),
        Err(Unavailable::NoBars)
    );
    assert_eq!(
        compute(
            &bars,
            DAY_MS,
            MarketHours::Continuous,
            Outcome::FirstBarHighBreak,
            &Filter::default()
        ),
        Err(Unavailable::NeedsIntraday)
    );
}

#[test]
fn bars_that_are_not_numbers_are_left_out() {
    let mut bars = daily(&[(1.0, 2.0, 0.5, 1.5), (1.0, 2.0, 0.5, 1.5)]);
    bars[1].close = f64::NAN;
    let stats = run(&bars, Outcome::DayUp, &Filter::default());
    assert_eq!(stats.sessions, 1);
}

/// Hourly bars of one UTC day starting at `hour`: `(high, low)` per bar, open and close mid.
fn hourly_day(day: i64, first_hour: i64, rows: &[(f64, f64)]) -> Vec<Kline> {
    rows.iter()
        .enumerate()
        .map(|(i, &(h, l))| {
            bar(
                ms(2024, 1, day, first_hour + i as i64, 0),
                HOUR,
                (h + l) / 2.0,
                h,
                l,
                (h + l) / 2.0,
            )
        })
        .collect()
}

#[test]
fn hourly_bars_fold_into_utc_days_and_report_the_bar_a_break_happened_in() {
    let mut bars = hourly_day(2, 0, &[(10.0, 9.0), (10.5, 9.5), (10.2, 8.5), (10.1, 9.9)]);
    bars.extend(hourly_day(
        3,
        0,
        &[(10.0, 9.0), (9.8, 9.2), (9.9, 9.1), (9.7, 9.3)],
    ));
    bars.extend(hourly_day(
        4,
        0,
        &[(10.0, 9.0), (9.8, 9.2), (9.9, 9.1), (10.4, 9.3)],
    ));
    let stats = compute(
        &bars,
        HOUR,
        MarketHours::Continuous,
        Outcome::FirstBarHighBreak,
        &Filter::default(),
    )
    .unwrap();
    assert!(stats.intraday);
    assert_eq!(stats.sessions, 3);
    // Day 2 breaks in bar 1, day 3 never, day 4 in bar 3.
    assert_eq!(stats.estimate, Estimate { hits: 2, n: 3 });
    let timing = stats.timing.unwrap();
    assert_eq!(
        (timing.n, timing.p25, timing.median, timing.p90),
        (2, 1, 1, 3)
    );
    let low = compute(
        &bars,
        HOUR,
        MarketHours::Continuous,
        Outcome::FirstBarLowBreak,
        &Filter::default(),
    )
    .unwrap();
    assert_eq!(low.estimate, Estimate { hits: 1, n: 3 });
}

#[test]
fn a_session_of_one_bar_has_no_first_bar_range() {
    let bars = hourly_day(2, 0, &[(10.0, 9.0)]);
    let stats = compute(
        &bars,
        HOUR,
        MarketHours::Continuous,
        Outcome::FirstBarHighBreak,
        &Filter::default(),
    )
    .unwrap();
    assert_eq!(stats.estimate.n, 0);
}

#[test]
fn gap_fill_on_hourly_bars_reports_the_bar_that_filled_it() {
    // Day 1 closes at 100; day 2 opens at 102 and its second bar reaches 100.
    let mut bars = hourly_day(
        1,
        20,
        &[(101.0, 99.0), (100.5, 99.5), (101.0, 100.0), (100.5, 99.5)],
    );
    let last = bars.last_mut().unwrap();
    last.close = 100.0;
    // 02 Jan 00:00 onward.
    bars.push(bar(ms(2024, 1, 2, 0, 0), HOUR, 102.0, 103.0, 101.5, 102.5));
    bars.push(bar(ms(2024, 1, 2, 1, 0), HOUR, 102.5, 103.0, 99.0, 100.0));
    let stats = compute(
        &bars,
        HOUR,
        MarketHours::Continuous,
        Outcome::GapFill,
        &Filter::default(),
    )
    .unwrap();
    assert_eq!(stats.estimate, Estimate { hits: 1, n: 1 });
    assert_eq!(stats.timing.unwrap().median, 1);
}

#[test]
fn a_us_stock_session_is_the_regular_hours_in_new_york() {
    // January: New York is UTC-5, so 09:30 is 14:30 UTC. Hourly bars at 14:00 (covering the open)
    // through 20:00 UTC; 21:00 UTC (16:00 New York) and 13:00 UTC (08:00) are extended hours.
    let mut bars = vec![bar(ms(2024, 1, 2, 13, 0), HOUR, 1.0, 50.0, 0.5, 1.0)];
    for hour in 14..=20 {
        bars.push(bar(ms(2024, 1, 2, hour, 0), HOUR, 10.0, 11.0, 9.0, 10.5));
    }
    bars.push(bar(ms(2024, 1, 2, 21, 0), HOUR, 1.0, 60.0, 0.1, 1.0));
    let days = sessions(&bars, HOUR, MarketHours::UsStock, true);
    assert_eq!(days.len(), 1);
    let day = days[0];
    assert_eq!(day.date, (2024, 1, 2));
    assert_eq!(day.weekday, 1, "a Tuesday");
    assert_eq!((day.start, day.end), (1, 8));
    assert_eq!(
        (day.open, day.high, day.low, day.close),
        (10.0, 11.0, 9.0, 10.5)
    );
}

#[test]
fn a_us_stock_session_follows_daylight_saving() {
    // July: New York is UTC-4, so the 13:00 UTC bar is 09:00 and covers the 09:30 open.
    let bars = vec![
        bar(ms(2024, 7, 2, 12, 0), HOUR, 1.0, 2.0, 0.5, 1.0),
        bar(ms(2024, 7, 2, 13, 0), HOUR, 10.0, 11.0, 9.0, 10.5),
    ];
    let days = sessions(&bars, HOUR, MarketHours::UsStock, true);
    assert_eq!(days.len(), 1);
    assert_eq!(days[0].start, 1);
}

#[test]
fn a_us_stock_has_no_weekend_sessions_and_notes_the_boundary_open() {
    let saturday = bar(ms(2024, 1, 6, 15, 0), HOUR, 10.0, 11.0, 9.0, 10.5);
    let stats = compute(
        &[saturday],
        HOUR,
        MarketHours::UsStock,
        Outcome::DayUp,
        &Filter::default(),
    );
    assert_eq!(stats, Err(Unavailable::NoBars));
    let tuesday = bar(ms(2024, 1, 2, 15, 0), HOUR, 10.0, 11.0, 9.0, 10.5);
    let stats = compute(
        &[tuesday],
        HOUR,
        MarketHours::UsStock,
        Outcome::DayUp,
        &Filter::default(),
    )
    .unwrap();
    assert!(stats.notes.contains(&Note::BoundaryOpen));
}

#[test]
fn a_continuous_market_with_no_gaps_says_why() {
    let bars = daily(&[(100.0, 101.0, 99.0, 100.0), (100.0, 101.0, 99.0, 100.5)]);
    let stats = run(&bars, Outcome::GapFill, &Filter::default());
    assert_eq!(stats.estimate.n, 0);
    assert_eq!(stats.notes, [Note::NoGaps]);
}

#[test]
fn halves_are_shown_only_with_enough_days_and_compare_by_interval_overlap() {
    // 40 daily bars alternating up and down: 50% in both halves.
    let rows: Vec<_> = (0..40)
        .map(|i| {
            if i % 2 == 0 {
                (1.0, 2.0, 0.5, 1.5)
            } else {
                (1.5, 2.0, 0.5, 1.0)
            }
        })
        .collect();
    let stats = run(&daily(&rows), Outcome::DayUp, &Filter::default());
    let halves = stats.stability.expect("20 days a half");
    assert_eq!((halves.older.n, halves.newer.n), (20, 20));
    assert!(halves.agree);

    let short = run(&daily(&rows[..15]), Outcome::DayUp, &Filter::default());
    assert_eq!(short.stability, None);

    // All up for the older half, all down for the newer: the halves disagree.
    let split: Vec<_> = (0..40)
        .map(|i| {
            if i < 20 {
                (1.0, 2.0, 0.5, 1.5)
            } else {
                (1.5, 2.0, 0.5, 1.0)
            }
        })
        .collect();
    let stats = run(&daily(&split), Outcome::DayUp, &Filter::default());
    assert!(!stats.stability.unwrap().agree);
}

#[test]
fn counts_are_grouped_by_year_in_order() {
    let mut bars = vec![bar(ms(2023, 12, 29, 0, 0), DAY_MS, 1.0, 2.0, 0.5, 1.5)];
    bars.push(bar(ms(2024, 1, 2, 0, 0), DAY_MS, 1.5, 2.0, 0.5, 1.0));
    bars.push(bar(ms(2024, 1, 3, 0, 0), DAY_MS, 1.0, 2.0, 0.5, 1.5));
    let stats = run(&bars, Outcome::DayUp, &Filter::default());
    assert_eq!(
        stats.per_year,
        [
            YearRow {
                year: 2023,
                estimate: Estimate { hits: 1, n: 1 }
            },
            YearRow {
                year: 2024,
                estimate: Estimate { hits: 1, n: 2 }
            },
        ]
    );
    assert_eq!(stats.span, Some(((2023, 12, 29), (2024, 1, 3))));
}

#[test]
fn names_slugs_and_descriptions_round_trip() {
    let slugs: Vec<_> = Outcome::ALL.iter().map(|outcome| outcome.slug()).collect();
    assert_eq!(
        slugs,
        [
            "gap_fill",
            "day_up",
            "prior_high_break",
            "prior_low_break",
            "first_bar_high_break",
            "first_bar_low_break"
        ]
    );
    assert_eq!(
        WeekdaySet::from_names(&["mon", "FRI"]).unwrap().names(),
        ["Mon", "Fri"]
    );
    assert_eq!(WeekdaySet::from_names(&["funday"]), None);
    assert_eq!(Filter::default().describe(), "every day");
    let filter = Filter {
        weekdays: WeekdaySet::from_names(&["Tue"]).unwrap(),
        gap: Side::Up,
        gap_min_pct: Some(0.1),
        gap_max_pct: Some(0.6),
        prior_day: Side::Down,
    };
    assert_eq!(filter.describe(), "Tue · gap up 0.1%–0.6% · prior day down");
    assert!(filter.is_narrowed() && !Filter::default().is_narrowed());
}

#[test]
fn alpaca_is_a_us_stock_and_crypto_is_continuous() {
    use crate::model::DataSource;
    assert_eq!(
        MarketHours::for_source(DataSource::Alpaca),
        MarketHours::UsStock
    );
    assert_eq!(
        MarketHours::for_source(DataSource::Binance),
        MarketHours::Continuous
    );
}
