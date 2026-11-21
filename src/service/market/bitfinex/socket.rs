//! What every Bitfinex socket has in common: where it connects, how it is kept alive and how
//! subscriptions are written.
//!
//! `wss://api-pub.bitfinex.com/ws/2`, JSON text. Every channel sends `[chanId,"hb"]` about every
//! 15 seconds, so a connection with a subscription is never silent; a quiet one is probed with
//! `{"event":"ping"}`, answered `{"event":"pong","ts":…}`. A connection holds only so many
//! subscriptions: measured on 2026-11-21 it took 33 to 56 (the number varied from one connection to
//! the next) and refused every one beyond with an `error` event (`subscribe: limit`, code 10305)
//! while the others stayed; the 30 of its documentation is what a feed plans for.

use crate::service::net::ws::{Decode, Keepalive, Session};
use serde_json::json;
use std::time::Duration;

const HOST: &str = "api-pub.bitfinex.com";

/// The subscriptions one connection is given; a feed with more opens another.
pub(super) const CHANNELS_PER_CONNECTION: usize = 30;

const KEEPALIVE: Keepalive = Keepalive::Text {
    ping: r#"{"event":"ping"}"#,
    pong: r#"{"event":"pong""#,
};

/// A connection that sends `subscribe` once connected.
pub(super) fn session(subscribe: Vec<String>) -> Session {
    Session {
        url: format!("wss://{HOST}/ws/2"),
        host: HOST.to_owned(),
        port: 443,
        subscribe,
        // The heartbeats are data: a minute without any means the connection is gone.
        max_data_silence: Duration::from_secs(60),
        decode: Decode::None,
        keepalive: KEEPALIVE,
    }
}

/// `{"event":"subscribe","channel":"candles","key":"trade:1m:tBTCUSD"}`; the acknowledgement or the
/// refusal is a message of its own.
pub(super) fn subscribe_candles(period: &str, symbol: &str) -> String {
    json!({"event": "subscribe", "channel": "candles", "key": format!("trade:{period}:{symbol}")})
        .to_string()
}

/// `{"event":"subscribe","channel":"ticker","symbol":"tBTCUSD"}`.
pub(super) fn subscribe_ticker(symbol: &str) -> String {
    json!({"event": "subscribe", "channel": "ticker", "symbol": symbol}).to_string()
}
