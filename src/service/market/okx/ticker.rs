//! The REST call behind a watchlist price of this venue, and the parser of its answer.

use super::BASE_URL;
use crate::model::DataSource;
use crate::model::watchlist::quote::{SourceQuote, VolumeKind};
use crate::service::watchlist_quotes::provider::{
    DOLLAR_QUOTES, QuoteFailure, QuoteRequest, get, json, number,
};
use serde_json::Value;
use std::collections::HashMap;

pub(super) fn fetch_quotes(
    requests: &[QuoteRequest],
) -> Result<HashMap<String, SourceQuote>, QuoteFailure> {
    let body = get(
        DataSource::Okx,
        &format!("{BASE_URL}/market/tickers?instType=SPOT"),
        &[],
    )?;
    parse_okx(&body, requests)
}

/// One OKX spot ticker (`/market/tickers` and the `tickers` channel carry the same fields): the
/// market's id and its price, with the change measured against the rolling 24h open.
pub(in crate::service) fn okx_ticker(ticker: &Value) -> Option<(String, SourceQuote)> {
    let id = ticker.get("instId")?.as_str()?.to_uppercase();
    let price = number(ticker.get("last")).filter(|price| *price > 0.0)?;
    let mut quote = SourceQuote {
        price,
        previous_day_price: number(ticker.get("open24h")).filter(|open| *open > 0.0),
        timestamp_ms: number(ticker.get("ts")).map(|ms| ms as i64),
        ..SourceQuote::default()
    };
    // Turnover is only a dollar figure on a dollar-quoted pair; otherwise report units traded.
    let dollar_quoted = DOLLAR_QUOTES.iter().any(|quote| id.ends_with(quote));
    if let (true, Some(turnover)) = (dollar_quoted, number(ticker.get("volCcy24h"))) {
        quote.volume_24h = Some(turnover);
        quote.volume_kind = Some(VolumeKind::QuoteCurrency);
    } else if let Some(units) = number(ticker.get("vol24h")) {
        quote.volume_24h = Some(units);
        quote.volume_kind = Some(VolumeKind::Base);
    }
    Some((id, quote))
}

/// `/market/tickers?instType=SPOT`: every spot market in one answer; the requested ones are kept.
pub(in crate::service) fn parse_okx(
    body: &str,
    requests: &[QuoteRequest],
) -> Result<HashMap<String, SourceQuote>, QuoteFailure> {
    let value = json(body)?;
    if value.get("code").and_then(Value::as_str) != Some("0") {
        let message = value.get("msg").and_then(Value::as_str).unwrap_or_default();
        return Err(QuoteFailure::Failed(format!("OKX error: {message}")));
    }
    let tickers: HashMap<String, SourceQuote> = value
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| QuoteFailure::Failed("Unexpected OKX response".to_owned()))?
        .iter()
        .filter_map(okx_ticker)
        .collect();
    Ok(requests
        .iter()
        .filter_map(|request| {
            let quote = tickers.get(&request.symbol.to_uppercase())?;
            Some((request.symbol.clone(), *quote))
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::watchlist_quotes::provider::request;

    #[test]
    fn okx_answers_every_spot_market_and_the_requested_ones_are_kept() {
        let body = r#"{"code":"0","data":[
            {"instId":"BTC-USDT","last":"64370.7","open24h":"64561.6","vol24h":"10021.2","volCcy24h":"648492231.3","ts":"1794904542764"},
            {"instId":"ETH-BTC","last":"0.05","open24h":"0.049","vol24h":"2000","volCcy24h":"100","ts":"1794904542765"},
            {"instId":"DEAD-USDT","last":"0","open24h":"1","vol24h":"1","volCcy24h":"1","ts":"1"},
            {"instId":"SOL-USDT","last":"150","open24h":"","vol24h":"5","volCcy24h":"750","ts":"1794904542766"}
        ]}"#;
        let quotes = parse_okx(
            body,
            &[
                request("BTC-USDT"),
                request("ETH-BTC"),
                request("DEAD-USDT"),
                request("MISSING-USDT"),
            ],
        )
        .unwrap();
        assert_eq!(
            quotes.len(),
            2,
            "unrequested, unlisted and priceless are left out"
        );
        let btc = &quotes["BTC-USDT"];
        assert_eq!(btc.price, 64_370.7);
        assert_eq!(btc.previous_day_price, Some(64_561.6));
        assert_eq!(btc.timestamp_ms, Some(1_794_904_542_764));
        assert_eq!(
            (btc.volume_24h, btc.volume_kind),
            (Some(648_492_231.3), Some(VolumeKind::QuoteCurrency))
        );
        // A pair quoted in BTC reports the units traded, not a dollar figure.
        let eth = &quotes["ETH-BTC"];
        assert_eq!(
            (eth.volume_24h, eth.volume_kind),
            (Some(2_000.0), Some(VolumeKind::Base))
        );
        // An empty open is no reference.
        let (_, sol) = okx_ticker(
            &serde_json::json!({"instId":"SOL-USDT","last":"150","open24h":"","vol24h":"5","volCcy24h":"750"}),
        )
        .unwrap();
        assert_eq!(sol.previous_day_price, None);
    }

    #[test]
    fn okx_reports_its_own_error_and_odd_answers() {
        let failure = parse_okx(
            r#"{"code":"50011","msg":"Too many requests","data":[]}"#,
            &[],
        );
        assert_eq!(
            failure.unwrap_err(),
            QuoteFailure::Failed("OKX error: Too many requests".to_owned())
        );
        assert!(parse_okx("not json", &[]).is_err());
        assert!(parse_okx(r#"{"code":"0"}"#, &[]).is_err());
    }
}
