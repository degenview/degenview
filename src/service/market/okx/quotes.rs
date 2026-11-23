//! OKX's `tickers` channel, on the `public` endpoint: the last trade plus the 24h open and volume
//! of every market asked for. A single subscribe message names them all; the socket is kept alive
//! with the text `"ping"`.

use super::Okx;
use super::ticker::okx_ticker;
use crate::model::DataSource;
use crate::service::net::ws::{Keepalive, Session};
use crate::service::watchlist_quotes::quote_stream::{QuoteProtocol, StreamTick};
use serde_json::{Value, json};
use std::time::Duration;

const HOST: &str = "ws.okx.com";

fn session(products: &[String]) -> Session {
    let args: Vec<Value> = products
        .iter()
        .map(|product| json!({"channel": "tickers", "instId": product}))
        .collect();
    Session {
        url: format!("wss://{HOST}/ws/v5/public"),
        host: HOST.to_owned(),
        port: 443,
        subscribe: vec![json!({"op": "subscribe", "args": args}).to_string()],
        // A thin market's ticker is quiet for minutes: the heartbeat answers say the socket is
        // alive, and REST refreshes a product the socket stays quiet about.
        max_data_silence: Duration::from_secs(600),
        keepalive: Keepalive::Text {
            ping: "ping",
            pong: "pong",
        },
    }
}

/// `{"arg":{"channel":"tickers","instId":"BTC-USDT"},"data":[{"instId":"BTC-USDT","last":"…",
/// "open24h":"…","vol24h":"…","volCcy24h":"…","ts":"…"}]}`.
fn parse_tick(text: &str) -> Option<StreamTick> {
    let value: Value = serde_json::from_str(text).ok()?;
    let (product_id, quote) = okx_ticker(value.get("data")?.as_array()?.first()?)?;
    Some(StreamTick {
        source: DataSource::Okx,
        product_id,
        quote,
    })
}

impl QuoteProtocol for Okx {
    fn session(&self, products: &[String]) -> Session {
        session(products)
    }

    fn parse(&self, text: &str) -> Option<StreamTick> {
        parse_tick(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::watchlist::quote::VolumeKind;

    #[test]
    fn one_message_subscribes_every_product() {
        let session = session(&["BTC-USDT".into(), "ETH-USDT".into()]);
        assert_eq!(session.url, "wss://ws.okx.com/ws/v5/public");
        let message: Value = serde_json::from_str(&session.subscribe[0]).unwrap();
        assert_eq!(message["op"], "subscribe");
        assert_eq!(
            message["args"],
            json!([
                {"channel": "tickers", "instId": "BTC-USDT"},
                {"channel": "tickers", "instId": "ETH-USDT"},
            ])
        );
    }

    #[test]
    fn parses_a_ticker_push_and_ignores_the_rest() {
        let text = r#"{"arg":{"channel":"tickers","instId":"BTC-USDT"},"data":[{"instType":"SPOT","instId":"BTC-USDT","last":"64370.7","open24h":"64561.6","vol24h":"10021.2","volCcy24h":"648492231.3","ts":"1794904542764"}]}"#;
        let tick = parse_tick(text).expect("tick");
        assert_eq!(tick.source, DataSource::Okx);
        assert_eq!(tick.product_id, "BTC-USDT");
        assert_eq!(tick.quote.price, 64_370.7);
        assert_eq!(tick.quote.previous_day_price, Some(64_561.6));
        assert_eq!(tick.quote.timestamp_ms, Some(1_794_904_542_764));
        assert_eq!(
            (tick.quote.volume_24h, tick.quote.volume_kind),
            (Some(648_492_231.3), Some(VolumeKind::QuoteCurrency))
        );

        for ignored in [
            r#"{"event":"subscribe","arg":{"channel":"tickers","instId":"BTC-USDT"},"connId":"a"}"#,
            r#"{"event":"error","msg":"Wrong URL or channel","code":"60018"}"#,
            r#"{"arg":{"channel":"tickers","instId":"BTC-USDT"},"data":[{"instId":"BTC-USDT","last":"0"}]}"#,
            r#"{"arg":{"channel":"tickers","instId":"BTC-USDT"},"data":[]}"#,
            "not json",
        ] {
            assert!(parse_tick(ignored).is_none(), "{ignored}");
        }
    }
}
