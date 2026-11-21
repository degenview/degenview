//! The REST call behind a watchlist price of this venue, and the parser of its answer.

use super::BASE_URL;
use crate::model::DataSource;
use crate::model::watchlist::quote::{SourceQuote, VolumeKind};
use crate::service::watchlist_quotes::Outcome;
use crate::service::watchlist_quotes::provider::{
    CONSECUTIVE_FAILURES, QuoteFailure, QuoteRequest, get, number, symbols_of,
};
use serde_json::Value;
use std::time::Duration;

/// 24h volume as a dollar turnover where the product is dollar-quoted, otherwise in base
/// units. Coinbase reports base volume only, so turnover is that volume priced at the last trade.
pub(in crate::service) fn coinbase_volume(
    product_id: &str,
    base_volume: f64,
    price: f64,
) -> (f64, VolumeKind) {
    let quote = product_id.rsplit('-').next().unwrap_or("").to_uppercase();
    if ["USD", "USDC", "USDT"].contains(&quote.as_str()) {
        (base_volume * price, VolumeKind::QuoteCurrency)
    } else {
        (base_volume, VolumeKind::Base)
    }
}

pub(super) fn fetch_quotes(requests: &[QuoteRequest]) -> Outcome {
    // No multi-product price call; `/stats` is one small request per product, paced under
    // the exchange's public limit. A product that fails is reported, and the rest still answer.
    let mut outcome = Outcome::default();
    let mut streak = 0;
    for (index, request) in requests.iter().enumerate() {
        if index > 0 {
            std::thread::sleep(Duration::from_millis(125));
        }
        let id = super::product_id(&request.symbol);
        match get(
            DataSource::Coinbase,
            &format!("{BASE_URL}/products/{id}/stats"),
            &[],
        ) {
            Ok(body) => {
                streak = 0;
                if let Some(quote) = parse_coinbase_stats(&body, &id) {
                    outcome.quotes.insert(request.symbol.clone(), quote);
                }
            }
            Err(QuoteFailure::Rejected(_)) => {
                streak = 0;
                outcome.rejected.insert(request.symbol.clone());
            }
            Err(failure) => {
                let stop = matches!(failure, QuoteFailure::RateLimited) || {
                    streak += 1;
                    streak >= CONSECUTIVE_FAILURES
                };
                let upto = if stop { requests.len() } else { index + 1 };
                outcome
                    .failed
                    .push((symbols_of(&requests[index..upto]), failure));
                if stop {
                    break;
                }
            }
        }
    }
    outcome
}

/// `/products/{id}/stats`: `{"open","high","low","last","volume"}` as strings.
pub(in crate::service) fn parse_coinbase_stats(
    body: &str,
    product_id: &str,
) -> Option<SourceQuote> {
    let value: Value = serde_json::from_str(body).ok()?;
    let price = number(value.get("last")).filter(|price| *price > 0.0)?;
    let mut quote = SourceQuote {
        price,
        previous_day_price: number(value.get("open")),
        timestamp_ms: Some(crate::clock::now_ms()),
        ..SourceQuote::default()
    };
    if let Some(base) = number(value.get("volume")) {
        let (volume, kind) = coinbase_volume(product_id, base, price);
        quote.volume_24h = Some(volume);
        quote.volume_kind = Some(kind);
    }
    Some(quote)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coinbase_stats_price_volume_in_dollars_only_for_dollar_pairs() {
        let body = r#"{"open":"100","high":"120","low":"90","last":"110","volume":"50"}"#;
        let usd = parse_coinbase_stats(body, "BTC-USD").unwrap();
        assert_eq!((usd.price, usd.previous_day_price), (110.0, Some(100.0)));
        assert_eq!(
            (usd.volume_24h, usd.volume_kind),
            (Some(5_500.0), Some(VolumeKind::QuoteCurrency))
        );
        let btc = parse_coinbase_stats(body, "ETH-BTC").unwrap();
        assert_eq!(
            (btc.volume_24h, btc.volume_kind),
            (Some(50.0), Some(VolumeKind::Base))
        );
        assert!(parse_coinbase_stats(r#"{"last":"0"}"#, "BTC-USD").is_none());
        assert!(parse_coinbase_stats("nope", "BTC-USD").is_none());
    }
}
