use super::catalog::Catalog;
use super::{MarketSource, SearchResult, fold_calendar, rank_results};
use crate::model::Kline;
use serde::{Deserialize, Serialize};
use serde_json::Value;

const BARS_URL: &str = "https://data.alpaca.markets/v2/stocks";

pub struct Alpaca;

pub const ALPACA: Alpaca = Alpaca;

/// What a search or a fetch says when no Alpaca keys are saved.
pub const MISSING_KEY: &str = "Set up your Alpaca API key before using stock charts.";

fn credentials() -> Result<(String, String), String> {
    crate::service::keychain_api_keys::alpaca_credentials().ok_or_else(|| MISSING_KEY.to_owned())
}

/// Quarterly and yearly bars don't exist on Alpaca; they fold monthly ones.
fn monthly_fold(interval: &str) -> Option<i64> {
    match interval {
        "3M" => Some(7_776_000),
        "1Y" => Some(31_536_000),
        _ => None,
    }
}

fn timeframe(interval: &str) -> &'static str {
    match interval {
        "1h" => "1Hour",
        "1d" => "1Day",
        "1w" => "1Week",
        "1M" => "1Month",
        _ => "1Day",
    }
}

fn seconds_per_candle(interval: &str) -> i64 {
    match interval {
        "1h" => 3_600,
        "1d" => 86_400,
        "1w" => 604_800,
        "1M" => 2_592_000,
        _ => 86_400,
    }
}

fn market_gap_factor(interval: &str) -> f64 {
    match interval {
        "1h" => 6.0,
        "1d" => 2.0,
        "1w" => 1.6,
        "1M" => 1.5,
        _ => 2.0,
    }
}

impl MarketSource for Alpaca {
    fn fetch_klines(&self, symbol: &str, interval: &str, limit: u32) -> Result<Vec<Kline>, String> {
        let fold_seconds = monthly_fold(interval);
        let source_interval = if fold_seconds.is_some() {
            "1M"
        } else {
            interval
        };
        let requested = fold_seconds
            .map(|_| (i64::from(limit) + 1) * 12)
            .unwrap_or(i64::from(limit));
        let lookback = seconds_per_candle(source_interval) as f64
            * requested.max(1) as f64
            * market_gap_factor(source_interval);
        let bars = fetch_bars(
            symbol,
            source_interval,
            super::now_ms() - (lookback * 1_000.0) as i64,
            None,
            requested as usize,
        )?;
        let shown = match fold_seconds {
            Some(seconds) => fold_calendar(&bars, seconds, bars.len() >= requested as usize),
            None => bars,
        };
        let start = shown.len().saturating_sub(limit as usize);
        Ok(shown[start..].to_vec())
    }

    fn fetch_klines_range(
        &self,
        symbol: &str,
        interval: &str,
        start_ms: i64,
        end_ms: i64,
        max: usize,
    ) -> Result<Vec<Kline>, String> {
        let fold_seconds = monthly_fold(interval);
        let source_interval = if fold_seconds.is_some() {
            "1M"
        } else {
            interval
        };
        // Alpaca's data starts in the 2010s; an earlier start only costs it a refusal.
        let start = start_ms.max(EARLIEST_MS);
        let wanted = max * fold_seconds.map_or(1, |seconds| (seconds / 2_592_000) as usize + 1);
        let bars: Vec<Kline> = fetch_bars(symbol, source_interval, start, Some(end_ms), wanted)?
            .into_iter()
            .filter(|bar| bar.open_time < end_ms)
            .collect();
        let shown = match fold_seconds {
            Some(seconds) => fold_calendar(&bars, seconds, bars.len() >= wanted),
            None => bars,
        };
        let skip = shown.len().saturating_sub(max);
        Ok(shown[skip..].to_vec())
    }

    fn search_tickers(&self, query: &str) -> Result<Vec<SearchResult>, String> {
        // Credentials first: with none saved, a cached list must not make search seem available.
        let (key, secret) = credentials()?;
        let assets = CATALOG.get(|| download_assets(&key, &secret))?;

        let needle = query.trim().to_uppercase();
        let results: Vec<SearchResult> = assets
            .iter()
            .filter(|asset| {
                asset.symbol.to_uppercase().contains(&needle)
                    || asset.name.to_uppercase().contains(&needle)
            })
            .map(|asset| {
                SearchResult::new(
                    crate::model::DataSource::Alpaca,
                    format!("{}{}{}", asset.symbol, super::NAME_SEPARATOR, asset.name),
                    asset.symbol.clone(),
                )
            })
            .collect();
        Ok(rank_results(results, query).into_iter().take(30).collect())
    }
}

/// One tradable US equity from `/v2/assets`: all the search needs of it.
#[derive(Clone, Debug, Deserialize, Serialize)]
struct Asset {
    symbol: String,
    name: String,
}

/// Every tradable asset (the response is tens of megabytes), downloaded at most once a day.
static CATALOG: Catalog<Asset> = Catalog::new("catalog_alpaca");

/// Forgets the list held in memory (after the disk cache was cleared).
pub(super) fn clear_catalog() {
    CATALOG.clear_memory();
}

fn download_assets(key: &str, secret: &str) -> Result<Vec<Asset>, String> {
    let url = "https://paper-api.alpaca.markets/v2/assets?status=active&asset_class=us_equity";
    let body = super::http::get_text_with_limit(
        url,
        &[("APCA-API-KEY-ID", key), ("APCA-API-SECRET-KEY", secret)],
        super::http::Retry::Transient,
        32 * 1024 * 1024,
    )
    .map_err(|error| format!("Alpaca error: {error}"))?;
    parse_assets(&body)
}

fn parse_assets(body: &str) -> Result<Vec<Asset>, String> {
    let value: Value =
        serde_json::from_str(body).map_err(|error| format!("Data error: {error}"))?;
    Ok(value
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .filter(|asset| asset.get("tradable").and_then(Value::as_bool) == Some(true))
        .filter_map(|asset| {
            Some(Asset {
                symbol: asset.get("symbol")?.as_str()?.to_owned(),
                name: asset.get("name")?.as_str()?.to_owned(),
            })
        })
        .collect())
}

/// 2000-01-01.
const EARLIEST_MS: i64 = 946_684_800_000;

/// The newest `limit` bars of `interval` that open from `start_ms` to `end_ms` (or now), oldest
/// first.
fn fetch_bars(
    symbol: &str,
    interval: &str,
    start_ms: i64,
    end_ms: Option<i64>,
    limit: usize,
) -> Result<Vec<Kline>, String> {
    let (key, secret) = credentials()?;
    let mut url = format!(
        "{BARS_URL}/{}/bars?timeframe={}&start={}&limit={}&adjustment=all&feed=iex&sort=desc",
        symbol.to_uppercase(),
        timeframe(interval),
        crate::format::iso8601(start_ms),
        limit.clamp(1, 10_000)
    );
    if let Some(end_ms) = end_ms {
        url.push_str(&format!("&end={}", crate::format::iso8601(end_ms)));
    }
    let body = super::http::get_text(
        &url,
        &[("APCA-API-KEY-ID", &key), ("APCA-API-SECRET-KEY", &secret)],
        super::http::Retry::Transient,
    )
    .map_err(|error| format!("Alpaca error: {error}"))?;
    let mut bars = parse_bars(&body)?;
    // The newest page of a market with no bars at all is an error; an older page with none is
    // the start of its history.
    if bars.is_empty() && end_ms.is_none() {
        return Err(format!(
            "No data is available for {} on Alpaca's free IEX feed.",
            symbol.to_uppercase()
        ));
    }
    bars.sort_by_key(|candle| candle.open_time);
    Ok(bars)
}

/// `{"bars": [{"t": ISO, "o", "h", "l", "c", "v", "vw"}]}` newest first.
fn parse_bars(body: &str) -> Result<Vec<Kline>, String> {
    let value: Value =
        serde_json::from_str(body).map_err(|error| format!("Data error: {error}"))?;
    let bars = value
        .get("bars")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let mut out = Vec::with_capacity(bars.len());
    for bar in bars {
        let (Some(t), Some(o), Some(h), Some(l), Some(c)) = (
            bar.get("t").and_then(Value::as_str).and_then(parse_iso8601),
            bar.get("o").and_then(Value::as_f64),
            bar.get("h").and_then(Value::as_f64),
            bar.get("l").and_then(Value::as_f64),
            bar.get("c").and_then(Value::as_f64),
        ) else {
            continue;
        };
        let volume = bar.get("v").and_then(Value::as_f64).unwrap_or(0.0);
        out.push(Kline {
            open_time: t,
            close_time: t,
            open: o,
            high: h,
            low: l,
            close: c,
            volume,
        });
    }
    Ok(out)
}

/// Alpaca stamps ISO-8601 with optional fractional seconds; milliseconds are plenty.
fn parse_iso8601(text: &str) -> Option<i64> {
    let text = text.trim_end_matches('Z');
    let (date, time) = text.split_once('T')?;
    let mut d = date.split('-');
    let year: i64 = d.next()?.parse().ok()?;
    let month: i64 = d.next()?.parse().ok()?;
    let day: i64 = d.next()?.parse().ok()?;
    let mut t = time.split(':');
    let hour: i64 = t.next()?.parse().ok()?;
    let minute: i64 = t.next()?.parse().ok()?;
    let second: i64 = t.next()?.split('.').next()?.parse().ok()?;
    let days = crate::format::days_from_civil(year, month, day);
    Some((days * 86_400 + hour * 3_600 + minute * 60 + second) * 1_000)
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_catalog_keeps_only_tradable_named_assets() {
        let body = r#"[
            {"symbol":"AAPL","name":"Apple Inc. Common Stock","tradable":true},
            {"symbol":"HALT","name":"Halted Corp","tradable":false},
            {"symbol":"NONAME","tradable":true}
        ]"#;
        let assets = parse_assets(body).expect("assets");
        assert_eq!(assets.len(), 1);
        assert_eq!(assets[0].symbol, "AAPL");
        assert_eq!(assets[0].name, "Apple Inc. Common Stock");
        assert!(parse_assets("[").is_err());
    }

    use super::*;

    #[test]
    fn parses_iso_timestamps() {
        assert_eq!(
            parse_iso8601("2026-10-09T00:00:00Z"),
            Some(1_791_504_000_000)
        );
        assert_eq!(
            parse_iso8601("2026-10-09T00:00:00.123456Z"),
            Some(1_791_504_000_000)
        );
    }

    #[test]
    fn folds_monthly_into_quarterly() {
        let jan = crate::format::days_from_civil(2026, 1, 1) * 86_400_000;
        let feb = crate::format::days_from_civil(2026, 2, 1) * 86_400_000;
        let apr = crate::format::days_from_civil(2026, 4, 1) * 86_400_000;
        let bars = vec![
            Kline {
                open_time: jan,
                close_time: jan,
                open: 10.0,
                high: 11.0,
                low: 9.0,
                close: 10.5,
                volume: 1.0,
            },
            Kline {
                open_time: feb,
                close_time: feb,
                open: 10.5,
                high: 12.0,
                low: 10.0,
                close: 11.0,
                volume: 2.0,
            },
            Kline {
                open_time: apr,
                close_time: apr,
                open: 11.0,
                high: 11.5,
                low: 10.8,
                close: 11.2,
                volume: 1.0,
            },
        ];
        let folded = fold_calendar(&bars, 7_776_000, false);
        assert_eq!(folded.len(), 2);
        assert_eq!(folded[0].open_time, jan);
        assert_eq!(folded[0].high, 12.0);
        assert_eq!(folded[0].low, 9.0);
        assert_eq!(folded[1].open_time, apr);
    }
}
