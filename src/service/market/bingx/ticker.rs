//! The REST call behind a watchlist price of this venue, and the quote that it and the `ticker`
//! socket topic both make of a market's 24 hours.
//!
//! `/openApi/spot/v1/ticker/24hr` answers every market in one call (206 KB, 645 rows on
//! 2026-11-21; given a `symbol` it answers that market, and several joined by commas are
//! `symbol is not found`, so it is always asked whole):
//! `{"symbol":"BTC-USDT","openPrice":61542.79,"lastPrice":58255.28,"volume":5034.26,
//! "quoteVolume":296070295.19,"openTime":1795160168676,"closeTime":1795246568676,…}`. The window
//! is rolling (`closeTime - openTime` is 24 hours), so `openPrice` is the price a day ago, which
//! is what a watchlist's change is measured against. `volume` is the base asset traded in it and
//! `quoteVolume` the quote asset.

use super::parse::answer;
use super::rest::get;
use crate::model::watchlist::quote::{SourceQuote, VolumeKind};
use crate::service::watchlist_quotes::provider::{
    DOLLAR_QUOTES, QuoteFailure, QuoteRequest, number,
};
use serde_json::Value;
use std::collections::HashMap;

/// What a market's 24 hours say, from a REST row or a socket push alike.
pub(super) struct Ticker {
    pub(super) last: Option<f64>,
    /// The price 24 hours ago.
    pub(super) open: Option<f64>,
    pub(super) base_volume: Option<f64>,
    pub(super) quote_volume: Option<f64>,
    pub(super) time_ms: Option<i64>,
}

impl Ticker {
    /// The quote of market `id` (BingX's name for it): the last price, the price a day ago and the
    /// volume, as the turnover of a dollar-quoted pair and the units traded otherwise. `None` for a
    /// market with no price.
    pub(super) fn quote(&self, id: &str) -> Option<SourceQuote> {
        let price = self.last.filter(|price| *price > 0.0)?;
        let mut quote = SourceQuote {
            price,
            previous_day_price: self.open.filter(|open| *open > 0.0),
            timestamp_ms: self.time_ms,
            ..SourceQuote::default()
        };
        let dollar_quoted = DOLLAR_QUOTES
            .iter()
            .any(|quote| id.ends_with(&format!("-{quote}")));
        if let (true, Some(turnover)) = (dollar_quoted, self.quote_volume) {
            quote.volume_24h = Some(turnover);
            quote.volume_kind = Some(VolumeKind::QuoteCurrency);
        } else if let Some(units) = self.base_volume {
            quote.volume_24h = Some(units);
            quote.volume_kind = Some(VolumeKind::Base);
        }
        Some(quote)
    }
}

/// The market (upper case, as BingX names it) and the 24 hours of a REST row.
fn row(row: &Value) -> Option<(String, Ticker)> {
    Some((
        row.get("symbol")?.as_str()?.to_uppercase(),
        Ticker {
            last: number(row.get("lastPrice")),
            open: number(row.get("openPrice")),
            base_volume: number(row.get("volume")),
            quote_volume: number(row.get("quoteVolume")),
            time_ms: number(row.get("closeTime")).map(|time| time as i64),
        },
    ))
}

pub(super) fn fetch_quotes(
    requests: &[QuoteRequest],
) -> Result<HashMap<String, SourceQuote>, QuoteFailure> {
    let body = get("/openApi/spot/v1/ticker/24hr").map_err(failure)?;
    parse_tickers(&body, requests)
}

fn failure(error: crate::service::ProviderError) -> QuoteFailure {
    if error.is_rate_limited() {
        QuoteFailure::RateLimited
    } else {
        QuoteFailure::Failed(error.to_string())
    }
}

/// The requested markets of a `ticker/24hr` answer that have a price, keyed by the symbol they
/// were asked as. A refusal of the call is the provider's own.
pub(super) fn parse_tickers(
    body: &str,
    requests: &[QuoteRequest],
) -> Result<HashMap<String, SourceQuote>, QuoteFailure> {
    let data = answer(body)
        .and_then(|answer| answer.data())
        .map_err(failure)?;
    let quotes: HashMap<String, SourceQuote> = data
        .as_array()
        .ok_or_else(|| QuoteFailure::Failed("Unexpected BingX response".to_owned()))?
        .iter()
        .filter_map(|value| {
            let (id, ticker) = row(value)?;
            let quote = ticker.quote(&id)?;
            Some((id, quote))
        })
        .collect();
    Ok(requests
        .iter()
        .filter_map(|request| {
            let quote = quotes.get(&super::market_id(&request.symbol)?)?;
            Some((request.symbol.clone(), *quote))
        })
        .collect())
}
