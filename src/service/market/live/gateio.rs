//! Gate.io's `spot.candlesticks` channel on the v4 socket. A subscribe request names one interval
//! and one market, and one market it does not know refuses its request, so each market is its own
//! request. The socket is kept alive with protocol pings, which the server answers.

use crate::model::{Kline, Timeframe};
use crate::service::market::gateio::{interval, pair_id};
use crate::service::net::ws::{Keepalive, Session};
use degenview_market::bucket::next_bucket_start_ms;
use serde_json::{Value, json};
use std::time::Duration;

const HOST: &str = "api.gateio.ws";

/// `None` when Gate.io has no interval of `base`.
pub(super) fn session(symbols: &[String], base: Timeframe) -> Option<Session> {
    let interval = interval(base)?;
    let subscribe = symbols
        .iter()
        .map(|symbol| {
            json!({
                "channel": "spot.candlesticks",
                "event": "subscribe",
                "payload": [interval, pair_id(symbol)],
            })
            .to_string()
        })
        .collect();
    Some(Session {
        url: format!("wss://{HOST}/ws/v4/"),
        host: HOST.to_owned(),
        port: 443,
        subscribe,
        // A thin market's candle is quiet for minutes: the answers to the pings say the socket is
        // alive.
        max_data_silence: Duration::from_secs(600),
        keepalive: Keepalive::Frame,
    })
}

/// `{"channel":"spot.candlesticks","event":"update","result":{"t":"1794918300","v":"6630.1",
/// "c":"64955.5","h":"64961.7","l":"64955.5","o":"64958.3","n":"1m_BTC_USDT","a":"0.102065",
/// "w":false}}`: `t` is the open in seconds, `v` the quote volume and `a` the base volume, a
/// running total. Subscription answers, errors and other channels carry no candle.
pub(super) fn parse_kline(text: &str, base: Timeframe) -> Option<(String, Kline)> {
    let value: Value = serde_json::from_str(text).ok()?;
    if value.get("channel")?.as_str()? != "spot.candlesticks"
        || value.get("event")?.as_str()? != "update"
    {
        return None;
    }
    let row = value.get("result")?;
    let number = |key: &str| row.get(key)?.as_str()?.parse::<f64>().ok();
    // `n` is `<interval>_<market>`, and an interval has no underscore.
    let symbol = row.get("n")?.as_str()?.split_once('_')?.1.to_uppercase();
    let open_time = row.get("t")?.as_str()?.parse::<i64>().ok()? * 1_000;
    Some((
        symbol,
        Kline {
            open_time,
            close_time: next_bucket_start_ms(base, open_time) - 1,
            open: number("o")?,
            high: number("h")?,
            low: number("l")?,
            close: number("c")?,
            volume: number("a")?,
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_market_is_its_own_subscribe_request_for_the_interval_of_the_base_candle() {
        let days = session(&["btc/usdt".into(), "ETH_USDT".into()], Timeframe::DAY_1).unwrap();
        assert_eq!(days.url, "wss://api.gateio.ws/ws/v4/");
        let messages: Vec<Value> = days
            .subscribe
            .iter()
            .map(|message| serde_json::from_str(message).unwrap())
            .collect();
        assert_eq!(
            messages,
            [
                json!({"channel": "spot.candlesticks", "event": "subscribe", "payload": ["1d", "BTC_USDT"]}),
                json!({"channel": "spot.candlesticks", "event": "subscribe", "payload": ["1d", "ETH_USDT"]}),
            ]
        );
        let weeks = session(&["BTC_USDT".into()], Timeframe::WEEK_1).unwrap();
        assert!(weeks.subscribe[0].contains(r#"["7d","BTC_USDT"]"#));
        assert_eq!(days.keepalive, Keepalive::Frame);
    }

    #[test]
    fn a_length_gate_does_not_document_has_no_session() {
        for text in ["3m", "2h", "6h", "12h", "3M"] {
            let timeframe = Timeframe::parse(text).unwrap();
            assert!(session(&["BTC_USDT".into()], timeframe).is_none(), "{text}");
        }
    }

    #[test]
    fn parses_an_update_with_the_close_of_its_bucket() {
        let text = r#"{"time":1794918322,"time_ms":1794918322926,"channel":"spot.candlesticks","event":"update","result":{"t":"1794918300","v":"6630.1164403","c":"64955.5","h":"64961.7","l":"64955.1","o":"64958.3","n":"1m_BTC_USDT","a":"0.102065","w":false}}"#;
        let (symbol, kline) = parse_kline(text, Timeframe::MINUTE_1).expect("parse");
        assert_eq!(symbol, "BTC_USDT");
        assert_eq!(kline.open_time, 1_794_918_300_000);
        assert_eq!(kline.close_time, 1_794_918_359_999);
        assert_eq!(
            (kline.open, kline.high, kline.low, kline.close),
            (64_958.3, 64_961.7, 64_955.1, 64_955.5)
        );
        assert_eq!(kline.volume, 0.102065);
    }

    #[test]
    fn answers_errors_and_other_channels_are_not_candles() {
        for ignored in [
            r#"{"channel":"spot.candlesticks","event":"subscribe","payload":["1m","BTC_USDT"],"result":{"status":"success"}}"#,
            r#"{"channel":"spot.candlesticks","event":"subscribe","payload":["1m","NOPE_USDT"],"error":{"code":2,"message":"unknown currency pair"},"result":{"status":"fail"}}"#,
            r#"{"channel":"spot.pong","event":"","result":null}"#,
            r#"{"channel":"spot.tickers","event":"update","result":{"currency_pair":"BTC_USDT"}}"#,
            r#"{"channel":"spot.candlesticks","event":"update","result":{"t":"1","n":"1m_BTC_USDT","o":"x"}}"#,
            "not json",
        ] {
            assert!(
                parse_kline(ignored, Timeframe::MINUTE_1).is_none(),
                "{ignored}"
            );
        }
    }
}
