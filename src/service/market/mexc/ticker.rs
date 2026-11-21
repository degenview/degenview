//! The REST call behind a watchlist price of this venue, and the parser of its answer.

use super::BASE_URL;
use crate::model::DataSource;
use crate::model::watchlist::quote::{SourceQuote, VolumeKind};
use crate::service::watchlist_quotes::provider::{
    DOLLAR_QUOTES, QuoteFailure, QuoteRequest, get, json, number,
};
use std::collections::HashMap;

pub(super) fn fetch_quotes(
    requests: &[QuoteRequest],
) -> Result<HashMap<String, SourceQuote>, QuoteFailure> {
    let body = get(DataSource::Mexc, &format!("{BASE_URL}/ticker/24hr"), &[])?;
    parse_mexc(&body, requests)
}

/// `/ticker/24hr`: every spot market in one answer (its `symbols` parameter is ignored); the
/// requested ones are kept. `openPrice` is the price 24 hours ago, so the change is a real rolling
/// one, and the quote-currency volume of a dollar-quoted pair is `quoteVolume`.
pub(in crate::service) fn parse_mexc(
    body: &str,
    requests: &[QuoteRequest],
) -> Result<HashMap<String, SourceQuote>, QuoteFailure> {
    let value = json(body)?;
    let tickers: HashMap<String, SourceQuote> = value
        .as_array()
        .ok_or_else(|| QuoteFailure::Failed("Unexpected MEXC response".to_owned()))?
        .iter()
        .filter_map(|ticker| {
            let id = ticker.get("symbol")?.as_str()?.to_uppercase();
            let price = number(ticker.get("lastPrice")).filter(|price| *price > 0.0)?;
            let mut quote = SourceQuote {
                price,
                previous_day_price: number(ticker.get("openPrice")).filter(|open| *open > 0.0),
                ..SourceQuote::default()
            };
            let dollar_quoted = DOLLAR_QUOTES.iter().any(|quote| id.ends_with(quote));
            if let (true, Some(turnover)) = (dollar_quoted, number(ticker.get("quoteVolume"))) {
                quote.volume_24h = Some(turnover);
                quote.volume_kind = Some(VolumeKind::QuoteCurrency);
            } else if let Some(units) = number(ticker.get("volume")) {
                quote.volume_24h = Some(units);
                quote.volume_kind = Some(VolumeKind::Base);
            }
            Some((id, quote))
        })
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
    fn mexc_answers_every_spot_market_with_the_price_24_hours_ago() {
        let body = r#"[
            {"symbol":"BTCUSDT","lastPrice":"64990.71","openPrice":"64185.22","volume":"5486.66","quoteVolume":"354947733.48"},
            {"symbol":"ETHBTC","lastPrice":"0.05","openPrice":"0.049","volume":"2000","quoteVolume":"100"},
            {"symbol":"DEADUSDT","lastPrice":"0","openPrice":"1","volume":"1","quoteVolume":"1"},
            {"symbol":"NEWUSDT","lastPrice":"2","openPrice":"0","volume":"1","quoteVolume":"2"}
        ]"#;
        let quotes = parse_mexc(
            body,
            &[
                request("BTCUSDT"),
                request("ETHBTC"),
                request("DEADUSDT"),
                request("NEWUSDT"),
                request("MISSINGUSDT"),
            ],
        )
        .unwrap();
        assert_eq!(
            quotes.len(),
            3,
            "unrequested, unlisted and priceless are left out"
        );
        let btc = &quotes["BTCUSDT"];
        assert_eq!(btc.price, 64_990.71);
        assert_eq!(btc.previous_day_price, Some(64_185.22));
        assert_eq!(
            (btc.volume_24h, btc.volume_kind),
            (Some(354_947_733.48), Some(VolumeKind::QuoteCurrency))
        );
        // A pair quoted in a coin reports the units traded.
        let eth = &quotes["ETHBTC"];
        assert_eq!(
            (eth.volume_24h, eth.volume_kind),
            (Some(2_000.0), Some(VolumeKind::Base))
        );
        // A market with no open price has no reference rather than a division by zero.
        assert_eq!(quotes["NEWUSDT"].previous_day_price, None);
    }

    #[test]
    fn mexc_odd_answers_and_refusals() {
        assert!(parse_mexc("not json", &[]).is_err());
        assert!(parse_mexc(r#"{"code":-1121,"msg":"Invalid symbol."}"#, &[]).is_err());
        assert!(matches!(
            classify_status(DataSource::Mexc, 403),
            QuoteFailure::Failed(message) if message.contains("not available in every country")
        ));
        assert_eq!(
            classify_status(DataSource::Mexc, 429),
            QuoteFailure::RateLimited
        );
    }
}
