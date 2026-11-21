//! Kraken's websocket v2 `ohlc` channel. One subscribe message names every market and the
//! interval; the server sends a `heartbeat` every second, which keeps the socket alive along with
//! the protocol pings.

use super::Kraken;
use super::{interval, pair};
use crate::format::parse_iso8601;
use crate::model::market::bucket::next_bucket_start_ms;
use crate::model::{Kline, Timeframe};
use crate::service::market::live::CandleProtocol;
use crate::service::net::ws::{Decode, Frame, Keepalive, Session};
use serde_json::{Value, json};
use std::time::Duration;

const HOST: &str = "ws.kraken.com";

/// `None` when Kraken has no interval of `base`.
fn session(symbols: &[String], base: Timeframe) -> Option<Session> {
    let symbols: Vec<String> = symbols.iter().map(|symbol| pair(symbol)).collect();
    let subscribe = json!({
        "method": "subscribe",
        "params": {
            "channel": "ohlc",
            "symbol": symbols,
            "interval": interval(base)?,
            // The newest 720 candles are fetched over REST; the socket only keeps the forming one.
            "snapshot": false,
        },
    });
    Some(Session {
        url: format!("wss://{HOST}/v2"),
        host: HOST.to_owned(),
        port: 443,
        subscribe: vec![subscribe.to_string()],
        max_data_silence: Duration::from_secs(30),
        decode: Decode::None,
        keepalive: Keepalive::Frame,
    })
}

/// `{"channel":"ohlc","type":"update","data":[{"symbol":"BTC/USD","open":64813.7,"high":…,
/// "low":…,"close":…,"volume":1383.85,"interval_begin":"2026-11-17T00:00:00.000000000Z",
/// "interval":1440,…}]}`. The volume is the running total of the forming candle. Status,
/// heartbeat and subscription replies carry no candle.
fn parse_kline(text: &str, base: Timeframe) -> Option<(String, Kline)> {
    let value: Value = serde_json::from_str(text).ok()?;
    if value.get("channel")?.as_str()? != "ohlc" || value.get("type")?.as_str()? != "update" {
        return None;
    }
    let row = value.get("data")?.as_array()?.first()?;
    let number = |key: &str| row.get(key)?.as_f64();
    let open_time = parse_iso8601(row.get("interval_begin")?.as_str()?)?;
    Some((
        row.get("symbol")?.as_str()?.to_uppercase(),
        Kline {
            open_time,
            close_time: next_bucket_start_ms(base, open_time) - 1,
            open: number("open")?,
            high: number("high")?,
            low: number("low")?,
            close: number("close")?,
            volume: number("volume")?,
        },
    ))
}

impl CandleProtocol for Kraken {
    fn sessions(&self, symbols: &[String], base: Timeframe) -> Option<Vec<Session>> {
        session(symbols, base).map(|session| vec![session])
    }

    fn parse(&self, frame: Frame<'_>, base: Timeframe) -> Option<(String, Kline)> {
        parse_kline(frame.text()?, base)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_message_subscribes_every_market_to_the_interval_of_the_base_candle() {
        let session = session(&["btc/usd".into(), "XDG/EUR".into()], Timeframe::DAY_1).unwrap();
        assert_eq!(session.url, "wss://ws.kraken.com/v2");
        let message: Value = serde_json::from_str(&session.subscribe[0]).unwrap();
        assert_eq!(
            message,
            json!({"method": "subscribe", "params": {
                "channel": "ohlc",
                "symbol": ["BTC/USD", "DOGE/EUR"],
                "interval": 1440,
                "snapshot": false,
            }})
        );
        assert_eq!(session.keepalive, Keepalive::Frame);
    }

    #[test]
    fn a_length_kraken_has_no_interval_for_has_no_session() {
        let eight_hours = Timeframe::parse("8h").unwrap();
        assert!(session(&["BTC/USD".into()], eight_hours).is_none());
        assert!(session(&["BTC/USD".into()], Timeframe::WEEK_1).is_none());
    }

    #[test]
    fn parses_an_update_with_the_close_of_its_bucket() {
        let text = r#"{"channel":"ohlc","type":"update","timestamp":"2026-11-17T10:17:54.974820754Z","data":[{"symbol":"BTC/USD","open":64813.7,"high":65118.0,"low":64116.6,"close":64268.8,"trades":61705,"volume":1383.85408542,"vwap":64571.3,"interval_begin":"2026-11-17T00:00:00.000000000Z","interval":1440,"timestamp":"2026-11-18T00:00:00.000000Z"}]}"#;
        let (symbol, kline) = parse_kline(text, Timeframe::DAY_1).expect("parse");
        assert_eq!(symbol, "BTC/USD");
        assert_eq!(kline.open_time, 1_794_873_600_000);
        assert_eq!(kline.close_time, 1_794_959_999_999);
        assert_eq!(
            (kline.open, kline.high, kline.low, kline.close),
            (64_813.7, 65_118.0, 64_116.6, 64_268.8)
        );
        assert_eq!(kline.volume, 1383.85408542);
    }

    #[test]
    fn status_heartbeat_replies_and_snapshots_are_not_candles() {
        for ignored in [
            r#"{"channel":"status","type":"update","data":[{"system":"online"}]}"#,
            r#"{"channel":"heartbeat"}"#,
            r#"{"method":"subscribe","result":{"channel":"ohlc","interval":1,"snapshot":false,"symbol":"BTC/USD"},"success":true}"#,
            r#"{"error":"Currency pair not supported XBT/USD","method":"subscribe","success":false}"#,
            r#"{"channel":"ohlc","type":"snapshot","data":[{"symbol":"BTC/USD","open":1,"high":1,"low":1,"close":1,"volume":1,"interval_begin":"2026-11-17T00:00:00.000000000Z"}]}"#,
            r#"{"channel":"ohlc","type":"update","data":[]}"#,
            "not json",
        ] {
            assert!(
                parse_kline(ignored, Timeframe::MINUTE_1).is_none(),
                "{ignored}"
            );
        }
    }
}
