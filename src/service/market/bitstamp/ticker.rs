//! The REST call behind a watchlist price of this venue.
//!
//! `/api/v2/ticker/` answers every market in one call (80 KB, 272 rows, perpetuals included):
//! `{"last":"57867.19","open":"58808.00","open_24":"61862.87","percent_change_24":"-6.46",
//! "volume":"2485.13","timestamp":"1795244706","pair":"BTC/USD","market_type":"SPOT",…}`. Its
//! `open` is where the UTC day opened; `open_24` is the price 24 hours ago, which is what a
//! watchlist's change is measured against (and is `0` for a market that has not traded in the 24
//! hours, which then has no change). `volume` is the base asset traded in them. A pair asked of
//! `/api/v2/ticker/<pair>/` that Bitstamp does not list gets this same all-markets answer, which is
//! why a market is looked up in the list and never asked for alone.
//!
//! The socket carries no ticker, only trades, so this call is where a watchlist's price a day ago
//! comes from, while the socket keeps the last price current (`quotes.rs`).

use super::market_id;
use super::rest::BASE_URL;
use crate::model::DataSource;
use crate::model::watchlist::quote::{SourceQuote, VolumeKind};
use crate::service::watchlist_quotes::provider::{QuoteFailure, QuoteRequest, get, json, number};
use serde_json::Value;
use std::collections::HashMap;

pub(super) fn fetch_quotes(
    requests: &[QuoteRequest],
) -> Result<HashMap<String, SourceQuote>, QuoteFailure> {
    let body = get(DataSource::Bitstamp, &format!("{BASE_URL}/ticker/"), &[])?;
    parse_tickers(&body, requests)
}

/// The market a ticker row is of, upper case as the app spells it: `BTC/USD` is `BTCUSD`.
fn market_of(row: &Value) -> Option<String> {
    let pair = row.get("pair")?.as_str()?;
    Some(
        pair.chars()
            .filter(|ch| ch.is_ascii_alphanumeric())
            .collect::<String>()
            .to_uppercase(),
    )
    .filter(|market| !market.is_empty())
}

/// A spot market's price and day from its row: the last trade, the price 24 hours ago and the base
/// asset traded since. `None` for a perpetual, which shares the list, and for a market with no trade.
fn quote(row: &Value) -> Option<SourceQuote> {
    if row.get("market_type")?.as_str()? != "SPOT" {
        return None;
    }
    let price = number(row.get("last")).filter(|price| *price > 0.0)?;
    let mut quote = SourceQuote {
        price,
        previous_day_price: number(row.get("open_24")).filter(|open| *open > 0.0),
        timestamp_ms: number(row.get("timestamp")).map(|seconds| seconds as i64 * 1_000),
        ..SourceQuote::default()
    };
    if let Some(units) = number(row.get("volume")) {
        quote.volume_24h = Some(units);
        quote.volume_kind = Some(VolumeKind::Base);
    }
    Some(quote)
}

/// The requested markets of an `/api/v2/ticker/` answer, keyed by the symbol they were asked as.
pub(super) fn parse_tickers(
    body: &str,
    requests: &[QuoteRequest],
) -> Result<HashMap<String, SourceQuote>, QuoteFailure> {
    let value = json(body)?;
    let quotes: HashMap<String, SourceQuote> = value
        .as_array()
        .ok_or_else(|| QuoteFailure::Failed("Unexpected Bitstamp response".to_owned()))?
        .iter()
        .filter_map(|row| Some((market_of(row)?, quote(row)?)))
        .collect();
    Ok(requests
        .iter()
        .filter_map(|request| {
            let quote = quotes.get(&market_id(&request.symbol)?.to_uppercase())?;
            Some((request.symbol.clone(), *quote))
        })
        .collect())
}
