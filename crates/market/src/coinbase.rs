//! Coinbase Exchange message shapes: candle rows and the websocket feed's subscription and
//! `ticker` messages. The transport (HTTP, paging, sockets, rate limits) is the caller's.

use serde_json::{Value, json};
use thiserror::Error;

use crate::Kline;
use crate::calendar::parse_iso8601;

/// The websocket feed's host; connect to `wss://` and this.
pub const STREAM_HOST: &str = "ws-feed.exchange.coinbase.com";

#[derive(Debug, Error)]
pub enum ParseError {
    #[error("{0}")]
    Json(#[from] serde_json::Error),
    #[error("expected a candle array")]
    NotAnArray,
}

/// Reads a `/products/{id}/candles` body: rows of `[time s, low, high, open, close, volume]`,
/// newest first. The result is oldest first, a bar being `granularity_secs` long. A row that is
/// not six numbers is skipped (a missing volume reads as 0).
pub fn parse_candles(body: &str, granularity_secs: i64) -> Result<Vec<Kline>, ParseError> {
    let value: Value = serde_json::from_str(body)?;
    let rows = value.as_array().ok_or(ParseError::NotAnArray)?;
    let mut candles = Vec::with_capacity(rows.len());
    for row in rows {
        let Some(fields) = row.as_array() else {
            continue;
        };
        if fields.len() < 6 {
            continue;
        }
        let Some(seconds) = fields[0].as_f64() else {
            continue;
        };
        let Some(low) = fields[1].as_f64() else {
            continue;
        };
        let Some(high) = fields[2].as_f64() else {
            continue;
        };
        let Some(open) = fields[3].as_f64() else {
            continue;
        };
        let Some(close) = fields[4].as_f64() else {
            continue;
        };
        let volume = fields[5].as_f64().unwrap_or(0.0);
        let open_time = (seconds * 1_000.0) as i64;
        candles.push(Kline {
            open_time,
            close_time: open_time + granularity_secs * 1_000 - 1,
            open,
            high,
            low,
            close,
            volume,
        });
    }
    candles.sort_by_key(|candle| candle.open_time);
    Ok(candles)
}

/// The message that subscribes a feed connection to the `ticker` and `heartbeat` channels of
/// `products`. Coinbase has no candle channel: the ticker carries every trade.
pub fn subscribe_message<S: AsRef<str>>(products: &[S]) -> String {
    let products: Vec<&str> = products.iter().map(AsRef::as_ref).collect();
    json!({
        "type": "subscribe",
        "product_ids": products,
        "channels": ["ticker", "heartbeat"],
    })
    .to_string()
}

/// One `ticker` message: the last trade of a product, with the rolling 24-hour figures.
#[derive(Clone, Debug, PartialEq)]
pub struct Ticker {
    /// Upper-cased, like `BTC-USD`.
    pub product_id: String,
    pub price: f64,
    /// The size of the trade, in the base currency. The message sent when a subscription starts
    /// repeats the previous trade.
    pub last_size: Option<f64>,
    pub trade_id: Option<u64>,
    pub time_ms: Option<i64>,
    pub open_24h: Option<f64>,
    pub volume_24h: Option<f64>,
}

/// Reads a feed message as a [`Ticker`]; `None` for any other message type, a message without a
/// product or with a price that is not positive, or text that is not JSON.
pub fn parse_ticker(text: &str) -> Option<Ticker> {
    let value: Value = serde_json::from_str(text).ok()?;
    if value.get("type")?.as_str()? != "ticker" {
        return None;
    }
    let number = |key: &str| -> Option<f64> { value.get(key)?.as_str()?.parse().ok() };
    Some(Ticker {
        price: number("price").filter(|price| *price > 0.0)?,
        product_id: value.get("product_id")?.as_str()?.to_uppercase(),
        last_size: number("last_size").filter(|size| *size >= 0.0),
        trade_id: value.get("trade_id").and_then(Value::as_u64),
        time_ms: value
            .get("time")
            .and_then(Value::as_str)
            .and_then(parse_iso8601),
        open_24h: number("open_24h").filter(|open| *open > 0.0),
        volume_24h: number("volume_24h"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const BODY: &str =
        "[[1700003600,99.0,103.0,101.0,102.5,12.5],[1700000000,98.0,102.0,100.0,101.0,9.0]]";

    #[test]
    fn rows_come_back_oldest_first_with_their_columns() {
        let candles = parse_candles(BODY, 3600).unwrap();
        assert_eq!(candles.len(), 2);
        assert_eq!(candles[0].open_time, 1_700_000_000_000);
        assert_eq!(
            candles[1],
            Kline {
                open_time: 1_700_003_600_000,
                close_time: 1_700_007_199_999,
                open: 101.0,
                high: 103.0,
                low: 99.0,
                close: 102.5,
                volume: 12.5,
            }
        );
        assert!(candles[1].is_up());
    }

    #[test]
    fn a_malformed_row_is_skipped_and_a_missing_volume_is_zero() {
        let body = r#"[[1700000000,98.0,102.0,100.0,101.0,null],[1700003600,"x",1,1,1,1],[1],5]"#;
        let candles = parse_candles(body, 60).unwrap();
        assert_eq!(candles.len(), 1);
        assert_eq!(candles[0].volume, 0.0);
    }

    #[test]
    fn an_error_object_is_not_a_candle_array() {
        let error = parse_candles(r#"{"message":"NotFound"}"#, 60).unwrap_err();
        assert!(matches!(error, ParseError::NotAnArray));
        assert_eq!(error.to_string(), "expected a candle array");
        assert!(matches!(
            parse_candles("<html>", 60),
            Err(ParseError::Json(_))
        ));
    }

    #[test]
    fn a_ticker_message_is_a_trade() {
        let text = r#"{"type":"ticker","product_id":"btc-usd","price":"67000.1","last_size":"0.25","trade_id":981,"open_24h":"66000","volume_24h":"10","time":"2024-01-15T15:00:00.250000Z"}"#;
        assert_eq!(
            parse_ticker(text),
            Some(Ticker {
                product_id: "BTC-USD".into(),
                price: 67_000.1,
                last_size: Some(0.25),
                trade_id: Some(981),
                time_ms: Some(1_705_330_800_250),
                open_24h: Some(66_000.0),
                volume_24h: Some(10.0),
            })
        );
    }

    #[test]
    fn optional_fields_may_be_missing_and_a_zero_open_is_no_reference() {
        let ticker = parse_ticker(
            r#"{"type":"ticker","product_id":"ETH-USD","price":"2000","open_24h":"0"}"#,
        )
        .unwrap();
        assert_eq!(
            (
                ticker.last_size,
                ticker.trade_id,
                ticker.time_ms,
                ticker.open_24h
            ),
            (None, None, None, None)
        );
    }

    #[test]
    fn other_messages_are_not_tickers() {
        for ignored in [
            r#"{"type":"heartbeat","product_id":"BTC-USD"}"#,
            r#"{"type":"subscriptions","channels":[]}"#,
            r#"{"type":"ticker","product_id":"BTC-USD","price":"0"}"#,
            r#"{"type":"ticker","price":"1"}"#,
            "not json",
        ] {
            assert_eq!(parse_ticker(ignored), None, "{ignored}");
        }
    }

    #[test]
    fn subscribing_names_the_products_and_channels() {
        let message: Value =
            serde_json::from_str(&subscribe_message(&["BTC-USD", "ETH-USD"])).unwrap();
        assert_eq!(message["type"], "subscribe");
        assert_eq!(message["product_ids"], json!(["BTC-USD", "ETH-USD"]));
        assert_eq!(message["channels"], json!(["ticker", "heartbeat"]));
    }
}
