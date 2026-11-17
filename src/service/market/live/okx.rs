//! OKX's `candle<bar>` channels, on the `business` endpoint (the `public` one refuses them). A
//! single subscribe message names every market; the socket is kept alive with the text `"ping"`.

use crate::model::{Kline, Timeframe};
use crate::service::market::okx::{bar, instrument_id, parse_row};
use crate::service::net::ws::{Keepalive, Session};
use serde_json::{Value, json};
use std::time::Duration;

const HOST: &str = "ws.okx.com";

/// `None` when OKX has no bar of `base`.
pub(super) fn session(symbols: &[String], base: Timeframe) -> Option<Session> {
    let channel = format!("candle{}", bar(base)?);
    let args: Vec<Value> = symbols
        .iter()
        .map(|symbol| json!({"channel": channel, "instId": instrument_id(symbol)}))
        .collect();
    Some(Session {
        url: format!("wss://{HOST}/ws/v5/business"),
        host: HOST.to_owned(),
        port: 443,
        subscribe: vec![json!({"op": "subscribe", "args": args}).to_string()],
        // A candle channel pushes every few seconds; the heartbeat answers say the socket is alive.
        max_data_silence: Duration::from_secs(120),
        keepalive: Keepalive::Text {
            ping: "ping",
            pong: "pong",
        },
    })
}

/// `{"arg":{"channel":"candle1m","instId":"BTC-USDT"},"data":[["1794904500000","o","h","l","c",
/// "vol",…,"0"]]}`; the subscription answers and errors carry an `event` and no data.
pub(super) fn parse_kline(text: &str, base: Timeframe) -> Option<(String, Kline)> {
    let value: Value = serde_json::from_str(text).ok()?;
    let symbol = value.pointer("/arg/instId")?.as_str()?.to_uppercase();
    let row = value.get("data")?.as_array()?.first()?;
    Some((symbol, parse_row(row, base).ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_message_subscribes_every_market_to_the_bar_of_the_base_candle() {
        let session = session(&["btc-usdt".into(), "ETH-USDT".into()], Timeframe::DAY_1).unwrap();
        assert_eq!(session.url, "wss://ws.okx.com/ws/v5/business");
        let message: Value = serde_json::from_str(&session.subscribe[0]).unwrap();
        assert_eq!(message["op"], "subscribe");
        assert_eq!(
            message["args"],
            json!([
                {"channel": "candle1Dutc", "instId": "BTC-USDT"},
                {"channel": "candle1Dutc", "instId": "ETH-USDT"},
            ])
        );
        assert_eq!(
            session.keepalive,
            Keepalive::Text {
                ping: "ping",
                pong: "pong"
            }
        );
    }

    #[test]
    fn a_length_okx_has_no_bar_for_has_no_session() {
        let eight_hours = Timeframe::parse("8h").unwrap();
        assert!(session(&["BTC-USDT".into()], eight_hours).is_none());
    }

    #[test]
    fn parses_a_candle_push_with_the_close_of_its_bucket() {
        let text = r#"{"arg":{"channel":"candle1H","instId":"BTC-USDT"},"data":[["1699999200000","10","12","9","11","42.5","450","450","0"]]}"#;
        let (symbol, kline) = parse_kline(text, Timeframe::HOUR_1).expect("parse");
        assert_eq!(symbol, "BTC-USDT");
        assert_eq!(kline.open_time, 1_699_999_200_000);
        assert_eq!(kline.close_time, 1_700_002_799_999);
        assert_eq!(
            (kline.open, kline.high, kline.low, kline.close),
            (10.0, 12.0, 9.0, 11.0)
        );
        assert_eq!(kline.volume, 42.5);
    }

    #[test]
    fn answers_and_errors_are_not_candles() {
        for ignored in [
            r#"{"event":"subscribe","arg":{"channel":"candle1m","instId":"BTC-USDT"},"connId":"a"}"#,
            r#"{"event":"error","msg":"Wrong URL or channel","code":"60018"}"#,
            r#"{"arg":{"channel":"candle1m","instId":"BTC-USDT"},"data":[]}"#,
            r#"{"arg":{"channel":"candle1m","instId":"BTC-USDT"},"data":[["x"]]}"#,
            "not json",
        ] {
            assert!(
                parse_kline(ignored, Timeframe::MINUTE_1).is_none(),
                "{ignored}"
            );
        }
    }
}
