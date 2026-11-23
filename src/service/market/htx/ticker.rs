//! The REST call behind a watchlist price of this venue, and the parser of what it and the `ticker`
//! socket topic both say about a market's price and volume.
//!
//! `/market/tickers` answers every market in one call (124 KB, about 620 rows):
//! `{"symbol":"btcusdt","open":59481.46,"close":55946.5,"amount":7265.5,"vol":4.3e8,…}`. Its `open`
//! is where the day that began at 16:00 UTC (UTC+8 midnight) opened, the same as HTX's own `1day`
//! bar, not what the price was 24 hours ago (measured on 2026-11-21: 59,481.46 against the 61,895.78
//! that `/market/detail` and the socket's `ticker` give for the rolling day), so REST reports no
//! change: the socket's `open` gives it, and the engine keeps that reference for a REST refresh of
//! a market the socket has gone quiet about. `amount` is the base asset traded in the 24 hours and
//! `vol` the quote asset.

use super::parse::checked;
use super::rest::BASE_URL;
use super::symbol_id;
use crate::model::DataSource;
use crate::model::watchlist::quote::{SourceQuote, VolumeKind};
use crate::service::watchlist_quotes::provider::{
    DOLLAR_QUOTES, QuoteFailure, QuoteRequest, get, number,
};
use serde_json::Value;
use std::collections::HashMap;

pub(super) fn fetch_quotes(
    requests: &[QuoteRequest],
) -> Result<HashMap<String, SourceQuote>, QuoteFailure> {
    let body = get(DataSource::Htx, &format!("{BASE_URL}/market/tickers"), &[])?;
    parse_tickers(&body, requests)
}

/// A market's price and volume from a ticker: the last trade (`lastPrice` where the topic has it,
/// else `close`) and the turnover (`vol`, in the quote currency) as the volume of a dollar-quoted
/// pair, the units traded (`amount`) otherwise. `id` is the market's upper-case name. `None` for a
/// market with no price.
pub(super) fn price_and_volume(id: &str, ticker: &Value) -> Option<SourceQuote> {
    let price = number(ticker.get("lastPrice"))
        .or_else(|| number(ticker.get("close")))
        .filter(|price| *price > 0.0)?;
    let mut quote = SourceQuote {
        price,
        ..SourceQuote::default()
    };
    let dollar_quoted = DOLLAR_QUOTES.iter().any(|quote| id.ends_with(quote));
    if let (true, Some(turnover)) = (dollar_quoted, number(ticker.get("vol"))) {
        quote.volume_24h = Some(turnover);
        quote.volume_kind = Some(VolumeKind::QuoteCurrency);
    } else if let Some(units) = number(ticker.get("amount")) {
        quote.volume_24h = Some(units);
        quote.volume_kind = Some(VolumeKind::Base);
    }
    Some(quote)
}

/// The answer of `/market/tickers`: every market in it that has a price; the requested ones are
/// kept. A refusal of the call is the provider's own.
pub(super) fn parse_tickers(
    body: &str,
    requests: &[QuoteRequest],
) -> Result<HashMap<String, SourceQuote>, QuoteFailure> {
    let answer = checked(body).map_err(|error| {
        if error.is_rate_limited() {
            QuoteFailure::RateLimited
        } else {
            QuoteFailure::Failed(error.to_string())
        }
    })?;
    let tickers: HashMap<String, SourceQuote> = answer
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| QuoteFailure::Failed("Unexpected HTX response".to_owned()))?
        .iter()
        .filter_map(|row| {
            let id = row.get("symbol")?.as_str()?.to_uppercase();
            let quote = price_and_volume(&id, row)?;
            Some((id, quote))
        })
        .collect();
    Ok(requests
        .iter()
        .filter_map(|request| {
            let quote = tickers.get(&symbol_id(&request.symbol)?.to_uppercase())?;
            Some((request.symbol.clone(), *quote))
        })
        .collect())
}
