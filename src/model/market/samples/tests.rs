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
