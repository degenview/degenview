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
    // One unknown pair refuses the whole call: the engine bisects for it.
    let pairs: Vec<&str> = requests
        .iter()
        .map(|request| request.symbol.as_str())
        .collect();
    // No wait for Kraken's one-a-second slot here: a round joins every source's request before
    // anything is applied, so a sleeping Kraken call (up to `ISOLATION_BUDGET` of them while a
    // refused pair is isolated) would hold back every other source's prices. A round is one call
    // and Kraken's own refusal backs the source off.
    let body = get(
        DataSource::Kraken,
        &format!("{BASE_URL}/Ticker?pair={}", pairs.join(",")),
        &[],
    )?;
    parse_kraken(&body, requests)
}

/// 24h volume as a dollar turnover where the pair is dollar-quoted, otherwise in base units.
/// Kraken reports the base volume and the 24h vwap, so turnover is their product.
pub(in crate::service) fn kraken_volume(
    pair: &str,
    base_volume: f64,
    vwap: f64,
) -> (f64, VolumeKind) {
    let pair = pair.to_uppercase();
    if vwap > 0.0 && DOLLAR_QUOTES.iter().any(|quote| pair.ends_with(quote)) {
        (base_volume * vwap, VolumeKind::QuoteCurrency)
    } else {
        (base_volume, VolumeKind::Base)
    }
}

/// `/Ticker?pair=BTC/USD,…`: the last trade `c[0]`, the 24h volume `v[1]` and vwap `p[1]`. Its
/// `o` is the UTC day's open, not what the price was a day ago, and Kraken's REST has no rolling
/// 24h open, so no change is reported: the ticker socket's `change` gives it, and the engine keeps
/// that reference for a REST refresh of a market the socket has gone quiet about. A refusal of the
/// call is the provider's own code.
pub(in crate::service) fn parse_kraken(
    body: &str,
    requests: &[QuoteRequest],
) -> Result<HashMap<String, SourceQuote>, QuoteFailure> {
    let value = json(body)?;
    if let Some(code) = value
        .get("error")
        .and_then(Value::as_array)
        .and_then(|errors| errors.first())
        .and_then(Value::as_str)
    {
        let text = code
            .split_once(':')
            .map_or(code, |(_, text)| text)
            .to_owned();
        return Err(if super::is_rate_limit(code) {
            QuoteFailure::RateLimited
        } else if code.starts_with("EQuery:") {
            QuoteFailure::Rejected(text)
        } else {
            QuoteFailure::Failed(format!("Kraken error: {text}"))
        });
    }
    let tickers = value
        .get("result")
        .and_then(Value::as_object)
        .ok_or_else(|| QuoteFailure::Failed("Unexpected Kraken response".to_owned()))?;
    let mut quotes = HashMap::new();
    for request in requests {
        let Some(ticker) = tickers.get(&request.symbol) else {
            continue;
        };
        let pair = |key: &str, index: usize| number(ticker.get(key)?.get(index));
        let Some(price) = pair("c", 0).filter(|price| *price > 0.0) else {
            continue;
        };
        let mut quote = SourceQuote {
            price,
            timestamp_ms: Some(crate::clock::now_ms()),
            ..SourceQuote::default()
        };
        if let (Some(base), Some(vwap)) = (pair("v", 1), pair("p", 1)) {
            let (volume, kind) = kraken_volume(&request.symbol, base, vwap);
            quote.volume_24h = Some(volume);
            quote.volume_kind = Some(kind);
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
    fn kraken_answers_by_the_pairs_asked_for_with_the_open_and_the_24h_volume() {
        let body = r#"{"error":[],"result":{
            "BTC/USD":{"c":["64297.1","0.0003"],"o":"64813.7","v":["1383.5","4526.9"],"p":["64571.4","64719.4"]},
            "ETH/BTC":{"c":["0.05","1"],"o":"0.049","v":["10","2000"],"p":["0.05","0.05"]},
            "DEAD/USD":{"c":["0","0"],"o":"1","v":["1","1"],"p":["1","1"]}
        }}"#;
        let quotes = parse_kraken(
            body,
            &[
                request("BTC/USD"),
                request("ETH/BTC"),
                request("DEAD/USD"),
                request("MISSING/USD"),
            ],
        )
        .unwrap();
        assert_eq!(quotes.len(), 2, "priceless and unlisted pairs are left out");
        let btc = &quotes["BTC/USD"];
        assert_eq!(btc.price, 64_297.1);
        // `o` is the UTC day's open, not what the price was a day ago: no change is reported.
        assert_eq!(btc.previous_day_price, None);
        assert_eq!(
            (btc.volume_24h, btc.volume_kind),
            (Some(4526.9 * 64_719.4), Some(VolumeKind::QuoteCurrency))
        );
        // A pair quoted in a coin reports the units traded.
        let eth = &quotes["ETH/BTC"];
        assert_eq!(
            (eth.volume_24h, eth.volume_kind),
            (Some(2_000.0), Some(VolumeKind::Base))
        );
    }

    #[test]
    fn kraken_refusals_are_told_apart_by_their_code() {
        let refused =
            |code: &str| parse_kraken(&format!(r#"{{"error":["{code}"]}}"#), &[]).unwrap_err();
        assert_eq!(
            refused("EQuery:Unknown asset pair"),
            QuoteFailure::Rejected("Unknown asset pair".to_owned())
        );
        assert_eq!(
            refused("EAPI:Rate limit exceeded"),
            QuoteFailure::RateLimited
        );
        assert_eq!(
            refused("EGeneral:Too many requests"),
            QuoteFailure::RateLimited
        );
        assert_eq!(
            refused("EService:Unavailable"),
            QuoteFailure::Failed("Kraken error: Unavailable".to_owned())
        );
        assert!(parse_kraken("not json", &[]).is_err());
        assert!(parse_kraken(r#"{"error":[]}"#, &[]).is_err());
    }
}
