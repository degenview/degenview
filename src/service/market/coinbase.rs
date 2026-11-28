use super::candles::plan;
use super::catalog::Catalog;
use super::catalog::search::{self, ListedMarket};
use super::ports::clock::Clock;
use super::ports::context::MarketContext;
use super::ports::http::{Http, Request};
use crate::model::market::bucket::FoldPlan;
use crate::model::{DataSource, Kline, Listing, Timeframe};
use crate::service::market::error::{ErrorKind, Statuses};
use crate::service::{MarketSource, ProviderError, SearchResult};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;

const BASE_URL: &str = "https://api.exchange.coinbase.com";
/// Candles one `start`/`end` request spans; Coinbase rejects spans over 300.
const PAGE_BUCKETS: i64 = 290;
/// Cap on backwards pages so a slow fold cannot loop forever.
const MAX_PAGES: usize = 60;

pub struct Coinbase {
    http: Arc<dyn Http>,
    clock: Arc<dyn Clock>,
    /// Every product that is online, downloaded at most once a day.
    pub(super) catalog: Catalog<Product>,
}

/// Coinbase ids are `BASE-QUOTE`. Accept `BASE/QUOTE` and a bare `BASE` (USD).
pub(crate) fn product_id(symbol: &str) -> String {
    let id = symbol.trim().to_uppercase().replace('/', "-");
    if id.contains('-') {
        id
    } else {
        format!("{id}-USD")
    }
}

/// The most candles a request can page back to: `MAX_PAGES` pages of `PAGE_BUCKETS`.
const MAX_CANDLES: usize = PAGE_BUCKETS as usize * MAX_PAGES;

impl Coinbase {
    pub fn new(context: &MarketContext) -> Self {
        Self {
            http: context.http.clone(),
            clock: context.clock.clone(),
            catalog: Catalog::new(
                Listing::spot(DataSource::Coinbase),
                "catalog_coinbase",
                context.cache.clone(),
            ),
        }
    }

    /// The candles to ask for to end with `limit` of `plan`'s target, and how many seconds each
    /// lasts. A request is capped at what the paging can reach: a longer one gives fewer candles
    /// (a 5-minute chart of a year reaches back about two months), never other ones.
    fn request(plan: FoldPlan, limit: usize) -> (i64, usize) {
        let seconds = plan.base().nominal_ms() / 1_000;
        (seconds, plan.base_count(limit).min(MAX_CANDLES))
    }
}

impl MarketSource for Coinbase {
    fn fetch_klines(
        &self,
        symbol: &str,
        timeframe: Timeframe,
        limit: u32,
    ) -> Result<Vec<Kline>, ProviderError> {
        let plan = plan(DataSource::Coinbase, timeframe)?;
        let (seconds, needed) = Self::request(plan, limit as usize);
        let (source, window_start) = self.source_candles(&product_id(symbol), seconds, needed)?;
        Ok(shaped(plan, source, needed, window_start, limit as usize))
    }

    fn fetch_klines_range(
        &self,
        symbol: &str,
        timeframe: Timeframe,
        start_ms: i64,
        end_ms: i64,
        max: usize,
    ) -> Result<Vec<Kline>, ProviderError> {
        let plan = plan(DataSource::Coinbase, timeframe)?;
        let (seconds, needed) = Self::request(plan, max);
        let (source, window_start) =
            self.source_candles_before(&product_id(symbol), seconds, start_ms, end_ms, needed)?;
        Ok(shaped(plan, source, needed, window_start, max))
    }

    fn search_tickers(&self, query: &str) -> Result<Vec<SearchResult>, ProviderError> {
        search::find(query, || self.catalog.get(|| self.download_products()))
    }
}

/// One tradable product from `/products`: all the search needs of it.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub(super) struct Product {
    id: String,
    base: String,
    quote: String,
}

impl ListedMarket for Product {
    const SOURCE: DataSource = DataSource::Coinbase;

    fn id(&self) -> &str {
        &self.id
    }

    fn sides(&self) -> Option<(&str, &str)> {
        Some((&self.base, &self.quote))
    }
}

/// What the statuses of a request for one product mean at Coinbase (candles, stats): a 404 names a
/// product it does not list. The product list has no table: a 404 there is a changed endpoint.
pub(in crate::service) const PRODUCT_STATUSES: Statuses = &[(404, ErrorKind::UnknownSymbol)];

impl Coinbase {
    fn download_products(&self) -> Result<Vec<Product>, ProviderError> {
        let body = self.http.fetch(
            DataSource::Coinbase,
            &Request::get(&format!("{BASE_URL}/products")),
        )?;
        parse_products(&body)
    }
}

fn parse_products(body: &str) -> Result<Vec<Product>, ProviderError> {
    let value: Value = serde_json::from_str(body)
        .map_err(|error| ProviderError::malformed(DataSource::Coinbase, error))?;
    Ok(value
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .filter(|product| product.get("status").and_then(Value::as_str) == Some("online"))
        .filter(|product| product.get("trading_disabled").and_then(Value::as_bool) != Some(true))
        .filter_map(|product| {
            Some(Product {
                id: product.get("id")?.as_str()?.to_owned(),
                base: product.get("base_currency")?.as_str()?.to_owned(),
                quote: product.get("quote_currency")?.as_str()?.to_owned(),
            })
        })
        .collect())
}

/// `source` shaped into `limit` candles of `plan`'s target. `window_start` is where the paging
/// stopped short of the product's history, so a first bucket that opened before it is a fragment.
fn shaped(
    plan: FoldPlan,
    source: Vec<Kline>,
    needed: usize,
    window_start: Option<i64>,
    limit: usize,
) -> Vec<Kline> {
    match window_start {
        Some(since) => plan.shape_since(source, needed, since, limit),
        None => plan.shape(source, needed, limit),
    }
}

impl Coinbase {
    /// The newest `needed` candles, oldest first, paged back from now, and where the paging
    /// stopped if that was short of the product's history (`None` when an empty page ended it).
    fn source_candles(
        &self,
        id: &str,
        granularity: i64,
        needed: usize,
    ) -> Result<(Vec<Kline>, Option<i64>), ProviderError> {
        let mut candles = self.fetch_page(id, granularity, None, None)?;
        let mut window_start = None;
        let mut pages = 0;
        while candles.len() < needed && pages < MAX_PAGES {
            let Some(oldest) = candles.first().map(|c| c.open_time) else {
                break;
            };
            std::thread::sleep(Duration::from_millis(120));
            let span = granularity * PAGE_BUCKETS * 1_000;
            let lower = oldest - span;
            let older = self.fetch_page(id, granularity, Some(lower), Some(oldest))?;
            let older: Vec<Kline> = older.into_iter().filter(|c| c.open_time < oldest).collect();
            if older.is_empty() {
                window_start = None;
                break;
            }
            candles = older.into_iter().chain(candles).collect();
            window_start = Some(lower);
            pages += 1;
        }
        Ok((candles, window_start))
    }

    /// The newest `needed` candles that open in `[start_ms, end_ms)`, oldest first, paged back from
    /// `end_ms`, and where the paging stopped if that was short of the product's history. An empty
    /// page before `start_ms` is the start of the product's history (`None`).
    fn source_candles_before(
        &self,
        id: &str,
        granularity: i64,
        start_ms: i64,
        end_ms: i64,
        needed: usize,
    ) -> Result<(Vec<Kline>, Option<i64>), ProviderError> {
        let span = granularity * PAGE_BUCKETS * 1_000;
        let mut candles: Vec<Kline> = Vec::new();
        let mut window_start = None;
        let mut upper = first_upper(end_ms, self.clock.now_ms(), granularity);
        for page in 0..MAX_PAGES {
            if candles.len() >= needed || upper <= start_ms {
                break;
            }
            if page > 0 {
                std::thread::sleep(Duration::from_millis(120));
            }
            let lower = (upper - span).max(start_ms).max(0);
            let mut batch = self.fetch_page(id, granularity, Some(lower), Some(upper))?;
            batch.retain(|c| c.open_time >= start_ms && c.open_time < upper);
            window_start = Some(lower);
            let Some(oldest) = batch.first().map(|c| c.open_time) else {
                if lower > start_ms {
                    window_start = None;
                }
                break;
            };
            upper = oldest;
            candles = batch.into_iter().chain(candles).collect();
        }
        Ok((candles, window_start))
    }

    fn fetch_page(
        &self,
        id: &str,
        granularity: i64,
        start: Option<i64>,
        end: Option<i64>,
    ) -> Result<Vec<Kline>, ProviderError> {
        let url = page_url(id, granularity, start.zip(end), self.clock.now_ms());
        let body = self.http.fetch(
            DataSource::Coinbase,
            &Request::get(&url).statuses(PRODUCT_STATUSES),
        )?;
        parse_page(&body, granularity)
    }
}

/// The address of one page of candles: those of `window`, or the newest ones as of `now_ms`.
///
/// The newest page names the moment it is asked for because Coinbase's cache answers one address
/// for 300 s (`max-age=300`, a `no-cache` request header ignored): asked without it, a chart ran up
/// to five minutes behind and then took several candles at once. The API ignores `end` without a
/// `start`, so the answer is still the newest candles, however far back a quiet market's reach.
fn page_url(id: &str, granularity: i64, window: Option<(i64, i64)>, now_ms: i64) -> String {
    let url = format!("{BASE_URL}/products/{id}/candles?granularity={granularity}");
    match window {
        Some((start, end)) => format!(
            "{url}&start={}&end={}",
            crate::format::iso8601(start),
            crate::format::iso8601(end)
        ),
        None => format!("{url}&end={}", crate::format::iso8601(now_ms)),
    }
}

/// Where paging back towards `end_ms` begins. No candle opens later than one bucket from now, and
/// a page that lies wholly in the future comes back empty, which reads as the end of the history.
fn first_upper(end_ms: i64, now_ms: i64, granularity: i64) -> i64 {
    end_ms.min(now_ms.saturating_add(granularity * 1_000))
}

/// `[time s, low, high, open, close, volume]`, newest first.
fn parse_page(body: &str, granularity: i64) -> Result<Vec<Kline>, ProviderError> {
    let value: Value = serde_json::from_str(body)
        .map_err(|error| ProviderError::malformed(DataSource::Coinbase, error))?;
    let rows = value
        .as_array()
        .ok_or_else(|| ProviderError::malformed(DataSource::Coinbase, "expected a candle array"))?;
    let mut candles = Vec::with_capacity(rows.len());
    for row in rows {
        let Some(fields) = row.as_array() else {
            continue;
        };
        if fields.len() < 6 {
            continue;
        }
        let Some(seconds) = fields[0].as_f64() else {
            continue;
        };
        let Some(low) = fields[1].as_f64() else {
            continue;
        };
        let Some(high) = fields[2].as_f64() else {
            continue;
        };
        let Some(open) = fields[3].as_f64() else {
            continue;
        };
        let Some(close) = fields[4].as_f64() else {
            continue;
        };
        let volume = fields[5].as_f64().unwrap_or(0.0);
        let open_time = (seconds * 1_000.0) as i64;
        candles.push(Kline {
            open_time,
            close_time: open_time + granularity * 1_000 - 1,
            open,
            high,
            low,
            close,
            volume,
        });
    }
    candles.sort_by_key(|candle| candle.open_time);
    Ok(candles)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::market::ports::fakes::Rig;
    use crate::service::market::ports::http::{self, Retry};

    const HOUR: i64 = 3_600_000;
    const NOON: i64 = Rig::NOON;

    fn venue(rig: &Rig) -> Coinbase {
        Coinbase::new(&rig.context())
    }

    /// An answer of the `count` hourly rows ending with the one that opens at `newest`, newest
    /// first, as `[time s, low, high, open, close, volume]`.
    fn hour_rows(newest: i64, count: usize) -> String {
        let rows: Vec<String> = (0..count as i64)
            .map(|back| {
                format!(
                    "[{},9.0,12.0,10.0,11.0,42.5]",
                    (newest - back * HOUR) / 1_000
                )
            })
            .collect();
        format!("[{}]", rows.join(","))
    }

    #[test]
    fn the_newest_candles_are_asked_for_at_an_address_that_names_the_clock() {
        let rig = Rig::new();
        rig.http.on("/candles?", &hour_rows(NOON - HOUR, 2));
        let candles = venue(&rig)
            .fetch_klines("btc", Timeframe::HOUR_1, 2)
            .expect("candles");
        let sent = rig.http.sent();
        assert_eq!(sent.len(), 1);
        assert_eq!(
            sent[0].url,
            format!(
                "https://api.exchange.coinbase.com/products/BTC-USD/candles?granularity=3600&end={}",
                crate::format::iso8601(NOON)
            )
        );
        assert_eq!(sent[0].retry, Retry::Transient);
        let opens: Vec<i64> = candles.iter().map(|candle| candle.open_time).collect();
        assert_eq!(opens, [NOON - 2 * HOUR, NOON - HOUR]);
    }

    #[test]
    fn a_longer_request_pages_back_a_window_at_a_time_from_the_oldest_candle_held() {
        let rig = Rig::new();
        let page = PAGE_BUCKETS as usize;
        let oldest = NOON - page as i64 * HOUR;
        rig.http
            .on("granularity=3600&end=", &hour_rows(NOON - HOUR, page));
        rig.http.on("&start=", &hour_rows(oldest - HOUR, 20));
        let candles = venue(&rig)
            .fetch_klines("BTC-USD", Timeframe::HOUR_1, 300)
            .expect("candles");
        let urls = rig.http.urls();
        let base = "https://api.exchange.coinbase.com/products/BTC-USD/candles?granularity=3600";
        assert_eq!(
            urls,
            [
                format!("{base}&end={}", crate::format::iso8601(NOON)),
                format!(
                    "{base}&start={}&end={}",
                    crate::format::iso8601(oldest - PAGE_BUCKETS * HOUR),
                    crate::format::iso8601(oldest)
                ),
            ]
        );
        assert_eq!(candles.len(), 300);
        assert_eq!(candles[299].open_time, NOON - HOUR);
    }

    #[test]
    fn a_range_is_asked_for_as_the_window_that_ends_with_it_and_never_in_the_future() {
        let rig = Rig::new();
        let (start, end) = (NOON - 10 * HOUR, NOON - 7 * HOUR);
        rig.http.on("&start=", &hour_rows(end - HOUR, 3));
        let candles = venue(&rig)
            .fetch_klines_range("BTC-USD", Timeframe::HOUR_1, start, end, 3)
            .expect("candles");
        assert_eq!(
            rig.http.urls(),
            [format!(
                "https://api.exchange.coinbase.com/products/BTC-USD/candles?granularity=3600&start={}&end={}",
                crate::format::iso8601(start),
                crate::format::iso8601(end)
            )]
        );
        assert_eq!(candles.len(), 3);
        assert_eq!(candles[0].open_time, end - 3 * HOUR);

        // A range that ends after the next candle opens is asked up to it, no further.
        let rig = Rig::new();
        rig.http.on("&start=", &hour_rows(NOON, 1));
        venue(&rig)
            .fetch_klines_range(
                "BTC-USD",
                Timeframe::HOUR_1,
                NOON - HOUR,
                NOON + 5 * HOUR,
                1,
            )
            .expect("candles");
        let url = rig.http.urls().remove(0);
        assert!(
            url.ends_with(&format!("&end={}", crate::format::iso8601(NOON + HOUR))),
            "{url}"
        );
    }

    #[test]
    fn what_coinbase_refuses_is_told_by_kind() {
        let kind_of = |answer: Result<&str, http::Error>| {
            let rig = Rig::new();
            match answer {
                Ok(body) => rig.http.on("/candles?", body),
                Err(error) => rig.http.fail("/candles?", error),
            };
            venue(&rig)
                .fetch_klines("NOPE-USD", Timeframe::HOUR_1, 5)
                .unwrap_err()
                .kind
        };
        assert_eq!(
            kind_of(Err(http::Error::Status(404))),
            ErrorKind::UnknownSymbol
        );
        assert_eq!(
            kind_of(Err(http::Error::Status(429))),
            ErrorKind::RateLimited
        );
        assert_eq!(
            kind_of(Ok(r#"{"message":"NotFound"}"#)),
            ErrorKind::Malformed("expected a candle array".to_owned())
        );
    }

    #[test]
    fn the_product_list_is_downloaded_once_a_day_and_searched_locally() {
        let rig = Rig::new();
        rig.http.on(
            "/products",
            r#"[{"id":"BTC-USD","base_currency":"BTC","quote_currency":"USD","status":"online"},
                {"id":"OLD-USD","base_currency":"OLD","quote_currency":"USD","status":"delisted"}]"#,
        );
        let found = venue(&rig).search_tickers("btc usd").expect("hits");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].symbol, "BTC-USD");
        venue(&rig).search_tickers("btc").expect("hits");
        assert_eq!(
            rig.http.urls(),
            ["https://api.exchange.coinbase.com/products"]
        );
    }

    #[test]
    fn the_catalog_keeps_only_online_enabled_products() {
        let body = r#"[
            {"id":"BTC-USD","base_currency":"BTC","quote_currency":"USD","status":"online"},
            {"id":"OLD-USD","base_currency":"OLD","quote_currency":"USD","status":"delisted"},
            {"id":"HALT-USD","base_currency":"HALT","quote_currency":"USD","status":"online","trading_disabled":true},
            {"id":"BAD","status":"online"}
        ]"#;
        let products = parse_products(body).expect("products");
        assert_eq!(products.len(), 1);
        assert_eq!(products[0].id, "BTC-USD");
        assert_eq!(
            (products[0].base.as_str(), products[0].quote.as_str()),
            ("BTC", "USD")
        );
        assert!(parse_products("{").is_err());
    }

    use crate::model::market::bucket;
    use std::sync::Arc;

    #[test]
    fn a_product_is_found_however_it_is_separated() {
        let products = vec![
            Product {
                id: "BTC-USDT".into(),
                base: "BTC".into(),
                quote: "USDT".into(),
            },
            Product {
                id: "ETH-BTC".into(),
                base: "ETH".into(),
                quote: "BTC".into(),
            },
        ];
        let products = Arc::new(products);
        let ask = |query: &str| search::find(query, || Ok(Arc::clone(&products))).unwrap();
        for query in ["btc-usdt", "BTCUSDT", "btc/usdt", "BTC USDT"] {
            let found = ask(query);
            assert_eq!(found.len(), 1, "{query}");
            assert_eq!(found[0].source, DataSource::Coinbase);
            assert_eq!(found[0].symbol, "BTC-USDT");
            assert_eq!(found[0].base.as_deref(), Some("BTC"));
            assert_eq!(found[0].quote.as_deref(), Some("USDT"));
        }
        assert_eq!(ask("BTC").len(), 2);
    }

    #[test]
    fn product_id_maps_bare_and_slashed_symbols() {
        assert_eq!(product_id("BTC"), "BTC-USD");
        assert_eq!(product_id("ETH/USD"), "ETH-USD");
        assert_eq!(product_id("BTC-USD"), "BTC-USD");
    }

    fn request(timeframe: &str, limit: usize) -> (i64, usize) {
        let plan = plan(DataSource::Coinbase, Timeframe::parse(timeframe).unwrap()).unwrap();
        Coinbase::request(plan, limit)
    }

    #[test]
    fn coinbases_granularities_are_asked_for_as_they_are() {
        for (token, seconds) in [
            ("1m", 60),
            ("5m", 300),
            ("15m", 900),
            ("1h", 3_600),
            ("6h", 21_600),
            ("1d", 86_400),
        ] {
            assert_eq!(request(token, 300), (seconds, 300), "{token}");
        }
    }

    #[test]
    fn other_lengths_fold_the_coarsest_granularity_that_nests() {
        // One extra candle's worth is asked for, the fragment the request's start cuts.
        assert_eq!(request("30m", 100), (900, 101 * 2));
        assert_eq!(request("4h", 100), (3_600, 101 * 4));
        assert_eq!(request("2h", 100), (3_600, 101 * 2));
        assert_eq!(request("12h", 100), (21_600, 101 * 2));
        assert_eq!(request("22m", 100), (60, 101 * 22));
        assert_eq!(request("1w", 100), (86_400, 101 * 7));
        assert_eq!(request("2d", 100), (86_400, 101 * 2));
        assert_eq!(request("3M", 100), (86_400, 101 * 93));
    }

    #[test]
    fn a_request_is_capped_at_what_the_paging_reaches() {
        assert_eq!(request("5m", 500).1, 500);
        assert_eq!(request("22m", 1_000), (60, MAX_CANDLES));
    }

    fn candle(open_time: i64, length_ms: i64, price: f64) -> Kline {
        Kline {
            open_time,
            close_time: open_time + length_ms - 1,
            open: price,
            high: price + 1.0,
            low: price - 1.0,
            close: price + 0.5,
            volume: 1.0,
        }
    }

    #[test]
    fn hourly_candles_fold_into_four_hour_candles_on_utc_boundaries() {
        let midnight = crate::format::days_from_civil(2026, 10, 5) * 86_400_000;
        let hour = 3_600_000;
        // 01:00 to 08:00: the 00:00 bar is partial, 04:00 is whole, 08:00 is partial.
        let hourly: Vec<Kline> = (1..=8)
            .map(|index| candle(midnight + index * hour, hour, 100.0 + index as f64))
            .collect();
        let folded = bucket::Grid::Utc.fold(&hourly, Timeframe::HOUR_4, false);
        let opens: Vec<i64> = folded.iter().map(|bar| bar.open_time).collect();
        assert_eq!(opens, [midnight, midnight + 4 * hour, midnight + 8 * hour]);
        // The whole 04:00 bar: open of 04:00, close of 07:00, extremes and volume over the four.
        let whole = &folded[1];
        assert_eq!((whole.open, whole.close), (104.0, 107.5));
        assert_eq!((whole.low, whole.high), (103.0, 108.0));
        assert_eq!(whole.volume, 4.0);
        // A page cut by the request drops the bucket it only half has.
        let cut = bucket::Grid::Utc.fold(&hourly, Timeframe::HOUR_4, true);
        assert_eq!(
            cut.first().map(|bar| bar.open_time),
            Some(midnight + 4 * hour)
        );
    }

    #[test]
    fn quarter_hours_fold_into_half_hours() {
        let midnight = crate::format::days_from_civil(2026, 10, 5) * 86_400_000;
        let quarter = 900_000;
        let candles: Vec<Kline> = (0..4)
            .map(|index| candle(midnight + index * quarter, quarter, 10.0 + index as f64))
            .collect();
        let folded = bucket::Grid::Utc.fold(&candles, Timeframe::MINUTE_30, false);
        assert_eq!(folded.len(), 2);
        assert_eq!(folded[1].open_time, midnight + 2 * quarter);
        assert_eq!((folded[1].open, folded[1].close), (12.0, 13.5));
        assert_eq!(folded[1].volume, 2.0);
    }

    #[test]
    fn folds_daily_into_weekly_on_monday_boundaries() {
        // 2026-10-05 is a Monday; 2026-10-06 is Tuesday.
        let monday = crate::format::days_from_civil(2026, 10, 5) * 86_400_000;
        let tuesday = crate::format::days_from_civil(2026, 10, 6) * 86_400_000;
        let daily = vec![
            Kline {
                open_time: monday,
                close_time: monday + 86_399_999,
                open: 10.0,
                high: 11.0,
                low: 9.0,
                close: 10.5,
                volume: 1.0,
            },
            Kline {
                open_time: tuesday,
                close_time: tuesday + 86_399_999,
                open: 10.5,
                high: 12.0,
                low: 10.0,
                close: 11.5,
                volume: 2.0,
            },
        ];
        let weekly = bucket::Grid::Utc.fold(&daily, Timeframe::WEEK_1, false);
        assert_eq!(weekly.len(), 1);
        assert_eq!(weekly[0].open_time, monday);
        assert_eq!(weekly[0].high, 12.0);
        assert_eq!(weekly[0].low, 9.0);
        assert_eq!(weekly[0].close, 11.5);
        assert_eq!(weekly[0].volume, 3.0);
    }

    /// Coinbase's cache answers one address for five minutes, so the newest candles must be asked
    /// for under an address that says when they were wanted.
    #[test]
    fn the_newest_candles_are_never_asked_for_at_the_same_address_twice() {
        let now = 1_800_000_000_000;
        let first = page_url("BTC-USD", 60, None, now);
        let later = page_url("BTC-USD", 60, None, now + 5_000);
        assert_ne!(first, later);
        // A start would cut a quiet market off: one bound alone leaves the answer the newest rows.
        assert!(!first.contains("start="), "{first}");
        // An older page names its own candles and may well be answered from the cache.
        let window = Some((now - 600_000, now - 300_000));
        assert_eq!(
            page_url("BTC-USD", 60, window, now),
            page_url("BTC-USD", 60, window, now + 5_000)
        );
    }

    #[test]
    fn parses_coinbase_page() {
        let body = r#"[[1700000000,9.0,12.0,10.0,11.0,42.5],[1700003600,10.0,11.0,11.0,10.5,1.0]]"#;
        let candles = parse_page(body, 3600).expect("valid");
        assert_eq!(candles.len(), 2);
        assert_eq!(candles[0].open_time, 1_700_000_000_000);
        assert_eq!(candles[0].high, 12.0);
        assert_eq!(candles[0].low, 9.0);
        assert_eq!(candles[0].open, 10.0);
        assert_eq!(candles[0].close, 11.0);
    }
}
