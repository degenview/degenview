//! The REST call behind a watchlist price of this venue, and the parser of what it and the
//! `ticker` socket channel both say about a market.
//!
//! `get_book_summary_by_currency` answers every future settled in one currency in a call
//! (`currency=BTC` the two inverse perpetuals, `currency=USDC` the ~150 linear ones, 120 KB; it
//! refuses `any`): `{"result":[{"instrument_name":"SOL_USDC-PERPETUAL","last":65.985,
//! "price_change":-8.5676,"volume_usd":628104737.7,…}]}`. A market's `ticker` carries the same in
//! `last_price` and `stats`.

use super::markets::{instrument_name, settlement_of};
use super::parse::answer;
use super::rest::BASE_URL;
use crate::model::DataSource;
use crate::model::watchlist::quote::{SourceQuote, VolumeKind};
use crate::service::watchlist_quotes::Outcome;
use crate::service::watchlist_quotes::provider::{
    QuoteFailure, QuoteRequest, from_change_percent, get, number,
};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};

/// The URL of the one call that prices every future settled in `currency`.
fn summary_url(currency: &str) -> String {
    format!("{BASE_URL}/get_book_summary_by_currency?currency={currency}&kind=future")
}

pub(super) fn fetch_quotes(requests: &[QuoteRequest]) -> Result<Outcome, QuoteFailure> {
    Ok(fetch_quotes_with(requests, |url| {
        get(DataSource::Deribit, url, &[])
    }))
}

/// One call per settlement currency that the markets asked for are in; a call that fails fails only
/// its markets (HTTP 400 is a currency Deribit does not have: its markets are rejected). A name that is no perpetual's is not asked for. `ask` answers a GET of the URL.
pub(super) fn fetch_quotes_with(
    requests: &[QuoteRequest],
    ask: impl Fn(&str) -> Result<String, QuoteFailure>,
) -> Outcome {
    let mut by_currency: BTreeMap<String, Vec<(&QuoteRequest, String)>> = BTreeMap::new();
    for request in requests {
        if let Some(name) = instrument_name(&request.symbol)
            && let Some(currency) = settlement_of(&name)
        {
            by_currency
                .entry(currency.to_owned())
                .or_default()
                .push((request, name));
        }
    }
    let mut outcome = Outcome::default();
    for (currency, asked) in by_currency {
        match ask(&summary_url(&currency)).and_then(|body| parse_summaries(&body)) {
            Ok(quotes) => {
                outcome
                    .quotes
                    .extend(asked.iter().filter_map(|(request, name)| {
                        Some((request.symbol.clone(), *quotes.get(name)?))
                    }))
            }
            Err(failure) => {
                let symbols = asked.iter().map(|(request, _)| request.symbol.clone());
                match failure {
                    // An unknown settlement currency (`NOPE-PERPETUAL`) is a name Deribit does not
                    // list: the name's fault, which the call is not asked again for.
                    QuoteFailure::Rejected(_) => outcome.rejected.extend(symbols),
                    failure => outcome.failed.push((symbols.collect(), failure)),
                }
            }
        }
    }
    outcome
}

/// A market's price and its day: the last trade, the price a day ago from the change in percent
/// over the last 24 hours and the dollars traded in that time. `None` for a market with no trade.
pub(super) fn day_quote(
    price: Option<f64>,
    change_percent: Option<f64>,
    turnover: Option<f64>,
) -> Option<SourceQuote> {
    let mut quote = from_change_percent(price.filter(|price| *price > 0.0)?, change_percent);
    if let Some(turnover) = turnover {
        quote.volume_24h = Some(turnover);
        quote.volume_kind = Some(VolumeKind::QuoteCurrency);
    }
    Some(quote)
}

/// The markets of a `get_book_summary_by_currency` answer that have a price, by name.
pub(super) fn parse_summaries(body: &str) -> Result<HashMap<String, SourceQuote>, QuoteFailure> {
    let result = answer(body).map_err(|error| QuoteFailure::Failed(error.to_string()))?;
    let rows = result
        .as_array()
        .ok_or_else(|| QuoteFailure::Failed("Unexpected Deribit response".to_owned()))?;
    Ok(rows
        .iter()
        .filter_map(|row| {
            let quote = day_quote(
                number(row.get("last")),
                number(row.get("price_change")),
                number(row.get("volume_usd")),
            )?;
            Some((row.get("instrument_name")?.as_str()?.to_owned(), quote))
        })
        .collect())
}

/// The price a `ticker` push carries (`data`): its market's name and what it says of it.
pub(super) fn ticker_quote(data: &Value) -> Option<(String, SourceQuote)> {
    let stats = data.get("stats");
    let mut quote = day_quote(
        number(data.get("last_price")),
        number(stats.and_then(|stats| stats.get("price_change"))),
        number(stats.and_then(|stats| stats.get("volume_usd"))),
    )?;
    quote.timestamp_ms = data.get("timestamp").and_then(Value::as_i64);
    Some((data.get("instrument_name")?.as_str()?.to_owned(), quote))
}
