//! The REST call behind a watchlist price of this venue, and the parser of its answer.

use crate::model::DataSource;
use crate::model::watchlist::quote::{SourceQuote, VolumeKind};
use crate::service::watchlist_quotes::provider::{QuoteFailure, QuoteRequest, get, json, number};
use serde_json::Value;
use std::collections::HashMap;

pub(super) fn fetch_quotes(
    requests: &[QuoteRequest],
) -> Result<HashMap<String, SourceQuote>, QuoteFailure> {
    let Some((key, secret)) = crate::service::keychain::api_keys::alpaca_credentials() else {
        return Err(QuoteFailure::CredentialsMissing);
    };
    let symbols = requests
        .iter()
        .map(|request| request.symbol.to_uppercase())
        .collect::<Vec<_>>()
        .join(",");
    let url = format!("https://data.alpaca.markets/v2/stocks/snapshots?symbols={symbols}&feed=iex");
    let body = get(
        DataSource::Alpaca,
        &url,
        &[("APCA-API-KEY-ID", &key), ("APCA-API-SECRET-KEY", &secret)],
    )?;
    parse_alpaca_snapshots(&body, requests)
}

/// `/v2/stocks/snapshots`: the latest trade, and the previous session's close as the
/// day-change reference. The daily bar stands in while a symbol has no trade yet.
pub(in crate::service) fn parse_alpaca_snapshots(
    body: &str,
    requests: &[QuoteRequest],
) -> Result<HashMap<String, SourceQuote>, QuoteFailure> {
    let value = json(body)?;
    let snapshots = value
        .as_object()
        .ok_or_else(|| QuoteFailure::Failed("Unexpected Alpaca response".to_owned()))?;
    let mut quotes = HashMap::new();
    for request in requests {
        let Some(snapshot) = snapshots.get(&request.symbol.to_uppercase()) else {
            continue;
        };
        let Some(price) = number(snapshot.pointer("/latestTrade/p"))
            .or_else(|| number(snapshot.pointer("/dailyBar/c")))
            .filter(|price| *price > 0.0)
        else {
            continue;
        };
        let volume = number(snapshot.pointer("/dailyBar/v"));
        quotes.insert(
            request.symbol.clone(),
            SourceQuote {
                price,
                previous_day_price: number(snapshot.pointer("/prevDailyBar/c")),
                volume_24h: volume,
                volume_kind: volume.map(|_| VolumeKind::Shares),
                timestamp_ms: snapshot
                    .pointer("/latestTrade/t")
                    .and_then(Value::as_str)
                    .and_then(crate::format::parse_iso8601),
            },
        );
    }
    Ok(quotes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::watchlist_quotes::provider::request;

    #[test]
    fn alpaca_prefers_the_latest_trade_and_falls_back_to_the_daily_bar() {
        let body = r#"{
            "AAPL":{"latestTrade":{"p":190.5,"t":"2024-01-15T15:00:00Z"},"dailyBar":{"c":190,"v":1000},"prevDailyBar":{"c":188}},
            "TSLA":{"dailyBar":{"c":200,"v":5}},
            "ZERO":{"latestTrade":{"p":0}}
        }"#;
        let quotes = parse_alpaca_snapshots(
            body,
            &[
                request("aapl"),
                request("TSLA"),
                request("ZERO"),
                request("MISSING"),
            ],
        )
        .unwrap();
        assert_eq!(quotes.len(), 2);
        let aapl = &quotes["aapl"];
        assert_eq!((aapl.price, aapl.previous_day_price), (190.5, Some(188.0)));
        assert_eq!(aapl.volume_kind, Some(VolumeKind::Shares));
        assert_eq!(aapl.timestamp_ms, Some(1_705_330_800_000));
        assert_eq!(quotes["TSLA"].price, 200.0);
        assert_eq!(quotes["TSLA"].previous_day_price, None);
    }
}
