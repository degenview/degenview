//! The REST call behind a watchlist price of this venue, and the parser of what it and the
//! `market_stats` socket channel both say about a market.
//!
//! `/exchangeStats` answers every market in one call (85 KB): `{"code":200,"order_book_stats":[
//! {"symbol":"BTC","last_trade_price":55780.7,"daily_quote_token_volume":1728668420.5,
//! "daily_price_change":-9.9,…}]}`. A market's stats on the socket carry the same fields.

use super::parse::answer;
use super::rest::BASE_URL;
use crate::model::DataSource;
use crate::model::watchlist::quote::{SourceQuote, VolumeKind};
use crate::service::watchlist_quotes::provider::{
    QuoteFailure, QuoteRequest, from_change_percent, get, number,
};
use serde_json::Value;
use std::collections::HashMap;

pub(super) fn fetch_quotes(
    requests: &[QuoteRequest],
) -> Result<HashMap<String, SourceQuote>, QuoteFailure> {
    // No wait for the one-a-second slot of `rest::get`: a round joins every source's request before
    // anything is applied, so a sleeping call would hold back every other source's prices. A round
    // is one call, and a refusal (HTTP 429) backs the source off.
    let body = get(
        DataSource::Lighter,
        &format!("{BASE_URL}/exchangeStats"),
        &[],
    )?;
    parse_stats(&body, requests)
}

/// A market's price and its day: the last trade, the price a day ago from the change in percent
/// over the last 24 hours (`daily_price_change`, checked against the hourly bars) and the dollars
/// traded in that time. `None` for a market with no trade.
pub(super) fn stats_quote(stats: &Value) -> Option<(String, SourceQuote)> {
    let symbol = stats.get("symbol")?.as_str()?.to_owned();
    let price = number(stats.get("last_trade_price")).filter(|price| *price > 0.0)?;
    let mut quote = from_change_percent(price, number(stats.get("daily_price_change")));
    if let Some(turnover) = number(stats.get("daily_quote_token_volume")) {
        quote.volume_24h = Some(turnover);
        quote.volume_kind = Some(VolumeKind::QuoteCurrency);
    }
    Some((symbol, quote))
}

/// The requested markets of an `/exchangeStats` answer, keyed by the symbol they were asked as.
pub(super) fn parse_stats(
    body: &str,
    requests: &[QuoteRequest],
) -> Result<HashMap<String, SourceQuote>, QuoteFailure> {
    let value = answer(body).map_err(|error| QuoteFailure::Failed(error.to_string()))?;
    let stats: HashMap<String, SourceQuote> = value
        .get("order_book_stats")
        .and_then(Value::as_array)
        .ok_or_else(|| QuoteFailure::Failed("Unexpected Lighter response".to_owned()))?
        .iter()
        .filter_map(stats_quote)
        .map(|(symbol, quote)| (symbol.to_uppercase(), quote))
        .collect();
    Ok(requests
        .iter()
        .filter_map(|request| {
            let quote = stats.get(&request.symbol.to_uppercase())?;
            Some((request.symbol.clone(), *quote))
        })
        .collect())
}
