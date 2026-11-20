//! Bybit's `kline.<interval>.<SYMBOL>` topics on the public spot socket. Each topic is its own
//! subscribe request: a request takes at most ten topics and one unknown symbol refuses the whole
//! of it, so a market that was delisted would silence its neighbours. The socket is kept alive
//! with `{"op":"ping"}`.

use crate::model::{Kline, Timeframe};
use crate::service::market::bybit::{KEEPALIVE, interval, symbol_id};
use crate::service::net::ws::Session;
use serde_json::{Value, json};
use std::time::Duration;

const HOST: &str = "stream.bybit.com";

/// `None` when Bybit has no interval of `base`.
pub(super) fn session(symbols: &[String], base: Timeframe) -> Option<Session> {
    let interval = interval(base)?;
    let subscribe = symbols
        .iter()
        .map(|symbol| {
            let topic = format!("kline.{interval}.{}", symbol_id(symbol));
            json!({"op": "subscribe", "args": [topic]}).to_string()
        })
        .collect();
    Some(Session {
        url: format!("wss://{HOST}/v5/public/spot"),
        host: HOST.to_owned(),
        port: 443,
        subscribe,
        // A thin market's kline is quiet for minutes: the pong says the socket is alive.
        max_data_silence: Duration::from_secs(600),
        keepalive: KEEPALIVE,
    })
}

/// `{"topic":"kline.1.BTCUSDT","type":"snapshot","data":[{"start":1794913860000,
/// "end":1794913919999,"interval":"1","open":"…","close":"…","high":"…","low":"…","volume":"…",
/// "turnover":"…","confirm":false}]}`. The volume is the running total of the forming candle.
/// Subscription replies, pongs and errors carry no candle.
pub(super) fn parse_kline(text: &str) -> Option<(String, Kline)> {
    let value: Value = serde_json::from_str(text).ok()?;
    let symbol = value
        .get("topic")?
        .as_str()?
        .strip_prefix("kline.")?
        .split_once('.')?
        .1
        .to_uppercase();
    let row = value.get("data")?.as_array()?.first()?;
    let number = |key: &str| row.get(key)?.as_str()?.parse::<f64>().ok();
    Some((
        symbol,
        Kline {
            open_time: row.get("start")?.as_i64()?,
            close_time: row.get("end")?.as_i64()?,
            open: number("open")?,
            high: number("high")?,
            low: number("low")?,
            close: number("close")?,
            volume: number("volume")?,
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_market_is_its_own_subscribe_request_for_the_interval_of_the_base_candle() {
        let days = session(&["btc/usdt".into(), "ETHUSDT".into()], Timeframe::DAY_1).unwrap();
        assert_eq!(days.url, "wss://stream.bybit.com/v5/public/spot");
        let messages: Vec<Value> = days
            .subscribe
            .iter()
            .map(|message| serde_json::from_str(message).unwrap())
            .collect();
        assert_eq!(
            messages,
            [
                json!({"op": "subscribe", "args": ["kline.D.BTCUSDT"]}),
                json!({"op": "subscribe", "args": ["kline.D.ETHUSDT"]}),
            ]
        );
        let hours = session(&["BTCUSDT".into()], Timeframe::HOUR_4).unwrap();
        assert!(hours.subscribe[0].contains("kline.240.BTCUSDT"));
    }

    #[test]
    fn a_length_bybit_has_no_interval_for_has_no_session() {
        let eight_hours = Timeframe::parse("8h").unwrap();
        assert!(session(&["BTCUSDT".into()], eight_hours).is_none());
        assert!(session(&["BTCUSDT".into()], Timeframe::parse("3M").unwrap()).is_none());
    }

    #[test]
    fn parses_a_push_whichever_its_type() {
        for kind in ["snapshot", "delta"] {
            let text = format!(
                r#"{{"type":"{kind}","topic":"kline.60.ETHUSDT","data":[{{"start":1794913200000,"end":1794916799999,"interval":"60","open":"1788.99","close":"1796.76","high":"1797.44","low":"1786.76","volume":"929.60082","turnover":"1666024.34","confirm":false,"timestamp":1794913916197}}],"ts":1794913916197}}"#
            );
            let (symbol, kline) = parse_kline(&text).expect("parse");
            assert_eq!(symbol, "ETHUSDT");
            assert_eq!(kline.open_time, 1_794_913_200_000);
            assert_eq!(kline.close_time, 1_794_916_799_999);
            assert_eq!(
                (kline.open, kline.high, kline.low, kline.close),
                (1788.99, 1797.44, 1786.76, 1796.76)
            );
            assert_eq!(kline.volume, 929.60082);
        }
    }

    #[test]
    fn acknowledgements_pongs_errors_and_other_topics_are_not_candles() {
        for ignored in [
            r#"{"success":true,"ret_msg":"subscribe","conn_id":"a","op":"subscribe"}"#,
            r#"{"success":true,"ret_msg":"pong","conn_id":"a","op":"ping"}"#,
            r#"{"success":false,"ret_msg":"Invalid symbol :[kline.1.NOPEUSDT]","op":"subscribe"}"#,
            r#"{"topic":"tickers.BTCUSDT","ts":1,"type":"snapshot","data":{"symbol":"BTCUSDT"}}"#,
            r#"{"topic":"kline.1.BTCUSDT","data":[]}"#,
            r#"{"topic":"kline.1.BTCUSDT","data":[{"start":1,"end":2,"open":"x"}]}"#,
            "not json",
        ] {
            assert!(parse_kline(ignored).is_none(), "{ignored}");
        }
    }
}
