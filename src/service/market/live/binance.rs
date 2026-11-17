//! Binance's combined kline stream: the symbols and interval are in the URL, nothing is sent.

use crate::model::{Kline, Timeframe};
use crate::service::net::ws::{Keepalive, Session};
use serde_json::Value;
use std::time::Duration;

const HOST: &str = "stream.binance.com";

pub(super) fn session(symbols: &[String], base: Timeframe) -> Session {
    let interval = base.interval();
    let streams = symbols
        .iter()
        .map(|symbol| format!("{}@kline_{interval}", symbol.to_lowercase()))
        .collect::<Vec<_>>()
        .join("/");
    Session {
        url: format!("wss://{HOST}/stream?streams={streams}"),
        host: HOST.to_owned(),
        port: 443,
        subscribe: None,
        max_data_silence: Duration::from_secs(60),
        keepalive: Keepalive::Frame,
    }
}

/// `{"stream":"btcusdt@kline_1h","data":{"k":{"t","o","h","l","c","v",…}}}`.
pub(super) fn parse_kline(text: &str) -> Option<(String, Kline)> {
    let value: Value = serde_json::from_str(text).ok()?;
    let stream = value.get("stream")?.as_str()?;
    let symbol = stream.split('@').next()?.to_uppercase();
    let k = value.pointer("/data/k")?;
    let t = k.get("t").and_then(Value::as_i64)?;
    // `T` is the bar's last millisecond, as in the REST rows. Without it the forming bar would read
    // as already closed (`close_time <= now`) and Pine would rebuild on every tick.
    let close_time = k.get("T").and_then(Value::as_i64).unwrap_or(t);
    let open = k.get("o").and_then(Value::as_str)?.parse().ok()?;
    let high = k.get("h").and_then(Value::as_str)?.parse().ok()?;
    let low = k.get("l").and_then(Value::as_str)?.parse().ok()?;
    let close = k.get("c").and_then(Value::as_str)?.parse().ok()?;
    let volume = k.get("v").and_then(Value::as_str)?.parse().ok()?;
    Some((
        symbol,
        Kline {
            open_time: t,
            close_time,
            open,
            high,
            low,
            close,
            volume,
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_symbols_and_interval_are_in_the_url() {
        let session = session(&["BTCUSDT".into(), "ETHUSDT".into()], Timeframe::HOUR_1);
        assert_eq!(
            session.url,
            "wss://stream.binance.com/stream?streams=btcusdt@kline_1h/ethusdt@kline_1h"
        );
        assert_eq!(session.subscribe, None);
    }

    #[test]
    fn parses_kline_stream_message() {
        let text = r#"{"stream":"btcusdt@kline_1h","data":{"k":{"t":1700000000000,"T":1700003599999,"o":"10","h":"12","l":"9","c":"11","v":"42.5","q":"0","x":false}}}"#;
        let (symbol, kline) = parse_kline(text).expect("parse");
        assert_eq!(symbol, "BTCUSDT");
        assert_eq!(kline.open_time, 1_700_000_000_000);
        assert_eq!(kline.close_time, 1_700_003_599_999);
        assert_eq!(kline.high, 12.0);
        assert_eq!(kline.low, 9.0);
        assert_eq!(kline.close, 11.0);
        assert_eq!(kline.volume, 42.5);
    }

    #[test]
    fn a_message_without_a_close_time_falls_back_to_the_open_time() {
        let text = r#"{"stream":"btcusdt@kline_1h","data":{"k":{"t":1700000000000,"o":"10","h":"12","l":"9","c":"11","v":"1"}}}"#;
        let (_, kline) = parse_kline(text).expect("parse");
        assert_eq!(kline.close_time, 1_700_000_000_000);
    }
}
