use super::*;

fn tf(count: u16, unit: TimeUnit) -> Timeframe {
    Timeframe::new(count, unit).unwrap()
}

#[test]
fn presets_and_legacy_lists_are_in_order_with_their_tokens() {
    let tokens: Vec<_> = Timeframe::PRESETS.iter().map(|tf| tf.interval()).collect();
    assert_eq!(
        tokens,
        [
            "1s", "15s", "30s", "45s", "1m", "5m", "15m", "30m", "1h", "4h", "8h", "12h", "1d",
            "1w", "1M", "1Y"
        ]
    );
    let legacy: Vec<_> = Timeframe::LEGACY.iter().map(|tf| tf.interval()).collect();
    assert_eq!(legacy, ["1h", "1d", "1w", "1M", "3M", "1Y"]);
    assert!(Timeframe::PRESETS.windows(2).all(|pair| pair[0] < pair[1]));
    // Every length older builds offered is a chip, but the quarter: it is typed (`3M`) or pinned.
    for legacy in Timeframe::LEGACY {
        assert_eq!(
            Timeframe::PRESETS.contains(&legacy),
            legacy != Timeframe::MONTH_3
        );
    }
}

#[test]
fn labels_use_lowercase_minutes_and_hours() {
    let labels: Vec<_> = Timeframe::PRESETS.iter().map(|tf| tf.label()).collect();
    assert_eq!(
        labels,
        [
            "1s", "15s", "30s", "45s", "1m", "5m", "15m", "30m", "1h", "4h", "8h", "12h", "1D",
            "1W", "1M", "1Y"
        ]
    );
    assert_eq!(tf(22, TimeUnit::Minute).to_string(), "22m");
    assert_eq!(tf(2, TimeUnit::Day).to_string(), "2D");
    assert_eq!(tf(24, TimeUnit::Hour).to_string(), "24h");
}

#[test]
fn canonical_forms_are_one_value() {
    assert_eq!(tf(60, TimeUnit::Minute), Timeframe::HOUR_1);
    assert_eq!(tf(1_440, TimeUnit::Minute), tf(24, TimeUnit::Hour));
    assert_eq!(tf(12, TimeUnit::Month), Timeframe::YEAR_1);
    // A day is a calendar day; 24 hours is not.
    assert_ne!(tf(24, TimeUnit::Hour), Timeframe::DAY_1);
    assert_eq!(tf(90, TimeUnit::Minute).unit(), TimeUnit::Minute);
    assert_eq!(tf(120, TimeUnit::Minute).count(), 2);
}

#[test]
fn counts_outside_a_units_limits_are_refused() {
    use TimeframeParseError::{OutOfRange, Zero};
    for (unit, max) in [
        (TimeUnit::Minute, 1_440),
        (TimeUnit::Hour, 24),
        (TimeUnit::Day, 365),
        (TimeUnit::Week, 52),
        (TimeUnit::Month, 12),
    ] {
        assert!(Timeframe::new(max, unit).is_ok(), "{unit:?} {max}");
        assert_eq!(
            Timeframe::new(max + 1, unit),
            Err(OutOfRange { unit, max }),
            "{unit:?}"
        );
        assert_eq!(Timeframe::new(0, unit), Err(Zero));
    }
}

#[test]
fn parse_reads_every_spelling_of_a_timeframe() {
    let cases = [
        ("22m", tf(22, TimeUnit::Minute)),
        ("22", tf(22, TimeUnit::Minute)),
        ("  22 m ", tf(22, TimeUnit::Minute)),
        ("60", Timeframe::HOUR_1),
        ("1440", tf(24, TimeUnit::Hour)),
        ("3h", tf(3, TimeUnit::Hour)),
        ("3H", tf(3, TimeUnit::Hour)),
        ("2D", tf(2, TimeUnit::Day)),
        ("2d", tf(2, TimeUnit::Day)),
        ("1W", Timeframe::WEEK_1),
        ("1w", Timeframe::WEEK_1),
        ("1M", Timeframe::MONTH_1),
        ("3M", Timeframe::MONTH_3),
        ("12M", Timeframe::YEAR_1),
        ("1Y", Timeframe::YEAR_1),
        ("1y", Timeframe::YEAR_1),
        ("05m", tf(5, TimeUnit::Minute)),
    ];
    for (text, expected) in cases {
        assert_eq!(Timeframe::parse(text), Ok(expected), "{text:?}");
    }
}

#[test]
fn parse_reads_the_labels_and_tokens_the_app_has_always_used() {
    for legacy in ["1H", "1D", "1W", "1M", "3M", "1Y"] {
        let parsed = Timeframe::parse(legacy).unwrap();
        assert!(Timeframe::LEGACY.contains(&parsed), "{legacy}");
    }
    for timeframe in Timeframe::PRESETS {
        assert_eq!(Timeframe::parse(&timeframe.interval()), Ok(timeframe));
        assert_eq!(Timeframe::parse(&timeframe.label()), Ok(timeframe));
    }
}

#[test]
fn parse_explains_what_is_wrong() {
    use TimeframeParseError::*;
    let cases = [
        ("", Malformed),
        ("   ", Malformed),
        ("m", Malformed),
        ("D", Malformed),
        ("22x", Malformed),
        ("22min", Malformed),
        ("2 2m", Malformed),
        ("-5m", Malformed),
        ("0", Zero),
        ("0h", Zero),
        ("0Y", Zero),
        (
            "60s",
            OutOfRange {
                unit: TimeUnit::Second,
                max: 59,
            },
        ),
        (
            "1441",
            OutOfRange {
                unit: TimeUnit::Minute,
                max: 1_440,
            },
        ),
        (
            "25h",
            OutOfRange {
                unit: TimeUnit::Hour,
                max: 24,
            },
        ),
        (
            "366D",
            OutOfRange {
                unit: TimeUnit::Day,
                max: 365,
            },
        ),
        (
            "53W",
            OutOfRange {
                unit: TimeUnit::Week,
                max: 52,
            },
        ),
        (
            "13M",
            OutOfRange {
                unit: TimeUnit::Month,
                max: 12,
            },
        ),
        (
            "99999999999m",
            OutOfRange {
                unit: TimeUnit::Minute,
                max: 1_440,
            },
        ),
        ("2Y", YearsUnsupported),
    ];
    for (text, expected) in cases {
        assert_eq!(Timeframe::parse(text), Err(expected), "{text:?}");
    }
    assert_eq!(
        Timeframe::parse("1441").unwrap_err().to_string(),
        "Minutes go from 1 to 1440"
    );
    assert_eq!(
        Timeframe::parse("60s").unwrap_err().to_string(),
        "Seconds go from 1 to 59"
    );
    assert_eq!(
        Timeframe::parse("0").unwrap_err().to_string(),
        "A timeframe can't be zero"
    );
    assert_eq!(
        Timeframe::parse("x").unwrap_err().to_string(),
        "Type a number and a unit: 30s, 22m, 3h, 2D, 1W, 3M"
    );
}

#[test]
fn legacy_values_keep_their_tokens_descriptions_and_lengths() {
    const DAY: i64 = 86_400_000;
    let expected = [
        (Timeframe::HOUR_1, "1h", "One-hour candles", 3_600_000),
        (Timeframe::DAY_1, "1d", "Daily candles", DAY),
        (Timeframe::WEEK_1, "1w", "Weekly candles", 7 * DAY),
        (Timeframe::MONTH_1, "1M", "Monthly candles", 30 * DAY),
        (Timeframe::MONTH_3, "3M", "Quarterly candles", 91 * DAY),
        (Timeframe::YEAR_1, "1Y", "Yearly candles", 365 * DAY),
    ];
    for (timeframe, token, description, length) in expected {
        assert_eq!(timeframe.interval(), token);
        assert_eq!(timeframe.description(), description);
        assert_eq!(timeframe.nominal_ms(), length);
    }
}

#[test]
fn other_lengths_are_described_generically() {
    assert_eq!(tf(22, TimeUnit::Minute).description(), "22-minute candles");
    assert_eq!(Timeframe::MINUTE_1.description(), "One-minute candles");
    assert_eq!(tf(2, TimeUnit::Day).description(), "2-day candles");
    assert_eq!(tf(3, TimeUnit::Hour).description(), "3-hour candles");
    assert_eq!(tf(2, TimeUnit::Week).description(), "2-week candles");
    assert_eq!(tf(6, TimeUnit::Month).description(), "6-month candles");
}

#[test]
fn nominal_lengths_are_exact_to_weeks_and_thirty_days_a_month() {
    assert_eq!(tf(22, TimeUnit::Minute).nominal_ms(), 22 * 60_000);
    assert_eq!(tf(24, TimeUnit::Hour).nominal_ms(), 86_400_000);
    assert_eq!(tf(2, TimeUnit::Week).nominal_ms(), 14 * 86_400_000);
    assert_eq!(tf(2, TimeUnit::Month).nominal_ms(), 60 * 86_400_000);
}

#[test]
fn timeframes_order_by_length_then_unit() {
    let mut mixed = [
        Timeframe::YEAR_1,
        tf(22, TimeUnit::Minute),
        Timeframe::DAY_1,
        tf(24, TimeUnit::Hour),
        Timeframe::MINUTE_1,
        tf(2, TimeUnit::Day),
        tf(7, TimeUnit::Day),
        Timeframe::WEEK_1,
    ];
    mixed.sort();
    assert_eq!(
        mixed.iter().map(|tf| tf.label()).collect::<Vec<_>>(),
        ["1m", "22m", "24h", "1D", "2D", "7D", "1W", "1Y"]
    );
}

#[test]
fn sections_group_by_unit() {
    assert_eq!(Timeframe::MINUTE_5.section(), Section::Minutes);
    assert_eq!(Timeframe::HOUR_4.section(), Section::Hours);
    for timeframe in [Timeframe::DAY_1, Timeframe::WEEK_1, Timeframe::YEAR_1] {
        assert_eq!(timeframe.section(), Section::DaysAndUp);
    }
    assert!(Timeframe::HOUR_4.is_intraday() && Timeframe::MINUTE_1.is_intraday());
    assert!(!Timeframe::DAY_1.is_intraday());
}

#[test]
fn legacy_values_serialize_as_the_old_variant_names() {
    let expected = [
        (Timeframe::HOUR_1, "\"Hour\""),
        (Timeframe::DAY_1, "\"Day\""),
        (Timeframe::WEEK_1, "\"Week\""),
        (Timeframe::MONTH_1, "\"Month\""),
        (Timeframe::MONTH_3, "\"Quarter\""),
        (Timeframe::YEAR_1, "\"Year\""),
    ];
    for (timeframe, json) in expected {
        assert_eq!(serde_json::to_string(&timeframe).unwrap(), json);
        assert_eq!(serde_json::from_str::<Timeframe>(json).unwrap(), timeframe);
    }
}

#[test]
fn other_values_round_trip_as_their_label() {
    for timeframe in [
        tf(22, TimeUnit::Minute),
        Timeframe::MINUTE_1,
        tf(24, TimeUnit::Hour),
        tf(2, TimeUnit::Day),
        tf(2, TimeUnit::Week),
        tf(6, TimeUnit::Month),
    ] {
        let json = serde_json::to_string(&timeframe).unwrap();
        assert_eq!(json, format!("\"{}\"", timeframe.label()));
        assert_eq!(serde_json::from_str::<Timeframe>(&json).unwrap(), timeframe);
    }
}

#[test]
fn text_that_is_no_timeframe_fails_to_decode() {
    for json in ["\"Fortnight\"", "\"0m\"", "\"2Y\"", "\"\"", "5"] {
        assert!(serde_json::from_str::<Timeframe>(json).is_err(), "{json}");
    }
}

#[test]
fn seconds_parse_print_and_sort_before_minutes() {
    let half_minute = tf(30, TimeUnit::Second);
    for text in ["30s", "30S", " 30 s "] {
        assert_eq!(Timeframe::parse(text), Ok(half_minute), "{text:?}");
    }
    assert_eq!(half_minute.label(), "30s");
    assert_eq!(half_minute.interval(), "30s");
    assert_eq!(half_minute.nominal_ms(), 30_000);
    assert_eq!(half_minute.description(), "30-second candles");
    assert_eq!(Timeframe::SECOND_1.description(), "One-second candles");
    assert_eq!(half_minute.section(), Section::Seconds);
    assert!(half_minute.is_intraday() && half_minute.is_subminute());
    assert!(!Timeframe::MINUTE_1.is_subminute());
    assert!(half_minute < Timeframe::MINUTE_1);
    assert!(Timeframe::SECOND_1 < half_minute);
    assert_eq!(
        Timeframe::parse("0s"),
        Err(TimeframeParseError::Zero),
        "a zero count"
    );
    assert_eq!(
        Timeframe::parse("60s"),
        Err(TimeframeParseError::OutOfRange {
            unit: TimeUnit::Second,
            max: 59,
        }),
        "a minute is written 1m"
    );
}

#[test]
fn seconds_round_trip_as_their_label() {
    for timeframe in [
        Timeframe::SECOND_1,
        Timeframe::SECOND_5,
        Timeframe::SECOND_10,
        Timeframe::SECOND_15,
        Timeframe::SECOND_30,
        Timeframe::SECOND_45,
    ] {
        let json = serde_json::to_string(&timeframe).unwrap();
        assert_eq!(json, format!("\"{}\"", timeframe.label()));
        assert_eq!(serde_json::from_str::<Timeframe>(&json).unwrap(), timeframe);
        assert_eq!(Timeframe::parse(&timeframe.interval()), Ok(timeframe));
    }
}
