use super::*;
use crate::model::market::bucket::next_bucket_start_ms;

#[test]
fn every_native_has_its_interval_and_nothing_else_does() {
    let intervals: Vec<&str> = DataSource::Gate
        .natives()
        .iter()
        .map(|native| interval(*native).expect("a native has an interval"))
        .collect();
    assert_eq!(
        intervals,
        [
            "1m", "5m", "15m", "30m", "1h", "4h", "8h", "1d", "7d", "30d"
        ]
    );
    // Gate.io also answers 3m, 2h, 6h and 12h, but documents none of them.
    for other in ["3m", "2h", "6h", "12h", "2D", "2W", "3M", "1Y"] {
        assert_eq!(interval(Timeframe::parse(other).unwrap()), None, "{other}");
    }
}

fn base_of(timeframe: &str) -> (&'static str, usize) {
    let plan = plan(DataSource::Gate, Timeframe::parse(timeframe).unwrap()).unwrap();
    (interval(plan.base()).unwrap(), plan.base_count(500))
}

#[test]
fn other_lengths_fold_the_coarsest_native_that_nests() {
    // A request for 500 candles also asks for the one the start cuts.
    assert_eq!(base_of("1h"), ("1h", 500));
    assert_eq!(base_of("22m"), ("1m", 501 * 22));
    assert_eq!(base_of("2h"), ("1h", 501 * 2));
    assert_eq!(base_of("12h"), ("4h", 501 * 3));
    assert_eq!(base_of("3D"), ("1d", 501 * 3));
    assert_eq!(base_of("3M"), ("30d", 501 * 3));
    assert_eq!(base_of("1Y"), ("30d", 501 * 12));
}

#[test]
fn a_request_wider_than_the_paging_is_capped_before_it_is_folded() {
    // 500 candles of 1,439 minutes is 720,939 one-minute candles; paging reaches 8,900.
    let wide = plan(DataSource::Gate, Timeframe::parse("1439m").unwrap()).unwrap();
    assert_eq!(PAGING.requested(wide, 500), PAGING.max_candles);
    let hour = plan(DataSource::Gate, Timeframe::HOUR_1).unwrap();
    assert_eq!(PAGING.requested(hour, 500), 500);
}

#[test]
fn a_request_stops_where_gate_stops_reaching_back() {
    let hour = 3_600_000;
    let now = 100_000 * hour;
    let base = Timeframe::HOUR_1;
    // The page right behind the newest candles has nearly all the reach left.
    assert_eq!(reach_left(now, now - 500 * hour, base), 8_399);
    // Near the end only what is left may be asked for, and past it nothing.
    assert_eq!(reach_left(now, now - 8_000 * hour, base), 899);
    assert_eq!(reach_left(now, now - 8_899 * hour, base), 0);
    assert_eq!(reach_left(now, now - 20_000 * hour, base), 0);
    // A cursor in the future (a clock that is behind) has all of it.
    assert_eq!(reach_left(now, now + hour, base), 8_899);
}

/// 12:00:30 UTC on 2026-11-16: the forming one-minute candle opened at 12:00:00.
const NOW_MS: i64 = 1_794_830_430_000;
const MINUTE: i64 = 60_000;

/// A fake Gate.io: a market whose one-minute candles (volume 1) reach back `depth` minutes
/// from the forming one, answering `fetch(to, limit)` as the API does and refusing, as the
/// real service does with a 400, a request that reaches 9,000 points back from now.
struct Fake {
    depth: i64,
    now: i64,
    asked: std::cell::Cell<usize>,
}

impl Fake {
    fn new(depth: i64) -> Self {
        Self::at(depth, NOW_MS)
    }

    /// The same market `now` milliseconds into the day: where the reach ends moves with it.
    fn at(depth: i64, now: i64) -> Self {
        Self {
            depth,
            now,
            asked: std::cell::Cell::new(0),
        }
    }

    fn answer(&self, to_s: i64, limit: usize) -> Result<Vec<Kline>, ProviderError> {
        self.asked.set(self.asked.get() + 1);
        let newest = self.now / MINUTE * MINUTE;
        let oldest = newest - (self.depth - 1) * MINUTE;
        let points_ago = (self.now - to_s * 1_000) / MINUTE + limit as i64;
        assert!(
            points_ago <= 9_000,
            "Gate.io refuses: too long ago ({points_ago})"
        );
        let last = (to_s * 1_000 / MINUTE * MINUTE).min(newest);
        Ok((0..limit as i64)
            .rev()
            .map(|back| last - back * MINUTE)
            .filter(|open| *open >= oldest)
            .map(|open_time| Kline {
                open_time,
                close_time: open_time + MINUTE - 1,
                open: 1.0,
                high: 1.0,
                low: 1.0,
                close: 1.0,
                volume: 1.0,
            })
            .collect())
    }
}

/// Whether `candle` holds every minute of its bucket: nothing was cut off it.
fn is_whole(candle: &Kline, base: Timeframe) -> bool {
    (candle.volume * MINUTE as f64) as i64
        == next_bucket_start_ms(base, candle.open_time) - candle.open_time
}

#[test]
fn a_22_minute_chart_cut_short_by_the_reach_drops_its_partial_first_candle() {
    let target = Timeframe::parse("22m").unwrap();
    let plan = plan(DataSource::Gate, target).unwrap();
    // Where the reach ends against the 22-minute grid depends on the minute it is asked in.
    for minute in 0..22 {
        let now = NOW_MS + minute * MINUTE;
        let market = Fake::at(20_000, now);
        let shown = collect(plan, now, 0, now + 1, 500, |to, limit| {
            market.answer(to, limit)
        })
        .expect("candles");
        assert!(shown.len() > 300, "{}", shown.len());
        assert!(
            is_whole(&shown[0], target),
            "minute {minute}: the oldest candle holds {} minutes of {}",
            shown[0].volume,
            (next_bucket_start_ms(target, shown[0].open_time) - shown[0].open_time) / MINUTE
        );
    }
}

#[test]
fn an_older_page_near_the_reach_is_cut_short_and_the_one_after_it_is_empty() {
    let target = Timeframe::parse("22m").unwrap();
    let plan = plan(DataSource::Gate, target).unwrap();
    let market = Fake::new(20_000);
    let newest = NOW_MS / MINUTE * MINUTE;
    let end = newest - 8_400 * MINUTE;
    let page = collect(plan, NOW_MS, 0, end, 1_000, |to, limit| {
        market.answer(to, limit)
    })
    .expect("an older page");
    assert!(!page.is_empty());
    assert!(is_whole(&page[0], target), "{}", page[0].volume);

    let asked = market.asked.get();
    let end = newest - 8_899 * MINUTE;
    let past = collect(plan, NOW_MS, 0, end, 1_000, |to, limit| {
        market.answer(to, limit)
    })
    .expect("nothing past the reach");
    assert!(past.is_empty());
    assert_eq!(
        market.asked.get(),
        asked,
        "no request is made past the reach"
    );
}

#[test]
fn a_market_that_began_inside_the_reach_keeps_its_first_candle_as_it_is() {
    let target = Timeframe::parse("22m").unwrap();
    let plan = plan(DataSource::Gate, target).unwrap();
    let market = Fake::new(3_000);
    let shown = collect(plan, NOW_MS, 0, NOW_MS + 1, 500, |to, limit| {
        market.answer(to, limit)
    })
    .expect("candles");
    // Listed 3,000 minutes ago: its first bucket is genuinely short.
    assert!(!is_whole(&shown[0], target), "{}", shown[0].volume);
}

#[test]
fn a_native_length_reaches_as_far_back_as_gate_lets_it() {
    let one = plan(DataSource::Gate, Timeframe::MINUTE_1).unwrap();
    let market = Fake::new(20_000);
    let all = collect(one, NOW_MS, 0, NOW_MS + 1, 8_900, |to, limit| {
        market.answer(to, limit)
    })
    .expect("candles");
    assert_eq!(all.len(), 8_899);
    assert!(
        all.windows(2)
            .all(|pair| pair[0].open_time < pair[1].open_time)
    );
}

#[test]
fn symbols_are_gateios_however_they_are_spelled() {
    assert_eq!(pair_id("btc/usdt"), "BTC_USDT");
    assert_eq!(pair_id("eth-usdc"), "ETH_USDC");
    assert_eq!(pair_id("ETH_BTC"), "ETH_BTC");
    assert_eq!(pair_id("sol"), "SOL_USDT");
}

// The close is the third field and the open the sixth; the base volume is the seventh.
const ROWS: &str = r#"[
    ["1699999200","450","11","12","9","10","42.5","true"],
    ["1700002800","60","12","13","10","11","5.5","false"]
]"#;

#[test]
fn rows_are_candles_with_the_close_of_their_bucket() {
    let candles = parse_candles(ROWS, Timeframe::HOUR_1).expect("valid response");
    assert_eq!(candles.len(), 2);
    assert_eq!(
        candles[0],
        Kline {
            open_time: 1_699_999_200_000,
            close_time: 1_700_002_799_999,
            open: 10.0,
            high: 12.0,
            low: 9.0,
            close: 11.0,
            volume: 42.5,
        }
    );
    assert_eq!(candles[1].open_time, 1_700_002_800_000);
}

#[test]
fn a_month_closes_when_the_next_calendar_month_opens() {
    // 2026-02-01 00:00 UTC.
    let row = serde_json::json!(["1769904000", "4", "1.5", "2", "0.5", "1", "3", "true"]);
    let candle = parse_row(&row, Timeframe::MONTH_1).expect("row");
    assert_eq!(candle.close_time, 1_772_323_200_000 - 1);
}

#[test]
fn odd_answers_are_errors() {
    assert!(parse_candles("not json", Timeframe::HOUR_1).is_err());
    assert!(parse_candles(r#"{"label":"INVALID_PARAM_VALUE"}"#, Timeframe::HOUR_1).is_err());
    assert_eq!(parse_candles("[]", Timeframe::HOUR_1), Ok(Vec::new()));
    let bad_field = r#"[["1700000000","1","x","1","1","1","1","true"]]"#;
    assert!(parse_candles(bad_field, Timeframe::HOUR_1).is_err());
    assert!(
        parse_candles(
            r#"[["x","1","1","1","1","1","1","true"]]"#,
            Timeframe::HOUR_1
        )
        .is_err()
    );
}

#[test]
fn the_catalog_keeps_only_tradable_markets() {
    let body = r#"[
        {"id":"BTC_USDT","base":"BTC","quote":"USDT","trade_status":"tradable"},
        {"id":"OLD_USDT","base":"OLD","quote":"USDT","trade_status":"untradable"},
        {"id":"SELL_USDT","base":"SELL","quote":"USDT","trade_status":"sellable"},
        {"id":"NOQUOTE","base":"X","quote":"","trade_status":"tradable"}
    ]"#;
    let pairs = parse_pairs(body).expect("pairs");
    assert_eq!(pairs.len(), 1);
    assert_eq!(
        (
            pairs[0].id.as_str(),
            pairs[0].base.as_str(),
            pairs[0].quote.as_str()
        ),
        ("BTC_USDT", "BTC", "USDT")
    );
    assert!(parse_pairs("not json").is_err());
    assert!(parse_pairs("{}").is_err());
}

#[test]
fn a_pair_is_found_however_it_is_separated() {
    let pairs = vec![
        Pair {
            id: "BTC_USDT".into(),
            base: "BTC".into(),
            quote: "USDT".into(),
        },
        Pair {
            id: "ETH_BTC".into(),
            base: "ETH".into(),
            quote: "BTC".into(),
        },
    ];
    for query in ["btcusdt", "BTC_USDT", "btc/usdt", "BTC USDT", "btc-usdt"] {
        let found = matching_pairs(&pairs, &normalize(query));
        assert_eq!(found.len(), 1, "{query}");
        assert_eq!(found[0].symbol, "BTC_USDT");
    }
    assert_eq!(matching_pairs(&pairs, "BTC").len(), 2);
}

/// Run with `cargo test --lib gateio::tests::live -- --ignored --nocapture`.
#[test]
#[ignore = "needs the network"]
fn live_gate_serves_candles_pairs_and_every_native() {
    let hits = GATE.search_tickers("btc-usdt").expect("search");
    assert_eq!(hits[0].symbol, "BTC_USDT");
    assert_eq!(hits[0].quote.as_deref(), Some("USDT"));
    for native in DataSource::Gate.natives() {
        // More than one page of the shortest bars, so paging is exercised.
        let wanted = if *native == Timeframe::MINUTE_1 {
            1_500
        } else {
            350
        };
        let candles = GATE
            .fetch_klines("BTC_USDT", *native, wanted)
            .expect("candles");
        // Months reach back only to the market's listing.
        if *native == Timeframe::MONTH_1 {
            assert!(candles.len() > 30, "{native}");
        } else {
            assert_eq!(candles.len(), wanted as usize, "{native}");
        }
        assert!(
            candles
                .windows(2)
                .all(|pair| pair[0].open_time < pair[1].open_time),
            "{native}"
        );
    }
    // A 22-minute chart wants eleven thousand one-minute candles and Gate.io reaches back to
    // 8,900 of them: about 408 candles (days restart the buckets), none of them a fragment.
    let folded = GATE
        .fetch_klines("BTC_USDT", Timeframe::parse("22m").unwrap(), 500)
        .expect("folded candles");
    assert!((395..=420).contains(&folded.len()), "{}", folded.len());
    // Paging back from the first 500 reaches the end of what Gate.io holds without an error,
    // and the page after it is empty.
    let hour = GATE
        .fetch_klines("BTC_USDT", Timeframe::MINUTE_1, 500)
        .expect("minutes");
    let mut end = hour[0].open_time;
    let mut total = hour.len();
    loop {
        let page = GATE
            .fetch_klines_range("BTC_USDT", Timeframe::MINUTE_1, 0, end, 1_000)
            .expect("an older page, or an empty one at the end");
        let Some(first) = page.first() else { break };
        total += page.len();
        end = first.open_time;
    }
    assert!((8_800..=8_900).contains(&total), "{total}");
    let error = GATE
        .fetch_klines("NOPE_USDT", Timeframe::HOUR_1, 5)
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Gate.io refused the request (HTTP 400): it does not list this market"
    );
}
