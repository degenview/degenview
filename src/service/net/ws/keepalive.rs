//! How a feed's connection is kept alive, as data: what a quiet socket sends, what the server's own
//! heartbeat looks like and what answers it.
//!
//! The decisions are pure (a message in, a verdict out) so each venue's heartbeat is tested with
//! frames and no network; `Health` in the parent module only carries them out. Heartbeat traffic
//! never reaches a feed's parser: it is [`Inbound::Absorbed`] or answered, not data.

use serde::Deserialize;
use serde_json::{Value, json};
use tungstenite::Message;

/// Heartbeat messages are small; a longer one is data, and is not parsed to find out.
const SMALL_MESSAGE_BYTES: usize = 512;

/// The request ids a JSON-RPC heartbeat uses, so their answers are told from the feed's own. A feed
/// using [`Keepalive::JsonRpcHeartbeat`] must not use these.
pub const JSON_RPC_SETUP_ID: u64 = 1_000_000_001;
pub const JSON_RPC_TEST_ID: u64 = 1_000_000_002;

/// How a connection is probed and how the server's own probes are answered.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Keepalive {
    /// A WebSocket ping frame, answered by a pong frame.
    Frame,
    /// A text message answered by a text message, for servers that define their own heartbeat
    /// (OKX: `"ping"` and `"pong"`; Bybit: `{"op":"ping"}` and a JSON answer that starts
    /// `{"success":true,"ret_msg":"pong"`). An answer is a message that begins with `pong`, and is
    /// not data: a feed of this kind is a stream, so one that answers but sends nothing within its
    /// silence limit is stuck.
    Text {
        ping: &'static str,
        pong: &'static str,
    },
    /// [`Self::Text`] for a feed whose market may trade nothing for hours (Hyperliquid:
    /// `{"method":"ping"}` and an answer that starts `{"channel":"pong"`; Lighter, Bitstamp). The
    /// answer is still no data, but it is all a quiet market can show, so it counts as hearing from
    /// the peer ([`Self::heard_is_alive`]).
    QuietText {
        ping: &'static str,
        pong: &'static str,
    },
    /// The server sends a JSON object holding only `ask` and the client echoes its value under
    /// `answer` (HTX: `{"ping":1492420473027}` is answered `{"pong":1492420473027}`). The client
    /// sends nothing on its own.
    #[cfg_attr(not(test), allow(dead_code))] // until a venue that needs it lands
    EchoJson {
        ask: &'static str,
        answer: &'static str,
    },
    /// The server sends the text `ask` and the client answers the text `answer` (BingX: `Ping`,
    /// then `Pong`). The client sends nothing on its own.
    #[cfg_attr(not(test), allow(dead_code))] // until a venue that needs it lands
    AnswerText {
        ask: &'static str,
        answer: &'static str,
    },
    /// JSON-RPC heartbeat (Deribit): after connecting the client asks for a heartbeat every
    /// `interval_secs` (`public/set_heartbeat`), the server sends `heartbeat` notifications, and
    /// one of type `test_request` is answered with `public/test`. The client sends nothing else.
    JsonRpcHeartbeat { interval_secs: u32 },
}

/// What an incoming text message is to the keepalive.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Inbound {
    /// Not heartbeat traffic: the feed's data.
    Data,
    /// Heartbeat traffic that needs no answer: proof the peer is there, never data.
    Absorbed,
    /// The server's probe: send this text back. Not data either.
    Answer(String),
}

impl Keepalive {
    /// What the keepalive makes of a text message.
    pub fn inspect(&self, text: &str) -> Inbound {
        match *self {
            Self::Frame => Inbound::Data,
            Self::Text { pong, .. } | Self::QuietText { pong, .. } => {
                if text.starts_with(pong) {
                    Inbound::Absorbed
                } else {
                    Inbound::Data
                }
            }
            Self::EchoJson { ask, answer } => echo_json(text, ask, answer),
            Self::AnswerText { ask, answer } => {
                if text.trim() == ask {
                    Inbound::Answer(answer.to_owned())
                } else {
                    Inbound::Data
                }
            }
            Self::JsonRpcHeartbeat { .. } => json_rpc(text),
        }
    }

    /// Whether the peer's heartbeat traffic, which is no data, is proof enough that the feed is
    /// alive: a market nobody trades is silent for hours, and a session whose limit on data silence
    /// ended it then would reconnect over and over. True where the heartbeat is all a quiet market
    /// sends (the server-driven kinds and [`Self::QuietText`]), so the session's silence is counted
    /// from the last message of any kind. False for [`Self::Frame`] and [`Self::Text`], whose feeds
    /// are streams that show data within their limit.
    pub fn heard_is_alive(&self) -> bool {
        !matches!(self, Self::Frame | Self::Text { .. })
    }

    /// What a quiet socket is sent to make the peer answer; nothing for a keepalive where the
    /// server is the one that asks.
    pub fn probe(&self) -> Option<Message> {
        match *self {
            Self::QuietText { ping, .. } => Some(Message::Text(ping.into())),
            Self::Frame => Some(Message::Ping(Vec::new().into())),
            Self::Text { ping, .. } => Some(Message::Text(ping.into())),
            Self::EchoJson { .. } | Self::AnswerText { .. } | Self::JsonRpcHeartbeat { .. } => None,
        }
    }

    /// The message sent once after every (re)connect, before the subscriptions.
    pub fn greeting(&self) -> Option<String> {
        match *self {
            Self::JsonRpcHeartbeat { interval_secs } => Some(
                json!({
                    "jsonrpc": "2.0",
                    "id": JSON_RPC_SETUP_ID,
                    "method": "public/set_heartbeat",
                    "params": { "interval": interval_secs },
                })
                .to_string(),
            ),
            _ => None,
        }
    }
}

fn echo_json(text: &str, ask: &str, answer: &str) -> Inbound {
    if text.len() > SMALL_MESSAGE_BYTES {
        return Inbound::Data;
    }
    let Ok(Value::Object(mut object)) = serde_json::from_str(text) else {
        return Inbound::Data;
    };
    // A ping carries nothing but its value; a data message that has the key as a field is data.
    if object.len() != 1 {
        return Inbound::Data;
    }
    match object.remove(ask) {
        Some(value) if value.is_number() || value.is_string() => {
            Inbound::Answer(json!({ answer: value }).to_string())
        }
        _ => Inbound::Data,
    }
}

/// The parts of a JSON-RPC message that tell a heartbeat from data.
#[derive(Deserialize)]
struct RpcHeader {
    id: Option<u64>,
    method: Option<String>,
    params: Option<RpcParams>,
}

#[derive(Deserialize)]
struct RpcParams {
    #[serde(rename = "type")]
    kind: Option<String>,
}

fn json_rpc(text: &str) -> Inbound {
    if text.len() > SMALL_MESSAGE_BYTES {
        return Inbound::Data;
    }
    let Ok(header) = serde_json::from_str::<RpcHeader>(text) else {
        return Inbound::Data;
    };
    match (header.method.as_deref(), header.id) {
        (Some("heartbeat"), _) => match header.params.and_then(|params| params.kind).as_deref() {
            Some("test_request") => Inbound::Answer(
                json!({
                    "jsonrpc": "2.0",
                    "id": JSON_RPC_TEST_ID,
                    "method": "public/test",
                    "params": {},
                })
                .to_string(),
            ),
            _ => Inbound::Absorbed,
        },
        // The answers to the heartbeat's own requests.
        (None, Some(JSON_RPC_SETUP_ID | JSON_RPC_TEST_ID)) => Inbound::Absorbed,
        _ => Inbound::Data,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HTX: Keepalive = Keepalive::EchoJson {
        ask: "ping",
        answer: "pong",
    };
    const BINGX: Keepalive = Keepalive::AnswerText {
        ask: "Ping",
        answer: "Pong",
    };
    const DERIBIT: Keepalive = Keepalive::JsonRpcHeartbeat { interval_secs: 30 };
    const HYPERLIQUID: Keepalive = Keepalive::QuietText {
        ping: r#"{"method":"ping"}"#,
        pong: r#"{"channel":"pong""#,
    };

    fn parsed(text: &str) -> Value {
        serde_json::from_str(text).unwrap()
    }

    #[test]
    fn htx_echoes_the_servers_ping_value_and_asks_nothing_itself() {
        let Inbound::Answer(reply) = HTX.inspect(r#"{"ping":1492420473027}"#) else {
            panic!("a ping is answered");
        };
        assert_eq!(parsed(&reply), json!({"pong": 1492420473027_u64}));
        assert!(HTX.probe().is_none());
        assert!(HTX.greeting().is_none());
    }

    #[test]
    fn htx_data_that_mentions_ping_is_still_data() {
        let kline = r#"{"ch":"market.btcusdt.kline.1min","ts":1,"tick":{"id":1,"open":1.0}}"#;
        assert_eq!(HTX.inspect(kline), Inbound::Data);
        assert_eq!(HTX.inspect(r#"{"ping":1,"ch":"x"}"#), Inbound::Data);
        assert_eq!(HTX.inspect(r#"{"ping":{"a":1}}"#), Inbound::Data);
        assert_eq!(HTX.inspect("not json"), Inbound::Data);
        // A long message is not even parsed.
        let long = format!(r#"{{"ping":"{}"}}"#, "x".repeat(SMALL_MESSAGE_BYTES));
        assert_eq!(HTX.inspect(&long), Inbound::Data);
    }

    #[test]
    fn bingx_answers_the_text_ping_with_the_text_pong() {
        assert_eq!(BINGX.inspect("Ping"), Inbound::Answer("Pong".to_owned()));
        assert_eq!(BINGX.inspect(" Ping\n"), Inbound::Answer("Pong".to_owned()));
        assert_eq!(BINGX.inspect(r#"{"code":0}"#), Inbound::Data);
        assert_eq!(BINGX.inspect("Pinger"), Inbound::Data);
        assert!(BINGX.probe().is_none());
    }

    #[test]
    fn deribit_asks_for_a_heartbeat_once_after_connecting() {
        let greeting = parsed(&DERIBIT.greeting().unwrap());
        assert_eq!(greeting["jsonrpc"], "2.0");
        assert_eq!(greeting["id"], JSON_RPC_SETUP_ID);
        assert_eq!(greeting["method"], "public/set_heartbeat");
        assert_eq!(greeting["params"], json!({"interval": 30}));
        assert!(DERIBIT.probe().is_none());
    }

    #[test]
    fn deribit_answers_a_test_request_and_absorbs_the_rest_of_the_heartbeat() {
        let request = r#"{"jsonrpc":"2.0","method":"heartbeat","params":{"type":"test_request"}}"#;
        let Inbound::Answer(reply) = DERIBIT.inspect(request) else {
            panic!("a test request is answered");
        };
        let reply = parsed(&reply);
        assert_eq!(reply["method"], "public/test");
        assert_eq!(reply["id"], JSON_RPC_TEST_ID);
        assert_eq!(reply["params"], json!({}));

        let beat = r#"{"jsonrpc":"2.0","method":"heartbeat","params":{"type":"heartbeat"}}"#;
        assert_eq!(DERIBIT.inspect(beat), Inbound::Absorbed);

        // The answers to our own two requests.
        let setup = format!(r#"{{"jsonrpc":"2.0","id":{JSON_RPC_SETUP_ID},"result":"ok"}}"#);
        assert_eq!(DERIBIT.inspect(&setup), Inbound::Absorbed);
        let test = format!(
            r#"{{"jsonrpc":"2.0","id":{JSON_RPC_TEST_ID},"result":{{"version":"1.2.26"}}}}"#
        );
        assert_eq!(DERIBIT.inspect(&test), Inbound::Absorbed);
    }

    #[test]
    fn deribit_data_and_the_feeds_own_answers_are_data() {
        let ticker = r#"{"jsonrpc":"2.0","method":"subscription","params":{"channel":"ticker.BTC-PERPETUAL.100ms","data":{"last_price":1.0}}}"#;
        assert_eq!(DERIBIT.inspect(ticker), Inbound::Data);
        // The feed's subscription answer carries the feed's id, not ours.
        let subscribed = r#"{"jsonrpc":"2.0","id":7,"result":["ticker.BTC-PERPETUAL.100ms"]}"#;
        assert_eq!(DERIBIT.inspect(subscribed), Inbound::Data);
        assert_eq!(DERIBIT.inspect("not json"), Inbound::Data);
        let long = format!(
            r#"{{"jsonrpc":"2.0","method":"heartbeat","params":{{"type":"{}"}}}}"#,
            "x".repeat(SMALL_MESSAGE_BYTES)
        );
        assert_eq!(DERIBIT.inspect(&long), Inbound::Data);
    }

    #[test]
    fn hyperliquid_pings_with_json_and_its_pong_is_not_data() {
        assert_eq!(
            HYPERLIQUID.probe(),
            Some(Message::Text(r#"{"method":"ping"}"#.into()))
        );
        assert_eq!(
            HYPERLIQUID.inspect(r#"{"channel":"pong"}"#),
            Inbound::Absorbed
        );
        assert_eq!(
            HYPERLIQUID.inspect(r#"{"channel":"candle","data":{}}"#),
            Inbound::Data
        );
    }

    #[test]
    fn a_quiet_feed_counts_the_heartbeat_as_hearing_the_peer_and_a_stream_does_not() {
        for quiet in [HTX, BINGX, DERIBIT, HYPERLIQUID] {
            assert!(quiet.heard_is_alive(), "{quiet:?}");
        }
        let stream = Keepalive::Text {
            ping: "ping",
            pong: "pong",
        };
        assert!(!stream.heard_is_alive());
        assert!(!Keepalive::Frame.heard_is_alive());
        // The answer is read the same either way.
        let quiet = Keepalive::QuietText {
            ping: "ping",
            pong: "pong",
        };
        assert_eq!(quiet.inspect("pong"), stream.inspect("pong"));
        assert_eq!(quiet.probe(), stream.probe());
    }

    #[test]
    fn a_frame_keepalive_pings_with_a_frame_and_all_text_is_data() {
        assert!(matches!(Keepalive::Frame.probe(), Some(Message::Ping(_))));
        assert_eq!(Keepalive::Frame.inspect("pong"), Inbound::Data);
        assert!(Keepalive::Frame.greeting().is_none());
    }
}
