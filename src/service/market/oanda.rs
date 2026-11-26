//! Oanda's v3 API: OHLCV candles of any instrument of the account, and the account's instrument
//! list for search. Both need a login — a personal access token and the account id (Settings ▸
//! Oanda) — sent as a bearer token; the candles endpoint itself takes only the token.
//!
//! Oanda aligns its hourly, daily and weekly bars to 5pm New York by default, which puts an H12
//! bar across two UTC days. Every request here instead asks for UTC alignment
//! (`alignmentTimezone=UTC&dailyAlignment=0&weeklyAlignment=Monday`), so the seconds, minutes,
//! hours, days and weeks it returns open exactly on the app's UTC grid. Its monthly candle is
//! stamped on the last day of the previous month even then, off that grid, so months are folded
//! from days rather than asked for.

use super::candles::plan;
use super::catalog::Catalog;
use super::search_rank::normalize;
use crate::model::{DataSource, Kline, TimeUnit, Timeframe};
use crate::service::{MarketSource, ProviderError, SearchResult, rank_results};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

/// Oanda's live trading API, the one the standalone `oandaohlc` downloader uses.
pub(in crate::service) const BASE_URL: &str = "https://api-fxtrade.oanda.com/v3";

pub struct Oanda;

pub const OANDA: Oanda = Oanda;

/// The most candles one Oanda request returns.
const MAX_PER_REQUEST: usize = 5_000;

/// Pages of a range are fetched side by side, as Binance's are.
const PARALLEL: usize = 8;

fn credentials() -> Result<(String, String), ProviderError> {
    crate::service::keychain::api_keys::oanda_credentials()
        .ok_or(ProviderError::MissingKey(DataSource::Oanda))
}

/// The `Authorization` header value for a token.
fn bearer(token: &str) -> String {
    format!("Bearer {}", token.trim())
}

/// The `granularity` value Oanda takes for a native length: `S5`, `M1`, `H4`, `D`, `W`, `M`.
fn granularity(bar: Timeframe) -> String {
    let count = bar.count();
    match bar.unit() {
        TimeUnit::Second => format!("S{count}"),
        TimeUnit::Minute => format!("M{count}"),
        TimeUnit::Hour => format!("H{count}"),
        TimeUnit::Day => "D".to_owned(),
        TimeUnit::Week => "W".to_owned(),
        TimeUnit::Month => "M".to_owned(),
    }
}

/// Settings ▸ Oanda ▸ Test Connection: the account's instrument list, asked for with the given
/// credentials (the ones typed in the fields, saved or not), which is what Oanda charts need them
/// for.
pub fn test_connection(account_id: &str, access_token: &str) -> Result<(), ProviderError> {
    let url = format!("{BASE_URL}/accounts/{}/instruments", account_id.trim());
    let auth = bearer(access_token);
    crate::service::net::http::get_text(
        &url,
        &[("Authorization", auth.as_str())],
        crate::service::net::http::Retry::Never,
    )
    .map(|_| ())
    .map_err(ProviderError::Oanda)
}

impl MarketSource for Oanda {
    fn fetch_klines(
        &self,
        symbol: &str,
        timeframe: Timeframe,
        limit: u32,
    ) -> Result<Vec<Kline>, ProviderError> {
        let plan = plan(DataSource::Oanda, timeframe)?;
        let requested = plan.base_count(limit as usize);
        if requested > MAX_PER_REQUEST {
            // More finer candles than one request holds: page back from now.
            let end_ms = crate::clock::now_ms() + 1;
            return self.fetch_klines_range(symbol, timeframe, 0, end_ms, limit as usize);
        }
        let body = candles_request(symbol, plan.base(), None, None, requested)?;
        Ok(plan.shape(
            parse_candles(&body, plan.base())?,
            requested,
            limit as usize,
        ))
    }

    fn fetch_klines_range(
        &self,
        symbol: &str,
        timeframe: Timeframe,
        start_ms: i64,
        end_ms: i64,
        max: usize,
    ) -> Result<Vec<Kline>, ProviderError> {
        let plan = plan(DataSource::Oanda, timeframe)?;
        let base = plan.base();
        let requested = plan.base_count(max);
        let page_ms = base.nominal_ms() * MAX_PER_REQUEST as i64;
        let fetch_page = |from: i64, to: i64| -> Result<Vec<Kline>, ProviderError> {
            let body = candles_request(symbol, base, Some(from), Some(to), MAX_PER_REQUEST)?;
            Ok(parse_candles(&body, base)?
                .into_iter()
                .filter(|candle| candle.open_time >= from && candle.open_time < to)
                .collect())
        };
        let mut candles = page_back(start_ms, end_ms, requested, page_ms, &fetch_page)?;
        candles.retain(|candle| candle.open_time >= start_ms && candle.open_time < end_ms);
        candles.sort_by_key(|candle| candle.open_time);
        candles.dedup_by_key(|candle| candle.open_time);
        if candles.len() > requested {
            candles.drain(..candles.len() - requested);
        }
        Ok(plan.shape(candles, requested, max))
    }

    fn search_tickers(&self, query: &str) -> Result<Vec<SearchResult>, ProviderError> {
        // Credentials first: with none saved, a cached list must not make search seem available.
        let (account_id, token) = credentials()?;
        let instruments = catalog(account_id.trim()).get(|| download_instruments(&account_id, &token))?;
        // `EUR/USD` and `eur_usd` find `EUR_USD`: Oanda writes an underscore.
        let needle = normalize(query);
        if needle.is_empty() {
            return Ok(Vec::new());
        }
        let results = matching_instruments(&instruments, &needle);
        Ok(rank_results(results, query).into_iter().take(30).collect())
    }
}

/// The URL of one candles request: mid prices of `bar`, aligned to UTC midnight so the bars open
/// on the app's grid, either the newest `count` of them or the ones in `[from_ms, to_ms)` when a
/// window is given. Oanda's spec says `count` must not be sent together with `from`/`to`, so a
/// window carries no count.
fn candles_url(
    instrument: &str,
    bar: Timeframe,
    from_ms: Option<i64>,
    to_ms: Option<i64>,
    count: usize,
) -> String {
    let mut url = format!(
        "{BASE_URL}/instruments/{instrument}/candles?price=M&granularity={}&alignmentTimezone=UTC&dailyAlignment=0&weeklyAlignment=Monday",
        granularity(bar),
    );
    if from_ms.is_none() && to_ms.is_none() {
        url.push_str(&format!("&count={}", count.clamp(1, MAX_PER_REQUEST)));
    } else {
        if let Some(from) = from_ms {
            url.push_str(&format!("&from={}", crate::format::iso8601(from)));
        }
        if let Some(to) = to_ms {
            url.push_str(&format!("&to={}", crate::format::iso8601(to)));
        }
    }
    url
}

/// How many pages in a row may come back empty before the answer counts as the start of the
/// market's history: a closed market (the weekend closes forex from Friday to Sunday) leaves
/// whole wall-clock pages empty, and that must not end the chart's history.
const MAX_EMPTY_PAGES: usize = 8;

/// Pages `[start_ms, end_ms)` back from `end_ms` in pages of `page_ms`, in batches of
/// `PARALLEL`, until `requested` candles are held, the floor `start_ms` is reached, or
/// `MAX_EMPTY_PAGES` pages in a row came back empty.
fn page_back(
    start_ms: i64,
    end_ms: i64,
    requested: usize,
    page_ms: i64,
    fetch: &(dyn Fn(i64, i64) -> Result<Vec<Kline>, ProviderError> + Sync),
) -> Result<Vec<Kline>, ProviderError> {
    let mut candles: Vec<Kline> = Vec::new();
    let mut empty_pages = 0;
    let mut next_end = end_ms;
    while next_end > start_ms && candles.len() < requested && empty_pages < MAX_EMPTY_PAGES {
        let mut starts: Vec<i64> = (1..=PARALLEL as i64)
            .map(|index| next_end - index * page_ms)
            .take_while(|start| *start > start_ms)
            .collect();
        if starts.is_empty() {
            // The rest of the range is shorter than a page: one last page at the floor.
            starts.push(start_ms);
        }
        let batch_end = next_end;
        let pages: Vec<Result<Vec<Kline>, ProviderError>> = std::thread::scope(|scope| {
            let handles: Vec<_> = starts
                .iter()
                .map(|&from| {
                    scope.spawn(move || {
                        let to = (from + page_ms).min(batch_end);
                        fetch(from, to)
                    })
                })
                .collect();
            handles
                .into_iter()
                .map(|handle| {
                    handle.join().unwrap_or_else(|_| {
                        Err(ProviderError::other("The candle download stopped."))
                    })
                })
                .collect()
        });
        let mut got = 0;
        for page in pages {
            let page = page?;
            got += page.len();
            candles.extend(page);
        }
        empty_pages = if got == 0 {
            empty_pages + starts.len()
        } else {
            0
        };
        next_end = *starts.last().expect("one start at least");
    }
    Ok(candles)
}

/// One candles request, with the saved credentials.
fn candles_request(
    instrument: &str,
    bar: Timeframe,
    from_ms: Option<i64>,
    to_ms: Option<i64>,
    count: usize,
) -> Result<String, ProviderError> {
    let (_, token) = credentials()?;
    let auth = bearer(&token);
    crate::service::net::http::get_text(
        &candles_url(instrument, bar, from_ms, to_ms, count),
        &[("Authorization", auth.as_str())],
        crate::service::net::http::Retry::Transient,
    )
    .map_err(ProviderError::Oanda)
}

/// `{"candles": [{"time": ISO, "volume": n, "complete": bool, "mid": {"o","h","l","c"}}]}`, oldest
/// first. The forming (incomplete) last candle is kept, as Binance's is.
fn parse_candles(body: &str, bar: Timeframe) -> Result<Vec<Kline>, ProviderError> {
    let value: Value = serde_json::from_str(body).map_err(ProviderError::data)?;
    let rows = value
        .get("candles")
        .and_then(Value::as_array)
        .ok_or_else(|| ProviderError::data("expected a candles object"))?;
    rows.iter().map(|row| parse_candle(row, bar)).collect()
}

fn parse_candle(row: &Value, bar: Timeframe) -> Result<Kline, ProviderError> {
    let open_time = row
        .get("time")
        .and_then(Value::as_str)
        .and_then(crate::format::parse_iso8601)
        .ok_or_else(|| ProviderError::data("invalid candle time"))?;
    let mid = row
        .get("mid")
        .ok_or_else(|| ProviderError::data("expected mid prices"))?;
    let price = |key: &str| -> Result<f64, ProviderError> {
        mid.get(key)
            .and_then(Value::as_str)
            .and_then(|text| text.parse().ok())
            .ok_or_else(|| ProviderError::data(format!("invalid price {key}")))
    };
    let volume = row.get("volume").and_then(Value::as_f64).unwrap_or(0.0);
    Ok(Kline {
        open_time,
        // The bars are asked for on the UTC grid, so a candle closes a millisecond before the
        // next one opens.
        close_time: open_time + bar.nominal_ms() - 1,
        open: price("o")?,
        high: price("h")?,
        low: price("l")?,
        close: price("c")?,
        volume,
    })
}

/// One instrument of the account from `/accounts/{id}/instruments`: all the search needs of it.
#[derive(Clone, Debug, Deserialize, Serialize)]
struct Instrument {
    name: String,
}

/// One catalog per account, each downloaded at most once a day. The instrument list belongs to
/// the account's credentials, so after an account switch the new account must not be searched
/// through the old account's list.
static CATALOGS: LazyLock<Mutex<HashMap<String, &'static Catalog<Instrument>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// The account's catalog, created on first use. Accounts are a handful, so the entries (and the
/// leaked namespaces they name) live for the app's lifetime and are never trimmed.
fn catalog(account_id: &str) -> &'static Catalog<Instrument> {
    let mut catalogs = CATALOGS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    *catalogs.entry(account_id.to_owned()).or_insert_with(|| {
        let namespace: &'static str =
            Box::leak(format!("catalog_oanda_{account_id}").into_boxed_str());
        Box::leak(Box::new(Catalog::new(namespace)))
    })
}

/// Forgets every account's list held in memory (after the disk cache was cleared).
pub(in crate::service) fn clear_catalog() {
    let catalogs = CATALOGS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    for catalog in catalogs.values() {
        catalog.clear_memory();
    }
}

fn download_instruments(account_id: &str, token: &str) -> Result<Vec<Instrument>, ProviderError> {
    let url = format!("{BASE_URL}/accounts/{account_id}/instruments");
    let auth = bearer(token);
    let body = crate::service::net::http::get_text(
        &url,
        &[("Authorization", auth.as_str())],
        crate::service::net::http::Retry::Transient,
    )
    .map_err(ProviderError::Oanda)?;
    parse_instruments(&body)
}

fn parse_instruments(body: &str) -> Result<Vec<Instrument>, ProviderError> {
    let value: Value = serde_json::from_str(body).map_err(ProviderError::data)?;
    Ok(value
        .get("instruments")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .filter_map(|instrument| {
            Some(Instrument {
                name: instrument.get("name")?.as_str()?.to_owned(),
            })
        })
        .collect())
}

/// The listed instruments whose name contains `needle` (letters and digits, upper case).
fn matching_instruments(instruments: &[Instrument], needle: &str) -> Vec<SearchResult> {
    instruments
        .iter()
        .filter(|instrument| normalize(&instrument.name).contains(needle))
        .map(|instrument| {
            let result = SearchResult::new(
                DataSource::Oanda,
                instrument.name.clone(),
                instrument.name.clone(),
            );
            // Oanda names a market `BASE_QUOTE`.
            match instrument.name.split_once('_') {
                Some((base, quote)) if !base.is_empty() && !quote.is_empty() => {
                    result.pair(base, quote)
                }
                _ => result,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    const DAY_MS: i64 = 86_400_000;

    #[test]
    fn parses_oanda_candles_with_their_forming_one() {
        let body = r#"{
            "instrument": "EUR_USD",
            "granularity": "H1",
            "candles": [
                {"time":"2026-10-09T13:00:00.000000000Z","volume":812,"complete":true,
                 "mid":{"o":"1.10000","h":"1.10500","l":"1.09900","c":"1.10200"}},
                {"time":"2026-10-09T14:00:00.000000000Z","volume":120,"complete":false,
                 "mid":{"o":"1.10200","h":"1.10300","l":"1.10100","c":"1.10250"}}
            ]
        }"#;
        let candles = parse_candles(body, Timeframe::HOUR_1).expect("valid response");
        assert_eq!(candles.len(), 2, "the forming candle is kept");
        let open = crate::format::days_from_civil(2026, 10, 9) * DAY_MS + 13 * 3_600_000;
        assert_eq!(
            candles[0],
            Kline {
                open_time: open,
                close_time: open + 3_600_000 - 1,
                open: 1.1,
                high: 1.105,
                low: 1.099,
                close: 1.102,
                volume: 812.0,
            }
        );
        assert_eq!(candles[1].open_time, open + 3_600_000);
        assert_eq!(candles[1].close_time, candles[1].open_time + 3_600_000 - 1);
    }

    #[test]
    fn rejects_error_objects() {
        let error = parse_candles(
            r#"{"errorCode":"some","errorMessage":"bad"}"#,
            Timeframe::HOUR_1,
        )
        .expect_err("object must fail");
        assert_eq!(error, ProviderError::data("expected a candles object"));
        assert!(parse_candles("not json", Timeframe::HOUR_1).is_err());
    }

    #[test]
    fn a_candles_request_asks_for_mid_prices_of_the_granularity_on_the_utc_grid() {
        let from = crate::format::days_from_civil(2026, 10, 9) * DAY_MS;
        let to = from + DAY_MS;
        assert_eq!(
            candles_url("EUR_USD", Timeframe::HOUR_4, None, None, 500),
            "https://api-fxtrade.oanda.com/v3/instruments/EUR_USD/candles?price=M&granularity=H4&alignmentTimezone=UTC&dailyAlignment=0&weeklyAlignment=Monday&count=500"
        );
        // A window carries no count: Oanda's spec refuses count together with from/to.
        assert_eq!(
            candles_url("XAU_USD", Timeframe::DAY_1, Some(from), Some(to), 5_000),
            format!(
                "https://api-fxtrade.oanda.com/v3/instruments/XAU_USD/candles?price=M&granularity=D&alignmentTimezone=UTC&dailyAlignment=0&weeklyAlignment=Monday&from={}&to={}",
                crate::format::iso8601(from),
                crate::format::iso8601(to)
            )
        );
        // The count is capped at what one request holds.
        assert!(
            candles_url("EUR_USD", Timeframe::MINUTE_1, None, None, 9_999).ends_with("count=5000")
        );
    }

    #[test]
    fn natives_map_to_oandas_granularities() {
        let tokens: Vec<String> = DataSource::Oanda
            .natives()
            .iter()
            .map(|native| granularity(*native))
            .collect();
        assert_eq!(
            tokens,
            [
                "S5", "S10", "S15", "S30", "M1", "M2", "M4", "M5", "M10", "M15", "M30", "H1", "H2",
                "H4", "H6", "H8", "H12", "D", "W"
            ]
        );
    }

    fn base_of(timeframe: &str) -> (String, usize) {
        let plan = plan(DataSource::Oanda, Timeframe::parse(timeframe).unwrap()).unwrap();
        (granularity(plan.base()), plan.base_count(500))
    }

    #[test]
    fn a_native_length_is_asked_for_as_it_is() {
        for (token, granularity) in [
            ("5s", "S5"),
            ("30s", "S30"),
            ("1m", "M1"),
            ("4m", "M4"),
            ("15m", "M15"),
            ("30m", "M30"),
            ("1h", "H1"),
            ("4h", "H4"),
            ("12h", "H12"),
            ("1d", "D"),
            ("1w", "W"),
        ] {
            assert_eq!(base_of(token), (granularity.to_owned(), 500), "{token}");
        }
    }

    #[test]
    fn other_lengths_fold_the_coarsest_native_that_nests() {
        // A request for 500 candles also asks for the one the start cuts.
        assert_eq!(base_of("22m"), ("M2".to_owned(), 501 * 11));
        assert_eq!(base_of("45m"), ("M15".to_owned(), 501 * 3));
        assert_eq!(base_of("3h"), ("H1".to_owned(), 501 * 3));
        assert_eq!(base_of("8h"), ("H8".to_owned(), 500));
        assert_eq!(base_of("2d"), ("D".to_owned(), 501 * 2));
        assert_eq!(base_of("2w"), ("W".to_owned(), 501 * 2));
        // Oanda's monthly candle is stamped off the UTC grid, so months fold from days, sized
        // for the longest month.
        assert_eq!(base_of("1M"), ("D".to_owned(), 501 * 31));
        assert_eq!(base_of("3M"), ("D".to_owned(), 501 * 93));
        assert_eq!(base_of("1Y"), ("D".to_owned(), 501 * 372));
        // Oanda has no second candles, so 1s cannot be folded to.
        assert!(plan(DataSource::Oanda, Timeframe::SECOND_1).is_err());
    }

    fn candle_at(t: i64) -> Kline {
        Kline {
            open_time: t,
            close_time: t,
            open: 1.0,
            high: 1.0,
            low: 1.0,
            close: 1.0,
            volume: 1.0,
        }
    }

    #[test]
    fn paging_back_crosses_a_closed_market_instead_of_ending_history() {
        // The market trades everywhere except [40, 60) — a "weekend".
        let fetch = |from: i64, to: i64| -> Result<Vec<Kline>, ProviderError> {
            Ok((from..to)
                .filter(|&t| t < 40 || t >= 60)
                .map(candle_at)
                .collect())
        };
        // The oldest candle held opens at 65, just after the gap: the pages right before it
        // are empty, and paging must go through them to the candles before the gap.
        let candles = page_back(0, 65, 5, 10, &fetch).expect("pages");
        assert!(candles.len() >= 5);
        assert!(candles.iter().any(|c| c.open_time == 62), "after the gap");
        assert!(candles.iter().any(|c| c.open_time == 38), "before the gap");
    }

    #[test]
    fn empty_pages_stop_after_the_cap_so_history_ends_once() {
        let calls = AtomicUsize::new(0);
        let fetch = |_from: i64, _to: i64| -> Result<Vec<Kline>, ProviderError> {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok(Vec::new())
        };
        let candles = page_back(0, 1_000, 100, 10, &fetch).expect("pages");
        assert!(candles.is_empty());
        assert_eq!(calls.load(Ordering::SeqCst), MAX_EMPTY_PAGES);
    }

    #[test]
    fn paging_stops_at_the_requested_floor() {
        let calls = AtomicUsize::new(0);
        let fetch = |from: i64, to: i64| -> Result<Vec<Kline>, ProviderError> {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok((from..to).filter(|&t| t >= 60).map(candle_at).collect())
        };
        // The range [50, 65) holds [50, 60) closed and [60, 65) traded.
        let candles = page_back(50, 65, 100, 10, &fetch).expect("pages");
        assert_eq!(candles.len(), 5);
        assert!(candles.iter().all(|c| (50..65).contains(&c.open_time)));
        // Two pages asked: [55, 65), then the partial page [50, 55) at the floor — never below.
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn a_series_of_more_than_one_requests_candles_is_paged_but_its_tail_is_not() {
        assert!(base_of("5m").1 <= MAX_PER_REQUEST);
        assert!(base_of("22m").1 > MAX_PER_REQUEST);
        let tail = plan(DataSource::Oanda, Timeframe::parse("22m").unwrap())
            .unwrap()
            .base_count(crate::service::cache::TAIL_LIMIT as usize);
        assert!(tail <= MAX_PER_REQUEST);
    }

    #[test]
    fn an_instrument_is_found_however_it_is_separated() {
        let instruments = vec![
            Instrument {
                name: "EUR_USD".into(),
            },
            Instrument {
                name: "XAU_USD".into(),
            },
            Instrument {
                name: "SPX500_USD".into(),
            },
        ];
        for query in ["eurusd", "EUR/USD", "eur_usd", "EUR USD"] {
            let found = matching_instruments(&instruments, &normalize(query));
            assert_eq!(found.len(), 1, "{query}");
            assert_eq!(found[0].symbol, "EUR_USD");
            assert_eq!(found[0].base.as_deref(), Some("EUR"));
            assert_eq!(found[0].quote.as_deref(), Some("USD"));
        }
        assert_eq!(matching_instruments(&instruments, "USD").len(), 3);
        assert_eq!(matching_instruments(&instruments, "XAU").len(), 1);
    }

    #[test]
    fn the_instrument_list_keeps_only_named_ones() {
        let body = r#"{"instruments":[
            {"name":"EUR_USD","type":"CURRENCY"},
            {"name":"XAU_USD","type":"METAL"},
            {"type":"CFD"}
        ],"lastTransactionID":"1"}"#;
        let instruments = parse_instruments(body).expect("instruments");
        assert_eq!(instruments.len(), 2);
        assert_eq!(instruments[0].name, "EUR_USD");
        assert!(parse_instruments("not json").is_err());
        assert!(parse_instruments("{}").expect("no instruments").is_empty());
    }

    #[test]
    fn each_account_searches_its_own_list_and_clearing_forgets_them_all() {
        use crate::service::cache::disk::{DiskCache, testing::TempCache};
        use crate::service::cache::lookup::LookupCache;

        let temp = TempCache::new();
        let disk: &'static DiskCache = Box::leak(Box::new(DiskCache::at(temp.path())));
        let now = || 1_700_000_000_000;
        let cache = LookupCache::for_tests(disk, Box::new(now));
        fn names(items: &std::sync::Arc<Vec<Instrument>>) -> Vec<&str> {
            items.iter().map(|i| i.name.as_str()).collect()
        }

        let first = catalog("account-7-a");
        let second = catalog("account-7-b");
        assert!(!std::ptr::eq(first, second), "different accounts, different catalogs");
        assert!(std::ptr::eq(first, catalog("account-7-a")), "the same account keeps its catalog");

        let a = first
            .get_from(&cache, || Ok(vec![Instrument { name: "EUR_USD".into() }]))
            .expect("account A's list");
        let b = second
            .get_from(&cache, || Ok(vec![Instrument { name: "USD_JPY".into() }]))
            .expect("account B's list");
        assert_eq!(names(&a), ["EUR_USD"]);
        assert_eq!(names(&b), ["USD_JPY"]);
        cache.flush("catalog_oanda_account-7-a");
        cache.flush("catalog_oanda_account-7-b");

        // A relaunch (empty memory, a fresh cache object) reads each account's own persisted
        // list, not the other account's.
        let relaunched = LookupCache::for_tests(disk, Box::new(now));
        first.clear_memory();
        second.clear_memory();
        let a = first
            .get_from(&relaunched, || Err(ProviderError::other("must not be asked")))
            .expect("account A's persisted list");
        let b = second
            .get_from(&relaunched, || Err(ProviderError::other("must not be asked")))
            .expect("account B's persisted list");
        assert_eq!(names(&a), ["EUR_USD"]);
        assert_eq!(names(&b), ["USD_JPY"]);

        // Clearing the catalogs (the disk cache was cleared) forgets them all: with an empty
        // disk too, both accounts download again.
        let empty = TempCache::new();
        let empty_disk: &'static DiskCache = Box::leak(Box::new(DiskCache::at(empty.path())));
        let empty_cache = LookupCache::for_tests(empty_disk, Box::new(|| 1_700_000_000_000));
        clear_catalog();
        let downloads = AtomicUsize::new(0);
        let again = first
            .get_from(&empty_cache, || {
                downloads.fetch_add(1, Ordering::SeqCst);
                Ok(vec![Instrument { name: "GBP_USD".into() }])
            })
            .expect("account A downloads again");
        assert_eq!(names(&again), ["GBP_USD"]);
        assert_eq!(downloads.load(Ordering::SeqCst), 1);
    }
}
