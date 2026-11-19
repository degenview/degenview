//! Bybit's `publicTrade.<SYMBOL>` topic on the public spot and linear sockets. One subscribe
//! request names the one market; an unknown symbol is refused with an ordinary reply the parser
//! skips.

use crate::model::taker_flow::{Market, Taker, Trade, Venue};
use crate::service::net::ws::{Keepalive, Session};
use serde_json::{Value, json};
use std::time::Duration;

const HOST: &str = "stream.bybit.com";

/// The `{ASSET}USDT` market of `market`; `asset` is upper case letters and digits.
pub(super) fn session(market: Market, asset: &str) -> Session {
    let path = match market {
        Market::Spot => "spot",
        Market::Perpetual => "linear",
    };
    Session {
        url: format!("wss://{HOST}/v5/public/{path}"),
        host: HOST.to_owned(),
        port: 443,
        subscribe: vec![
            json!({"op": "subscribe", "args": [format!("publicTrade.{asset}USDT")]}).to_string(),
        ],
        // A thin market can be quiet for a long while: the pong says the socket is alive.
        max_data_silence: Duration::from_secs(600),
        keepalive: Keepalive::Text {
            ping: r#"{"op":"ping"}"#,
            pong: r#"{"success":true,"ret_msg":"pong""#,
        },
    }
}

/// `{"topic":"publicTrade.BTCUSDT","type":"snapshot","ts":…,"data":[{"T":…,"s":"BTCUSDT",
/// "S":"Buy","v":"0.001","p":"16578.50",…}]}`. `S` is the side of the taker.
pub(super) fn parse(venue: Venue, text: &str) -> Vec<Trade> {
    let Ok(value) = serde_json::from_str::<Value>(text) else {
        return Vec::new();
    };
    let is_trade = value
        .get("topic")
        .and_then(Value::as_str)
        .is_some_and(|topic| topic.starts_with("publicTrade."));
    let Some(rows) = value
        .get("data")
        .and_then(Value::as_array)
        .filter(|_| is_trade)
    else {
        return Vec::new();
    };
    rows.iter()
        .filter_map(|row| parse_row(venue, row))
        .collect()
}

fn parse_row(venue: Venue, row: &Value) -> Option<Trade> {
    let number = |key: &str| row.get(key)?.as_str()?.parse::<f64>().ok();
    let taker = match row.get("S")?.as_str()? {
        "Buy" => Taker::Buy,
        "Sell" => Taker::Sell,
        _ => return None,
    };
    let notional = number("p")? * number("v")?;
    let time_ms = row.get("T")?.as_i64()?;
    (notional > 0.0).then_some(Trade {
        venue,
        notional,
        taker,
        time_ms,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const PUSH: &str = r#"{"topic":"publicTrade.BTCUSDT","type":"snapshot","ts":1672304486868,"data":[{"T":1672304486865,"s":"BTCUSDT","S":"Buy","v":"0.001","p":"16578.50","L":"PlusTick","i":"x","BT":false},{"T":1672304486866,"s":"BTCUSDT","S":"Sell","v":"0.5","p":"16578.00","i":"y","BT":false}]}"#;

    fn venue() -> Venue {
        Venue::ALL[2]
    }

    #[test]
    fn one_subscribe_request_on_the_spot_or_the_linear_socket() {
        let spot = session(Market::Spot, "BTC");
        assert_eq!(spot.url, "wss://stream.bybit.com/v5/public/spot");
        let sent: Value = serde_json::from_str(&spot.subscribe[0]).unwrap();
        assert_eq!(
            sent,
            json!({"op": "subscribe", "args": ["publicTrade.BTCUSDT"]})
        );
        assert_eq!(spot.subscribe.len(), 1);
        let perp = session(Market::Perpetual, "ETH");
        assert_eq!(perp.url, "wss://stream.bybit.com/v5/public/linear");
        assert!(matches!(perp.keepalive, Keepalive::Text { .. }));
    }

    #[test]
    fn the_side_is_the_takers_and_the_size_is_in_the_base_coin() {
        let trades = parse(venue(), PUSH);
        assert_eq!(trades.len(), 2);
        assert_eq!(trades[0].taker, Taker::Buy);
        assert_eq!(trades[0].notional, 16578.50 * 0.001);
        assert_eq!(trades[0].time_ms, 1672304486865);
        assert_eq!(trades[1].taker, Taker::Sell);
        assert_eq!(trades[1].venue, venue());
    }

    #[test]
    fn acks_pongs_refusals_and_bad_rows_are_ignored() {
        for ignored in [
            "not json",
            r#"{"success":true,"ret_msg":"","op":"subscribe"}"#,
            r#"{"success":false,"ret_msg":"error:handler not found,topic:publicTrade.NOPEUSDT","op":"subscribe"}"#,
            r#"{"success":true,"ret_msg":"pong","op":"ping"}"#,
            r#"{"topic":"kline.1.BTCUSDT","data":[{"T":1,"S":"Buy","v":"1","p":"1"}]}"#,
            r#"{"topic":"publicTrade.BTCUSDT","data":[]}"#,
            r#"{"topic":"publicTrade.BTCUSDT","data":[{"T":1,"S":"None","v":"1","p":"1"}]}"#,
            r#"{"topic":"publicTrade.BTCUSDT","data":[{"T":1,"S":"Buy","v":"0","p":"1"}]}"#,
        ] {
            assert!(parse(venue(), ignored).is_empty(), "{ignored}");
        }
    }
}
