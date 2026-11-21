//! The REST call behind a watchlist price of this venue, and the parser of what it and the `ticker`
//! channel both say about a market.
//!
//! `/v2/tickers?symbols=tBTCUSD,tETHUSD` answers a row per listed market (an unknown one is left
//! out): `[SYMBOL, BID, BID_SIZE, ASK, ASK_SIZE, DAILY_CHANGE, DAILY_CHANGE_RELATIVE, LAST_PRICE,
//! VOLUME, HIGH, LOW, …]`. The channel's push is the same row without the symbol. DAILY_CHANGE is
//! the price change over the last 24 hours (a rolling window: its implied open differs from the
//! 1D candle's, measured on 2026-11-21) and VOLUME the base asset traded in them.

use super::parse::refusal;
use super::{BASE_URL, symbol};
use crate::model::DataSource;
use crate::model::watchlist::quote::{SourceQuote, VolumeKind};
use crate::service::watchlist_quotes::provider::{QuoteFailure, QuoteRequest, get};
use serde_json::Value;
use std::collections::HashMap;

pub(super) fn fetch_quotes(
    requests: &[QuoteRequest],
) -> Result<HashMap<String, SourceQuote>, QuoteFailure> {
    let symbols: Vec<String> = requests
        .iter()
        .filter_map(|request| symbol(&request.symbol))
        .collect();
    if symbols.is_empty() {
        return Ok(HashMap::new());
    }
    let body = get(
        DataSource::Bitfinex,
        &format!("{BASE_URL}/tickers?symbols={}", symbols.join(",")),
        &[],
    )?;
    parse_tickers(&body, requests)
}

/// A market's price and day from the fields of a ticker, from BID on: the last trade, the price 24
/// hours ago (the last trade less the change) and the base asset traded since. `None` for a market
/// with no trade.
pub(super) fn quote(fields: &[Value]) -> Option<SourceQuote> {
    let number = |index: usize| fields.get(index)?.as_f64();
    let price = number(6).filter(|price| *price > 0.0)?;
    let mut quote = SourceQuote {
        price,
        previous_day_price: number(4)
            .map(|change| price - change)
            .filter(|previous| *previous > 0.0),
        ..SourceQuote::default()
    };
    if let Some(units) = number(7) {
        quote.volume_24h = Some(units);
        quote.volume_kind = Some(VolumeKind::Base);
    }
    Some(quote)
}

/// The requested markets of a `/v2/tickers` answer, keyed by the symbol they were asked as.
pub(super) fn parse_tickers(
    body: &str,
    requests: &[QuoteRequest],
) -> Result<HashMap<String, SourceQuote>, QuoteFailure> {
    let value: Value = serde_json::from_str(body)
        .map_err(|error| QuoteFailure::Failed(format!("Data error: {error}")))?;
    if let Some(refusal) = refusal(&value) {
        return Err(QuoteFailure::Failed(refusal.to_string()));
    }
    let rows = value
        .as_array()
        .ok_or_else(|| QuoteFailure::Failed("Unexpected Bitfinex response".to_owned()))?;
    let quotes: HashMap<&str, SourceQuote> = rows
        .iter()
        .filter_map(|row| {
            let row = row.as_array()?;
            Some((row.first()?.as_str()?, quote(row.get(1..)?)?))
        })
        .collect();
    Ok(requests
        .iter()
        .filter_map(|request| {
            let quote = quotes.get(symbol(&request.symbol)?.as_str())?;
            Some((request.symbol.clone(), *quote))
        })
        .collect())
}
