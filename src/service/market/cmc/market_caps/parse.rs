//! The global-metrics history as CoinMarketCap's website answers it. Pure: no request is made here.

use crate::model::market::cmc::global::GlobalPoint;
use crate::service::ProviderError;
use crate::service::net::http;
use serde_json::Value;

/// The readings of a response, oldest first. A reading without a time or the figures an index is
/// made of is left out.
pub(super) fn readings(body: &str) -> Result<Vec<GlobalPoint>, ProviderError> {
    let value: Value = serde_json::from_str(body).map_err(ProviderError::data)?;
    check_status(&value)?;
    let rows = value
        .pointer("/data/quotes")
        .and_then(Value::as_array)
        .ok_or_else(|| ProviderError::data("expected a quotes array"))?;
    let mut points: Vec<GlobalPoint> = rows.iter().filter_map(reading).collect();
    points.sort_by_key(|point| point.time_ms);
    points.dedup_by_key(|point| point.time_ms);
    Ok(points)
}

fn reading(row: &Value) -> Option<GlobalPoint> {
    let quote = row.pointer("/quote/0")?;
    Some(GlobalPoint {
        time_ms: crate::format::parse_iso8601(row.get("timestamp")?.as_str()?)?,
        total_cap: finite(quote.get("totalMarketCap")?)?,
        altcoin_cap: finite(quote.get("altcoinMarketCap")?)?,
        btc_dominance: finite(row.get("btcDominance")?)?,
        // Ether did not exist when the history starts.
        eth_dominance: row.get("ethDominance").and_then(finite).unwrap_or(0.0),
    })
}

fn finite(value: &Value) -> Option<f64> {
    value.as_f64().filter(|number| number.is_finite())
}

/// The website's API answers HTTP 200 with its own status: `error_code` is a string, `"0"` for
/// success. A code in the range of HTTP errors is that error (`"500"` is "the system is busy" and
/// also a request it refuses, `"429"` a rate limit); any other code is the API's own message.
fn check_status(value: &Value) -> Result<(), ProviderError> {
    let Some(status) = value.get("status") else {
        return Ok(());
    };
    let code = match status.get("error_code") {
        None | Some(Value::Null) => return Ok(()),
        Some(Value::String(text)) => text.trim().parse::<u16>().ok(),
        Some(number) => number.as_u64().and_then(|code| u16::try_from(code).ok()),
    };
    match code {
        Some(0) => Ok(()),
        Some(code @ 400..=599) => Err(ProviderError::CoinMarketCap(http::Error::Status(code))),
        Some(code) => Err(ProviderError::other(
            status
                .get("error_message")
                .and_then(Value::as_str)
                .map_or_else(|| format!("CoinMarketCap error {code}"), str::to_owned),
        )),
        None => Err(ProviderError::data("unreadable status code")),
    }
}
