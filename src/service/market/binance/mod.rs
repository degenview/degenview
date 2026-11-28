//! Binance's REST market data, spot and USDⓈ-M perpetuals alike: a symbol says which
//! (`model::market::binance`) and [`endpoints`] where that market is served.

use super::candles::plan;
use super::search_rank::normalize;
use crate::model::market::binance::split_market;
use crate::model::{DataSource, InstrumentId, Kline, Listing, Timeframe};
use crate::service::market::error::ErrorKind;
use crate::service::net::url::encode_component;
use crate::service::{MarketSource, ProviderError, SearchResult};
use serde_json::Value;

mod catalog;
mod endpoints;

pub(in crate::service) use catalog::tick_size;
pub(in crate::service) use endpoints::{Endpoints, endpoints};

pub struct Binance;

pub const BINANCE: Binance = Binance;

/// What Binance's HTTP statuses mean beyond a rate limit: a 400 names a market it does not list.
pub(in crate::service::market) const STATUSES: &[(u16, ErrorKind)] =
    &[(400, ErrorKind::UnknownSymbol)];

/// A request for `listing`'s market data.
fn get(listing: Listing, url: &str) -> Result<String, ProviderError> {
    crate::service::net::http::get_text(url, &[], crate::service::net::http::Retry::Transient)
        .map_err(|error| ProviderError::http(listing, error))
}

/// The most klines one Binance request returns.
const MAX_PER_REQUEST: usize = 1_000;

/// What one `symbol` of the app asks Binance for: where, the pair as a query value, and the
/// market's own listing.
struct Request {
    endpoints: &'static Endpoints,
    pair: String,
    listing: Listing,
}

impl Request {
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
        let request = Request::of(symbol);
        let plan = plan(request.listing, timeframe)?;
        let requested = plan.base_count(limit as usize);
        if requested > MAX_PER_REQUEST {
            // More finer candles than one request holds: page back from now.
            let end_ms = crate::clock::now_ms() + 1;
            return self.fetch_klines_range(symbol, timeframe, 0, end_ms, limit as usize);
        }
        let url = format!(
            "{}?symbol={}&interval={}&limit={requested}",
            request.endpoints.klines_url,
            request.pair,
            plan.base().interval()
        );
        let body = get(request.listing, &url)?;
        Ok(plan.shape(parse_klines(&body)?, requested, limit as usize))
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
        let request = Request::of(symbol);
        let plan = plan(request.listing, timeframe)?;
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
                        let (interval, request) = (&interval, &request);
                        scope.spawn(move || {
                            let to = (from + page_ms).min(end_ms) - 1;
                            let url = format!(
                                "{}?symbol={}&interval={interval}&startTime={from}&endTime={to}&limit={PAGE}",
                                request.endpoints.klines_url,
                                request.pair
                            );
                            parse_klines(&get(request.listing, &url)?)
                        })
                    })
                    .collect();
                handles
                    .into_iter()
                    .map(|handle| {
                        handle
                            .join()
                            .unwrap_or_else(|_| Err(ProviderError::interrupted(request.listing)))
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
        catalog::search(query, &needle)
    }
}

fn parse_klines(body: &str) -> Result<Vec<Kline>, ProviderError> {
    let value: Value = serde_json::from_str(body)
        .map_err(|error| ProviderError::malformed(DataSource::Binance, error))?;
    let rows = value
        .as_array()
        .ok_or_else(|| ProviderError::malformed(DataSource::Binance, "expected a candle array"))?;
    rows.iter().map(parse_row).collect()
}

fn parse_row(row: &Value) -> Result<Kline, ProviderError> {
    let value = |index: usize| -> Result<f64, ProviderError> {
        row.get(index)
            .and_then(Value::as_str)
            .and_then(|text| text.parse().ok())
            .ok_or_else(|| {
                ProviderError::malformed(DataSource::Binance, format!("invalid field {index}"))
            })
    };
    let time = |index: usize| -> Result<i64, ProviderError> {
        row.get(index).and_then(Value::as_i64).ok_or_else(|| {
            ProviderError::malformed(DataSource::Binance, format!("invalid timestamp {index}"))
        })
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

    #[test]
    fn parses_binance_rows() {
        let body = r#"[[1700000000000,"10.0","12.0","9.0","11.0","42.5",1700003599999]]"#;
        let candles = parse_klines(body).expect("valid response");
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
        let spot = Request::of("btcusdt");
        assert_eq!(spot.pair, "BTCUSDT");
        assert!(spot.endpoints.klines_url.contains("api.binance.com/api/v3"));
        let perp = Request::of("btcusdt.p");
        assert_eq!(perp.pair, "BTCUSDT");
        assert_eq!(perp.listing, Listing::BINANCE_PERPETUAL);
        assert!(
            perp.endpoints
                .klines_url
                .contains("fapi.binance.com/fapi/v1")
        );
        assert_eq!(Request::of("龙USDT.P").pair, "%E9%BE%99USDT");
        assert_eq!(Request::of("btc.p").pair, "BTCUSDT");
        assert_eq!(Request::of("btc/usdt").pair, "BTCUSDT");
    }

    #[test]
    fn rejects_error_objects() {
        let error = parse_klines(r#"{"code":-1121,"msg":"Invalid symbol."}"#)
            .expect_err("object must fail");
        assert_eq!(
            error,
            ProviderError::malformed(DataSource::Binance, "expected a candle array")
        );
    }
}
