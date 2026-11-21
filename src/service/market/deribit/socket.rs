//! What every Deribit socket has in common: where it connects, how it is kept alive and how
//! subscriptions are written.
//!
//! JSON-RPC over `wss://www.deribit.com/ws/api/v2`. Measured on 2026-11-21: a `public/subscribe`
//! that names a channel Deribit does not know (an unlisted instrument, a resolution it does not
//! serve) is answered with the channels it did subscribe, the unknown one left out and the rest
//! working, so every market of a feed shares one connection. 918 channels were subscribed on one
//! (in requests of 100: a single request of 918 closes the connection, one of 750 is accepted). The
//! server's own heartbeat keeps it alive, which `Keepalive::JsonRpcHeartbeat` asks for after every
//! connect.

use crate::service::net::ws::{Decode, Keepalive, Session};
use serde_json::json;
use std::time::Duration;

const HOST: &str = "www.deribit.com";

/// The server sends a heartbeat every 30 seconds, and half a second later a `test_request` that
/// the client answers.
const KEEPALIVE: Keepalive = Keepalive::JsonRpcHeartbeat { interval_secs: 30 };

/// The most channels one subscribe request names.
const CHANNELS_PER_REQUEST: usize = 100;

/// `{"jsonrpc":"2.0","id":2,"method":"public/subscribe","params":{"channels":[…]}}`. The ids start
/// at 2: the heartbeat's own requests have theirs.
fn subscribe(id: usize, channels: &[String]) -> String {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "public/subscribe",
        "params": { "channels": channels },
    })
    .to_string()
}

/// A connection that subscribes to every one of `channels`.
pub(super) fn session(channels: &[String]) -> Session {
    Session {
        url: format!("wss://{HOST}/ws/api/v2"),
        host: HOST.to_owned(),
        port: 443,
        subscribe: channels
            .chunks(CHANNELS_PER_REQUEST)
            .enumerate()
            .map(|(index, chunk)| subscribe(index + 2, chunk))
            .collect(),
        // A market nobody trades is quiet for hours: the heartbeats say the socket is alive.
        max_data_silence: Duration::from_secs(600),
        decode: Decode::None,
        keepalive: KEEPALIVE,
    }
}

/// The channel and the data of a `{"method":"subscription","params":{"channel":…,"data":{…}}}`
/// push; nothing for any other message (the answer to a subscribe, a heartbeat).
pub(super) fn push(text: &str) -> Option<(String, serde_json::Value)> {
    let mut value: serde_json::Value = serde_json::from_str(text).ok()?;
    if value.get("method")?.as_str()? != "subscription" {
        return None;
    }
    let params = value.get_mut("params")?;
    let channel = params.get("channel")?.as_str()?.to_owned();
    Some((channel, params.get_mut("data")?.take()))
}
