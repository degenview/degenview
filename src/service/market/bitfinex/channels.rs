//! What a message of Bitfinex's public socket is, and which subscription a channel number is.
//!
//! Every push is `[chanId, payload]`: the number names the subscription, not the market, so a
//! connection learns what each number is from the `subscribed` event that opens it and remembers
//! that until it ends ([`Channels`]). Heartbeats are `[chanId,"hb"]`, and a refusal is an `error`
//! event (`symbol: invalid`, `subscribe: limit`, `subscribe: dup`) that leaves the connection and its
//! other subscriptions as they were. An `info` event can tell the client to connect again (20051: the
//! server is restarting; 20061: maintenance is over and every channel is to be subscribed again);
//! 20060, the start of maintenance, asks only to pause, which a client that sends nothing does.

use serde_json::Value;
use std::collections::HashMap;

/// What a channel was subscribed for.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum Subscription {
    /// The candles of `period` (`1m`) of a market, named as the app spells it (`BTCUSD`).
    Candles { market: String, period: String },
    /// The ticker of a market.
    Ticker { market: String },
}

/// One message of the socket.
#[derive(Debug, PartialEq)]
pub(super) enum Message<'a> {
    /// A subscription is open: this number is it from now on.
    Subscribed(i64, Subscription),
    /// A push on a channel; `payload` is whatever follows the number.
    Push(i64, &'a Value),
    /// The server asks for a new connection: it is restarting, or maintenance is over and the
    /// subscriptions of this one are gone.
    Reconnect,
    /// A heartbeat, a refusal, a notice or anything else that is not data.
    Other,
}

/// The `info` codes that ask the client to connect again.
const RECONNECT_CODES: [i64; 2] = [20051, 20061];

/// `["tBTCUSD"]` → `BTCUSD`: the market as the app spells it, from the `t` the API puts in front.
pub(super) fn market_of(symbol: &str) -> Option<&str> {
    symbol.strip_prefix('t').filter(|market| !market.is_empty())
}

/// What the `subscribed` event `event` opened.
fn subscription(event: &Value) -> Option<Subscription> {
    match event.get("channel")?.as_str()? {
        "candles" => {
            // `trade:1m:tBTCUSD`: the market is everything after the second colon (`tAVAX:USD`).
            let mut parts = event.get("key")?.as_str()?.splitn(3, ':');
            let (kind, period, symbol) = (parts.next()?, parts.next()?, parts.next()?);
            if kind != "trade" {
                return None;
            }
            Some(Subscription::Candles {
                market: market_of(symbol)?.to_owned(),
                period: period.to_owned(),
            })
        }
        "ticker" => Some(Subscription::Ticker {
            market: market_of(event.get("symbol")?.as_str()?)?.to_owned(),
        }),
        _ => None,
    }
}

/// Reads one message. `value` is the message already parsed.
pub(super) fn message(value: &Value) -> Message<'_> {
    if let Some(array) = value.as_array() {
        return match (array.first().and_then(Value::as_i64), array.get(1)) {
            (Some(id), Some(payload)) if payload.is_array() => Message::Push(id, payload),
            _ => Message::Other,
        };
    }
    match value.get("event").and_then(Value::as_str) {
        Some("subscribed") => {}
        Some("info")
            if value
                .get("code")
                .and_then(Value::as_i64)
                .is_some_and(|code| RECONNECT_CODES.contains(&code)) =>
        {
            return Message::Reconnect;
        }
        _ => return Message::Other,
    }
    match (
        value.get("chanId").and_then(Value::as_i64),
        subscription(value),
    ) {
        (Some(id), Some(subscription)) => Message::Subscribed(id, subscription),
        _ => Message::Other,
    }
}

/// The subscriptions of one connection, by channel number.
#[derive(Debug, Default)]
pub(super) struct Channels {
    open: HashMap<i64, Subscription>,
    /// The server asked for a new connection and nobody has acted on it yet.
    reconnect: bool,
}

impl Channels {
    /// A new connection: the numbers of the last are void.
    pub(super) fn clear(&mut self) {
        self.open.clear();
        self.reconnect = false;
    }

    /// Whether the server asked for a new connection since the last time this was asked.
    pub(super) fn take_reconnect(&mut self) -> bool {
        std::mem::take(&mut self.reconnect)
    }

    /// Takes in a `subscribed` event, and answers what a push on a channel is for.
    pub(super) fn track<'a>(&mut self, message: Message<'a>) -> Option<(&Subscription, &'a Value)> {
        match message {
            Message::Subscribed(id, subscription) => {
                self.open.insert(id, subscription);
                None
            }
            Message::Push(id, payload) => Some((self.open.get(&id)?, payload)),
            Message::Reconnect => {
                self.reconnect = true;
                None
            }
            Message::Other => None,
        }
    }
}
