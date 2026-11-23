//! The REST call behind a watchlist price of this venue, and the parser of its answer.

use crate::model::DataSource;
use crate::model::watchlist::quote::{SourceQuote, VolumeKind};
use crate::service::watchlist_quotes::provider::{
    QuoteFailure, QuoteRequest, from_change_percent, get, json, number,
};
use serde_json::Value;
use std::collections::HashMap;

pub(super) fn fetch_quotes(
    requests: &[QuoteRequest],
) -> Result<HashMap<String, SourceQuote>, QuoteFailure> {
    let ids = requests
        .iter()
        .map(|request| request.symbol.to_lowercase())
        .collect::<Vec<_>>()
        .join(",");
    // One `/coins/markets` call prices every coin id, costing the limiter a single slot.
    crate::service::net::rate_limit::coingecko().wait_for_slot();
    let url = format!(
        "https://api.coingecko.com/api/v3/coins/markets?vs_currency=usd&ids={ids}\
         &sparkline=false&price_change_percentage=24h&per_page={}",
        requests.len().min(250)
    );
    parse_coingecko_markets(&get(DataSource::CoinGecko, &url, &[])?, requests)
}

pub(in crate::service) fn parse_coingecko_markets(
    body: &str,
    requests: &[QuoteRequest],
) -> Result<HashMap<String, SourceQuote>, QuoteFailure> {
    let value = json(body)?;
    let coins: HashMap<String, &Value> = value
        .as_array()
        .ok_or_else(|| QuoteFailure::Failed("Unexpected CoinGecko response".to_owned()))?
        .iter()
        .filter_map(|coin| Some((coin.get("id")?.as_str()?.to_lowercase(), coin)))
        .collect();
    let mut quotes = HashMap::new();
    for request in requests {
        let Some(coin) = coins.get(&request.symbol.to_lowercase()) else {
            continue;
        };
        let Some(price) = number(coin.get("current_price")).filter(|price| *price > 0.0) else {
            continue;
        };
        let mut quote = from_change_percent(price, number(coin.get("price_change_percentage_24h")));
        quote.volume_24h = number(coin.get("total_volume"));
        quote.volume_kind = quote.volume_24h.map(|_| VolumeKind::QuoteCurrency);
        quote.timestamp_ms = coin
            .get("last_updated")
            .and_then(Value::as_str)
            .and_then(crate::format::parse_iso8601);
        quotes.insert(request.symbol.clone(), quote);
    }
    Ok(quotes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::watchlist_quotes::provider::request;

    #[test]
    fn coingecko_derives_the_previous_price_from_the_percent_change() {
        let body = r#"[
            {"id":"bitcoin","current_price":110,"price_change_percentage_24h":10,"total_volume":5000,"last_updated":"2024-01-15T15:00:00.123Z"},
            {"id":"dead","current_price":null}
        ]"#;
        let quotes = parse_coingecko_markets(
            body,
            &[request("Bitcoin"), request("dead"), request("absent")],
        )
        .unwrap();
        assert_eq!(quotes.len(), 1);
        let btc = &quotes["Bitcoin"];
        assert!((btc.previous_day_price.unwrap() - 100.0).abs() < 1e-9);
        assert_eq!(btc.volume_kind, Some(VolumeKind::QuoteCurrency));
        assert_eq!(btc.timestamp_ms, Some(1_705_330_800_123));
    }
}
