use super::*;
use crate::model::market::bucket::fold_base;

fn tf(text: &str) -> Timeframe {
    Timeframe::parse(text).unwrap()
}

/// Lengths a user may pick, short to long, odd ones among them.
const TABLE: [&str; 24] = [
    "1m", "2m", "3m", "5m", "7m", "13m", "22m", "30m", "45m", "1h", "3h", "10h", "24h", "1D", "2D",
    "3D", "5D", "1W", "2W", "1M", "2M", "3M", "6M", "1Y",
];

const CANDLE_SOURCES: [DataSource; 5] = [
    DataSource::Binance,
    DataSource::Coinbase,
    DataSource::Okx,
    DataSource::Alpaca,
    DataSource::DexScreener,
];

#[test]
fn every_native_is_its_own_base_and_lists_are_shortest_first() {
    for source in CANDLE_SOURCES {
        let natives = source.natives();
        assert!(
            natives.windows(2).all(|pair| pair[0] < pair[1]),
            "{source:?}"
        );
        for native in natives {
            assert_eq!(
                fold_base(*native, natives),
                Some(*native),
                "{source:?} {native}"
            );
        }
    }
}

#[test]
fn okx_folds_what_it_cuts_off_the_utc_grid_and_the_year() {
    let base = |text: &str| {
        DataSource::Okx
            .plan(tf(text))
            .map(|plan| plan.base().label())
    };
    // Its own 2D and 3D are counted from days that are not ours, so days are folded.
    assert_eq!(base("2D").as_deref(), Some("1D"));
    assert_eq!(base("3D").as_deref(), Some("1D"));
    // It has no 8-hour bar, and no year.
    assert_eq!(base("8h").as_deref(), Some("4h"));
    assert_eq!(base("1Y").as_deref(), Some("3M"));
    assert_eq!(base("22m").as_deref(), Some("1m"));
    for text in TABLE {
        assert!(DataSource::Okx.serves(tf(text)), "{text}");
    }
}

#[test]
fn a_source_without_candles_has_no_natives() {
    for source in [
        DataSource::CoinGecko,
        DataSource::Polymarket,
        DataSource::Kalshi,
        DataSource::CoinMarketCap,
    ] {
        assert!(source.natives().is_empty(), "{source:?}");
        assert_eq!(source.plan(Timeframe::HOUR_1), None, "{source:?}");
    }
}

#[test]
fn binance_has_the_documented_intervals_without_its_own_three_days() {
    let labels: Vec<String> = DataSource::Binance
        .natives()
        .iter()
        .map(|t| t.label())
        .collect();
    assert_eq!(
        labels,
        [
            "1m", "3m", "5m", "15m", "30m", "1h", "2h", "4h", "6h", "8h", "12h", "1D", "1W", "1M"
        ]
    );
    // Its 3d bars start on a day that is not a multiple of three since 1970, so 3D is a fold of 1D.
    assert_eq!(
        DataSource::Binance.plan(tf("3D")).unwrap().base(),
        Timeframe::DAY_1
    );
}

#[test]
fn coinbase_has_its_six_granularities() {
    let seconds: Vec<i64> = DataSource::Coinbase
        .natives()
        .iter()
        .map(|t| t.nominal_ms() / 1_000)
        .collect();
    assert_eq!(seconds, [60, 300, 900, 3_600, 21_600, 86_400]);
}

#[test]
fn alpaca_serves_only_what_divides_the_hour_and_the_day_week_and_month() {
    let labels: Vec<String> = DataSource::Alpaca
        .natives()
        .iter()
        .map(|t| t.label())
        .collect();
    assert_eq!(labels, ["1m", "5m", "15m", "30m", "1h", "1D", "1W", "1M"]);
    // Alpaca has 3Hour and 4Min bars; those are not on our grid, so they are folded.
    assert_eq!(
        DataSource::Alpaca.plan(tf("3h")).unwrap().base(),
        Timeframe::HOUR_1
    );
    assert_eq!(
        DataSource::Alpaca.plan(tf("4m")).unwrap().base(),
        Timeframe::MINUTE_1
    );
}

#[test]
fn geckoterminal_has_its_seven_aggregates() {
    let labels: Vec<String> = DataSource::DexScreener
        .natives()
        .iter()
        .map(|t| t.label())
        .collect();
    assert_eq!(labels, ["1m", "5m", "15m", "1h", "4h", "12h", "1D"]);
}

#[test]
fn a_candle_source_serves_every_length_it_can_fold_from_its_candles() {
    for source in CANDLE_SOURCES {
        for text in TABLE {
            let timeframe = tf(text);
            assert_eq!(
                source.serves(timeframe),
                source.plan(timeframe).is_some(),
                "{source:?} {text}"
            );
            // Each of these has a minute and a day candle to fold from.
            assert!(source.serves(timeframe), "{source:?} {text}");
        }
    }
}

#[test]
fn coingecko_serves_what_its_samples_can_build() {
    for (text, served) in [
        ("1m", false),
        ("2m", false),
        ("7m", false),
        ("22m", false),
        ("5m", true),
        ("10m", true),
        ("30m", true),
        ("1h", true),
        ("3h", true),
        ("1D", true),
        ("1W", true),
        ("1M", true),
        ("1Y", true),
    ] {
        assert_eq!(DataSource::CoinGecko.serves(tf(text)), served, "{text}");
    }
}

#[test]
fn lines_follow_any_timeframe() {
    for source in [
        DataSource::Polymarket,
        DataSource::Kalshi,
        DataSource::CoinMarketCap,
    ] {
        for text in TABLE {
            assert!(source.serves(tf(text)), "{source:?} {text}");
        }
    }
}

#[test]
fn exchanges_page_back_at_any_length() {
    for source in [
        DataSource::Binance,
        DataSource::Coinbase,
        DataSource::Okx,
        DataSource::Alpaca,
    ] {
        for text in TABLE {
            assert!(source.supports_older_history(tf(text)), "{source:?} {text}");
        }
    }
    for source in [
        DataSource::CoinGecko,
        DataSource::Polymarket,
        DataSource::Kalshi,
    ] {
        assert!(
            !source.supports_older_history(Timeframe::HOUR_1),
            "{source:?}"
        );
    }
}

#[test]
fn geckoterminal_pages_back_while_a_page_of_its_base_candles_is_short_of_180_days() {
    // A page is a thousand base candles: 41 days of hours, 166 of four hours, 500 of twelve.
    for (text, pages) in [
        ("1m", true),
        ("5m", true),
        ("15m", true),
        ("22m", true),
        ("1h", true),
        ("3h", true),
        ("4h", true),
        ("8h", true),
        ("12h", false),
        ("24h", false),
        ("1D", false),
        ("1W", false),
        ("1M", false),
    ] {
        assert_eq!(
            DataSource::DexScreener.supports_older_history(tf(text)),
            pages,
            "{text}"
        );
    }
}
