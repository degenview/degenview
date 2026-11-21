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
    let body = get(DataSource::Gate, &format!("{BASE_URL}/tickers"), &[])?;
    parse_gateio(&body, requests)
}

/// One Gate.io spot ticker (`/spot/tickers` and the `spot.tickers` channel carry the same fields):
/// the market and its price, with the price a day ago taken from the 24h change in percent
/// (`change_percentage`), and the quote-currency volume of a dollar-quoted pair, the units traded
/// otherwise.
pub(in crate::service) fn gateio_ticker(ticker: &Value) -> Option<(String, SourceQuote)> {
    let id = ticker.get("currency_pair")?.as_str()?.to_uppercase();
    let price = number(ticker.get("last")).filter(|price| *price > 0.0)?;
    let mut quote = SourceQuote {
        price,
        previous_day_price: number(ticker.get("change_percentage"))
            .filter(|percent| percent.is_finite() && *percent > -100.0)
            .map(|percent| price / (1.0 + percent / 100.0)),
        ..SourceQuote::default()
    };
    let dollar_quoted = DOLLAR_QUOTES.iter().any(|quote| id.ends_with(quote));
    if let (true, Some(turnover)) = (dollar_quoted, number(ticker.get("quote_volume"))) {
        quote.volume_24h = Some(turnover);
        quote.volume_kind = Some(VolumeKind::QuoteCurrency);
    } else if let Some(units) = number(ticker.get("base_volume")) {
        quote.volume_24h = Some(units);
        quote.volume_kind = Some(VolumeKind::Base);
    }
    Some((id, quote))
}

/// `/spot/tickers`: every spot market in one answer; the requested ones are kept.
pub(in crate::service) fn parse_gateio(
    body: &str,
    requests: &[QuoteRequest],
) -> Result<HashMap<String, SourceQuote>, QuoteFailure> {
    let value = json(body)?;
    let tickers: HashMap<String, SourceQuote> = value
        .as_array()
        .ok_or_else(|| QuoteFailure::Failed("Unexpected Gate.io response".to_owned()))?
        .iter()
        .filter_map(gateio_ticker)
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
    fn gate_answers_every_spot_market_with_the_price_a_day_ago_from_the_change() {
        let body = r#"[
            {"currency_pair":"BTC_USDT","last":"65100","change_percentage":"2","base_volume":"14040.69","quote_volume":"908513512.9"},
            {"currency_pair":"ETH_BTC","last":"0.05","change_percentage":"-1.5","base_volume":"2000","quote_volume":"100"},
            {"currency_pair":"DEAD_USDT","last":"0","change_percentage":"0","base_volume":"1","quote_volume":"1"},
            {"currency_pair":"WILD_USDT","last":"1","change_percentage":"-100","base_volume":"1","quote_volume":"1"}
        ]"#;
        let quotes = parse_gateio(
            body,
            &[
                request("BTC_USDT"),
                request("ETH_BTC"),
                request("DEAD_USDT"),
                request("WILD_USDT"),
                request("MISSING_USDT"),
            ],
        )
        .unwrap();
        assert_eq!(
            quotes.len(),
            3,
            "unrequested, unlisted and priceless are left out"
        );
        let btc = &quotes["BTC_USDT"];
        assert_eq!(btc.price, 65_100.0);
        assert!((btc.previous_day_price.unwrap() - 63_823.529).abs() < 0.001);
        assert_eq!(
            (btc.volume_24h, btc.volume_kind),
            (Some(908_513_512.9), Some(VolumeKind::QuoteCurrency))
        );
        // A pair quoted in a coin reports the units traded; a fall of 1.5 % has a higher past.
        let eth = &quotes["ETH_BTC"];
        assert!(eth.previous_day_price.unwrap() > 0.05);
        assert_eq!(
            (eth.volume_24h, eth.volume_kind),
            (Some(2_000.0), Some(VolumeKind::Base))
        );
        // A change of -100 % leaves no price a day ago to divide by.
        assert_eq!(quotes["WILD_USDT"].previous_day_price, None);
    }

    #[test]
    fn gate_odd_answers_and_refusals() {
        assert!(parse_gateio("not json", &[]).is_err());
        assert!(parse_gateio(r#"{"label":"INVALID_CURRENCY"}"#, &[]).is_err());
        assert!(matches!(
            classify_status(DataSource::Gate, 403),
            QuoteFailure::Failed(message) if message.contains("not available in every country")
        ));
        assert_eq!(
            classify_status(DataSource::Gate, 429),
            QuoteFailure::RateLimited
        );
    }
}
