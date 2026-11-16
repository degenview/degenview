use super::candles::plan;
use crate::model::market::bucket::{self, FoldPlan};
use crate::model::market::natives::GECKOTERMINAL_PAGE;
use crate::model::{DataSource, Kline, TimeUnit, Timeframe};
use crate::service::ProviderError;
use serde_json::Value;

const BASE_URL: &str = "https://api.geckoterminal.com/api/v2";

/// What one OHLCV request asks for: the native bucket a timeframe is made from and how many of
/// them.
struct Window {
    plan: FoldPlan,
    /// GeckoTerminal's `timeframe` (`minute`, `hour`, `day`) and the `aggregate` of it.
    timeframe: &'static str,
    aggregate: u16,
    fetch_limit: usize,
}

impl Window {
    fn new(plan: FoldPlan, limit: usize) -> Window {
        let base = plan.base();
        Window {
            plan,
            timeframe: match base.unit() {
                TimeUnit::Minute => "minute",
                TimeUnit::Hour => "hour",
                // Its natives hold no weeks or months.
                TimeUnit::Day | TimeUnit::Week | TimeUnit::Month => "day",
            },
            aggregate: base.count(),
            fetch_limit: plan.base_count(limit).clamp(1, GECKOTERMINAL_PAGE),
        }
    }

    fn bucket_ms(&self) -> i64 {
        self.plan.base().nominal_ms()
    }
}

/// OHLCV for an on-chain DEX pool. `pair_address` is a contract address; the
/// network is resolved through GeckoTerminal's pool search.
///
/// GeckoTerminal only lists intervals that traded and tops out at daily bars, so empty buckets
/// are filled flat and coarser bars are folded on the calendar, which makes the series
/// line up with an exchange's.
pub fn fetch_klines(
    pair_address: &str,
    timeframe: Timeframe,
    limit: u32,
) -> Result<Vec<Kline>, ProviderError> {
    let window = Window::new(plan(DataSource::DexScreener, timeframe)?, limit as usize);
    let network = network_for(pair_address)?;
    let side = side_for(&network, pair_address);
    let raw = fetch_ohlcv(&network, pair_address, side, &window, None)?;
    if raw.is_empty() {
        return Err(ProviderError::other("No chart data for this pool"));
    }
    let mut candles = shape(raw, &window, crate::service::now_ms());
    let skip = candles.len().saturating_sub(limit as usize);
    candles.drain(..skip);
    Ok(candles)
}

/// Native buckets with their gaps filled, folded to the timeframe when that is coarser, oldest
/// first.
fn shape(mut raw: Vec<Kline>, window: &Window, now_ms: i64) -> Vec<Kline> {
    raw.sort_by_key(|candle| candle.open_time);
    raw.dedup_by_key(|candle| candle.open_time);
    let truncated = raw.len() >= window.fetch_limit;
    let filled = bucket::fill_gaps(raw, window.bucket_ms(), now_ms);
    window.plan.fold(filled, truncated)
}

/// Why an OHLCV request brought no candles.
enum Fetch {
    /// The free API only serves the last 180 days; the request asked for older ones.
    BeyondHistory,
    Failed(ProviderError),
}

impl From<Fetch> for ProviderError {
    fn from(fetch: Fetch) -> Self {
        match fetch {
            Fetch::BeyondHistory => {
                ProviderError::other("GeckoTerminal has no older candles for this pool")
            }
            Fetch::Failed(error) => error,
        }
    }
}

/// Candles of one pool, newest first as served. `before` (seconds) asks for those opening before it.
fn fetch_ohlcv(
    network: &str,
    pool: &str,
    side: Side,
    window: &Window,
    before: Option<i64>,
) -> Result<Vec<Kline>, Fetch> {
    let mut url = format!(
        "{BASE_URL}/networks/{network}/pools/{pool}/ohlcv/{}?aggregate={}&limit={}&currency=usd",
        window.timeframe, window.aggregate, window.fetch_limit
    );
    if side == Side::Quote {
        url.push_str("&token=quote");
    }
    if let Some(before) = before {
        url.push_str(&format!("&before_timestamp={before}"));
    }
    crate::service::net::rate_limit::geckoterminal().wait_for_slot();
    let body = match crate::service::net::http::get_text(
        &url,
        &[],
        crate::service::net::http::Retry::Never,
    ) {
        Ok(body) => body,
        Err(crate::service::net::http::Error::Status(401)) if before.is_some() => {
            return Err(Fetch::BeyondHistory);
        }
        Err(error) => return Err(Fetch::Failed(error.into())),
    };
    parse_ohlcv(&body, window.bucket_ms()).map_err(Fetch::Failed)
}

/// An older page: the candles of `timeframe` opening in `[start_ms, end_ms)`, at most `max` (the
/// newest), oldest first, folded from one page of the native candles before `end_ms`. Nothing
/// older than 180 days exists on the free API, which ends the history with an empty page.
pub fn fetch_klines_range(
    pair_address: &str,
    timeframe: Timeframe,
    start_ms: i64,
    end_ms: i64,
    max: usize,
) -> Result<Vec<Kline>, ProviderError> {
    let window = Window::new(plan(DataSource::DexScreener, timeframe)?, max);
    let network = network_for(pair_address)?;
    let side = side_for(&network, pair_address);
    let raw = match fetch_ohlcv(&network, pair_address, side, &window, Some(end_ms / 1_000)) {
        Ok(raw) => raw,
        Err(Fetch::BeyondHistory) => return Ok(Vec::new()),
        Err(Fetch::Failed(error)) => return Err(error),
    };
    // Filled up to the slot before `end_ms`, so no gap opens between this page and what is held.
    let mut candles = shape(raw, &window, end_ms - 1);
    candles.retain(|candle| candle.open_time >= start_ms && candle.open_time < end_ms);
    let skip = candles.len().saturating_sub(max);
    Ok(candles.split_off(skip))
}

/// `[timestamp_seconds, open, high, low, close, volumeUSD]`, newest first. A bucket closes a
/// millisecond before the next opens, as on an exchange.
fn parse_ohlcv(body: &str, bucket_ms: i64) -> Result<Vec<Kline>, ProviderError> {
    let value: Value = serde_json::from_str(body).map_err(ProviderError::data)?;
    let rows = value
        .pointer("/data/attributes/ohlcv_list")
        .and_then(Value::as_array)
        .ok_or_else(|| ProviderError::other("No chart data for this pool"))?;
    let mut candles = Vec::with_capacity(rows.len());
    for row in rows {
        let Some(fields) = row.as_array() else {
            continue;
        };
        if fields.len() < 6 {
            continue;
        }
        let (Some(seconds), Some(open), Some(high), Some(low), Some(close)) = (
            fields[0].as_f64(),
            fields[1].as_f64(),
            fields[2].as_f64(),
            fields[3].as_f64(),
            fields[4].as_f64(),
        ) else {
            continue;
        };
        let volume = fields[5].as_f64().unwrap_or(0.0);
        let open_time = (seconds * 1_000.0) as i64;
        candles.push(Kline {
            open_time,
            close_time: open_time + bucket_ms - 1,
            open,
            high,
            low,
            close,
            volume,
        });
    }
    Ok(candles)
}

/// Which token of a pool a chart follows. GeckoTerminal serves a pool's candles for its base
/// token unless asked for the quote, and names the base by the token contracts' order on chain,
/// so a pool can list the stablecoin first ("USDT / MCADE"). DEXScreener, which the markets are
/// searched on, always puts the traded token first, so such a pool's default candles are the
/// stablecoin's: a flat $1 where the token trades for a fraction of a cent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Side {
    Base,
    Quote,
}

/// Tokens a pool is priced in rather than traded for: stablecoins and wrapped native coins.
const QUOTE_ASSETS: [&str; 20] = [
    "USDT", "USDC", "USDC.E", "DAI", "USDE", "USDS", "FDUSD", "TUSD", "PYUSD", "BUSD", "USD1",
    "WETH", "ETH", "WBNB", "BNB", "WSOL", "SOL", "WAVAX", "WPOL", "WMATIC",
];

fn is_quote_asset(symbol: &str) -> bool {
    QUOTE_ASSETS.contains(&symbol.trim().to_uppercase().as_str())
}

/// The side of a pool holding the token being traded: the quote side only when the base is a
/// quote asset and the quote is not. A pool of two quote assets (ETH / USDC) keeps its base.
fn traded_side(base_symbol: &str, quote_symbol: &str) -> Side {
    if is_quote_asset(base_symbol) && !is_quote_asset(quote_symbol) {
        Side::Quote
    } else {
        Side::Base
    }
}

/// `(base, quote)` token symbols of a pool, from `/pools/{pool}?include=base_token,quote_token`.
fn parse_pool_symbols(body: &str) -> Option<(String, String)> {
    let value: Value = serde_json::from_str(body).ok()?;
    let token_id = |relationship: &str| {
        value
            .pointer(&format!("/data/relationships/{relationship}/data/id"))
            .and_then(Value::as_str)
    };
    let (base_id, quote_id) = (token_id("base_token")?, token_id("quote_token")?);
    let included = value.get("included").and_then(Value::as_array)?;
    let symbol = |id: &str| {
        included
            .iter()
            .find(|token| token.get("id").and_then(Value::as_str) == Some(id))
            .and_then(|token| token.pointer("/attributes/symbol"))
            .and_then(Value::as_str)
            .map(str::to_owned)
    };
    Some((symbol(base_id)?, symbol(quote_id)?))
}

/// Where a pool's traded side is remembered: the order of a pool's tokens never changes.
const SIDE_NAMESPACE: &str = "geckoterminal_sides";
const SIDE_TTL: std::time::Duration = std::time::Duration::from_secs(90 * 86_400);

/// Which token of `pool` to chart. When the pool's tokens cannot be read the base is used, as
/// before, and the answer is not remembered, so the next request asks again.
fn side_for(network: &str, pool: &str) -> Side {
    let key = format!("{network}:{}", pool.to_lowercase());
    let cache = crate::service::cache::lookup::shared();
    if let Some(hit) = cache.get::<String>(SIDE_NAMESPACE, &key, SIDE_TTL)
        && hit.fresh
    {
        return if hit.value == "quote" {
            Side::Quote
        } else {
            Side::Base
        };
    }
    let url = format!("{BASE_URL}/networks/{network}/pools/{pool}?include=base_token,quote_token");
    crate::service::net::rate_limit::geckoterminal().wait_for_slot();
    let Some((base, quote)) =
        crate::service::net::http::get_text(&url, &[], crate::service::net::http::Retry::Never)
            .ok()
            .and_then(|body| parse_pool_symbols(&body))
    else {
        return Side::Base;
    };
    let side = traded_side(&base, &quote);
    cache.put(
        SIDE_NAMESPACE,
        &key,
        &if side == Side::Quote { "quote" } else { "base" },
    );
    side
}

/// Where a pool's network is remembered: a pool never changes chain, and finding it costs a
/// rate-limited request.
const NETWORK_NAMESPACE: &str = "geckoterminal_networks";
const NETWORK_TTL: std::time::Duration = std::time::Duration::from_secs(30 * 86_400);

/// Which chain a pool address sits on, from memory or disk when it was found before.
fn network_for(pair_address: &str) -> Result<String, ProviderError> {
    let key = pair_address.to_lowercase();
    let cache = crate::service::cache::lookup::shared();
    if let Some(hit) = cache.get::<String>(NETWORK_NAMESPACE, &key, NETWORK_TTL)
        && hit.fresh
    {
        return Ok(hit.value);
    }
    let network = lookup_network(pair_address)?;
    cache.put(NETWORK_NAMESPACE, &key, &network);
    Ok(network)
}

/// Asks GeckoTerminal which chain a pool address sits on. The per-pool endpoints are namespaced by
/// network, and DEXScreener's chain names don't match GeckoTerminal's ids.
fn lookup_network(pair_address: &str) -> Result<String, ProviderError> {
    let key = pair_address.to_lowercase();
    let url = format!("{BASE_URL}/search/pools?query={pair_address}");
    crate::service::net::rate_limit::geckoterminal().wait_for_slot();
    let body =
        crate::service::net::http::get_text(&url, &[], crate::service::net::http::Retry::Never)?;
    let value: Value = serde_json::from_str(&body).map_err(ProviderError::data)?;
    let suffix = format!("_{key}");
    for pool in value
        .get("data")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
    {
        let Some(id) = pool.get("id").and_then(Value::as_str) else {
            continue;
        };
        let address = pool
            .pointer("/attributes/address")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let matches = address.to_lowercase() == key || id.to_lowercase().ends_with(&suffix);
        if !matches {
            continue;
        }
        if let Some(position) = id.to_lowercase().rfind(&suffix) {
            let network = &id[..position];
            if !network.is_empty() {
                return Ok(network.to_owned());
            }
        }
    }
    Err(ProviderError::other("Pool not indexed by GeckoTerminal"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(timeframe: &str, limit: usize) -> Window {
        let timeframe = Timeframe::parse(timeframe).unwrap();
        Window::new(plan(DataSource::DexScreener, timeframe).unwrap(), limit)
    }

    #[test]
    fn a_native_length_is_asked_for_as_it_is() {
        for (token, unit, aggregate) in [
            ("1m", "minute", 1),
            ("5m", "minute", 5),
            ("15m", "minute", 15),
            ("1h", "hour", 1),
            ("4h", "hour", 4),
            ("12h", "hour", 12),
            ("1d", "day", 1),
        ] {
            let hourly = window(token, 500);
            assert_eq!(
                (hourly.timeframe, hourly.aggregate),
                (unit, aggregate),
                "{token}"
            );
            assert_eq!(hourly.fetch_limit, 500, "{token}");
            assert!(hourly.plan.is_native());
        }
    }

    #[test]
    fn other_lengths_fold_the_coarsest_native_that_nests_up_to_a_page() {
        let three_hours = window("3h", 100);
        assert_eq!((three_hours.timeframe, three_hours.aggregate), ("hour", 1));
        assert_eq!(three_hours.fetch_limit, 101 * 3);
        let weekly = window("1w", 500);
        assert_eq!(weekly.timeframe, "day");
        assert!(!weekly.plan.is_native());
        assert_eq!(weekly.fetch_limit, GECKOTERMINAL_PAGE);
        let odd = window("22m", 100);
        assert_eq!((odd.timeframe, odd.aggregate), ("minute", 1));
    }

    fn daily(start: i64, days: &[i64]) -> Vec<Kline> {
        days.iter()
            .map(|day| {
                let open_time = start + day * 86_400_000;
                Kline {
                    open_time,
                    close_time: open_time,
                    open: 10.0,
                    high: 12.0 + *day as f64,
                    low: 9.0,
                    close: 11.0,
                    volume: 1.0,
                }
            })
            .collect()
    }

    #[test]
    fn weeks_are_folded_on_the_calendar_not_in_equal_runs() {
        // 2024-01-01 is a Monday. Days 2 and 3 never traded, and the pool has far fewer days than
        // the 500 bars asked for, which used to come back as mislabelled daily bars.
        let monday = crate::format::days_from_civil(2024, 1, 1) * 86_400_000;
        let raw = daily(monday, &[0, 1, 4, 5, 6, 7, 8, 9, 10]);
        let now = monday + 10 * 86_400_000 + 1_000;
        let weeks = shape(raw, &window("1w", 500), now);
        assert_eq!(weeks.len(), 2);
        assert_eq!(weeks[0].open_time, monday);
        assert_eq!(weeks[0].close_time, monday + 7 * 86_400_000 - 1);
        assert_eq!(weeks[1].open_time, monday + 7 * 86_400_000);
        // Filled days are flat, so only days that traded add volume.
        assert_eq!(weeks[0].volume, 5.0);
        assert_eq!(weeks[0].high, 18.0);
    }

    #[test]
    fn empty_days_are_filled_up_to_today() {
        let start = crate::format::days_from_civil(2024, 1, 1) * 86_400_000;
        let now = start + 6 * 86_400_000 + 5;
        let days = shape(daily(start, &[0, 1, 4]), &window("1d", 500), now);
        assert_eq!(days.len(), 7);
        assert_eq!(days[2].volume, 0.0);
        assert_eq!(days[2].open, 11.0);
        assert_eq!(days[6].open_time, start + 6 * 86_400_000);
        assert_eq!(days[6].close_time, days[6].open_time + 86_400_000 - 1);
    }

    #[test]
    fn a_pool_listing_the_stablecoin_first_is_charted_by_its_quote() {
        assert_eq!(traded_side("USDT", "MCADE"), Side::Quote);
        assert_eq!(traded_side("usdc", "WEN"), Side::Quote);
        assert_eq!(traded_side("WBNB", "WSM"), Side::Quote);
        assert_eq!(traded_side("MCADE", "USDT"), Side::Base);
        assert_eq!(traded_side("WSM", "WBNB"), Side::Base);
        // Two quote assets, or two traded tokens: nothing says to flip, so the base stays.
        assert_eq!(traded_side("ETH", "USDC"), Side::Base);
        assert_eq!(traded_side("PEPE", "WIF"), Side::Base);
    }

    #[test]
    fn pool_symbols_come_from_the_included_tokens() {
        let body = r#"{"data":{"relationships":{
            "base_token":{"data":{"id":"eth_0xdac1","type":"token"}},
            "quote_token":{"data":{"id":"eth_0xed32","type":"token"}}}},
          "included":[
            {"id":"eth_0xed32","type":"token","attributes":{"symbol":"MCADE"}},
            {"id":"eth_0xdac1","type":"token","attributes":{"symbol":"USDT"}}]}"#;
        assert_eq!(
            parse_pool_symbols(body),
            Some(("USDT".to_owned(), "MCADE".to_owned()))
        );
        assert_eq!(parse_pool_symbols(r#"{"data":{}}"#), None);
        assert_eq!(parse_pool_symbols("not json"), None);
    }
}
