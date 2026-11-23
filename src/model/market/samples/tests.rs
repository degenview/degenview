use super::*;

fn days(timeframe: &str, limit: u32) -> Option<i64> {
    chart_days(Timeframe::parse(timeframe).unwrap(), limit)
}

#[test]
fn hours_are_built_from_hourly_samples_over_as_many_days_as_the_candles_cover() {
    assert_eq!(days("1h", 500), Some(21));
    assert_eq!(days("2h", 500), Some(42));
    assert_eq!(days("3h", 500), Some(63));
    // A tail refresh asks for a day: five-minute samples, which fold into the hour as well.
    assert_eq!(days("1h", 4), Some(1));
}

#[test]
fn a_length_only_five_minute_samples_divide_gets_a_day() {
    for timeframe in ["5m", "10m", "30m", "90m"] {
        assert_eq!(days(timeframe, 500), Some(1), "{timeframe}");
    }
}

#[test]
fn days_and_up_reach_as_far_back_as_the_free_api_does() {
    for timeframe in ["1D", "2D", "1W", "1M", "3M", "1Y"] {
        assert_eq!(days(timeframe, 500), Some(365), "{timeframe}");
    }
    assert_eq!(days("1D", 30), Some(30));
}

#[test]
fn a_length_no_sample_spacing_divides_is_unsupported() {
    // One minute is finer than any sample; 22 and 7 minutes are not whole fives.
    for timeframe in ["1m", "2m", "7m", "13m", "22m"] {
        assert_eq!(days(timeframe, 500), None, "{timeframe}");
    }
}

#[test]
fn folds_prices_into_buckets() {
    let prices = vec![
        (1_700_000_000_000, 10.0),
        (1_700_001_800_000, 12.0), // same hour bucket as above
        (1_700_003_600_000, 8.0),  // next hour bucket
    ];
    let candles = fold_samples(&prices, Timeframe::HOUR_1);
    assert_eq!(candles.len(), 2);
    assert_eq!(candles[0].high, 12.0);
    assert_eq!(candles[0].low, 10.0);
    assert_eq!(candles[0].close, 12.0);
    assert_eq!(candles[1].close, 8.0);
    assert_eq!(candles[1].open, 12.0);
    assert_eq!(candles[1].high, 12.0);
    assert_eq!(candles[1].low, 8.0);
}

#[test]
fn first_candle_opens_at_its_first_sample() {
    let candles = fold_samples(&[(1_700_000_000_000, 10.0)], Timeframe::HOUR_1);
    assert_eq!(candles[0].open, 10.0);
    assert_eq!(candles[0].close, 10.0);
}

#[test]
fn single_sample_buckets_get_a_body_and_a_colour() {
    // One sample per hour, as `/market_chart` returns for 2–90 days.
    let prices = vec![
        (1_700_000_000_000, 10.0),
        (1_700_003_600_000, 9.0),
        (1_700_007_200_000, 11.0),
    ];
    let candles = fold_samples(&prices, Timeframe::HOUR_1);
    assert_eq!(candles.len(), 3);
    assert!(!candles[1].is_up());
    assert_eq!((candles[1].open, candles[1].close), (10.0, 9.0));
    assert_eq!((candles[1].high, candles[1].low), (10.0, 9.0));
    assert!(candles[2].is_up());
    assert_eq!((candles[2].open, candles[2].close), (9.0, 11.0));
}
