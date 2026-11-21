use super::markets::{Market, matching, parse_dexes, parse_perps, parse_spot};
use super::rest::{candles_of, page_before_with};
use super::*;
use crate::model::market::bucket::{bucket_start_ms, next_bucket_start_ms};
use crate::model::{TimeUnit, watchlist};

const HOUR: i64 = 3_600_000;
const DAY: i64 = 24 * HOUR;

fn tf(text: &str) -> Timeframe {
    Timeframe::parse(text).unwrap()
}

#[test]
fn every_native_has_its_interval_and_nothing_else_does() {
    let intervals: Vec<String> = DataSource::Hyperliquid
        .natives()
        .iter()
        .map(|native| interval(*native).expect("a native has an interval"))
        .collect();
    assert_eq!(
        intervals,
        [
            "1m", "3m", "5m", "15m", "30m", "1h", "2h", "4h", "8h", "12h", "1d", "3d"
        ]
    );
    // Hyperliquid has `2m` neither; its `1w` and `1M` are not cut where this app cuts.
    for other in ["2m", "6h", "2d", "1w", "1M", "3M", "1Y"] {
        assert_eq!(interval(tf(other)), None, "{other}");
    }
}

/// The opens the API answered on 2026-11-21 (`candleSnapshot` for BTC): its bars against this app's
/// grid, which decides what is native and what is folded.
#[test]
fn only_the_bars_that_open_where_this_app_cuts_them_are_native() {
    let on_grid = |timeframe: Timeframe, open: i64| bucket_start_ms(timeframe, open) == open;
    // 1d opens at midnight; 3d on a day number divisible by three.
    assert!(on_grid(Timeframe::DAY_1, 1_794_960_000_000));
    assert!(on_grid(
        Timeframe::new(3, TimeUnit::Day).unwrap(),
        1_794_960_000_000
    ));
    // The 1w bar opened Thursday 2026-11-19, a week counted from 1970-01-01; ours open on Monday.
    assert!(!on_grid(Timeframe::WEEK_1, 1_795_046_400_000));
    // The 1M bar opened 2026-11-03: thirty days counted from 1970-01-01, not the calendar month.
    assert!(!on_grid(Timeframe::MONTH_1, 1_793_664_000_000));
    assert_eq!(
        next_bucket_start_ms(Timeframe::MONTH_1, 1_793_664_000_000),
        1_796_083_200_000,
        "a month here ends with the calendar month"
    );
    for native in DataSource::Hyperliquid.natives() {
        // Every native cuts a day in whole bars, or is a whole number of days.
        assert!(DAY % native.nominal_ms() == 0 || native.nominal_ms() % DAY == 0);
    }
}

fn base_of(timeframe: &str) -> (String, usize) {
    let plan = plan(DataSource::Hyperliquid, tf(timeframe)).unwrap();
    (interval(plan.base()).unwrap(), plan.base_count(500))
}

#[test]
fn weeks_months_and_other_lengths_fold_the_coarsest_native_that_nests() {
    assert_eq!(base_of("1h"), ("1h".to_owned(), 500));
    assert_eq!(base_of("3D"), ("3d".to_owned(), 500));
    assert_eq!(base_of("22m"), ("1m".to_owned(), 501 * 22));
    assert_eq!(base_of("6h"), ("2h".to_owned(), 501 * 3));
    assert_eq!(base_of("1W"), ("1d".to_owned(), 501 * 7));
    assert_eq!(base_of("1M"), ("1d".to_owned(), 501 * 31));
    assert_eq!(base_of("1Y"), ("1d".to_owned(), 501 * 372));
}

fn candle_at(open_time: i64) -> Kline {
    Kline {
        open_time,
        close_time: open_time + HOUR - 1,
        open: 1.0,
        high: 1.0,
        low: 1.0,
        close: 1.0,
        volume: 1.0,
    }
}

/// What the API keeps: one candle an hour, the newest `kept` of them, up to `now`.
fn kept_hours(now: i64, kept: i64) -> impl FnMut(i64, i64) -> Result<Vec<Kline>, ProviderError> {
    move |start, end| {
        Ok((0..kept)
            .rev()
            .map(|age| (now / HOUR - age) * HOUR)
            .filter(|open| (start..=end).contains(open))
            .map(candle_at)
            .collect())
    }
}

#[test]
fn a_page_asks_for_a_window_with_room_for_gaps_and_keeps_the_newest_candles() {
    let now = 5_000 * HOUR;
    let mut windows = Vec::new();
    let mut inner = kept_hours(now, 4_000);
    let page = page_before_with(
        |start, end| {
            windows.push((start, end));
            inner(start, end)
        },
        false,
        Timeframe::HOUR_1,
        0,
        now,
        4,
    )
    .unwrap();
    assert_eq!(windows.len(), 1, "a refresh is one request");
    assert!(windows[0].1 < now, "the page is before `after`");
    assert_eq!(page.candles.len(), 4);
    assert_eq!(
        page.candles.last().map(|candle| candle.open_time),
        Some(now - HOUR)
    );
}

#[test]
fn a_page_that_holds_too_few_is_asked_again_for_everything_kept() {
    let now = 5_000 * HOUR;
    let mut windows = Vec::new();
    // Hyperliquid keeps 300 candles, so asking for 400 reaches the start of what it has.
    let mut inner = kept_hours(now, 300);
    let page = page_before_with(
        |start, end| {
            windows.push((start, end));
            inner(start, end)
        },
        false,
        Timeframe::HOUR_1,
        0,
        now,
        400,
    )
    .unwrap();
    assert_eq!(windows.len(), 2);
    assert_eq!(windows[1].0, 0, "the second question has no start");
    assert_eq!(
        page.candles.len(),
        299,
        "the candle at `after` is not before it"
    );
    assert_eq!(page.thin, Some(false), "299 candles are not thin");
}

#[test]
fn a_refresh_of_a_market_that_traded_nothing_for_hours_is_one_request() {
    let now = 5_000 * HOUR;
    let mut asked = 0;
    let page = page_before_with(
        |_, _| {
            asked += 1;
            Ok(Vec::new())
        },
        false,
        Timeframe::MINUTE_1,
        0,
        now,
        4,
    )
    .unwrap();
    assert_eq!(asked, 1, "a few bars are never worth the whole history");
    assert!(page.candles.is_empty());
    assert_eq!(
        page.thin, None,
        "nothing is learned of the market's history"
    );
}

#[test]
fn a_page_never_asks_before_the_start_it_was_given() {
    let now = 5_000 * HOUR;
    let start = now - 10 * HOUR;
    let mut windows = Vec::new();
    let mut inner = kept_hours(now, 4_000);
    let page = page_before_with(
        |from, end| {
            windows.push((from, end));
            inner(from, end)
        },
        false,
        Timeframe::HOUR_1,
        start,
        now,
        400,
    )
    .unwrap();
    // The window of 400 hours is clamped to the range, which already is everything asked for.
    assert_eq!(windows, [(start, now - 1)]);
    assert_eq!(page.candles.len(), 10);
    assert_eq!(page.thin, None, "a range says nothing of the whole history");
}

#[test]
fn a_thin_market_is_asked_for_whole_in_one_request_and_forgotten_as_thin_when_it_grows() {
    let now = 5_000 * HOUR;
    let mut windows = Vec::new();
    // 30 candles in all: a window of a refresh would find them or not, the whole history is as cheap.
    let mut inner = kept_hours(now, 30);
    let first = page_before_with(
        |start, end| {
            windows.push((start, end));
            inner(start, end)
        },
        false,
        Timeframe::HOUR_1,
        0,
        now,
        400,
    )
    .unwrap();
    assert_eq!(windows.len(), 2);
    assert_eq!(first.thin, Some(true));

    windows.clear();
    let mut inner = kept_hours(now, 30);
    let refresh = page_before_with(
        |start, end| {
            windows.push((start, end));
            inner(start, end)
        },
        true,
        Timeframe::HOUR_1,
        0,
        now,
        4,
    )
    .unwrap();
    assert_eq!(windows, [(0, now - 1)], "one request, for all of it");
    assert_eq!(refresh.candles.len(), 4);
    assert_eq!(refresh.thin, Some(true));

    let mut inner = kept_hours(now, 300);
    let grown = page_before_with(&mut inner, true, Timeframe::HOUR_1, 0, now, 4).unwrap();
    assert_eq!(grown.thin, Some(false));
}

#[test]
fn only_a_null_with_a_500_is_a_name_hyperliquid_does_not_list() {
    use crate::service::net::http;
    let refused = |status, body: &str| {
        Err(http::Error::Refused {
            status,
            body: body.to_owned(),
        })
    };
    let unknown = candles_of(refused(500, "null"), "btc", Timeframe::HOUR_1).unwrap_err();
    assert_eq!(
        unknown,
        ProviderError::unknown_market(
            DataSource::Hyperliquid,
            "btc",
            "names are case-sensitive, like BTC or kPEPE"
        )
    );
    assert_eq!(
        unknown.to_string(),
        "Hyperliquid does not list btc; names are case-sensitive, like BTC or kPEPE"
    );
    // Any other 500, a limit or a transport failure is what it is, and is not a missing market.
    for error in [
        candles_of(refused(500, "Internal error"), "btc", Timeframe::HOUR_1),
        candles_of(refused(429, "null"), "btc", Timeframe::HOUR_1),
        candles_of(Err(http::Error::Status(500)), "btc", Timeframe::HOUR_1),
    ] {
        assert!(!matches!(
            error.unwrap_err(),
            ProviderError::UnknownMarket { .. }
        ));
    }
    assert!(
        candles_of(refused(429, ""), "btc", Timeframe::HOUR_1)
            .unwrap_err()
            .is_rate_limited()
    );
}

#[test]
fn a_failed_window_is_the_answer() {
    let error = page_before_with(
        |_, _| Err(ProviderError::other("down")),
        false,
        Timeframe::HOUR_1,
        0,
        1_000 * HOUR,
        10,
    );
    assert_eq!(error.err(), Some(ProviderError::other("down")));
}

const CANDLES: &str = r#"[
    {"t":1795226400000,"T":1795229999999,"s":"@107","i":"1h","o":"60.482","c":"59.254","h":"60.549","l":"59.176","v":"132276.09","n":4654},
    {"t":1795222800000,"T":1795226399999,"s":"@107","i":"1h","o":"60.602","c":"60.482","h":"60.878","l":"60.141","v":"49038.36","n":3166}
]"#;

#[test]
fn candles_are_parsed_and_leave_oldest_first() {
    let candles = parse::parse_candles(CANDLES, Timeframe::HOUR_1).expect("valid response");
    assert_eq!(candles.len(), 2);
    assert_eq!(
        candles[0],
        Kline {
            open_time: 1_795_222_800_000,
            close_time: 1_795_226_399_999,
            open: 60.602,
            high: 60.878,
            low: 60.141,
            close: 60.482,
            volume: 49_038.36,
        }
    );
    assert_eq!(candles[1].open_time, 1_795_226_400_000);
    assert_eq!(
        parse::parse_candles("[]", Timeframe::HOUR_1),
        Ok(Vec::new())
    );
}

#[test]
fn an_answer_that_is_no_candle_list_is_a_data_error() {
    for body in [
        "null",
        r#"{"error":"x"}"#,
        r#"[{"t":1,"o":"1","h":"1","l":"1","c":"1"}]"#,
        r#"[{"t":"1","o":"1","h":"1","l":"1","c":"1","v":"1"}]"#,
        r#"[{"t":1,"o":"x","h":"1","l":"1","c":"1","v":"1"}]"#,
        "not json",
    ] {
        assert!(
            matches!(
                parse::parse_candles(body, Timeframe::HOUR_1),
                Err(ProviderError::Data(_))
            ),
            "{body}"
        );
    }
}

const META: &str = r#"{"universe":[
    {"szDecimals":5,"name":"BTC","maxLeverage":40,"marginTableId":56},
    {"szDecimals":1,"name":"MATIC","maxLeverage":20,"marginTableId":20,"isDelisted":true},
    {"szDecimals":0,"name":"kPEPE","maxLeverage":10,"marginTableId":10}
],"marginTables":[],"collateralToken":0}"#;

const XYZ_META: &str = r#"{"universe":[
    {"szDecimals":4,"name":"xyz:XYZ100","maxLeverage":30,"marginTableId":30,"growthMode":"enabled"},
    {"szDecimals":3,"name":"xyz:SP500","maxLeverage":25,"marginTableId":25},
    {"szDecimals":3,"name":"xyz:VIX","maxLeverage":10,"marginTableId":10,"isDelisted":true},
    {"szDecimals":2,"name":"xyz:SPCX","maxLeverage":10,"marginTableId":10}
]}"#;

const FLX_META: &str = r#"{"universe":[
    {"name":"flx:USA500","maxLeverage":10,"isDelisted":false},
    {"name":"km:US500","maxLeverage":25}
]}"#;

const SPOT_META: &str = r#"{"tokens":[
    {"name":"USDC","szDecimals":8,"weiDecimals":8,"index":0,"isCanonical":true},
    {"name":"PURR","szDecimals":0,"weiDecimals":5,"index":1,"isCanonical":true},
    {"name":"HYPE","szDecimals":2,"weiDecimals":8,"index":150,"isCanonical":false}
],"universe":[
    {"tokens":[1,0],"name":"PURR/USDC","index":0,"isCanonical":true},
    {"tokens":[150,0],"name":"@107","index":107,"isCanonical":false},
    {"tokens":[999,0],"name":"@999","index":999,"isCanonical":false}
]}"#;

fn listing() -> Vec<Market> {
    let mut markets = parse_perps(META).unwrap();
    markets.extend(parse_perps(XYZ_META).unwrap());
    markets.extend(parse_perps(FLX_META).unwrap());
    markets.extend(parse_spot(SPOT_META).unwrap());
    markets
}

fn ids(markets: &[Market]) -> Vec<&str> {
    markets.iter().map(|market| market.id.as_str()).collect()
}

#[test]
fn delisted_perpetuals_are_left_out_and_the_index_perpetuals_are_named() {
    let perps = parse_perps(XYZ_META).unwrap();
    assert_eq!(ids(&perps), ["xyz:XYZ100", "xyz:SP500", "xyz:SPCX"]);
    assert_eq!(perps[1].also_called.as_deref(), Some("S&P 500 (SPX)"));
    assert_eq!(perps[0].also_called.as_deref(), Some("Nasdaq 100 (NDX)"));
    assert_eq!(perps[2].also_called, None);
    assert!(perps.iter().all(|market| market.kind == MarketKind::Perp));
    assert_eq!(ids(&parse_perps(META).unwrap()), ["BTC", "kPEPE"]);
    assert!(parse_perps(r#"{"universe":"x"}"#).is_err());
}

#[test]
fn the_dexes_are_the_named_entries_after_the_main_dex() {
    let body = r#"[null,{"name":"xyz","fullName":"XYZ","deployer":"0x88"},{"name":"flx"},{"fullName":"nameless"}]"#;
    assert_eq!(parse_dexes(body).unwrap(), ["xyz", "flx"]);
    assert!(parse_dexes("{}").is_err());
}

#[test]
fn a_spot_pair_is_named_by_its_tokens_found_by_index_not_position() {
    let pairs = parse_spot(SPOT_META).unwrap();
    assert_eq!(
        ids(&pairs),
        ["PURR/USDC", "@107"],
        "a pair with an unknown token is left out"
    );
    assert_eq!(pairs[1].pair, Some(("HYPE".to_owned(), "USDC".to_owned())));
    assert!(pairs.iter().all(|market| market.kind == MarketKind::Spot));
}

/// The first hit a typed query gets, as the picker would rank it.
fn first_hit(query: &str) -> Option<SearchResult> {
    let hits = matching(&listing(), &normalize(query));
    rank_results(hits, query).into_iter().next()
}

#[test]
fn the_index_perpetuals_are_found_by_what_traders_call_them() {
    for query in ["S&P", "S&P 500", "SP500", "sp 500", "SPX"] {
        let hit = first_hit(query).unwrap_or_else(|| panic!("{query} finds nothing"));
        assert_eq!(hit.full_symbol, "xyz:SP500", "{query}");
        assert_eq!(hit.symbol, "xyz:SP500 — S&P 500 (SPX)", "{query}");
        assert_eq!(hit.kind, MarketKind::Perp);
    }
    for query in ["Nasdaq", "NDX", "Nasdaq 100", "xyz100", "xyz:XYZ100"] {
        let hit = first_hit(query).unwrap_or_else(|| panic!("{query} finds nothing"));
        assert_eq!(hit.full_symbol, "xyz:XYZ100", "{query}");
    }
}

#[test]
fn other_dexes_index_perpetuals_are_still_listed() {
    let hits = rank_results(matching(&listing(), "500"), "500");
    let found: Vec<&str> = hits.iter().map(|hit| hit.full_symbol.as_str()).collect();
    assert_eq!(found.len(), 3);
    for id in ["xyz:SP500", "flx:USA500", "km:US500"] {
        assert!(found.contains(&id), "{id} in {found:?}");
    }
}

#[test]
fn a_perpetual_and_a_spot_pair_are_listed_as_what_they_are() {
    let btc = first_hit("btc").expect("BTC");
    assert_eq!(
        (btc.symbol.as_str(), btc.full_symbol.as_str()),
        ("BTC", "BTC")
    );
    assert_eq!(btc.kind, MarketKind::Perp);
    assert_eq!(btc.base_asset(), "BTC");
    assert_eq!(first_hit("kpepe").expect("kPEPE").full_symbol, "kPEPE");

    let hype = first_hit("hype usdc").expect("HYPE/USDC");
    assert_eq!(hype.symbol, "HYPE/USDC");
    assert_eq!(hype.full_symbol, "@107");
    assert_eq!(
        (hype.base.as_deref(), hype.quote.as_deref()),
        (Some("HYPE"), Some("USDC"))
    );
    assert_eq!(hype.kind, MarketKind::Spot);
    assert_eq!(hype.base_asset(), "HYPE");
    assert_eq!(
        first_hit("purr").expect("PURR/USDC").full_symbol,
        "PURR/USDC"
    );
    assert!(first_hit("zzzz").is_none());
}

#[test]
fn a_market_is_perpetual_unless_its_name_is_a_spot_pair() {
    let kind = |id: &str| HYPERLIQUID.kind_of(&InstrumentId::new(DataSource::Hyperliquid, id));
    for perp in ["BTC", "kPEPE", "xyz:SP500", "hyna:BTC"] {
        assert_eq!(kind(perp), MarketKind::Perp, "{perp}");
    }
    for spot in ["@107", "PURR/USDC"] {
        assert_eq!(kind(spot), MarketKind::Spot, "{spot}");
    }
    let profile_kinds = DataSource::Hyperliquid.profile().kinds;
    assert!(profile_kinds.contains(&MarketKind::Perp) && profile_kinds.contains(&MarketKind::Spot));
}

#[test]
fn a_name_is_kept_as_hyperliquid_lists_it() {
    assert_eq!(coin_id("kPEPE"), "kPEPE");
    assert_eq!(coin_id(" xyz:SP500 "), "xyz:SP500");
    assert_eq!(ids::dex_of("xyz:SP500"), Some("xyz"));
    assert_eq!(ids::dex_of("BTC"), None);
    let source = DataSource::Hyperliquid;
    assert_eq!(source.display_symbol("xyz:SP500"), "xyz:SP500");
    assert_eq!(source.display_symbol("kPEPE"), "kPEPE");
    assert_eq!(source.base_asset("xyz:SP500"), "SP500");
    assert_eq!(source.base_asset("BTC"), "BTC");
    assert_eq!(source.base_asset("PURR/USDC"), "PURR");
    // Two names that differ only in case are two markets.
    assert_ne!(
        InstrumentId::new(source, "kPEPE").key(),
        InstrumentId::new(source, "KPEPE").key()
    );
}

#[test]
fn a_typed_venue_prefix_leaves_the_colon_of_a_dex_market_in_the_name() {
    let query = crate::model::market::venue::parse("HYPERLIQUID:xyz:SP500").expect("a prefix");
    assert_eq!(query.source, Some(DataSource::Hyperliquid));
    assert_eq!(query.symbol, "xyz:SP500");
    let lower = crate::model::market::venue::parse("hyperliquid: BTC").expect("a prefix");
    assert_eq!(
        (lower.source, lower.symbol.as_str()),
        (Some(DataSource::Hyperliquid), "BTC")
    );
    assert_eq!(
        InstrumentId::new(DataSource::Hyperliquid, "xyz:SP500").key(),
        "HYPERLIQUID:xyz:SP500"
    );
    assert_eq!(
        InstrumentId::rekey("HYPERLIQUID:xyz:SP500").as_deref(),
        Some("HYPERLIQUID:xyz:SP500")
    );
}

#[test]
fn a_watchlist_text_names_a_market_and_reads_it_back() {
    let text = "HYPERLIQUID:BTC\nHYPERLIQUID:xyz:SP500\nhyperliquid:kPEPE\nHYPERLIQUID:PURR/USDC\nHYPERLIQUID:@107";
    let parsed = watchlist::text::parse(text);
    assert!(parsed.skipped.is_empty(), "{:?}", parsed.skipped);
    let keys: Vec<String> = parsed
        .rows
        .iter()
        .filter_map(|row| match row {
            watchlist::text::Row::Instrument(item) => Some(item.instrument.key()),
            watchlist::text::Row::Section(_) => None,
        })
        .collect();
    assert_eq!(
        keys,
        [
            "HYPERLIQUID:BTC",
            "HYPERLIQUID:xyz:SP500",
            "HYPERLIQUID:kPEPE",
            "HYPERLIQUID:PURR/USDC",
            "HYPERLIQUID:@107"
        ]
    );
    let skipped = watchlist::text::parse("HYPERLIQUID:BTC USDT\nHYPERLIQUID:bad!\nHYPERLIQUID:");
    assert_eq!(skipped.skipped.len(), 3);
    assert!(skipped.skipped[0].reason.contains("exchange lists them"));
}

#[test]
fn a_market_checked_by_symbol_keeps_its_dex_prefix() {
    let id = watchlist::text::validate_market(DataSource::Hyperliquid, "xyz:SP500").unwrap();
    assert_eq!(id.symbol, "xyz:SP500");
    assert!(watchlist::text::validate_market(DataSource::Hyperliquid, "xyz SP500").is_err());
    // A colon is still refused where it is not part of a name.
    assert!(watchlist::text::validate_market(DataSource::Binance, "BTC:USDT").is_err());
}

/// Run with `cargo test --lib hyperliquid::tests::live_hyperliquid -- --ignored --nocapture`.
#[test]
#[ignore = "needs the network"]
fn live_hyperliquid_serves_candles_pages_back_and_streams() {
    use crate::service::market::live::LiveFeed;
    use crate::service::watchlist_quotes::quote_stream::QuoteSocket;
    use std::time::{Duration, Instant};

    let now = crate::clock::now_ms();
    let hourly = HYPERLIQUID
        .fetch_klines("BTC", Timeframe::HOUR_1, 500)
        .unwrap();
    println!(
        "BTC 1h: {} candles, newest opens {}",
        hourly.len(),
        hourly.last().unwrap().open_time
    );
    assert!(hourly.len() >= 400);
    assert!(
        hourly
            .windows(2)
            .all(|pair| pair[0].open_time < pair[1].open_time)
    );
    assert!(
        hourly
            .iter()
            .all(|candle| candle.volume >= 0.0 && candle.close > 0.0)
    );

    let oldest = hourly[0].open_time;
    let older = HYPERLIQUID
        .fetch_klines_range("BTC", Timeframe::HOUR_1, 0, oldest, 500)
        .unwrap();
    println!(
        "BTC 1h one page back: {} candles ending {}",
        older.len(),
        older.last().unwrap().open_time
    );
    assert!(older.len() >= 400 && older.last().unwrap().open_time < oldest);

    // Everything Hyperliquid keeps is ~5,000 candles: a page beyond is empty, not an error.
    let beyond = HYPERLIQUID
        .fetch_klines_range("BTC", Timeframe::MINUTE_1, 0, now - 10 * DAY, 500)
        .unwrap();
    println!("BTC 1m ten days back: {} candles", beyond.len());
    assert!(beyond.is_empty());

    // Every native opens on this app's grid; a Thursday week does not.
    for native in DataSource::Hyperliquid.natives() {
        let candles = HYPERLIQUID.fetch_klines("BTC", *native, 30).unwrap();
        assert!(!candles.is_empty(), "{native}");
        for candle in &candles {
            assert_eq!(
                bucket_start_ms(*native, candle.open_time),
                candle.open_time,
                "{native}"
            );
        }
    }
    let weekly = rest::info(&serde_json::json!({"type": "candleSnapshot", "req": {"coin": "BTC", "interval": "1w", "startTime": now - 60 * DAY, "endTime": now}})).unwrap();
    let weeks = parse::parse_candles(&weekly, Timeframe::WEEK_1).unwrap();
    println!(
        "BTC own 1w opens: {:?}",
        weeks
            .iter()
            .map(|week| week.open_time / DAY % 7)
            .collect::<Vec<_>>()
    );
    assert!(weeks.iter().all(|week| (week.open_time / DAY) % 7 == 0
        && bucket_start_ms(Timeframe::WEEK_1, week.open_time) != week.open_time));
    let folded = HYPERLIQUID
        .fetch_klines("BTC", Timeframe::WEEK_1, 10)
        .unwrap();
    println!(
        "BTC 1w folded from 1d: {} weeks, newest opens {}",
        folded.len(),
        folded.last().unwrap().open_time
    );
    assert!(
        folded
            .iter()
            .all(|week| bucket_start_ms(Timeframe::WEEK_1, week.open_time) == week.open_time)
    );

    let index = HYPERLIQUID
        .fetch_klines("xyz:SP500", Timeframe::HOUR_1, 200)
        .unwrap();
    println!(
        "xyz:SP500 1h: {} candles, last close {}",
        index.len(),
        index.last().unwrap().close
    );
    assert!(index.len() >= 100);
    let spot = HYPERLIQUID
        .fetch_klines("@107", Timeframe::HOUR_1, 50)
        .unwrap();
    println!(
        "@107 (HYPE/USDC) 1h: {} candles, last close {}",
        spot.len(),
        spot.last().unwrap().close
    );
    assert!(!spot.is_empty());
    let unknown = HYPERLIQUID.fetch_klines("NOPE", Timeframe::HOUR_1, 5);
    println!("NOPE: {unknown:?}");
    assert!(unknown.is_err());

    let hits = HYPERLIQUID.search_tickers("S&P 500").unwrap();
    println!(
        "search S&P 500: {:?}",
        hits.iter()
            .map(|hit| hit.symbol.as_str())
            .collect::<Vec<_>>()
    );
    assert_eq!(hits[0].full_symbol, "xyz:SP500");
    let hype = HYPERLIQUID.search_tickers("hype/usdc").unwrap();
    assert!(hype.iter().any(|hit| hit.full_symbol == "@107"));

    let requests: Vec<QuoteRequest> = ["BTC", "xyz:SP500", "@107", "NOPE"]
        .iter()
        .map(|symbol| QuoteRequest {
            symbol: (*symbol).to_owned(),
            chain: None,
        })
        .collect();
    let outcome = HYPERLIQUID.rest_quotes(&requests).unwrap();
    for symbol in ["BTC", "xyz:SP500", "@107"] {
        let quote = outcome
            .quotes
            .get(symbol)
            .unwrap_or_else(|| panic!("no REST quote for {symbol}"));
        println!("REST quote {symbol}: {quote:?}");
        assert!(quote.price > 0.0 && quote.previous_day_price.is_some());
    }
    assert!(!outcome.quotes.contains_key("NOPE") && outcome.failed.is_empty());

    // A market Hyperliquid does not list must not silence the others on the socket.
    let feed = LiveFeed::connect(
        DataSource::Hyperliquid,
        vec!["NOPE".to_owned(), "BTC".to_owned(), "xyz:SP500".to_owned()],
        Timeframe::MINUTE_1,
        eframe::egui::Context::default(),
    )
    .expect("Hyperliquid streams one-minute candles");
    let (sender, receiver) = std::sync::mpsc::channel();
    let _quotes = QuoteSocket::connect(
        DataSource::Hyperliquid,
        vec!["NOPE".to_owned(), "BTC".to_owned()],
        sender,
    )
    .expect("Hyperliquid has a ticker socket");
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut seen = std::collections::BTreeSet::new();
    while seen.len() < 2 {
        for (symbol, kline) in feed.inbox.drain() {
            assert!(kline.close > 0.0 && kline.close_time > kline.open_time);
            if !seen.contains(&symbol) {
                println!("socket candle {symbol}: {kline:?}");
            }
            seen.insert(symbol);
        }
        assert!(Instant::now() < deadline, "only {seen:?} in 30 s");
        std::thread::sleep(Duration::from_millis(200));
    }
    assert_eq!(seen.into_iter().collect::<Vec<_>>(), ["BTC", "xyz:SP500"]);
    let tick = receiver
        .recv_timeout(Duration::from_secs(30))
        .expect("a quote tick in 30 s");
    println!("socket quote {}: {:?}", tick.product_id, tick.quote);
    assert_eq!(tick.product_id, "BTC");
    assert!(tick.quote.price > 0.0 && tick.quote.previous_day_price.is_some());
}
