//! Oanda's v3 API: OHLCV candles of any instrument of the account, and the account's instrument
//! list for search. Both need a login — a personal access token and the account id (Settings ▸
//! Oanda) — sent as a bearer token; the candles endpoint itself takes only the token.
//!
//! Oanda stamps its day, week and month candles at 5pm New York (21:00 or 22:00 UTC) rather than
//! at UTC midnight: the offset into the UTC day is kept, as Alpaca's New York midnight offset is,
//! so a fold still lands the candle in the bucket its stamp sits in.

use super::candles::plan;
use super::catalog::Catalog;
use super::search_rank::normalize;
use crate::model::{DataSource, Kline, TimeUnit, Timeframe};
use crate::service::{MarketSource, ProviderError, SearchResult, rank_results};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Oanda's live trading API, the one the standalone `oandaohlc` downloader uses.
pub(in crate::service) const BASE_URL: &str = "https://api-fxtrade.oanda.com/v3";

const DAY_MS: i64 = 86_400_000;

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
        const PAGE: usize = MAX_PER_REQUEST;
        let plan = plan(DataSource::Oanda, timeframe)?;
        let base = plan.base();
        let requested = plan.base_count(max);
        let step = base.nominal_ms();
        let page_ms = step * PAGE as i64;
        let first = (end_ms - step * requested as i64).max(start_ms);
        let starts: Vec<i64> = (0..)
            .map(|index| first + index * page_ms)
            .take_while(|start| *start < end_ms)
            .collect();
        let mut candles: Vec<Kline> = Vec::new();
        for batch in starts.chunks(PARALLEL) {
            let pages: Vec<Result<Vec<Kline>, ProviderError>> = std::thread::scope(|scope| {
                let handles: Vec<_> = batch
                    .iter()
                    .map(|&from| {
                        scope.spawn(move || {
                            let page_end = (from + page_ms).min(end_ms);
                            let body =
                                candles_request(symbol, base, Some(from), Some(page_end), PAGE)?;
                            Ok(parse_candles(&body, base)?
                                .into_iter()
                                .filter(|candle| {
                                    candle.open_time >= from && candle.open_time < page_end
                                })
                                .collect::<Vec<_>>())
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
            for page in pages {
                candles.extend(page?);
            }
        }
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
        let instruments = CATALOG.get(|| download_instruments(&account_id, &token))?;
        // `EUR/USD` and `eur_usd` find `EUR_USD`: Oanda writes an underscore.
        let needle = normalize(query);
        if needle.is_empty() {
            return Ok(Vec::new());
        }
        let results = matching_instruments(&instruments, &needle);
        Ok(rank_results(results, query).into_iter().take(30).collect())
    }
}

/// The URL of one candles request: mid prices of `bar`, the newest `count` of them, or the ones
/// in `[from_ms, to_ms)` when a window is given.
fn candles_url(
    instrument: &str,
    bar: Timeframe,
    from_ms: Option<i64>,
    to_ms: Option<i64>,
    count: usize,
) -> String {
    let mut url = format!(
        "{BASE_URL}/instruments/{instrument}/candles?price=M&granularity={}&count={}",
        granularity(bar),
        count.clamp(1, MAX_PER_REQUEST)
    );
    if let Some(from) = from_ms {
        url.push_str(&format!("&from={}", crate::format::iso8601(from)));
    }
    if let Some(to) = to_ms {
        url.push_str(&format!("&to={}", crate::format::iso8601(to)));
    }
    url
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
        close_time: close_ms(bar, open_time),
        open: price("o")?,
        high: price("h")?,
        low: price("l")?,
        close: price("c")?,
        volume,
    })
}

/// Where a candle of `bar` that opens at `open_ms` ends: a millisecond before the next one opens.
/// A month's stamp is the last day of the month before the one the candle covers (5pm New York),
/// so its close is the last day of the covered month at the same time of day.
fn close_ms(bar: Timeframe, open_ms: i64) -> i64 {
    match bar.unit() {
        TimeUnit::Month => {
            let days = open_ms.div_euclid(DAY_MS);
            let (year, month, _) = crate::format::civil_from_days(days);
            let (covered_year, covered_month) = if month == 12 {
                (year + 1, 1)
            } else {
                (year, month + 1)
            };
            // The first day of the month after the covered one; the day before it is the close.
            let after = crate::format::days_from_civil(
                covered_year + covered_month / 12,
                covered_month % 12 + 1,
                1,
            );
            (after - 1) * DAY_MS + (open_ms - days * DAY_MS) - 1
        }
        _ => open_ms + bar.nominal_ms() - 1,
    }
}

/// One instrument of the account from `/accounts/{id}/instruments`: all the search needs of it.
#[derive(Clone, Debug, Deserialize, Serialize)]
struct Instrument {
    name: String,
}

/// The account's instruments, downloaded at most once a day.
static CATALOG: Catalog<Instrument> = Catalog::new("catalog_oanda");

/// Forgets the list held in memory (after the disk cache was cleared).
pub(in crate::service) fn clear_catalog() {
    CATALOG.clear_memory();
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
    fn a_monthly_candle_closes_on_the_last_day_of_the_month_it_covers() {
        // Oanda stamps January's candle on 31 Jan 2025 at 22:00 UTC (5pm New York); it covers
        // February, so it closes on 28 Feb 2025 at 22:00 UTC, a millisecond before 29 Feb would
        // open — 2025 is not a leap year.
        let jan31 = crate::format::days_from_civil(2025, 1, 31) * DAY_MS + 22 * 3_600_000;
        let close = close_ms(Timeframe::MONTH_1, jan31);
        let feb28 = crate::format::days_from_civil(2025, 2, 28) * DAY_MS + 22 * 3_600_000;
        assert_eq!(close, feb28 - 1);
        // A leap February's candle closes on the 29th.
        let jan31_2024 = crate::format::days_from_civil(2024, 1, 31) * DAY_MS + 22 * 3_600_000;
        let feb29_2024 = crate::format::days_from_civil(2024, 2, 29) * DAY_MS + 22 * 3_600_000;
        assert_eq!(close_ms(Timeframe::MONTH_1, jan31_2024), feb29_2024 - 1);
        // December's stamp covers January.
        let dec31 = crate::format::days_from_civil(2024, 12, 31) * DAY_MS + 22 * 3_600_000;
        let jan31_2025 = crate::format::days_from_civil(2025, 1, 31) * DAY_MS + 22 * 3_600_000;
        assert_eq!(close_ms(Timeframe::MONTH_1, dec31), jan31_2025 - 1);
    }

    #[test]
    fn daily_and_weekly_candles_span_their_nominal_length() {
        let day = crate::format::days_from_civil(2026, 10, 9) * DAY_MS + 21 * 3_600_000;
        assert_eq!(close_ms(Timeframe::DAY_1, day), day + DAY_MS - 1);
        assert_eq!(close_ms(Timeframe::WEEK_1, day), day + 7 * DAY_MS - 1);
        assert_eq!(close_ms(Timeframe::MINUTE_5, day), day + 5 * 60_000 - 1);
    }

    #[test]
    fn a_candles_request_asks_for_mid_prices_of_the_granularity() {
        let from = crate::format::days_from_civil(2026, 10, 9) * DAY_MS;
        let to = from + DAY_MS;
        assert_eq!(
            candles_url("EUR_USD", Timeframe::HOUR_4, None, None, 500),
            "https://api-fxtrade.oanda.com/v3/instruments/EUR_USD/candles?price=M&granularity=H4&count=500"
        );
        assert_eq!(
            candles_url("XAU_USD", Timeframe::DAY_1, Some(from), Some(to), 5_000),
            format!(
                "https://api-fxtrade.oanda.com/v3/instruments/XAU_USD/candles?price=M&granularity=D&count=5000&from={}&to={}",
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
                "H4", "H6", "H8", "H12", "D", "W", "M"
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
            ("1M", "M"),
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
        assert_eq!(base_of("3M"), ("M".to_owned(), 501 * 3));
        assert_eq!(base_of("1Y"), ("M".to_owned(), 501 * 12));
        // Oanda has no second candles, so 1s cannot be folded to.
        assert!(plan(DataSource::Oanda, Timeframe::SECOND_1).is_err());
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
}
