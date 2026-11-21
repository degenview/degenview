//! Hyperliquid's `candle` channel: the forming candle of a market and interval, pushed whenever a
//! trade changes it. Each market gets a connection of its own, because a subscription to a name
//! Hyperliquid does not list closes the connection and would take its neighbours with it.

use super::Hyperliquid;
use super::ids::coin_id;
use super::interval;
use super::parse::candle;
use super::socket::{channel_message, session};
use crate::model::{Kline, Timeframe};
use crate::service::market::live::CandleProtocol;
use crate::service::net::ws::{Frame, Session};
use serde_json::json;

/// `None` when Hyperliquid has no candle of `base`.
fn candle_sessions(symbols: &[String], base: Timeframe) -> Option<Vec<Session>> {
    let interval = interval(base)?;
    Some(
        symbols
            .iter()
            .map(|symbol| {
                session(&[json!({"type": "candle", "coin": coin_id(symbol), "interval": interval})])
            })
            .collect(),
    )
}

/// `{"channel":"candle","data":{"t":1795230240000,"T":1795230299999,"s":"BTC","i":"1m",
/// "o":"58404.0","c":"58404.0","h":"58413.0","l":"58371.0","v":"21.34999","n":147}}`. The volume
/// is the running total of the forming candle. Acknowledgements, errors and pongs carry no candle.
fn parse_candle(text: &str, base: Timeframe) -> Option<(String, Kline)> {
    let (channel, data) = channel_message(text)?;
    if channel != "candle" {
        return None;
    }
    let kline = candle(&data, base).ok()?;
    Some((data.get("s")?.as_str()?.to_owned(), kline))
}

impl CandleProtocol for Hyperliquid {
    fn sessions(&self, symbols: &[String], base: Timeframe) -> Option<Vec<Session>> {
        candle_sessions(symbols, base)
    }

    fn parse(&self, frame: Frame<'_>, base: Timeframe) -> Option<(String, Kline)> {
        parse_candle(frame.text()?, base)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[test]
    fn every_market_is_a_connection_of_its_own_for_the_interval_of_the_base_candle() {
        let sessions =
            candle_sessions(&["BTC".into(), "xyz:SP500".into()], Timeframe::HOUR_4).unwrap();
        assert_eq!(sessions.len(), 2);
        let subscribed: Vec<Value> = sessions
            .iter()
            .map(|session| {
                assert_eq!(session.url, "wss://api.hyperliquid.xyz/ws");
                assert_eq!(session.subscribe.len(), 1);
                serde_json::from_str(&session.subscribe[0]).unwrap()
            })
            .collect();
        assert_eq!(
            subscribed,
            [
                json!({"method": "subscribe", "subscription": {"type": "candle", "coin": "BTC", "interval": "4h"}}),
                json!({"method": "subscribe", "subscription": {"type": "candle", "coin": "xyz:SP500", "interval": "4h"}}),
            ]
        );
    }

    #[test]
    fn a_length_hyperliquid_has_no_candle_for_has_no_session() {
        for other in ["2m", "6h", "1w", "1M"] {
            let base = Timeframe::parse(other).unwrap();
            assert!(candle_sessions(&["BTC".into()], base).is_none(), "{other}");
        }
    }

    #[test]
    fn parses_a_pushed_candle_with_its_running_volume() {
        let text = r#"{"channel":"candle","data":{"t":1795230240000,"T":1795230299999,"s":"xyz:SP500","i":"1m","o":"6978.6","c":"6978.1","h":"6979.4","l":"6978.0","v":"3.322","n":42}}"#;
        let (symbol, kline) = parse_candle(text, Timeframe::MINUTE_1).expect("candle");
        assert_eq!(symbol, "xyz:SP500");
        assert_eq!(kline.open_time, 1_795_230_240_000);
        assert_eq!(kline.close_time, 1_795_230_299_999);
        assert_eq!(
            (kline.open, kline.high, kline.low, kline.close, kline.volume),
            (6978.6, 6979.4, 6978.0, 6978.1, 3.322)
        );
    }

    #[test]
    fn acknowledgements_pongs_errors_and_other_channels_are_not_candles() {
        for ignored in [
            r#"{"channel":"subscriptionResponse","data":{"method":"subscribe","subscription":{"type":"candle","interval":"1m","coin":"BTC"}}}"#,
            r#"{"channel":"pong"}"#,
            r#"{"channel":"error","data":"Invalid subscription"}"#,
            r#"{"channel":"activeAssetCtx","data":{"coin":"BTC","ctx":{"markPx":"58413.0"}}}"#,
            r#"{"channel":"candle","data":{"t":1,"s":"BTC","o":"x"}}"#,
            r#"{"channel":"candle","data":[]}"#,
            "not json",
        ] {
            assert!(
                parse_candle(ignored, Timeframe::MINUTE_1).is_none(),
                "{ignored}"
            );
        }
    }
}
