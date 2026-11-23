//! What every Lighter socket has in common: where it connects, how it is kept alive and how a
//! subscription is written.
//!
//! Measured on 2026-11-21: a connection that sends nothing is closed after two minutes (code 1000,
//! "i/o timeout"), and a text `{"type":"ping"}` is answered `{"type":"pong"}` and counts as
//! something sent. A channel Lighter does not know (a market number it does not list, a resolution
//! it does not serve) is answered with an `error` message and the connection stays, so every market
//! of a feed shares one connection. 224 subscriptions on one were accepted.

use crate::service::net::ws::{Decode, Keepalive, Session};
use serde_json::json;
use std::time::Duration;

const HOST: &str = "mainnet.zklighter.elliot.ai";

const KEEPALIVE: Keepalive = Keepalive::QuietText {
    ping: r#"{"type":"ping"}"#,
    pong: r#"{"type":"pong"}"#,
};

/// `{"type":"subscribe","channel":"candle/1/1m"}`.
fn subscribe(channel: &str) -> String {
    json!({"type": "subscribe", "channel": channel}).to_string()
}

/// A connection that subscribes to every one of `channels`.
pub(super) fn session(channels: &[String]) -> Session {
    Session {
        url: format!("wss://{HOST}/stream"),
        host: HOST.to_owned(),
        port: 443,
        subscribe: channels.iter().map(|channel| subscribe(channel)).collect(),
        // A market nobody trades is quiet for hours: the pongs say the socket is alive.
        max_data_silence: Duration::from_secs(600),
        decode: Decode::None,
        keepalive: KEEPALIVE,
    }
}
