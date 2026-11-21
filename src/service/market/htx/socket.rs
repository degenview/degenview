//! What every HTX socket has in common: where it connects, how it is kept alive and how requests
//! and subscriptions are written.
//!
//! `wss://api.huobi.pro/ws`, every message a gzip binary frame. The server pings every few seconds
//! with `{"ping":1795239308084}` and expects `{"pong":1795239308084}` back; a client that misses a
//! couple is disconnected. Measured on 2026-11-21: a `sub` of an unknown market is answered with an
//! error message and the connection and its other subscriptions stay up, and all 616 online markets
//! were subscribed on one connection, so every market of a feed shares one.

use crate::service::net::ws::{Decode, Keepalive, Session};
use serde_json::json;
use std::time::Duration;

const HOST: &str = "api.huobi.pro";

const KEEPALIVE: Keepalive = Keepalive::EchoJson {
    ask: "ping",
    answer: "pong",
};

/// The id of the history request, which its answer carries back.
pub(super) const HISTORY_ID: &str = "history";

/// A connection that sends `subscribe` once connected.
pub(super) fn session(subscribe: Vec<String>) -> Session {
    Session {
        url: format!("wss://{HOST}/ws"),
        host: HOST.to_owned(),
        port: 443,
        subscribe,
        // A market nobody trades is quiet for hours: the server's pings say the socket is alive.
        max_data_silence: Duration::from_secs(600),
        decode: Decode::Gzip,
        keepalive: KEEPALIVE,
    }
}

/// The topic of a market's candles of `period`.
pub(super) fn kline_topic(id: &str, period: &str) -> String {
    format!("market.{id}.kline.{period}")
}

/// The topic of a market's 24 hour ticker.
pub(super) fn ticker_topic(id: &str) -> String {
    format!("market.{id}.ticker")
}

/// `{"sub":"market.btcusdt.kline.1min","id":"degenview"}`; the acknowledgement or the refusal of
/// an unknown market is a message of its own.
pub(super) fn subscribe(topic: &str) -> String {
    json!({"sub": topic, "id": "degenview"}).to_string()
}

/// `{"req":"market.btcusdt.kline.4hour","id":"history","from":…,"to":…}`: the candles that open
/// from `from_s` to `to_s` (seconds, both included), oldest first, in a window of at most 900
/// lengths. The history reaches back to the market's first candle.
pub(super) fn history_request(id: &str, period: &str, from_s: i64, to_s: i64) -> String {
    json!({
        "req": kline_topic(id, period),
        "id": HISTORY_ID,
        "from": from_s,
        "to": to_s,
    })
    .to_string()
}
