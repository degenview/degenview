//! Bybit's `tickers.<SYMBOL>` topics on the public spot socket: the last trade, the price 24 hours
//! ago, the volume and the turnover of every market asked for. Each topic is its own subscribe
//! request (a request takes at most ten, and one unknown symbol refuses the whole of it); the
//! socket is kept alive with `{"op":"ping"}`.

use super::Bybit;
use super::KEEPALIVE;
use super::ticker::bybit_ticker;
use crate::model::DataSource;
use crate::service::net::ws::{Decode, Session};
use crate::service::watchlist_quotes::quote_stream::{QuoteProtocol, StreamTick};
use serde_json::{Value, json};
use std::time::Duration;

const HOST: &str = "stream.bybit.com";

fn session(products: &[String]) -> Session {
    Session {
        url: format!("wss://{HOST}/v5/public/spot"),
        host: HOST.to_owned(),
        port: 443,
        subscribe: products
            .iter()
            .map(|product| {
                json!({"op": "subscribe", "args": [format!("tickers.{product}")]}).to_string()
            })
            .collect(),
        // A thin market's ticker is quiet for minutes: the pong says the socket is alive, and REST
        // refreshes a product the socket stays quiet about.
        max_data_silence: Duration::from_secs(600),
        decode: Decode::None,
        keepalive: KEEPALIVE,
    }
}

/// `{"topic":"tickers.BTCUSDT","ts":1794913918523,"type":"snapshot","data":{"symbol":"BTCUSDT",
/// "lastPrice":"64303.3","prevPrice24h":"64675.6","volume24h":"…","turnover24h":"…",…}}`.
fn parse_tick(text: &str) -> Option<StreamTick> {
    let value: Value = serde_json::from_str(text).ok()?;
    value.get("topic")?.as_str()?.strip_prefix("tickers.")?;
    let (product_id, mut quote) = bybit_ticker(value.get("data")?)?;
    quote.timestamp_ms = value.get("ts").and_then(Value::as_i64);
    Some(StreamTick {
        source: DataSource::Bybit,
        product_id,
        quote,
    })
}

impl QuoteProtocol for Bybit {
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
    fn every_product_is_its_own_subscribe_request() {
        let session = session(&["BTCUSDT".into(), "ETHUSDC".into()]);
        assert_eq!(session.url, "wss://stream.bybit.com/v5/public/spot");
        let messages: Vec<Value> = session
            .subscribe
            .iter()
            .map(|message| serde_json::from_str(message).unwrap())
            .collect();
        assert_eq!(
            messages,
            [
                json!({"op": "subscribe", "args": ["tickers.BTCUSDT"]}),
                json!({"op": "subscribe", "args": ["tickers.ETHUSDC"]}),
            ]
        );
    }

    #[test]
    fn parses_a_ticker_with_the_price_a_day_ago_and_ignores_the_rest() {
        let text = r#"{"topic":"tickers.BTCUSDT","ts":1794913918523,"type":"snapshot","cs":1,"data":{"symbol":"BTCUSDT","lastPrice":"64303.3","highPrice24h":"65691.2","lowPrice24h":"63906.1","prevPrice24h":"64675.6","volume24h":"12400.5","turnover24h":"801846107.8","price24hPcnt":"-0.0058","usdIndexPrice":"64231.4"}}"#;
        let tick = parse_tick(text).expect("tick");
        assert_eq!(tick.source, DataSource::Bybit);
        assert_eq!(tick.product_id, "BTCUSDT");
        assert_eq!(tick.quote.price, 64_303.3);
        assert_eq!(tick.quote.previous_day_price, Some(64_675.6));
        assert_eq!(tick.quote.timestamp_ms, Some(1_794_913_918_523));
        assert_eq!(
            (tick.quote.volume_24h, tick.quote.volume_kind),
            (Some(801_846_107.8), Some(VolumeKind::QuoteCurrency))
        );

        for ignored in [
            r#"{"success":true,"ret_msg":"subscribe","op":"subscribe"}"#,
            r#"{"success":true,"ret_msg":"pong","op":"ping"}"#,
            r#"{"topic":"kline.1.BTCUSDT","data":[]}"#,
            r#"{"topic":"tickers.BTCUSDT","data":{"symbol":"BTCUSDT","lastPrice":"0"}}"#,
            r#"{"topic":"tickers.BTCUSDT","data":[]}"#,
            "not json",
        ] {
            assert!(parse_tick(ignored).is_none(), "{ignored}");
        }
    }
}
