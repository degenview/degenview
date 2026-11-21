//! What every BingX socket has in common: where it connects, how it is kept alive, how topics are
//! subscribed and what BingX calls a candle length on the socket.
//!
//! `wss://open-api-ws.bingx.com/market`, every message a gzip binary frame. A `sub` of a topic is
//! answered `{"code":0,"id":…,"msg":"SUCCESS"}`, or for a topic that does not exist (an unknown
//! market, a length the socket has no name for) `{"code":100400,…,"msg":"dataType is error: …"}`
//! with the connection and its other topics staying up. A connection takes 200 topics; the 201st
//! and every one after is refused with code 100416 (measured on 2026-11-21). The server pings every
//! 30 seconds with `{"ping":"<id>","time":"…"}` (the first within 25 seconds of connecting); it
//! dropped neither a client that never answered (8 pings, 4 minutes) nor one that answered
//! `Pong`.

use crate::model::Timeframe;
use crate::service::net::ws::{Decode, Keepalive, Session};
use serde_json::json;
use std::time::Duration;

const HOST: &str = "open-api-ws.bingx.com";

/// The topics one connection is given; a feed with more opens another.
pub(super) const TOPICS_PER_CONNECTION: usize = 200;

const KEEPALIVE: Keepalive = Keepalive::AnswerJson {
    ask: "ping",
    answer: "Pong",
};

/// A connection that sends `subscribe` once connected.
pub(super) fn session(subscribe: Vec<String>) -> Session {
    Session {
        url: format!("wss://{HOST}/market"),
        host: HOST.to_owned(),
        port: 443,
        subscribe,
        // A market nobody trades is quiet for hours: the server's pings say the socket is alive.
        max_data_silence: Duration::from_secs(600),
        decode: Decode::Gzip,
        keepalive: KEEPALIVE,
    }
}

/// The connections that carry `topics` (already written as subscriptions), 200 to a connection.
pub(super) fn sessions(topics: Vec<String>) -> Vec<Session> {
    topics
        .chunks(TOPICS_PER_CONNECTION)
        .map(|group| session(group.to_vec()))
        .collect()
}

/// What the socket calls a candle length. It is not the REST name (`1h` is `60min`, and its
/// `1hour`, `1d` and `1M` are refused), which is why this is a table of its own.
pub(super) fn token(timeframe: Timeframe) -> Option<&'static str> {
    Some(match timeframe.interval().as_str() {
        "1m" => "1min",
        "3m" => "3min",
        "5m" => "5min",
        "15m" => "15min",
        "30m" => "30min",
        "1h" => "60min",
        "2h" => "2hour",
        "4h" => "4hour",
        "8h" => "8hour",
        _ => return None,
    })
}

/// The topic of a market's candles of `token`.
pub(super) fn kline_topic(id: &str, token: &str) -> String {
    format!("{id}@kline_{token}")
}

/// The topic of a market's 24 hour ticker.
pub(super) fn ticker_topic(id: &str) -> String {
    format!("{id}@ticker")
}

/// `{"id":"degenview","reqType":"sub","dataType":"BTC-USDT@kline_1min"}`; the acknowledgement or the
/// refusal is a message of its own.
pub(super) fn subscribe(topic: &str) -> String {
    json!({"id": "degenview", "reqType": "sub", "dataType": topic}).to_string()
}
