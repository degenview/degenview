//! Binance's REST market data, spot and USDⓈ-M perpetuals alike: a symbol says which
//! (`model::market::binance`) and [`endpoints`] where that market is served.

use super::candles::plan;
use super::catalog::Catalog;
use super::ports::clock::Clock;
use super::ports::context::MarketContext;
use super::ports::http::{Http, Request};
use super::search_rank::normalize;
use crate::model::exchange::Market;
use crate::model::market::binance::split_market;
use crate::model::{DataSource, InstrumentId, Kline, Listing, Timeframe};
use crate::service::market::error::{ErrorKind, Statuses};
use crate::service::net::url::encode_component;
use crate::service::{MarketSource, ProviderError, SearchResult};
use catalog::Pair;
use serde_json::Value;
use std::sync::Arc;

mod catalog;
mod endpoints;

pub(in crate::service) use endpoints::{Endpoints, endpoints};

pub struct Binance {
    http: Arc<dyn Http>,
    clock: Arc<dyn Clock>,
    /// Every spot market that trades, downloaded at most once a day.
    pub(super) spot_catalog: Catalog<Pair>,
    /// Every perpetual that trades.
    pub(super) perpetual_catalog: Catalog<Pair>,
}

impl Binance {
    pub fn new(context: &MarketContext) -> Self {
        Self {
            http: context.http.clone(),
            clock: context.clock.clone(),
            spot_catalog: Catalog::new(
                Listing::spot(DataSource::Binance),
                "catalog_binance",
                context.cache.clone(),
            ),
            perpetual_catalog: Catalog::new(
                Listing::BINANCE_PERPETUAL,
                "catalog_binance_futures",
                context.cache.clone(),
            ),
        }
    }

    fn catalog(&self, market: Market) -> &Catalog<Pair> {
        match market {
            Market::Spot => &self.spot_catalog,
            Market::Perpetual => &self.perpetual_catalog,
        }
    }

    /// A request for the klines of `listing`'s market.
    fn get(&self, listing: Listing, url: &str) -> Result<String, ProviderError> {
        self.http
            .fetch(listing, &Request::get(url).statuses(MARKET_STATUSES))
    }
}

/// What the statuses of a request that names a market mean at Binance (klines, a ticker asked for
/// by name): a 400 names a market it does not list. A request that names none (the exchange's
/// market list, the whole ticker) has no table: a 400 there says nothing about any market.
pub(in crate::service) const MARKET_STATUSES: Statuses = &[(400, ErrorKind::UnknownSymbol)];

/// The most klines one Binance request returns.
const MAX_PER_REQUEST: usize = 1_000;

/// What one `symbol` of the app asks Binance for: where, the pair as a query value, and the
/// market's own listing.
struct Target {
    endpoints: &'static Endpoints,
    pair: String,
    listing: Listing,
}

impl Target {
    fn of(symbol: &str) -> Self {
        // Any spelling the app takes (`btc`, `BTC/USDT.P`) names the market the same way here as
        // in its live feed and its key.
        let canonical = InstrumentId::new(DataSource::Binance, symbol).canonical_symbol();
        let (pair, market) = split_market(&canonical);
        Self {
            endpoints: endpoints(market),
            pair: encode_component(pair),
            listing: Listing::new(DataSource::Binance, market),
        }
    }
}

impl MarketSource for Binance {
    fn fetch_klines(
        &self,
        symbol: &str,
        timeframe: Timeframe,
        limit: u32,
    ) -> Result<Vec<Kline>, ProviderError> {
        let target = Target::of(symbol);
        let plan = plan(target.listing, timeframe)?;
        let requested = plan.base_count(limit as usize);
        if requested > MAX_PER_REQUEST {
            // More finer candles than one request holds: page back from now.
            let end_ms = self.clock.now_ms() + 1;
            return self.fetch_klines_range(symbol, timeframe, 0, end_ms, limit as usize);
        }
        let url = format!(
            "{}?symbol={}&interval={}&limit={requested}",
            target.endpoints.klines_url,
            target.pair,
            plan.base().interval()
        );
        let body = self.get(target.listing, &url)?;
        Ok(plan.shape(
            parse_klines(target.listing, &body)?,
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
        // Pages are fetched side by side: their start times follow from the interval alone.
        const PARALLEL: usize = 8;
        let target = Target::of(symbol);
        let plan = plan(target.listing, timeframe)?;
        let interval = plan.base().interval();
        let requested = plan.base_count(max);
        let step = plan.base().nominal_ms();
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
                        let (interval, target) = (&interval, &target);
                        scope.spawn(move || {
                            let to = (from + page_ms).min(end_ms) - 1;
                            let url = format!(
                                "{}?symbol={}&interval={interval}&startTime={from}&endTime={to}&limit={PAGE}",
                                target.endpoints.klines_url,
                                target.pair
                            );
                            parse_klines(target.listing, &self.get(target.listing, &url)?)
                        })
                    })
                    .collect();
                handles
                    .into_iter()
                    .map(|handle| {
                        handle
                            .join()
                            .unwrap_or_else(|_| Err(ProviderError::interrupted(target.listing)))
                    })
                    .collect()
            });
            for page in pages {
                candles.extend(page?);
            }
        }
        candles.sort_by_key(|candle| candle.open_time);
        candles.dedup_by_key(|candle| candle.open_time);
        if candles.len() > requested {
            candles.drain(..candles.len() - requested);
        }
        Ok(plan.shape(candles, requested, max))
    }

    fn search_tickers(&self, query: &str) -> Result<Vec<SearchResult>, ProviderError> {
        // `BTC-USDT` and `btc/usdt` find `BTCUSDT`: Binance writes no separator.
        let needle = normalize(query);
        if needle.is_empty() {
            return Ok(Vec::new());
        }
        self.search(query, &needle)
    }
}

fn parse_klines(listing: Listing, body: &str) -> Result<Vec<Kline>, ProviderError> {
    let value: Value =
        serde_json::from_str(body).map_err(|error| ProviderError::malformed(listing, error))?;
    let rows = value
        .as_array()
        .ok_or_else(|| ProviderError::malformed(listing, "expected a candle array"))?;
    rows.iter().map(|row| parse_row(listing, row)).collect()
}

fn parse_row(listing: Listing, row: &Value) -> Result<Kline, ProviderError> {
    let value = |index: usize| -> Result<f64, ProviderError> {
        row.get(index)
            .and_then(Value::as_str)
            .and_then(|text| text.parse().ok())
            .ok_or_else(|| ProviderError::malformed(listing, format!("invalid field {index}")))
    };
    let time = |index: usize| -> Result<i64, ProviderError> {
        row.get(index)
            .and_then(Value::as_i64)
            .ok_or_else(|| ProviderError::malformed(listing, format!("invalid timestamp {index}")))
    };

    Ok(Kline {
        open_time: time(0)?,
        open: value(1)?,
        high: value(2)?,
        low: value(3)?,
        close: value(4)?,
        volume: value(5)?,
        close_time: time(6)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::market::ports::fakes::Rig;
    use crate::service::net::http::{self, Retry};

    const MINUTE: i64 = 60_000;
    const NOON: i64 = Rig::NOON;

    fn venue(rig: &Rig) -> Binance {
        Binance::new(&rig.context())
    }

    /// An answer of the `count` one-minute klines that open from `first`, oldest first.
    fn minute_rows(first: i64, count: usize) -> String {
        let rows: Vec<String> = (0..count as i64)
            .map(|index| {
                let open = first + index * MINUTE;
                format!(
                    r#"[{open},"10","12","9","11","1.5",{},"15",3,"1","1","0"]"#,
                    open + MINUTE - 1
                )
            })
            .collect();
        format!("[{}]", rows.join(","))
    }

    #[test]
    fn the_newest_candles_are_one_request_of_the_markets_own_api() {
        let rig = Rig::new();
        rig.http
            .on("api.binance.com", &minute_rows(NOON - 3 * MINUTE, 3));
        rig.http
            .on("fapi.binance.com", &minute_rows(NOON - 2 * MINUTE, 2));
        let binance = venue(&rig);
        let spot = binance
            .fetch_klines("btc/usdt", Timeframe::MINUTE_1, 3)
            .expect("candles");
        let perpetual = binance
            .fetch_klines("BTCUSDT.P", Timeframe::MINUTE_1, 2)
            .expect("candles");
        let sent = rig.http.sent();
        assert_eq!(
            sent.iter()
                .map(|sent| sent.url.as_str())
                .collect::<Vec<_>>(),
            [
                "https://api.binance.com/api/v3/klines?symbol=BTCUSDT&interval=1m&limit=3",
                "https://fapi.binance.com/fapi/v1/klines?symbol=BTCUSDT&interval=1m&limit=2",
            ]
        );
        assert!(sent.iter().all(|sent| sent.retry == Retry::Transient));
        assert_eq!((spot.len(), perpetual.len()), (3, 2));
        assert_eq!(spot[0].open_time, NOON - 3 * MINUTE);
    }

    #[test]
    fn a_range_is_fetched_as_windows_whose_starts_follow_from_the_interval() {
        let rig = Rig::new();
        let end = NOON;
        let first = end - 1_500 * MINUTE;
        let second = first + 1_000 * MINUTE;
        rig.http
            .on(&format!("startTime={first}&"), &minute_rows(first, 1_000));
        rig.http
            .on(&format!("startTime={second}&"), &minute_rows(second, 500));
        let candles = venue(&rig)
            .fetch_klines_range("BTCUSDT", Timeframe::MINUTE_1, 0, end, 1_500)
            .expect("candles");
        // The pages are asked side by side, so in no fixed order.
        let mut urls = rig.http.urls();
        urls.sort();
        let base = "https://api.binance.com/api/v3/klines?symbol=BTCUSDT&interval=1m";
        let mut expected = vec![
            format!("{base}&startTime={first}&endTime={}&limit=1000", second - 1),
            format!("{base}&startTime={second}&endTime={}&limit=1000", end - 1),
        ];
        expected.sort();
        assert_eq!(urls, expected);
        assert_eq!(candles.len(), 1_500);
        assert_eq!(candles[0].open_time, first);
        assert_eq!(candles[1_499].open_time, end - MINUTE);
    }

    #[test]
    fn a_request_for_more_than_one_page_of_the_newest_pages_back_from_the_clock() {
        let rig = Rig::new();
        let first = NOON + 1 - 1_200 * MINUTE;
        rig.http
            .on(&format!("startTime={first}&"), &minute_rows(first, 1_000));
        rig.http.on(
            &format!("startTime={}&", first + 1_000 * MINUTE),
            &minute_rows(first + 1_000 * MINUTE, 200),
        );
        let candles = venue(&rig)
            .fetch_klines("BTCUSDT", Timeframe::MINUTE_1, 1_200)
            .expect("candles");
        let urls = rig.http.urls();
        assert_eq!(urls.len(), 2);
        assert!(
            urls.iter()
                .any(|url| url.contains(&format!("startTime={first}&endTime="))),
            "{urls:?}"
        );
        assert!(
            urls.iter()
                .any(|url| url.ends_with(&format!("endTime={NOON}&limit=1000"))),
            "the last window ends with the clock: {urls:?}"
        );
        assert_eq!(candles.len(), 1_200);
    }

    #[test]
    fn what_binance_refuses_is_told_by_kind_and_by_the_market_it_was_asked_of() {
        let error_of = |symbol: &str, answer: Result<&str, http::Error>| {
            let rig = Rig::new();
            match answer {
                Ok(body) => rig.http.on("/klines?", body),
                Err(error) => rig.http.fail("/klines?", error),
            };
            venue(&rig)
                .fetch_klines(symbol, Timeframe::HOUR_1, 5)
                .unwrap_err()
        };
        assert_eq!(
            error_of("NOPEUSDT", Err(http::Error::Status(400))).kind,
            ErrorKind::UnknownSymbol
        );
        let limited = error_of("BTCUSDT.P", Err(http::Error::Status(429)));
        assert_eq!(limited.kind, ErrorKind::RateLimited);
        assert_eq!(limited.venue, Listing::BINANCE_PERPETUAL);
        assert_eq!(
            error_of("BTCUSDT", Ok(r#"{"code":-1121,"msg":"Invalid symbol."}"#)).kind,
            ErrorKind::Malformed("expected a candle array".to_owned())
        );
    }

    #[test]
    fn parses_binance_rows() {
        let body = r#"[[1700000000000,"10.0","12.0","9.0","11.0","42.5",1700003599999]]"#;
        let candles =
            parse_klines(Listing::spot(DataSource::Binance), body).expect("valid response");
        assert_eq!(candles.len(), 1);
        assert_eq!(
            candles[0],
            Kline {
                open_time: 1_700_000_000_000,
                close_time: 1_700_003_599_999,
                open: 10.0,
                high: 12.0,
                low: 9.0,
                close: 11.0,
                volume: 42.5,
            }
        );
    }

    fn base_of(timeframe: &str) -> (String, usize) {
        let plan = plan(DataSource::Binance, Timeframe::parse(timeframe).unwrap()).unwrap();
        (plan.base().interval(), plan.base_count(500))
    }

    #[test]
    fn a_native_length_is_asked_for_as_it_is() {
        for token in [
            "1m", "3m", "5m", "15m", "30m", "1h", "2h", "4h", "6h", "8h", "12h",
        ] {
            assert_eq!(base_of(token), (token.to_owned(), 500), "{token}");
        }
        assert_eq!(base_of("1d"), ("1d".to_owned(), 500));
        assert_eq!(base_of("1w"), ("1w".to_owned(), 500));
        assert_eq!(base_of("1M"), ("1M".to_owned(), 500));
    }

    #[test]
    fn other_lengths_fold_the_coarsest_native_that_nests() {
        // A request for 500 candles also asks for the one the start cuts.
        assert_eq!(base_of("22m"), ("1m".to_owned(), 501 * 22));
        assert_eq!(base_of("6m"), ("3m".to_owned(), 501 * 2));
        assert_eq!(base_of("3h"), ("1h".to_owned(), 501 * 3));
        assert_eq!(base_of("10h"), ("2h".to_owned(), 501 * 5));
        assert_eq!(base_of("24h"), ("12h".to_owned(), 501 * 2));
        // Binance's own 3d is not on the grid the other sources cut, so days are folded.
        assert_eq!(base_of("3d"), ("1d".to_owned(), 501 * 3));
        assert_eq!(base_of("2w"), ("1w".to_owned(), 501 * 2));
        assert_eq!(base_of("3M"), ("1M".to_owned(), 501 * 3));
        assert_eq!(base_of("1Y"), ("1M".to_owned(), 501 * 12));
    }

    #[test]
    fn a_series_of_more_than_one_requests_candles_is_paged_but_its_tail_is_not() {
        assert!(base_of("5m").1 <= MAX_PER_REQUEST);
        assert!(base_of("22m").1 > MAX_PER_REQUEST);
        let tail = plan(DataSource::Binance, Timeframe::parse("22m").unwrap())
            .unwrap()
            .base_count(crate::service::cache::TAIL_LIMIT as usize);
        assert!(tail <= MAX_PER_REQUEST);
    }

    #[test]
    fn a_symbol_is_asked_of_its_own_markets_api() {
        let spot = Target::of("btcusdt");
        assert_eq!(spot.pair, "BTCUSDT");
        assert!(spot.endpoints.klines_url.contains("api.binance.com/api/v3"));
        let perp = Target::of("btcusdt.p");
        assert_eq!(perp.pair, "BTCUSDT");
        assert_eq!(perp.listing, Listing::BINANCE_PERPETUAL);
        assert!(
            perp.endpoints
                .klines_url
                .contains("fapi.binance.com/fapi/v1")
        );
        assert_eq!(Target::of("龙USDT.P").pair, "%E9%BE%99USDT");
        assert_eq!(Target::of("btc.p").pair, "BTCUSDT");
        assert_eq!(Target::of("btc/usdt").pair, "BTCUSDT");
    }

    #[test]
    fn rejects_error_objects() {
        for listing in [
            Listing::spot(DataSource::Binance),
            Listing::BINANCE_PERPETUAL,
        ] {
            let error = parse_klines(listing, r#"{"code":-1121,"msg":"Invalid symbol."}"#)
                .expect_err("object must fail");
            assert_eq!(
                error,
                ProviderError::malformed(listing, "expected a candle array"),
                "a failure names the list that was asked"
            );
        }
    }
}
