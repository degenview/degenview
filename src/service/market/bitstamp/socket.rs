//! What every Bitstamp socket has in common: where it connects, how it is kept alive and how
//! subscriptions are written.
//!
//! `wss://ws.bitstamp.net`, JSON text. Measured on 2026-11-21: it has no candle or ticker channel
//! (`ohlc_btcusd` is `{"event":"bts:error",…"Invalid channel provided."}`), only
//! `live_trades_<market>`, which every market of a feed shares one connection for (all 252 spot
//! markets were subscribed on one). A subscription of a market Bitstamp does not list is
//! acknowledged like any other and never trades. The server sends no heartbeat of its own and kept
//! a quiet connection open for the 15 minutes it was watched, but it answers a
//! `{"event":"bts:heartbeat"}` of the client with
//! `{"event":"bts:heartbeat","channel":"","data":{"status":"success"}}`, which is what keeps the
//! connection probed. It may ask the client to reconnect with `{"event":"bts:request_reconnect"}`
//! (the documented way a server restart is announced; never seen), which the readers of both feeds
//! answer by asking the loop for a new connection (`FrameReader::reconnect`).

use super::market_id;
use super::parse::TRADES_CHANNEL;
use crate::service::net::ws::{Decode, Keepalive, Session};
use serde_json::json;
use std::time::Duration;

const HOST: &str = "ws.bitstamp.net";

const KEEPALIVE: Keepalive = Keepalive::QuietText {
    ping: r#"{"event":"bts:heartbeat"}"#,
    pong: r#"{"event":"bts:heartbeat""#,
};

/// A connection that sends `subscribe` once connected.
fn session(subscribe: Vec<String>) -> Session {
    Session {
        url: format!("wss://{HOST}"),
        host: HOST.to_owned(),
        port: 443,
        subscribe,
        // A market nobody trades is quiet for hours: the answers to the client's heartbeat, which
        // are not data, count as hearing Bitstamp (`Keepalive::QuietText`), so a quiet feed is not
        // reconnected for want of a trade.
        max_data_silence: Duration::from_secs(600),
        decode: Decode::None,
        keepalive: KEEPALIVE,
    }
}

/// `{"event":"bts:subscribe","data":{"channel":"live_trades_btcusd"}}`; the acknowledgement is a
/// message of its own.
fn subscribe_trades(market: &str) -> String {
    json!({"event": "bts:subscribe", "data": {"channel": format!("{TRADES_CHANNEL}{market}")}})
        .to_string()
}

/// The connection that streams the trades of `symbols`, each market once, however it was spelled.
/// Text that cannot name a market is left out.
pub(super) fn trades_session(symbols: &[String]) -> Session {
    let mut subscriptions: Vec<String> = Vec::new();
    for market in symbols.iter().filter_map(|symbol| market_id(symbol)) {
        let subscription = subscribe_trades(&market);
        if !subscriptions.contains(&subscription) {
            subscriptions.push(subscription);
        }
    }
    session(subscriptions)
}
