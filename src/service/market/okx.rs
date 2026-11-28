//! OKX spot candles and its market list. Public endpoints, no key.
//!
//! Rows are `[open time ms, open, high, low, close, base volume, quote volume, quote volume,
//! confirm]`, all strings, newest first. `/market/candles` serves the newest 300; older ones are
//! paged back with `/market/history-candles` and an `after` cursor. Failures arrive as HTTP 200
//! with a `code` other than `"0"`.

use super::catalog::Catalog;
use super::catalog::search::{self, ListedMarket};
use super::kline_endpoint::{self, KlineEndpoint};
use super::paging::Paging;
use super::ports::clock::Clock;
use super::ports::context::MarketContext;
use super::ports::http::{Http, Request};
use crate::model::market::bucket::next_bucket_start_ms;
use crate::model::{DataSource, InstrumentId, Kline, Listing, Timeframe};
use crate::service::market::error::{Asked, ErrorKind};
use crate::service::{MarketSource, ProviderError, SearchResult};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;

const BASE_URL: &str = "https://www.okx.com/api/v5";

pub struct Okx {
    http: Arc<dyn Http>,
    clock: Arc<dyn Clock>,
    /// Every spot market that is live, downloaded at most once a day.
    pub(super) catalog: Catalog<Pair>,
}

impl Okx {
    pub fn new(context: &MarketContext) -> Self {
        Self {
            http: context.http.clone(),
            clock: context.clock.clone(),
            catalog: Catalog::new(
                Listing::spot(DataSource::Okx),
                "catalog_okx",
                context.cache.clone(),
            ),
        }
    }
}

/// 300 rows a request, 120 pages (a 22-minute chart of 500 candles is 37), and a pause between
/// them for its 20 requests per 2 seconds per IP.
const PAGING: Paging = Paging {
    page: 300,
    max_candles: 36_000,
    pause: Duration::from_millis(100),
};

/// OKX's bar token for a candle length it serves itself: `5m`, `4H`, and the `utc` bars from six
/// hours up (`6Hutc`, `1Dutc`, `1Wutc`, `1Mutc`, `3Mutc`), because its plain ones are cut at UTC+8
/// and the app's buckets are UTC.
pub(crate) fn bar(timeframe: Timeframe) -> Option<String> {
    if !DataSource::Okx.natives().contains(&timeframe) {
        return None;
    }
    let token = timeframe.interval();
    let (count, unit) = token.split_at(token.len() - 1);
    Some(match unit {
        "s" | "m" => token,
        "h" if count.parse::<u16>().ok()? < 6 => format!("{count}H"),
        _ => format!("{count}{}utc", unit.to_uppercase()),
    })
}

/// OKX's id of a market: `BTC-USDT`. `BTC/USDT` and a bare `BTC` (USDT) are accepted.
pub(crate) fn instrument_id(symbol: &str) -> String {
    InstrumentId::new(DataSource::Okx, symbol).canonical_symbol()
}

/// OKX's candle endpoints.
impl KlineEndpoint for Okx {
    const SOURCE: DataSource = DataSource::Okx;
    const PAGING: Paging = PAGING;

    fn id(&self, symbol: &str) -> String {
        instrument_id(symbol)
    }

    fn page(
        &self,
        id: &str,
        base: Timeframe,
        limit: usize,
        before: Option<i64>,
    ) -> Result<Vec<Kline>, ProviderError> {
        self.fetch_page(id, base, limit, before)
    }
}

impl MarketSource for Okx {
    fn fetch_klines(
        &self,
        symbol: &str,
        timeframe: Timeframe,
        limit: u32,
    ) -> Result<Vec<Kline>, ProviderError> {
        kline_endpoint::latest(self, self.clock.now_ms(), symbol, timeframe, limit as usize)
    }

    fn fetch_klines_range(
        &self,
        symbol: &str,
        timeframe: Timeframe,
        start_ms: i64,
        end_ms: i64,
        max: usize,
    ) -> Result<Vec<Kline>, ProviderError> {
        kline_endpoint::range(self, symbol, timeframe, start_ms, end_ms, max)
    }

    fn search_tickers(&self, query: &str) -> Result<Vec<SearchResult>, ProviderError> {
        search::find(query, || self.catalog.get(|| self.download_pairs()))
    }
}

impl Okx {
    /// A page of candles: the newest 300 from `/market/candles`, those opening before `before`
    /// from `/market/history-candles`, which takes the `after` cursor.
    fn fetch_page(
        &self,
        id: &str,
        base: Timeframe,
        limit: usize,
        before: Option<i64>,
    ) -> Result<Vec<Kline>, ProviderError> {
        let bar = bar(base).ok_or_else(|| ProviderError::unsupported(DataSource::Okx, base))?;
        let endpoint = if before.is_some() {
            "history-candles"
        } else {
            "candles"
        };
        let mut url = format!("{BASE_URL}/market/{endpoint}?instId={id}&bar={bar}&limit={limit}");
        if let Some(before) = before {
            url.push_str(&format!("&after={before}"));
        }
        parse_candles(&self.get(&url)?, base)
    }

    fn get(&self, url: &str) -> Result<String, ProviderError> {
        self.http.fetch(DataSource::Okx, &Request::get(url))
    }
}

/// The `data` rows of an answer to a request that `asked` about markets or a list, or what its
/// `code` says when it failed: OKX's `50011` is "too many requests" and `51001` "instrument does not
/// exist", which only a request that named an instrument can mean.
pub(crate) fn data_rows(body: &str, asked: Asked) -> Result<Vec<Value>, ProviderError> {
    let value: Value = serde_json::from_str(body)
        .map_err(|error| ProviderError::malformed(DataSource::Okx, error))?;
    let code = value.get("code").and_then(Value::as_str);
    if code != Some("0") {
        return Err(match code {
            Some("50011") => ProviderError::new(DataSource::Okx, ErrorKind::RateLimited),
            Some("51001") if asked == Asked::Markets => {
                ProviderError::new(DataSource::Okx, ErrorKind::UnknownSymbol)
            }
            _ => ProviderError::refused_because(
                DataSource::Okx,
                value.get("msg").and_then(Value::as_str),
            ),
        });
    }
    value
        .get("data")
        .and_then(Value::as_array)
        .cloned()
        .ok_or_else(|| ProviderError::malformed(DataSource::Okx, "expected a data array"))
}

/// Candles of length `base`, oldest first.
fn parse_candles(body: &str, base: Timeframe) -> Result<Vec<Kline>, ProviderError> {
    let mut candles = data_rows(body, Asked::Markets)?
        .iter()
        .map(|row| parse_row(row, base))
        .collect::<Result<Vec<_>, _>>()?;
    candles.sort_by_key(|candle| candle.open_time);
    Ok(candles)
}

pub(crate) fn parse_row(row: &Value, base: Timeframe) -> Result<Kline, ProviderError> {
    let text = |index: usize| -> Result<&str, ProviderError> {
        row.get(index).and_then(Value::as_str).ok_or_else(|| {
            ProviderError::malformed(DataSource::Okx, format!("invalid field {index}"))
        })
    };
    let number = |index: usize| -> Result<f64, ProviderError> {
        text(index)?.parse().map_err(|_| {
            ProviderError::malformed(DataSource::Okx, format!("invalid field {index}"))
        })
    };
    let open_time: i64 = text(0)?
        .parse()
        .map_err(|_| ProviderError::malformed(DataSource::Okx, "invalid timestamp"))?;
    Ok(Kline {
        open_time,
        close_time: next_bucket_start_ms(base, open_time) - 1,
        open: number(1)?,
        high: number(2)?,
        low: number(3)?,
        close: number(4)?,
        volume: number(5)?,
    })
}

/// One tradable spot market from `/public/instruments`: all the search needs of it.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub(super) struct Pair {
    id: String,
    base: String,
    quote: String,
}

impl ListedMarket for Pair {
    const SOURCE: DataSource = DataSource::Okx;

    fn id(&self) -> &str {
        &self.id
    }

    fn sides(&self) -> Option<(&str, &str)> {
        Some((&self.base, &self.quote))
    }
}

impl Okx {
    fn download_pairs(&self) -> Result<Vec<Pair>, ProviderError> {
        parse_pairs(&self.get(&format!("{BASE_URL}/public/instruments?instType=SPOT"))?)
    }
}

fn parse_pairs(body: &str) -> Result<Vec<Pair>, ProviderError> {
    Ok(data_rows(body, Asked::List)?
        .iter()
        .filter(|row| row.get("state").and_then(Value::as_str) == Some("live"))
        .filter_map(|row| {
            let field = |key: &str| row.get(key)?.as_str().filter(|text| !text.is_empty());
            Some(Pair {
                id: field("instId")?.to_owned(),
                base: field("baseCcy")?.to_owned(),
                quote: field("quoteCcy")?.to_owned(),
            })
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::market::candles::plan;
    use crate::service::market::ports::fakes::Rig;
    use crate::service::market::ports::http::{self, Retry};
    use std::sync::Arc;

    const MINUTE: i64 = 60_000;
    const NOON: i64 = Rig::NOON;

    fn venue(rig: &Rig) -> Okx {
        Okx::new(&rig.context())
    }

    /// An answer of `count` one-minute rows, newest first, the newest opening at `newest`.
    fn minute_rows(newest: i64, count: usize) -> String {
        let rows: Vec<String> = (0..count as i64)
            .map(|back| {
                format!(
                    r#"["{}","10","12","9","11","1.5","16","16","1"]"#,
                    newest - back * MINUTE
                )
            })
            .collect();
        format!(r#"{{"code":"0","msg":"","data":[{}]}}"#, rows.join(","))
    }

    #[test]
    fn every_native_has_its_bar_and_nothing_else_does() {
        let bars: Vec<String> = DataSource::Okx
            .natives()
            .iter()
            .map(|native| bar(*native).expect("a native has a bar"))
            .collect();
        assert_eq!(
            bars,
            [
                "1s", "1m", "3m", "5m", "15m", "30m", "1H", "2H", "4H", "6Hutc", "12Hutc", "1Dutc",
                "1Wutc", "1Mutc", "3Mutc"
            ]
        );
        for other in ["2m", "8h", "2D", "3D", "2W", "2M", "6M", "1Y"] {
            assert_eq!(bar(Timeframe::parse(other).unwrap()), None, "{other}");
        }
    }

    fn base_of(timeframe: &str) -> (String, usize) {
        let plan = plan(DataSource::Okx, Timeframe::parse(timeframe).unwrap()).unwrap();
        (bar(plan.base()).unwrap(), plan.base_count(500))
    }

    #[test]
    fn other_lengths_fold_the_coarsest_native_that_nests() {
        // A request for 500 candles also asks for the one the start cuts.
        assert_eq!(base_of("1h"), ("1H".to_owned(), 500));
        assert_eq!(base_of("22m"), ("1m".to_owned(), 501 * 22));
        assert_eq!(base_of("45s"), ("1s".to_owned(), 501 * 45));
        assert_eq!(base_of("3h"), ("1H".to_owned(), 501 * 3));
        assert_eq!(base_of("8h"), ("4H".to_owned(), 501 * 2));
        assert_eq!(base_of("3D"), ("1Dutc".to_owned(), 501 * 3));
        assert_eq!(base_of("1Y"), ("3Mutc".to_owned(), 501 * 4));
    }

    #[test]
    fn symbols_are_okx_ids() {
        assert_eq!(instrument_id("btc-usdt"), "BTC-USDT");
        assert_eq!(instrument_id("ETH/USDC"), "ETH-USDC");
        assert_eq!(instrument_id("sol"), "SOL-USDT");
    }

    const ROWS: &str = r#"{"code":"0","msg":"","data":[
        ["1700002800000","11","13","10","12","5.5","60","60","0"],
        ["1699999200000","10","12","9","11","42.5","450","450","1"]
    ]}"#;

    #[test]
    fn rows_come_newest_first_and_leave_oldest_first() {
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
        let row = serde_json::json!(["1769904000000", "1", "2", "0.5", "1.5", "3", "4", "4", "1"]);
        let candle = parse_row(&row, Timeframe::MONTH_1).expect("row");
        // 2026-03-01 00:00 UTC, less a millisecond: February has 28 days.
        assert_eq!(candle.close_time, 1_772_323_200_000 - 1);
    }

    #[test]
    fn an_error_answer_is_a_kind_or_okxs_message() {
        let body = r#"{"code":"51001","msg":"Instrument ID doesn't exist.","data":[]}"#;
        assert_eq!(
            parse_candles(body, Timeframe::HOUR_1),
            Err(ProviderError::new(
                DataSource::Okx,
                ErrorKind::UnknownSymbol
            ))
        );
        let limited = r#"{"code":"50011","msg":"Too many requests","data":[]}"#;
        assert_eq!(
            parse_candles(limited, Timeframe::HOUR_1),
            Err(ProviderError::new(DataSource::Okx, ErrorKind::RateLimited))
        );
        let refused = r#"{"code":"50013","msg":"System is busy","data":[]}"#;
        assert_eq!(
            parse_candles(refused, Timeframe::HOUR_1),
            Err(ProviderError::refused(DataSource::Okx, "System is busy"))
        );
        assert!(parse_candles("not json", Timeframe::HOUR_1).is_err());
        assert!(parse_candles(r#"{"code":"0"}"#, Timeframe::HOUR_1).is_err());
        let bad_field = r#"{"code":"0","data":[["1700000000000","x","1","1","1","1"]]}"#;
        assert!(parse_candles(bad_field, Timeframe::HOUR_1).is_err());
    }

    /// Run with `cargo test --lib okx::tests::live -- --ignored --nocapture`.
    #[test]
    #[ignore = "needs the network"]
    fn live_okx_serves_candles_pairs_and_a_chart_of_every_native() {
        let okx = &crate::service::wiring::venues().okx;
        let hits = okx.search_tickers("btc-usdt").expect("search");
        assert_eq!(hits[0].symbol, "BTC-USDT");
        assert_eq!(hits[0].quote.as_deref(), Some("USDT"));
        for native in DataSource::Okx.natives() {
            let candles = okx.fetch_klines("BTC-USDT", *native, 350).expect("candles");
            // Months and quarters reach back only to the market's listing.
            if *native >= Timeframe::MONTH_1 {
                assert!(candles.len() > 30, "{native}");
            } else {
                assert_eq!(candles.len(), 350, "{native}");
            }
            assert!(
                candles
                    .windows(2)
                    .all(|pair| pair[0].open_time < pair[1].open_time),
                "{native}"
            );
        }
        // A 22-minute chart of 500 candles is eleven thousand one-minute ones: 37 pages.
        let folded = okx
            .fetch_klines("BTC-USDT", Timeframe::parse("22m").unwrap(), 500)
            .expect("folded candles");
        assert_eq!(folded.len(), 500);
        let error = okx
            .fetch_klines("NOPE-USDT", Timeframe::HOUR_1, 5)
            .unwrap_err();
        assert_eq!(error.kind, ErrorKind::UnknownSymbol);
    }

    #[test]
    fn the_newest_candles_are_one_request_and_come_back_oldest_first() {
        let rig = Rig::new();
        rig.http.on("/market/candles?", ROWS);
        let candles = venue(&rig)
            .fetch_klines("btc/usdt", Timeframe::HOUR_1, 2)
            .expect("candles");
        let sent = rig.http.sent();
        assert_eq!(sent.len(), 1);
        assert_eq!(
            sent[0].url,
            "https://www.okx.com/api/v5/market/candles?instId=BTC-USDT&bar=1H&limit=2"
        );
        assert_eq!(sent[0].retry, Retry::Transient);
        let opens: Vec<i64> = candles.iter().map(|candle| candle.open_time).collect();
        assert_eq!(opens, [1_699_999_200_000, 1_700_002_800_000]);
    }

    #[test]
    fn a_long_request_for_the_newest_pages_back_from_the_clock_by_the_after_cursor() {
        let rig = Rig::new();
        rig.http
            .on("&limit=300&after=", &minute_rows(NOON - MINUTE, 300));
        rig.http
            .on("&limit=100&after=", &minute_rows(NOON - 301 * MINUTE, 100));
        let candles = venue(&rig)
            .fetch_klines("BTC-USDT", Timeframe::MINUTE_1, 400)
            .expect("candles");
        // History pages, each asked for the candles that open before the oldest one held.
        let base = "https://www.okx.com/api/v5/market/history-candles?instId=BTC-USDT&bar=1m";
        assert_eq!(
            rig.http.urls(),
            [
                format!("{base}&limit=300&after={}", NOON + 1),
                format!("{base}&limit=100&after={}", NOON - 300 * MINUTE),
            ]
        );
        assert_eq!(candles.len(), 400);
        assert_eq!(candles[0].open_time, NOON - 400 * MINUTE);
    }

    #[test]
    fn a_range_asks_only_for_what_lies_before_its_end() {
        let rig = Rig::new();
        let (start, end) = (NOON - 500 * MINUTE, NOON - 100 * MINUTE);
        rig.http
            .on("&limit=300&after=", &minute_rows(end - MINUTE, 300));
        rig.http
            .on("&limit=100&after=", &minute_rows(end - 301 * MINUTE, 100));
        let candles = venue(&rig)
            .fetch_klines_range("BTC-USDT", Timeframe::MINUTE_1, start, end, 400)
            .expect("candles");
        // The second page is cut to what is left of the 400 candles asked for.
        let base = "https://www.okx.com/api/v5/market/history-candles?instId=BTC-USDT&bar=1m";
        assert_eq!(
            rig.http.urls(),
            [
                format!("{base}&limit=300&after={end}"),
                format!("{base}&limit=100&after={}", end - 300 * MINUTE),
            ]
        );
        assert_eq!(candles.len(), 400);
        assert_eq!(candles[0].open_time, start);
        assert_eq!(candles[399].open_time, end - MINUTE);
    }

    #[test]
    fn what_okx_refuses_is_told_by_kind() {
        let kind_of = |answer: Result<&str, http::Error>| {
            let rig = Rig::new();
            match answer {
                Ok(body) => rig.http.on("/market/candles?", body),
                Err(error) => rig.http.fail("/market/candles?", error),
            };
            venue(&rig)
                .fetch_klines("BTC-USDT", Timeframe::HOUR_1, 5)
                .unwrap_err()
                .kind
        };
        assert_eq!(
            kind_of(Ok(
                r#"{"code":"51001","msg":"Instrument ID doesn't exist.","data":[]}"#
            )),
            ErrorKind::UnknownSymbol
        );
        assert_eq!(
            kind_of(Ok(
                r#"{"code":"50011","msg":"Too many requests","data":[]}"#
            )),
            ErrorKind::RateLimited
        );
        assert_eq!(
            kind_of(Ok(r#"{"code":"50013","msg":"System is busy","data":[]}"#)),
            ErrorKind::Refused(Some("System is busy".to_owned()))
        );
        assert_eq!(
            kind_of(Err(http::Error::Status(500))),
            ErrorKind::Transport(http::Error::Status(500))
        );
    }

    #[test]
    fn the_market_list_is_downloaded_once_a_day_and_searched_locally() {
        let rig = Rig::new();
        rig.http.on(
            "/public/instruments?instType=SPOT",
            r#"{"code":"0","data":[
                {"instId":"BTC-USDT","state":"live","baseCcy":"BTC","quoteCcy":"USDT"},
                {"instId":"OLD-USDT","state":"suspend","baseCcy":"OLD","quoteCcy":"USDT"}]}"#,
        );
        let okx = venue(&rig);
        let found = okx.search_tickers("btcusdt").expect("hits");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].symbol, "BTC-USDT");
        okx.search_tickers("old").expect("hits");
        venue(&rig).search_tickers("btc").expect("hits");
        assert_eq!(rig.http.urls().len(), 1);
    }

    #[test]
    fn the_catalog_keeps_only_live_markets_with_all_their_fields() {
        let body = r#"{"code":"0","data":[
            {"instId":"BTC-USDT","state":"live","baseCcy":"BTC","quoteCcy":"USDT"},
            {"instId":"OLD-USDT","state":"suspend","baseCcy":"OLD","quoteCcy":"USDT"},
            {"instId":"NOQUOTE","state":"live","baseCcy":"X","quoteCcy":""}
        ]}"#;
        let pairs = parse_pairs(body).expect("pairs");
        assert_eq!(pairs.len(), 1);
        assert_eq!(
            (
                pairs[0].id.as_str(),
                pairs[0].base.as_str(),
                pairs[0].quote.as_str()
            ),
            ("BTC-USDT", "BTC", "USDT")
        );
        assert!(parse_pairs("not json").is_err());
    }

    #[test]
    fn a_pair_is_found_however_it_is_separated() {
        let pairs = vec![
            Pair {
                id: "BTC-USDT".into(),
                base: "BTC".into(),
                quote: "USDT".into(),
            },
            Pair {
                id: "ETH-BTC".into(),
                base: "ETH".into(),
                quote: "BTC".into(),
            },
        ];
        let pairs = Arc::new(pairs);
        for query in ["btcusdt", "BTC-USDT", "btc/usdt", "BTC USDT"] {
            let found = search::find(query, || Ok(Arc::clone(&pairs))).unwrap();
            assert_eq!(found.len(), 1, "{query}");
            assert_eq!(found[0].source, DataSource::Okx);
            assert_eq!(found[0].symbol, "BTC-USDT");
            assert_eq!(found[0].base.as_deref(), Some("BTC"));
            assert_eq!(found[0].quote.as_deref(), Some("USDT"));
        }
        let all = search::find("BTC", || Ok(Arc::clone(&pairs))).unwrap();
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn instrument_does_not_exist_is_an_unknown_market_only_to_a_request_that_named_one() {
        let body = r#"{"code":"51001","msg":"Instrument ID doesn't exist.","data":[]}"#;
        assert_eq!(
            data_rows(body, Asked::Markets),
            Err(ProviderError::new(
                DataSource::Okx,
                ErrorKind::UnknownSymbol
            ))
        );
        // The instrument list and the whole ticker name none.
        assert_eq!(
            data_rows(body, Asked::List),
            Err(ProviderError::refused(
                DataSource::Okx,
                "Instrument ID doesn't exist."
            ))
        );
        assert!(
            parse_pairs(body).is_err_and(
                |error| error.kind.scope() == crate::service::market::error::Scope::Source
            )
        );
    }
}
