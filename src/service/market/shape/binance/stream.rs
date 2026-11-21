//! The combined stream socket: a topic is `<symbol>@kline_<interval>`, a message is
//! `{"stream": topic, "data": …}`. The topics go in the URL; another topic is no data.

use super::Config;
use crate::model::{Kline, Timeframe};
use crate::service::net::ws::{Decode, Keepalive, Session};
use serde_json::Value;

impl Config {
    /// The connections that stream the forming candle of length `base` for `symbols`, one per
    /// `max_streams` markets: the topics are in the URL, nothing is sent.
    pub(in crate::service) fn kline_sessions(
        &self,
        symbols: &[String],
        base: Timeframe,
    ) -> Vec<Session> {
        let interval = base.interval();
        symbols
            .chunks(self.max_streams)
            .map(|chunk| {
                let topics = chunk
                    .iter()
                    .map(|symbol| format!("{}@kline_{interval}", symbol.to_lowercase()))
                    .collect::<Vec<_>>()
                    .join("/");
                self.session(
                    format!("wss://{}/stream?streams={topics}", self.stream_host),
                    Vec::new(),
                )
            })
            .collect()
    }

    fn session(&self, url: String, subscribe: Vec<String>) -> Session {
        Session {
            url,
            host: self.stream_host.to_owned(),
            port: 443,
            subscribe,
            max_data_silence: self.max_silence,
            decode: Decode::None,
            keepalive: Keepalive::Frame,
        }
    }
}

/// The topic's kind and symbol of a message, and its data: `btcusdt@kline_1h` is `("kline_1h",
/// "BTCUSDT")`. Nothing for a message without a topic (the acknowledgement of a `SUBSCRIBE`).
fn topic(value: &Value) -> Option<(&str, String, &Value)> {
    let (symbol, kind) = value.get("stream")?.as_str()?.split_once('@')?;
    Some((kind, symbol.to_uppercase(), value.get("data")?))
}

/// `{"stream":"btcusdt@kline_1h","data":{"k":{"t","o","h","l","c","v",…}}}`.
pub(in crate::service) fn parse_kline(text: &str) -> Option<(String, Kline)> {
    let value: Value = serde_json::from_str(text).ok()?;
    let (kind, symbol, data) = topic(&value)?;
    if !kind.starts_with("kline_") {
        return None;
    }
    let k = data.get("k")?;
    let t = k.get("t").and_then(Value::as_i64)?;
    // `T` is the bar's last millisecond, as in the REST rows. Without it the forming bar would read
    // as already closed (`close_time <= now`) and Pine would rebuild on every tick.
    let close_time = k.get("T").and_then(Value::as_i64).unwrap_or(t);
    let open = k.get("o").and_then(Value::as_str)?.parse().ok()?;
    let high = k.get("h").and_then(Value::as_str)?.parse().ok()?;
    let low = k.get("l").and_then(Value::as_str)?.parse().ok()?;
    let close = k.get("c").and_then(Value::as_str)?.parse().ok()?;
    let volume = k.get("v").and_then(Value::as_str)?.parse().ok()?;
    Some((
        symbol,
        Kline {
            open_time: t,
            close_time,
            open,
            high,
            low,
            close,
            volume,
        },
    ))
}
