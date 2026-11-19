//! Bybit's `allLiquidation.<SYMBOL>` topics on the public linear (USDT and USDC contracts) socket.
//! Bybit has no topic for every market, so the most traded ones are asked for: each topic is its
//! own subscribe request, because one unknown symbol refuses the whole of a request.

use crate::model::liquidation::{Exchange, Liquidation, Side};
use crate::service::market::bybit::get;
use crate::service::net::ws::{Keepalive, Session};
use serde_json::{Value, json};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

const HOST: &str = "stream.bybit.com";
const TICKERS: &str = "https://api.bybit.com/v5/market/tickers?category=linear";
/// How many markets are followed: those with the most turnover in the last day.
const MARKETS: usize = 60;
const RETRY_AFTER: Duration = Duration::from_secs(10);

/// The session over the busiest markets, once their list is known; `None` when `shutdown` came
/// first.
pub(super) fn session(shutdown: &AtomicBool) -> Option<Session> {
    let symbols = loop {
        if let Some(symbols) = get(TICKERS)
            .ok()
            .and_then(|body| busiest_markets(&body, MARKETS))
        {
            break symbols;
        }
        if !sleep_unless_stopped(shutdown, RETRY_AFTER) {
            return None;
        }
    };
    Some(session_for(&symbols))
}

fn session_for(symbols: &[String]) -> Session {
    Session {
        url: format!("wss://{HOST}/v5/public/linear"),
        host: HOST.to_owned(),
        port: 443,
        subscribe: symbols
            .iter()
            .map(|symbol| {
                json!({"op": "subscribe", "args": [format!("allLiquidation.{symbol}")]}).to_string()
            })
            .collect(),
        // A calm hour can pass without one: the pong says the socket is alive.
        max_data_silence: Duration::from_secs(600),
        keepalive: Keepalive::Text {
            ping: r#"{"op":"ping"}"#,
            pong: r#"{"success":true,"ret_msg":"pong""#,
        },
    }
}

/// Waits `span` in slices; `false` when `shutdown` was set meanwhile.
fn sleep_unless_stopped(shutdown: &AtomicBool, span: Duration) -> bool {
    let slice = Duration::from_millis(250);
    let mut waited = Duration::ZERO;
    while waited < span {
        if shutdown.load(Ordering::Acquire) {
            return false;
        }
        std::thread::sleep(slice);
        waited += slice;
    }
    !shutdown.load(Ordering::Acquire)
}

/// `{"retCode":0,"result":{"list":[{"symbol":"BTCUSDT","turnover24h":"…"},…]}}`: the `count`
/// symbols with the most turnover. `None` when the answer is a refusal or lists nothing.
fn busiest_markets(body: &str, count: usize) -> Option<Vec<String>> {
    let value: Value = serde_json::from_str(body).ok()?;
    if value.get("retCode")?.as_i64()? != 0 {
        return None;
    }
    let mut markets: Vec<(f64, String)> = value
        .pointer("/result/list")?
        .as_array()?
        .iter()
        .filter_map(|row| {
            let turnover = row.get("turnover24h")?.as_str()?.parse::<f64>().ok()?;
            Some((turnover, row.get("symbol")?.as_str()?.to_owned()))
        })
        .collect();
    markets.sort_by(|a, b| b.0.total_cmp(&a.0));
    markets.truncate(count);
    (!markets.is_empty()).then(|| markets.into_iter().map(|(_, symbol)| symbol).collect())
}

/// `{"topic":"allLiquidation.ROSEUSDT","type":"snapshot","ts":…,"data":[{"T":…,"s":"ROSEUSDT",
/// "S":"Buy","v":"20000","p":"0.04499"}]}`. `S` is the position that was liquidated: a Buy is a
/// long.
pub(super) fn parse(text: &str) -> Vec<Liquidation> {
    let Ok(value) = serde_json::from_str::<Value>(text) else {
        return Vec::new();
    };
    let is_liquidation = value
        .get("topic")
        .and_then(Value::as_str)
        .is_some_and(|topic| topic.starts_with("allLiquidation."));
    let Some(rows) = value
        .get("data")
        .and_then(Value::as_array)
        .filter(|_| is_liquidation)
    else {
        return Vec::new();
    };
    rows.iter().filter_map(parse_row).collect()
}

fn parse_row(row: &Value) -> Option<Liquidation> {
    let number = |key: &str| row.get(key)?.as_str()?.parse::<f64>().ok();
    let side = match row.get("S")?.as_str()? {
        "Buy" => Side::Long,
        "Sell" => Side::Short,
        _ => return None,
    };
    let (price, quantity) = (number("p")?, number("v")?);
    let symbol = row.get("s")?.as_str()?.to_owned();
    let time_ms = row.get("T")?.as_i64()?;
    (price > 0.0 && quantity > 0.0).then_some(Liquidation {
        exchange: Exchange::Bybit,
        symbol,
        side,
        price,
        quantity,
        time_ms,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const PUSH: &str = r#"{"topic":"allLiquidation.ROSEUSDT","type":"snapshot","ts":1739502303204,"data":[{"T":1739502302929,"s":"ROSEUSDT","S":"Sell","v":"20000","p":"0.04499"},{"T":1739502302930,"s":"ROSEUSDT","S":"Buy","v":"100","p":"0.045"}]}"#;

    #[test]
    fn one_subscribe_request_per_market_on_the_linear_socket() {
        let session = session_for(&["BTCUSDT".to_owned(), "ETHUSDT".to_owned()]);
        assert_eq!(session.url, "wss://stream.bybit.com/v5/public/linear");
        let sent: Vec<Value> = session
            .subscribe
            .iter()
            .map(|message| serde_json::from_str(message).unwrap())
            .collect();
        assert_eq!(
            sent,
            [
                json!({"op": "subscribe", "args": ["allLiquidation.BTCUSDT"]}),
                json!({"op": "subscribe", "args": ["allLiquidation.ETHUSDT"]}),
            ]
        );
        assert!(matches!(session.keepalive, Keepalive::Text { .. }));
    }

    #[test]
    fn a_buy_is_a_long_liquidated_and_a_sell_a_short() {
        let rows = parse(PUSH);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].side, Side::Short);
        assert_eq!((rows[0].price, rows[0].quantity), (0.04499, 20_000.0));
        assert_eq!(rows[0].time_ms, 1739502302929);
        assert_eq!(rows[0].exchange, Exchange::Bybit);
        assert_eq!(rows[0].symbol, "ROSEUSDT");
        assert_eq!(rows[1].side, Side::Long);
    }

    #[test]
    fn acks_pongs_other_topics_and_bad_rows_are_ignored() {
        for ignored in [
            "not json",
            r#"{"success":true,"ret_msg":"","conn_id":"x","op":"subscribe"}"#,
            r#"{"success":true,"ret_msg":"pong","op":"ping"}"#,
            r#"{"topic":"kline.1.BTCUSDT","data":[{"T":1,"s":"BTCUSDT","S":"Buy","v":"1","p":"1"}]}"#,
            r#"{"topic":"allLiquidation.BTCUSDT","data":[]}"#,
            r#"{"topic":"allLiquidation.BTCUSDT","data":[{"T":1,"s":"BTCUSDT","S":"Flat","v":"1","p":"1"}]}"#,
            r#"{"topic":"allLiquidation.BTCUSDT","data":[{"T":1,"s":"BTCUSDT","S":"Buy","v":"0","p":"1"}]}"#,
        ] {
            assert!(parse(ignored).is_empty(), "{ignored}");
        }
    }

    #[test]
    fn the_busiest_markets_are_those_with_the_most_turnover() {
        let body = r#"{"retCode":0,"result":{"list":[
            {"symbol":"SMALLUSDT","turnover24h":"10"},
            {"symbol":"BTCUSDT","turnover24h":"9000"},
            {"symbol":"ETHUSDT","turnover24h":"500"},
            {"symbol":"BROKEN","turnover24h":"x"}]}}"#;
        assert_eq!(busiest_markets(body, 2).unwrap(), ["BTCUSDT", "ETHUSDT"]);
        assert_eq!(busiest_markets(body, 10).unwrap().len(), 3);
    }

    #[test]
    fn a_refusal_or_an_empty_list_has_no_markets() {
        assert!(busiest_markets(r#"{"retCode":10006,"result":{"list":[]}}"#, 5).is_none());
        assert!(busiest_markets(r#"{"retCode":0,"result":{"list":[]}}"#, 5).is_none());
        assert!(busiest_markets("nope", 5).is_none());
    }

    #[test]
    fn waiting_stops_at_once_when_the_feed_is_shut_down() {
        let stopped = AtomicBool::new(true);
        assert!(!sleep_unless_stopped(&stopped, Duration::from_secs(60)));
        assert!(sleep_unless_stopped(
            &AtomicBool::new(false),
            Duration::ZERO
        ));
    }
}
