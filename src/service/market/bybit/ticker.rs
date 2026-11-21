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
        DataSource::Bybit,
        &format!("{BASE_URL}/tickers?category=spot"),
        &[],
    )?;
    parse_bybit(&body, requests)
}

/// One Bybit spot ticker (`/market/tickers` and the `tickers` topic carry the same fields): the
/// market and its price, with the change measured against the price 24 hours ago
/// (`prevPrice24h`) and the turnover (`turnover24h`, in the quote currency) as the volume of a
/// dollar-quoted pair, the units traded (`volume24h`) otherwise.
pub(in crate::service) fn bybit_ticker(ticker: &Value) -> Option<(String, SourceQuote)> {
    let id = ticker.get("symbol")?.as_str()?.to_uppercase();
    let price = number(ticker.get("lastPrice")).filter(|price| *price > 0.0)?;
    let mut quote = SourceQuote {
        price,
        previous_day_price: number(ticker.get("prevPrice24h")).filter(|open| *open > 0.0),
        ..SourceQuote::default()
    };
    let dollar_quoted = DOLLAR_QUOTES.iter().any(|quote| id.ends_with(quote));
    if let (true, Some(turnover)) = (dollar_quoted, number(ticker.get("turnover24h"))) {
        quote.volume_24h = Some(turnover);
        quote.volume_kind = Some(VolumeKind::QuoteCurrency);
    } else if let Some(units) = number(ticker.get("volume24h")) {
        quote.volume_24h = Some(units);
        quote.volume_kind = Some(VolumeKind::Base);
    }
    Some((id, quote))
}

/// `/market/tickers?category=spot`: every spot market in one answer; the requested ones are kept.
pub(in crate::service) fn parse_bybit(
    body: &str,
    requests: &[QuoteRequest],
) -> Result<HashMap<String, SourceQuote>, QuoteFailure> {
    let value = json(body)?;
    let code = value
        .get("retCode")
        .and_then(Value::as_i64)
        .ok_or_else(|| QuoteFailure::Failed("Unexpected Bybit response".to_owned()))?;
    if code != 0 {
        return Err(if super::is_rate_limit(code) {
            QuoteFailure::RateLimited
        } else {
            let message = value
                .get("retMsg")
                .and_then(Value::as_str)
                .unwrap_or_default();
            QuoteFailure::Failed(format!("Bybit error: {message}"))
        });
    }
    let tickers: HashMap<String, SourceQuote> = value
        .pointer("/result/list")
        .and_then(Value::as_array)
        .ok_or_else(|| QuoteFailure::Failed("Unexpected Bybit response".to_owned()))?
        .iter()
        .filter_map(bybit_ticker)
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
    use crate::service::watchlist_quotes::provider::classify_status;
    use crate::service::watchlist_quotes::provider::request;

    #[test]
    fn bybit_answers_every_spot_market_with_the_real_24h_reference() {
        let body = r#"{"retCode":0,"retMsg":"OK","result":{"category":"spot","list":[
            {"symbol":"BTCUSDT","lastPrice":"64298.1","prevPrice24h":"64675.6","volume24h":"12398.7","turnover24h":"801728667.2"},
            {"symbol":"ETHBTC","lastPrice":"0.05","prevPrice24h":"0.049","volume24h":"2000","turnover24h":"100"},
            {"symbol":"DEADUSDT","lastPrice":"0","prevPrice24h":"1","volume24h":"1","turnover24h":"1"}
        ]},"time":1}"#;
        let quotes = parse_bybit(
            body,
            &[
                request("BTCUSDT"),
                request("ETHBTC"),
                request("DEADUSDT"),
                request("MISSINGUSDT"),
            ],
        )
        .unwrap();
        assert_eq!(
            quotes.len(),
            2,
            "unrequested, unlisted and priceless are left out"
        );
        let btc = &quotes["BTCUSDT"];
        assert_eq!(btc.price, 64_298.1);
        assert_eq!(btc.previous_day_price, Some(64_675.6));
        assert_eq!(
            (btc.volume_24h, btc.volume_kind),
            (Some(801_728_667.2), Some(VolumeKind::QuoteCurrency))
        );
        // A pair quoted in a coin reports the units traded.
        let eth = &quotes["ETHBTC"];
        assert_eq!(
            (eth.volume_24h, eth.volume_kind),
            (Some(2_000.0), Some(VolumeKind::Base))
        );
    }

    #[test]
    fn bybit_reports_its_own_refusals() {
        let refused = |code: i64| {
            parse_bybit(&format!(r#"{{"retCode":{code},"retMsg":"No"}}"#), &[]).unwrap_err()
        };
        assert_eq!(refused(10_006), QuoteFailure::RateLimited);
        assert_eq!(refused(10_018), QuoteFailure::RateLimited);
        assert_eq!(
            refused(10_001),
            QuoteFailure::Failed("Bybit error: No".to_owned())
        );
        assert!(parse_bybit("not json", &[]).is_err());
        assert!(parse_bybit(r#"{"retCode":0}"#, &[]).is_err());
        assert!(matches!(
            classify_status(DataSource::Bybit, 403),
            QuoteFailure::Failed(message) if message.contains("not available in every country")
        ));
        assert_eq!(
            classify_status(DataSource::Bybit, 429),
            QuoteFailure::RateLimited
        );
    }
}
