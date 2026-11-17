//! MEXC's `spot@public.kline.v3.api.pb@<SYMBOL>@<interval>` topics on its spot socket, which speaks
//! Protocol Buffers only (the JSON topics are refused). A connection takes at most 30
//! subscriptions, so the markets are spread over as many connections as that needs, one topic per
//! subscribe request so that a market it does not list refuses only itself. The socket is kept
//! alive with `{"method":"PING"}`.

use crate::model::market::bucket::next_bucket_start_ms;
use crate::model::{Kline, Timeframe};
use crate::service::market::mexc::{socket_interval, symbol_id};
use crate::service::net::protobuf::field;
use crate::service::net::ws::{Frame, Keepalive, Session};
use serde_json::json;
use std::time::Duration;

const HOST: &str = "wbs-api.mexc.com";

/// The most topics one connection takes.
const TOPICS_PER_CONNECTION: usize = 30;

const KLINE_CHANNEL: &str = "spot@public.kline.v3.api.pb@";

/// The wrapper's field for the kline body, and the body's own fields.
const WRAPPER_CHANNEL: u32 = 1;
const WRAPPER_SYMBOL: u32 = 3;
const WRAPPER_KLINE: u32 = 308;
const KLINE_WINDOW_START: u32 = 2;
const KLINE_OPEN: u32 = 3;
const KLINE_CLOSE: u32 = 4;
const KLINE_HIGH: u32 = 5;
const KLINE_LOW: u32 = 6;
const KLINE_VOLUME: u32 = 7;

/// The connections for `symbols`; `None` when MEXC has no interval of `base`.
pub(super) fn sessions(symbols: &[String], base: Timeframe) -> Option<Vec<Session>> {
    let interval = socket_interval(base)?;
    let topics: Vec<String> = symbols
        .iter()
        .map(|symbol| format!("{KLINE_CHANNEL}{}@{interval}", symbol_id(symbol)))
        .collect();
    Some(
        topics
            .chunks(TOPICS_PER_CONNECTION)
            .map(|chunk| Session {
                url: format!("wss://{HOST}/ws"),
                host: HOST.to_owned(),
                port: 443,
                subscribe: chunk
                    .iter()
                    .map(|topic| json!({"method": "SUBSCRIPTION", "params": [topic]}).to_string())
                    .collect(),
                // A thin market's candle is quiet for minutes: the pong says the socket is alive.
                max_data_silence: Duration::from_secs(600),
                keepalive: Keepalive::Text {
                    ping: r#"{"method":"PING"}"#,
                    pong: r#"{"id":0,"code":0,"msg":"PONG""#,
                },
            })
            .collect(),
    )
}

/// A binary push: the wrapper holds the channel (field 1), the symbol (3) and, for a kline, the
/// body (308) with the window start in seconds (2), the prices as strings (open 3, close 4, high 5,
/// low 6) and the volume in the base asset (7). Subscription answers are text and carry no candle.
pub(super) fn parse_kline(frame: Frame<'_>, base: Timeframe) -> Option<(String, Kline)> {
    let Frame::Binary(message) = frame else {
        return None;
    };
    field(message, WRAPPER_CHANNEL)?
        .text()?
        .starts_with(KLINE_CHANNEL)
        .then_some(())?;
    let symbol = field(message, WRAPPER_SYMBOL)?.text()?.to_uppercase();
    let body = field(message, WRAPPER_KLINE)?.bytes()?;
    let price = |number: u32| field(body, number)?.text()?.parse::<f64>().ok();
    let open_time = i64::try_from(field(body, KLINE_WINDOW_START)?.varint()?).ok()? * 1_000;
    Some((
        symbol,
        Kline {
            open_time,
            close_time: next_bucket_start_ms(base, open_time) - 1,
            open: price(KLINE_OPEN)?,
            high: price(KLINE_HIGH)?,
            low: price(KLINE_LOW)?,
            close: price(KLINE_CLOSE)?,
            volume: price(KLINE_VOLUME)?,
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::net::protobuf::encode::{nested, text, varint};

    /// A kline push as MEXC sends it (captured from the live socket, one minute of BTCUSDT).
    fn kline_frame(channel: &str, symbol: &str) -> Vec<u8> {
        let mut body = text(1, "Min1");
        body.extend(varint(2, 1_794_920_160));
        body.extend(text(3, "64983.8"));
        body.extend(text(4, "64960.6"));
        body.extend(text(5, "64983.8"));
        body.extend(text(6, "64958.32"));
        body.extend(text(7, "3.41012162"));
        body.extend(text(8, "221524.57"));
        body.extend(varint(9, 1_794_920_220));
        let mut message = text(1, channel);
        message.extend(text(3, symbol));
        message.extend(text(4, "2fb942154ef44a4ab2ef98c8afb6a4a7"));
        message.extend(varint(5, 1_794_920_181_910));
        message.extend(nested(308, &body));
        message
    }

    #[test]
    fn markets_are_spread_over_connections_of_thirty_topics_one_request_each() {
        let symbols: Vec<String> = (0..65).map(|index| format!("COIN{index}USDT")).collect();
        let connections = sessions(&symbols, Timeframe::MINUTE_1).unwrap();
        let counts: Vec<usize> = connections
            .iter()
            .map(|session| session.subscribe.len())
            .collect();
        assert_eq!(counts, [30, 30, 5]);
        assert_eq!(connections[0].url, "wss://wbs-api.mexc.com/ws");
        let first: serde_json::Value = serde_json::from_str(&connections[0].subscribe[0]).unwrap();
        assert_eq!(
            first,
            json!({"method": "SUBSCRIPTION", "params": ["spot@public.kline.v3.api.pb@COIN0USDT@Min1"]})
        );
        let hours = sessions_for("btc/usdt", Timeframe::HOUR_4);
        assert!(hours.contains("@BTCUSDT@Hour4"), "{hours}");
        assert!(sessions(&[], Timeframe::MINUTE_1).unwrap().is_empty());
    }

    fn sessions_for(symbol: &str, base: Timeframe) -> String {
        sessions(&[symbol.to_owned()], base).unwrap()[0].subscribe[0].clone()
    }

    #[test]
    fn a_length_mexc_has_no_topic_for_has_no_connection() {
        for text in ["3m", "2h", "6h", "12h", "3M"] {
            let timeframe = Timeframe::parse(text).unwrap();
            assert!(sessions(&["BTCUSDT".into()], timeframe).is_none(), "{text}");
        }
    }

    #[test]
    fn a_ping_is_answered_by_a_pong_that_a_subscription_answer_is_not() {
        let session = &sessions(&["BTCUSDT".into()], Timeframe::MINUTE_1).unwrap()[0];
        let Keepalive::Text { ping, pong } = session.keepalive else {
            panic!("a text keepalive");
        };
        assert_eq!(ping, r#"{"method":"PING"}"#);
        assert!(r#"{"id":0,"code":0,"msg":"PONG"}"#.starts_with(pong));
        assert!(
            !r#"{"id":0,"code":0,"msg":"spot@public.kline.v3.api.pb@BTCUSDT@Min1"}"#
                .starts_with(pong)
        );
    }

    #[test]
    fn parses_a_kline_push_with_the_close_of_its_bucket() {
        let frame = kline_frame("spot@public.kline.v3.api.pb@BTCUSDT@Min1", "BTCUSDT");
        let (symbol, kline) =
            parse_kline(Frame::Binary(&frame), Timeframe::MINUTE_1).expect("parse");
        assert_eq!(symbol, "BTCUSDT");
        assert_eq!(kline.open_time, 1_794_920_160_000);
        assert_eq!(kline.close_time, 1_794_920_219_999);
        assert_eq!(
            (kline.open, kline.high, kline.low, kline.close),
            (64_983.8, 64_983.8, 64_958.32, 64_960.6)
        );
        // The base volume, not the quote volume (the eighth field).
        assert_eq!(kline.volume, 3.41012162);
    }

    #[test]
    fn answers_other_topics_text_and_broken_frames_are_not_candles() {
        let ticker = kline_frame("spot@public.miniTicker.v3.api.pb@BTCUSDT@UTC+8", "BTCUSDT");
        assert!(parse_kline(Frame::Binary(&ticker), Timeframe::MINUTE_1).is_none());
        assert!(
            parse_kline(
                Frame::Text(r#"{"id":0,"code":0,"msg":"PONG"}"#),
                Timeframe::MINUTE_1
            )
            .is_none()
        );
        assert!(parse_kline(Frame::Binary(&[]), Timeframe::MINUTE_1).is_none());
        assert!(parse_kline(Frame::Binary(&[0x0a, 0x7f, b'x']), Timeframe::MINUTE_1).is_none());
        // A kline with a price that is not a number.
        let mut frame = text(1, "spot@public.kline.v3.api.pb@BTCUSDT@Min1");
        frame.extend(text(3, "BTCUSDT"));
        let mut body = varint(2, 1);
        body.extend(text(3, "x"));
        frame.extend(nested(308, &body));
        assert!(parse_kline(Frame::Binary(&frame), Timeframe::MINUTE_1).is_none());
    }
}
