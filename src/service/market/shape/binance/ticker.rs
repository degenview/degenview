//! The 24 h ticker as watchlist prices: pure functions over the response text. `openPrice` is the
//! rolling 24 h open, a truer day-change reference than a candle picked from the last 25 hours.

use crate::model::DataSource;
use crate::model::watchlist::quote::{SourceQuote, VolumeKind};
use crate::service::watchlist_quotes::provider::{
    DOLLAR_QUOTES, QuoteFailure, QuoteRequest, json, number,
};
use serde_json::Value;
use std::collections::HashMap;

/// A price from the figures of one ticker. Turnover is only a dollar figure on a dollar-quoted
/// pair; otherwise report units traded.
pub(super) fn quote_of(
    symbol: &str,
    price: f64,
    open: Option<f64>,
    time_ms: Option<i64>,
    quote_volume: Option<f64>,
    base_volume: Option<f64>,
) -> SourceQuote {
    let mut quote = SourceQuote {
        price,
        previous_day_price: open,
        timestamp_ms: time_ms,
        ..SourceQuote::default()
    };
    let dollar_quoted = DOLLAR_QUOTES.iter().any(|quote| symbol.ends_with(quote));
    if let (true, Some(turnover)) = (dollar_quoted, quote_volume) {
        quote.volume_24h = Some(turnover);
        quote.volume_kind = Some(VolumeKind::QuoteCurrency);
    } else if let Some(units) = base_volume {
        quote.volume_24h = Some(units);
        quote.volume_kind = Some(VolumeKind::Base);
    }
    quote
}

/// `/ticker/24hr`, an array with `lastPrice`, `openPrice`, `volume`, `quoteVolume` and
/// `closeTime` for each market. A market the answer lacks, or without a price, is omitted.
pub(in crate::service) fn parse_quotes(
    source: DataSource,
    body: &str,
    requests: &[QuoteRequest],
) -> Result<HashMap<String, SourceQuote>, QuoteFailure> {
    let value = json(body)?;
    let tickers: HashMap<String, &Value> = value
        .as_array()
        .ok_or_else(|| {
            QuoteFailure::Failed(format!("Unexpected {} response", source.display_name()))
        })?
        .iter()
        .filter_map(|ticker| Some((ticker.get("symbol")?.as_str()?.to_uppercase(), ticker)))
        .collect();
    let mut quotes = HashMap::new();
    for request in requests {
        let symbol = request.symbol.to_uppercase();
        let Some(ticker) = tickers.get(&symbol) else {
            continue;
        };
        let Some(price) = number(ticker.get("lastPrice")).filter(|price| *price > 0.0) else {
            continue;
        };
        let quote = quote_of(
            &symbol,
            price,
            number(ticker.get("openPrice")),
            ticker.get("closeTime").and_then(Value::as_i64),
            number(ticker.get("quoteVolume")),
            number(ticker.get("volume")),
        );
        quotes.insert(request.symbol.clone(), quote);
    }
    Ok(quotes)
}
