use super::{fill_gaps, fold_calendar, interval_seconds};
use crate::model::Kline;
use serde_json::Value;

const BASE_URL: &str = "https://api.geckoterminal.com/api/v2";
const MAX_CANDLES: i64 = 1_000;

/// Buckets GeckoTerminal serves natively, in seconds.
const BUCKETS: [(&str, i64, i64); 7] = [
    ("minute", 1, 60),
    ("minute", 5, 300),
    ("minute", 15, 900),
    ("hour", 1, 3_600),
    ("hour", 4, 14_400),
    ("hour", 12, 43_200),
    ("day", 1, 86_400),
];

struct Window {
    timeframe: &'static str,
    aggregate: i64,
    /// Seconds in one native bucket.
    bucket_secs: i64,
    fetch_limit: usize,
    needs_aggregation: bool,
}

impl Window {
    fn for_interval(interval: &str, limit: usize) -> Window {
        let target = interval_seconds(interval).unwrap_or(3_600);
        let span = target * limit as i64;
        let bucket = BUCKETS
            .iter()
            .rev()
            .find(|(_, _, seconds)| *seconds <= target)
            .unwrap_or(&BUCKETS[0]);
        let wanted = (span + bucket.2 - 1) / bucket.2;
        Window {
            timeframe: bucket.0,
            aggregate: bucket.1,
            bucket_secs: bucket.2,
            fetch_limit: wanted.clamp(1, MAX_CANDLES) as usize,
            needs_aggregation: bucket.2 < target,
        }
    }
}

/// OHLCV for an on-chain DEX pool. `pair_address` is a contract address; the
/// network is resolved through GeckoTerminal's pool search.
///
/// GeckoTerminal only lists intervals that traded and tops out at daily bars, so empty hours and
/// days are filled flat and coarser bars are folded on the calendar, which makes the series
/// line up with an exchange's.
pub fn fetch_klines(pair_address: &str, interval: &str, limit: u32) -> Result<Vec<Kline>, String> {
    let window = Window::for_interval(interval, limit as usize);
    let network = network_for(pair_address)?;
    let side = side_for(&network, pair_address);
    let raw = fetch_ohlcv(&network, pair_address, side, &window, None)?;
    if raw.is_empty() {
        return Err("No chart data for this pool".to_owned());
    }
    let candles = shape(raw, interval, &window, super::now_ms());
    let start = candles.len().saturating_sub(limit as usize);
    Ok(candles[start..].to_vec())
}

/// Native buckets with their gaps filled, folded to `interval` when that is coarser, oldest first.
fn shape(mut raw: Vec<Kline>, interval: &str, window: &Window, now_ms: i64) -> Vec<Kline> {
    raw.sort_by_key(|candle| candle.open_time);
    raw.dedup_by_key(|candle| candle.open_time);
    let target = interval_seconds(interval).unwrap_or(window.bucket_secs);
    let truncated = raw.len() >= window.fetch_limit;
    let filled = fill_gaps(raw, window.bucket_secs * 1_000, now_ms);
    if window.needs_aggregation {
        return fold_calendar(&filled, target, truncated);
    }
    // A bucket closes a millisecond before the next opens, as on an exchange.
    filled
        .into_iter()
        .map(|candle| Kline {
            close_time: candle.open_time + target * 1_000 - 1,
            ..candle
        })
        .collect()
}

/// Why an OHLCV request brought no candles.
enum Fetch {
    /// The free API only serves the last 180 days; the request asked for older ones.
    BeyondHistory,
    Failed(String),
}

impl From<Fetch> for String {
    fn from(fetch: Fetch) -> String {
        match fetch {
            Fetch::BeyondHistory => "GeckoTerminal has no older candles for this pool".to_owned(),
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
    super::rate_limit::geckoterminal().wait_for_slot();
    let body = match super::http::get_text(&url, &[], super::http::Retry::Never) {
        Ok(body) => body,
        Err(super::http::Error::Status(401)) if before.is_some() => {
            return Err(Fetch::BeyondHistory);
        }
        Err(error) => return Err(Fetch::Failed(format!("Network error: {error}"))),
    };
    parse_ohlcv(&body).map_err(Fetch::Failed)
}

/// An older page of hourly or daily candles: those opening in `[start_ms, end_ms)`, at most `max`
/// (the newest), oldest first. Nothing older than 180 days exists on the free API, which ends the
/// history with an empty page. Coarser bars are folded from the 180 days a fetch already holds.
pub fn fetch_klines_range(
    pair_address: &str,
    interval: &str,
    start_ms: i64,
    end_ms: i64,
    max: usize,
) -> Result<Vec<Kline>, String> {
    let window = Window::for_interval(interval, max);
    if window.needs_aggregation {
        return Err("GeckoTerminal pages hourly and daily candles only".to_owned());
    }
    let network = network_for(pair_address)?;
    let side = side_for(&network, pair_address);
    let raw = match fetch_ohlcv(&network, pair_address, side, &window, Some(end_ms / 1_000)) {
        Ok(raw) => raw,
        Err(Fetch::BeyondHistory) => return Ok(Vec::new()),
        Err(Fetch::Failed(error)) => return Err(error),
    };
    // Filled up to the slot before `end_ms`, so no gap opens between this page and what is held.
    let mut candles = shape(raw, interval, &window, end_ms - 1);
    candles.retain(|candle| candle.open_time >= start_ms && candle.open_time < end_ms);
    let skip = candles.len().saturating_sub(max);
    Ok(candles.split_off(skip))
}

/// `[timestamp_seconds, open, high, low, close, volumeUSD]`, newest first.
fn parse_ohlcv(body: &str) -> Result<Vec<Kline>, String> {
    let value: Value =
        serde_json::from_str(body).map_err(|error| format!("Data error: {error}"))?;
    let rows = value
        .pointer("/data/attributes/ohlcv_list")
        .and_then(Value::as_array)
        .ok_or_else(|| "No chart data for this pool".to_owned())?;
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
            close_time: open_time,
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
    let cache = super::lookup_cache::shared();
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
    super::rate_limit::geckoterminal().wait_for_slot();
    let Some((base, quote)) = super::http::get_text(&url, &[], super::http::Retry::Never)
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
fn network_for(pair_address: &str) -> Result<String, String> {
    let key = pair_address.to_lowercase();
    let cache = super::lookup_cache::shared();
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
fn lookup_network(pair_address: &str) -> Result<String, String> {
    let key = pair_address.to_lowercase();
    let url = format!("{BASE_URL}/search/pools?query={pair_address}");
    super::rate_limit::geckoterminal().wait_for_slot();
    let body = super::http::get_text(&url, &[], super::http::Retry::Never)
        .map_err(|error| format!("Network error: {error}"))?;
    let value: Value =
        serde_json::from_str(&body).map_err(|error| format!("Data error: {error}"))?;
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
    Err("Pool not indexed by GeckoTerminal".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_bucket_and_caps_fetch_limit() {
        let hourly = Window::for_interval("1h", 500);
        assert_eq!(hourly.timeframe, "hour");
        assert_eq!(hourly.aggregate, 1);
        assert_eq!(hourly.fetch_limit, 500);
        assert!(!hourly.needs_aggregation);

        let weekly = Window::for_interval("1w", 500);
        assert_eq!(weekly.timeframe, "day");
        assert!(weekly.needs_aggregation);
        assert_eq!(weekly.fetch_limit, MAX_CANDLES as usize);
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
        let window = Window::for_interval("1w", 500);
        let now = monday + 10 * 86_400_000 + 1_000;
        let weeks = shape(raw, "1w", &window, now);
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
        let window = Window::for_interval("1d", 500);
        let now = start + 6 * 86_400_000 + 5;
        let days = shape(daily(start, &[0, 1, 4]), "1d", &window, now);
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
