use super::*;
use crate::format::days_from_civil;
use crate::model::TimeUnit;
use crate::model::chart::kind::ChartGroup;

fn utc(year: i64, month: i64, day: i64, hour: i64, minute: i64, second: i64) -> i64 {
    days_from_civil(year, month, day) * DAY_MS
        + hour * HOUR_MS
        + minute * MINUTE_MS
        + second * SECOND_MS
}

fn candle(timeframe: Timeframe, at_ms: i64) -> Kline {
    let open_time = bucket_start_ms(timeframe, at_ms);
    Kline {
        open_time,
        close_time: next_bucket_start_ms(timeframe, open_time) - 1,
        open: 1.0,
        high: 1.0,
        low: 1.0,
        close: 1.0,
        volume: 1.0,
    }
}

fn input<'a>(timeframe: Timeframe, newest: Option<&'a Kline>, now_ms: i64) -> CountdownInput<'a> {
    CountdownInput {
        chart_type: ChartType::Candles,
        source: DataSource::Binance,
        timeframe,
        newest,
        now_ms,
        data_age_ms: Some(1_000),
        refresh_ms: 5_000,
        replay: false,
    }
}

fn remaining(timeframe: Timeframe, now_ms: i64) -> Option<i64> {
    let newest = candle(timeframe, now_ms);
    Countdown::resolve(&input(timeframe, Some(&newest), now_ms)).map(|count| count.remaining_ms)
}

#[test]
fn a_bar_under_an_hour_reads_minutes_and_seconds() {
    assert_eq!(format(9 * SECOND_MS), "00:09");
    assert_eq!(format(45 * SECOND_MS), "00:45");
    assert_eq!(format(4 * MINUTE_MS + 32 * SECOND_MS), "04:32");
    assert_eq!(format(HOUR_MS - 1), "59:59");
}

#[test]
fn seconds_count_down_to_zero_and_never_below() {
    assert_eq!(format(1_999), "00:01");
    assert_eq!(format(1_000), "00:01");
    assert_eq!(format(999), "00:00");
    assert_eq!(format(0), "00:00");
    assert_eq!(format(-5_000), "00:00");
    assert_eq!(format(i64::MIN), "00:00");
}

#[test]
fn a_bar_under_a_day_reads_hours_too() {
    assert_eq!(format(HOUR_MS), "01:00:00");
    assert_eq!(format(HOUR_MS + 24 * MINUTE_MS), "01:24:00");
    assert_eq!(format(3 * HOUR_MS + 42 * MINUTE_MS), "03:42:00");
    // TradingView on a daily bar, 2 h 36 min 39 s from the close.
    assert_eq!(
        format(2 * HOUR_MS + 36 * MINUTE_MS + 39 * SECOND_MS),
        "02:36:39"
    );
    assert_eq!(format(DAY_MS - 1), "23:59:59");
}

#[test]
fn a_bar_of_a_day_or_more_reads_days_and_hours_rounded_up() {
    assert_eq!(format(DAY_MS), "1d 0h");
    assert_eq!(format(2 * DAY_MS + 14 * HOUR_MS), "2d 14h");
    assert_eq!(format(18 * DAY_MS + 6 * HOUR_MS), "18d 6h");
    // TradingView on a weekly bar, 4 days 2 h 36 min from the close, and a monthly one.
    let left = 4 * DAY_MS + 2 * HOUR_MS + 36 * MINUTE_MS + 40 * SECOND_MS;
    assert_eq!(format(left), "4d 3h");
    assert_eq!(format(5 * DAY_MS + 2 * HOUR_MS + 36 * MINUTE_MS), "5d 3h");
    assert_eq!(format(i64::MAX), format(i64::MAX));
}

#[test]
fn the_text_changes_where_the_next_repaint_is_asked() {
    for remaining in [
        1,
        999,
        1_000,
        1_001,
        61_500,
        HOUR_MS,
        DAY_MS - 1,
        DAY_MS,
        DAY_MS + 1,
        3 * DAY_MS + 2 * HOUR_MS + 36 * MINUTE_MS,
    ] {
        let wait = next_repaint(remaining).as_millis() as i64;
        assert!(wait >= 1, "{remaining}");
        assert_eq!(
            format(remaining - wait + 1),
            format(remaining),
            "the text holds until just before the repaint ({remaining})"
        );
        if remaining < SECOND_MS {
            // The last second: the next text is the next bar's, and the repaint is asked for the
            // moment after the close.
            assert!(wait > remaining, "{remaining}");
        } else {
            assert_ne!(
                format(remaining - wait),
                format(remaining),
                "and is different at it ({remaining})"
            );
        }
    }
    assert_eq!(next_repaint(-3), Duration::from_millis(1));
}

#[test]
fn every_timeframe_closes_where_the_bucket_grid_does() {
    let now = utc(2026, 11, 25, 21, 23, 20) + 400;
    assert_eq!(remaining(Timeframe::MINUTE_5, now), Some(99_600));
    assert_eq!(remaining(Timeframe::MINUTE_1, now), Some(39_600));
    assert_eq!(
        remaining(Timeframe::HOUR_1, now),
        Some(36 * MINUTE_MS + 39_600)
    );
    // TradingView at the same moment: a daily bar `02:36:39`, a weekly one `4d 3h`, a monthly
    // one `5d 3h`.
    let day = remaining(Timeframe::DAY_1, now).expect("daily");
    assert_eq!(format(day), "02:36:39");
    let week = remaining(Timeframe::WEEK_1, now).expect("weekly");
    assert_eq!(format(week), "4d 3h");
    let month = remaining(Timeframe::MONTH_1, now).expect("monthly");
    assert_eq!(format(month), "5d 3h");
}

#[test]
fn months_close_on_their_own_length_and_the_year_turns() {
    let february = utc(2028, 2, 10, 0, 0, 0);
    assert_eq!(
        remaining(Timeframe::MONTH_1, february),
        Some(utc(2028, 3, 1, 0, 0, 0) - february),
        "a leap February has 29 days"
    );
    let december = utc(2026, 12, 31, 23, 0, 0);
    assert_eq!(remaining(Timeframe::MONTH_1, december), Some(HOUR_MS));
    assert_eq!(remaining(Timeframe::YEAR_1, december), Some(HOUR_MS));
    let quarter = utc(2026, 11, 25, 0, 0, 0);
    assert_eq!(
        remaining(Timeframe::new(3, TimeUnit::Month).expect("3M"), quarter),
        Some(utc(2027, 1, 1, 0, 0, 0) - quarter)
    );
}

#[test]
fn a_week_closes_on_monday_midnight_utc() {
    let wednesday = utc(2026, 11, 25, 12, 0, 0);
    assert_eq!(
        remaining(Timeframe::WEEK_1, wednesday),
        Some(utc(2026, 11, 30, 0, 0, 0) - wednesday)
    );
}

#[test]
fn an_intraday_bar_is_cut_short_at_midnight() {
    let timeframe = Timeframe::new(22, TimeUnit::Minute).expect("22m");
    let late = utc(2026, 11, 25, 23, 50, 0);
    assert_eq!(remaining(timeframe, late), Some(10 * MINUTE_MS));
}

#[test]
fn the_count_is_exact_at_a_close_and_just_after_it() {
    let close = utc(2026, 11, 25, 21, 25, 0);
    assert_eq!(remaining(Timeframe::MINUTE_5, close - 1), Some(1));
    assert_eq!(
        remaining(Timeframe::MINUTE_5, close),
        Some(5 * MINUTE_MS),
        "the next bar opens at the close"
    );
}

#[test]
fn the_count_rolls_over_before_the_next_candle_arrives() {
    let timeframe = Timeframe::MINUTE_5;
    let behind = candle(timeframe, utc(2026, 11, 25, 21, 20, 0));
    let now = utc(2026, 11, 25, 21, 25, 3);
    let count = Countdown::resolve(&input(timeframe, Some(&behind), now)).expect("counts");
    assert_eq!(count.remaining_ms, 5 * MINUTE_MS - 3 * SECOND_MS);
    assert_eq!(count.close_ms, utc(2026, 11, 25, 21, 30, 0));
}

#[test]
fn a_market_with_no_trade_in_the_bar_still_counts() {
    let timeframe = Timeframe::MINUTE_1;
    let quiet = candle(timeframe, utc(2026, 11, 25, 20, 0, 0));
    let now = utc(2026, 11, 25, 21, 25, 30);
    assert!(Countdown::resolve(&input(timeframe, Some(&quiet), now)).is_some());
}

#[test]
fn the_count_is_a_function_of_the_clock_alone() {
    let timeframe = Timeframe::MINUTE_5;
    let newest = candle(timeframe, utc(2026, 11, 25, 21, 20, 0));
    let at =
        |now| Countdown::resolve(&input(timeframe, Some(&newest), now)).map(|c| c.remaining_ms);
    let start = utc(2026, 11, 25, 21, 21, 0);
    // A missed frame, a sleep and a clock set back change nothing but the instant asked about.
    assert_eq!(at(start), Some(4 * MINUTE_MS));
    assert_eq!(at(start + 90 * SECOND_MS), Some(150 * SECOND_MS));
    assert_eq!(at(start), Some(4 * MINUTE_MS));
}

#[test]
fn a_provider_off_the_grid_shows_nothing() {
    let timeframe = Timeframe::MINUTE_5;
    let now = utc(2026, 11, 25, 21, 23, 0);
    let mut newest = candle(timeframe, now);
    newest.close_time += 60_000;
    assert_eq!(
        Countdown::resolve(&input(timeframe, Some(&newest), now)),
        None
    );
    newest.close_time -= 60_000 - 999;
    assert!(Countdown::resolve(&input(timeframe, Some(&newest), now)).is_some());
}

#[test]
fn nothing_loaded_stale_or_replayed_shows_nothing() {
    let timeframe = Timeframe::MINUTE_5;
    let now = utc(2026, 11, 25, 21, 23, 0);
    let newest = candle(timeframe, now);
    let show = |change: &dyn Fn(&mut CountdownInput)| {
        let mut data = input(timeframe, Some(&newest), now);
        change(&mut data);
        Countdown::resolve(&data)
    };
    assert!(show(&|_| {}).is_some());
    assert_eq!(show(&|data| data.newest = None), None, "no candles");
    assert_eq!(show(&|data| data.replay = true), None, "a replay");
    assert_eq!(
        show(&|data| data.data_age_ms = None),
        None,
        "nothing ever arrived"
    );
    assert!(show(&|data| data.data_age_ms = Some(-1_000)).is_some());
    assert_eq!(
        show(&|data| data.data_age_ms = Some(-3_600_000)),
        None,
        "a clock set back an hour"
    );
    assert!(show(&|data| data.data_age_ms = Some(14_999)).is_some());
    assert_eq!(
        show(&|data| data.data_age_ms = Some(15_001)),
        None,
        "three refreshes without an answer"
    );
}

#[test]
fn a_slow_source_is_given_its_own_time_to_answer() {
    let timeframe = Timeframe::HOUR_1;
    let now = utc(2026, 11, 25, 21, 23, 0);
    let newest = candle(timeframe, now);
    let mut data = input(timeframe, Some(&newest), now);
    data.source = DataSource::Kraken;
    data.refresh_ms = 15_000;
    data.data_age_ms = Some(40_000);
    assert!(Countdown::resolve(&data).is_some());
    data.data_age_ms = Some(46_000);
    assert_eq!(Countdown::resolve(&data), None);
}

#[test]
fn price_based_types_have_no_countdown_and_heikin_ashi_has() {
    for chart_type in ChartType::ALL {
        let price_based = chart_type.group() == ChartGroup::PriceBased;
        assert_eq!(
            chart_type.is_time_based(),
            !price_based || chart_type == ChartType::HeikinAshi,
            "{chart_type:?}"
        );
        assert_eq!(
            eligible(chart_type, DataSource::Binance),
            chart_type.is_time_based()
        );
    }
}

#[test]
fn only_sources_with_utc_candles_count_down() {
    let counting: Vec<_> = DataSource::ALL
        .into_iter()
        .filter(|source| source.counts_down_bars())
        .collect();
    assert_eq!(
        counting,
        [
            DataSource::Binance,
            DataSource::Coinbase,
            DataSource::Okx,
            DataSource::Kraken,
            DataSource::Bybit,
            DataSource::Gate,
            DataSource::Mexc,
            DataSource::DexScreener,
        ]
    );
}

#[test]
fn the_axis_is_measured_with_the_widest_text_of_each_form() {
    assert_eq!(widest_texts(Timeframe::MINUTE_5), ["59:59"]);
    assert_eq!(widest_texts(Timeframe::HOUR_1), ["59:59", "23:59:59"]);
    assert_eq!(widest_texts(Timeframe::DAY_1), ["59:59", "23:59:59"]);
    assert_eq!(
        widest_texts(Timeframe::WEEK_1),
        ["59:59", "23:59:59", "7d 23h"]
    );
}
