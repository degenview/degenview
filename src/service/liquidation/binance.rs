//! Binance USDⓈ-M futures' all-market liquidation stream, `!forceOrder@arr`: the stream is in the
//! URL, so nothing is sent. Binance pushes at most the latest liquidation of each symbol per
//! second, so a cascade is undercounted. The stream also carries COIN-M contracts (`st: 2`), whose
//! sizes are contracts worth a fixed dollar amount, not coins: they are left out, so every figure is
//! a USDⓈ-M one.

use crate::model::liquidation::{Exchange, Liquidation, Side};
use crate::service::net::ws::{Keepalive, Session};
use serde_json::Value;
use std::time::Duration;

const HOST: &str = "fstream.binance.com";

pub(super) fn session() -> Session {
    Session {
        url: format!("wss://{HOST}/market/ws/!forceOrder@arr"),
        host: HOST.to_owned(),
        port: 443,
        subscribe: Vec::new(),
        // A calm hour can pass without a liquidation of any market: the ping says it is alive.
        max_data_silence: Duration::from_secs(600),
        keepalive: Keepalive::Frame,
    }
}

/// `{"e":"forceOrder","E":…,"o":{"s":"BTCUSDT","S":"SELL","q":"0.014","p":"9910","ap":"9910",
/// "z":"0.014","T":1568014460893,…},"ps":"BTCUSDT","st":1}`. The order that closes a long position
/// is a sale. Only what has filled (`z`) was liquidated: an order with nothing filled yet is not a
/// liquidation of anything.
pub(super) fn parse(text: &str) -> Vec<Liquidation> {
    parse_order(text).into_iter().collect()
}

fn parse_order(text: &str) -> Option<Liquidation> {
    let value: Value = serde_json::from_str(text).ok()?;
    if value.get("e")?.as_str()? != "forceOrder" {
        return None;
    }
    // USDⓈ-M is `st` 1 (and what carries no `st` at all); COIN-M is 2.
    if value
        .get("st")
        .and_then(Value::as_i64)
        .is_some_and(|st| st != 1)
    {
        return None;
    }
    let order = value.get("o")?;
    let number = |key: &str| order.get(key)?.as_str()?.parse::<f64>().ok();
    let side = match order.get("S")?.as_str()? {
        "SELL" => Side::Long,
        "BUY" => Side::Short,
        _ => return None,
    };
    // The average fill price (the order's own while it has none) and the size that filled.
    let price = number("ap")
        .filter(|price| *price > 0.0)
        .or_else(|| number("p"))?;
    let quantity = number("z").filter(|size| *size > 0.0)?;
    let symbol = order.get("s")?.as_str()?.to_owned();
    let time_ms = order.get("T")?.as_i64()?;
    (price > 0.0 && quantity > 0.0).then_some(Liquidation {
        exchange: Exchange::Binance,
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

    const SELL: &str = r#"{"e":"forceOrder","E":1568014460893,"o":{"s":"BTCUSDT","S":"SELL","o":"LIMIT","f":"IOC","q":"0.014","p":"9910","ap":"9920","X":"FILLED","l":"0.014","z":"0.012","T":1568014460893}}"#;

    #[test]
    fn the_stream_is_in_the_url_and_nothing_is_sent() {
        let session = session();
        assert_eq!(
            session.url,
            "wss://fstream.binance.com/market/ws/!forceOrder@arr"
        );
        assert!(session.subscribe.is_empty());
        assert!(matches!(session.keepalive, Keepalive::Frame));
    }

    #[test]
    fn a_sale_is_a_long_liquidated_at_the_average_price_and_filled_size() {
        let [one] = parse(SELL).try_into().unwrap();
        assert_eq!(one.exchange, Exchange::Binance);
        assert_eq!(one.symbol, "BTCUSDT");
        assert_eq!(one.side, Side::Long);
        assert_eq!(one.price, 9920.0);
        assert_eq!(one.quantity, 0.012);
        assert_eq!(one.time_ms, 1568014460893);
    }

    #[test]
    fn a_purchase_is_a_short_liquidated() {
        let [one] = parse(&SELL.replace("SELL", "BUY")).try_into().unwrap();
        assert_eq!(one.side, Side::Short);
    }

    #[test]
    fn the_order_price_stands_in_while_there_is_no_average() {
        let no_average = SELL.replace(r#""ap":"9920""#, r#""ap":"0""#);
        let [one] = parse(&no_average).try_into().unwrap();
        assert_eq!((one.price, one.quantity), (9910.0, 0.012));
    }

    #[test]
    fn an_order_with_nothing_filled_is_not_a_liquidation() {
        let unfilled = SELL.replace(r#""z":"0.012""#, r#""z":"0""#);
        assert!(parse(&unfilled).is_empty());
    }

    #[test]
    fn only_usdm_contracts_count() {
        let usdm = SELL.replace(r#"}}"#, r#"},"ps":"BTCUSDT","st":1}"#);
        assert_eq!(parse(&usdm).len(), 1);
        // A COIN-M contract: one is worth a fixed $100, not one coin.
        let coinm = SELL
            .replace(r#""s":"BTCUSDT""#, r#""s":"BTCUSD_PERP""#)
            .replace(r#"}}"#, r#"},"ps":"BTCUSD","st":2}"#);
        assert!(parse(&coinm).is_empty());
        assert_eq!(parse(SELL).len(), 1, "a message with no st stays");
    }

    #[test]
    fn anything_else_is_ignored() {
        for ignored in [
            "not json",
            "{}",
            r#"{"e":"markPriceUpdate","o":{}}"#,
            r#"{"e":"forceOrder","o":{"s":"BTCUSDT","S":"HOLD","q":"1","p":"1","T":1}}"#,
            r#"{"e":"forceOrder","o":{"s":"BTCUSDT","S":"SELL","q":"0","p":"0","T":1}}"#,
            r#"{"e":"forceOrder","o":{"s":"BTCUSDT","S":"SELL","q":"1","p":"1"}}"#,
        ] {
            assert!(parse(ignored).is_empty(), "{ignored}");
        }
    }
}
