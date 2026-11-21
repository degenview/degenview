//! The REST call behind a watchlist price of this venue, and the parser of its answer.

use crate::model::DataSource;
use crate::model::watchlist::quote::{SourceQuote, VolumeKind};
use crate::service::watchlist_quotes::provider::{
    DOLLAR_QUOTES, QuoteFailure, QuoteRequest, get, json, number,
};
use serde_json::Value;
use std::collections::HashMap;

/// The batch ticker URL. `symbols` is a JSON array, so its brackets and quotes are
/// percent-encoded: sent raw they are not valid in a URL, the HTTP client refuses the request,
/// and every Binance quote in every watchlist stays empty.
pub(in crate::service) fn binance_url(requests: &[QuoteRequest]) -> String {
    let list = requests
        .iter()
        .map(|request| format!("%22{}%22", request.symbol.to_uppercase()))
        .collect::<Vec<_>>()
        .join("%2C");
    format!("https://api.binance.com/api/v3/ticker/24hr?symbols=%5B{list}%5D&type=MINI")
}

pub(super) fn fetch_quotes(
    requests: &[QuoteRequest],
) -> Result<HashMap<String, SourceQuote>, QuoteFailure> {
    let body = get(DataSource::Binance, &binance_url(requests), &[])?;
    parse_binance(&body, requests)
}

/// `/ticker/24hr?type=MINI`: `openPrice` is the rolling 24h open, a truer day-change
/// reference than a candle picked from the last 25 hours.
pub(in crate::service) fn parse_binance(
    body: &str,
    requests: &[QuoteRequest],
) -> Result<HashMap<String, SourceQuote>, QuoteFailure> {
    let value = json(body)?;
    let tickers: HashMap<String, &Value> = value
        .as_array()
        .ok_or_else(|| QuoteFailure::Failed("Unexpected Binance response".to_owned()))?
        .iter()
        .filter_map(|ticker| Some((ticker.get("symbol")?.as_str()?.to_uppercase(), ticker)))
        .collect();
    let mut quotes = HashMap::new();
    for request in requests {
        let symbol = request.symbol.to_uppercase();
        let Some(ticker) = tickers.get(&symbol) else {
            continue;
        };
        let Some(price) = number(ticker.get("lastPrice")).filter(|price| *price > 0.0) else {
            continue;
        };
        let mut quote = SourceQuote {
            price,
            previous_day_price: number(ticker.get("openPrice")),
            timestamp_ms: ticker.get("closeTime").and_then(Value::as_i64),
            ..SourceQuote::default()
        };
        // Turnover is only a dollar figure on a dollar-quoted pair; otherwise report units traded.
        let dollar_quoted = DOLLAR_QUOTES.iter().any(|quote| symbol.ends_with(quote));
        if let (true, Some(turnover)) = (dollar_quoted, number(ticker.get("quoteVolume"))) {
            quote.volume_24h = Some(turnover);
            quote.volume_kind = Some(VolumeKind::QuoteCurrency);
        } else if let Some(units) = number(ticker.get("volume")) {
            quote.volume_24h = Some(units);
            quote.volume_kind = Some(VolumeKind::Base);
        }
        quotes.insert(request.symbol.clone(), quote);
    }
    Ok(quotes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::watchlist_quotes::provider::request;

    #[test]
    fn binance_mini_ticker_with_dollar_and_base_volume() {
        let body = r#"[
            {"symbol":"BTCUSDT","lastPrice":"67000.5","openPrice":"66000","volume":"100","quoteVolume":"6700000","closeTime":1700000000000},
            {"symbol":"ETHBTC","lastPrice":"0.05","openPrice":"0.049","volume":"2000","quoteVolume":"100","closeTime":1700000000001}
        ]"#;
        let quotes = parse_binance(
            body,
            &[request("BTCUSDT"), request("ETHBTC"), request("GONEUSDT")],
        )
        .unwrap();
        assert_eq!(quotes.len(), 2, "an unanswered symbol is omitted");
        let btc = &quotes["BTCUSDT"];
        assert_eq!(
            (btc.price, btc.previous_day_price),
            (67_000.5, Some(66_000.0))
        );
        assert_eq!(
            (btc.volume_24h, btc.volume_kind),
            (Some(6_700_000.0), Some(VolumeKind::QuoteCurrency))
        );
        assert_eq!(btc.timestamp_ms, Some(1_700_000_000_000));
        let eth = &quotes["ETHBTC"];
        assert_eq!(
            (eth.volume_24h, eth.volume_kind),
            (Some(2_000.0), Some(VolumeKind::Base)),
            "no dollar figure on a non-dollar pair"
        );
    }

    #[test]
    fn the_binance_url_encodes_its_symbol_list() {
        let url = binance_url(&[request("btcusdt"), request("ETHUSDT")]);
        assert_eq!(
            url,
            "https://api.binance.com/api/v3/ticker/24hr?symbols=%5B%22BTCUSDT%22%2C%22ETHUSDT%22%5D&type=MINI"
        );
        assert!(!url.contains(['[', ']', '"']), "only URL-safe characters");
    }

    #[test]
    fn binance_rejects_non_arrays_and_zero_prices() {
        assert!(matches!(
            parse_binance(r#"{"code":-1121}"#, &[request("X")]),
            Err(QuoteFailure::Failed(_))
        ));
        let zero = r#"[{"symbol":"AUSDT","lastPrice":"0","openPrice":"1"}]"#;
        assert!(parse_binance(zero, &[request("AUSDT")]).unwrap().is_empty());
    }
}
