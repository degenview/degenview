//! What every Hyperliquid socket has in common: where it connects, how it is kept alive, how a
//! subscription is written and how a message is split into its channel and data.
//!
//! One connection allows 1,000 subscriptions and an IP 100 connections. A candle subscription for
//! a name Hyperliquid does not list closes the connection without a word (the `activeAssetCtx` one
//! is answered with an `error` message), so the callers keep such a subscription to a connection
//! of its own.

use crate::service::net::ws::{Decode, Keepalive, Session};
use serde_json::{Value, json};
use std::time::Duration;

const HOST: &str = "api.hyperliquid.xyz";

/// `{"method":"ping"}` is answered `{"channel":"pong"}`; the server closes a connection that was
/// silent for a minute.
const KEEPALIVE: Keepalive = Keepalive::QuietText {
    ping: r#"{"method":"ping"}"#,
    pong: r#"{"channel":"pong""#,
};

/// A connection that subscribes to every one of `subscriptions`.
pub(super) fn session(subscriptions: &[Value]) -> Session {
    Session {
        url: format!("wss://{HOST}/ws"),
        host: HOST.to_owned(),
        port: 443,
        subscribe: subscriptions
            .iter()
            .map(|subscription| {
                json!({"method": "subscribe", "subscription": subscription}).to_string()
            })
            .collect(),
        // A market nobody trades is quiet for hours: the pongs say the socket is alive.
        max_data_silence: Duration::from_secs(600),
        decode: Decode::None,
        keepalive: KEEPALIVE,
    }
}

/// `{"channel":"candle","data":{…}}` as its channel and data; nothing for a message that is not
/// one. Subscription acknowledgements, errors and pongs have no `data` object to speak of, and
/// the callers look at the channel.
pub(super) fn channel_message(text: &str) -> Option<(String, Value)> {
    let mut value: Value = serde_json::from_str(text).ok()?;
    let channel = value.get("channel")?.as_str()?.to_owned();
    Some((channel, value.get_mut("data")?.take()))
}
