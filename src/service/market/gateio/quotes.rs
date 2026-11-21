//! Gate.io's `spot.tickers` channel on the v4 socket: the last trade, the 24h change in percent,
//! the volume and the turnover of a market. A request that names a market Gate.io does not know
//! is refused whole, so each market is its own request. The socket is kept alive with protocol
//! pings, which the server answers.

use super::Gate;
use super::ticker::gateio_ticker;
use crate::model::DataSource;
use crate::service::net::ws::{Keepalive, Session};
use crate::service::watchlist_quotes::quote_stream::{QuoteProtocol, StreamTick};
use serde_json::{Value, json};
use std::time::Duration;

const HOST: &str = "api.gateio.ws";

fn session(products: &[String]) -> Session {
    Session {
        url: format!("wss://{HOST}/ws/v4/"),
        host: HOST.to_owned(),
        port: 443,
        subscribe: products
            .iter()
            .map(|product| {
                json!({"channel": "spot.tickers", "event": "subscribe", "payload": [product]})
                    .to_string()
            })
            .collect(),
        // A thin market's ticker is quiet for minutes: the answers to the pings say the socket is
        // alive, and REST refreshes a product the socket stays quiet about.
        max_data_silence: Duration::from_secs(600),
        keepalive: Keepalive::Frame,
    }
}

/// `{"time_ms":1794918485791,"channel":"spot.tickers","event":"update","result":{
/// "currency_pair":"BTC_USDT","last":"65057.9","change_percentage":"1.5527","base_volume":"…",
/// "quote_volume":"…",…}}`.
fn parse_tick(text: &str) -> Option<StreamTick> {
    let value: Value = serde_json::from_str(text).ok()?;
    if value.get("channel")?.as_str()? != "spot.tickers"
        || value.get("event")?.as_str()? != "update"
    {
        return None;
    }
    let (product_id, mut quote) = gateio_ticker(value.get("result")?)?;
    quote.timestamp_ms = value.get("time_ms").and_then(Value::as_i64);
    Some(StreamTick {
        source: DataSource::Gate,
        product_id,
        quote,
    })
}

impl QuoteProtocol for Gate {
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
        let session = session(&["BTC_USDT".into(), "ETH_BTC".into()]);
        assert_eq!(session.url, "wss://api.gateio.ws/ws/v4/");
        let messages: Vec<Value> = session
            .subscribe
            .iter()
            .map(|message| serde_json::from_str(message).unwrap())
            .collect();
        assert_eq!(
            messages,
            [
                json!({"channel": "spot.tickers", "event": "subscribe", "payload": ["BTC_USDT"]}),
                json!({"channel": "spot.tickers", "event": "subscribe", "payload": ["ETH_BTC"]}),
            ]
        );
    }

    #[test]
    fn parses_a_ticker_with_the_price_a_day_ago_from_the_change_and_ignores_the_rest() {
        let text = r#"{"time":1794918485,"time_ms":1794918485791,"channel":"spot.tickers","event":"update","result":{"currency_pair":"BTC_USDT","last":"65100","lowest_ask":"65100.1","highest_bid":"65100","change_percentage":"2","base_volume":"14040.69","quote_volume":"908513512.9","high_24h":"65683.5","low_24h":"63908.7"}}"#;
        let tick = parse_tick(text).expect("tick");
        assert_eq!(tick.source, DataSource::Gate);
        assert_eq!(tick.product_id, "BTC_USDT");
        assert_eq!(tick.quote.price, 65_100.0);
        // 2 % up on the day: a day ago it was 65,100 / 1.02.
        let previous = tick.quote.previous_day_price.expect("a reference");
        assert!((previous - 63_823.529).abs() < 0.001, "{previous}");
        assert_eq!(tick.quote.timestamp_ms, Some(1_794_918_485_791));
        assert_eq!(
            (tick.quote.volume_24h, tick.quote.volume_kind),
            (Some(908_513_512.9), Some(VolumeKind::QuoteCurrency))
        );

        for ignored in [
            r#"{"channel":"spot.tickers","event":"subscribe","payload":["BTC_USDT"],"result":{"status":"success"}}"#,
            r#"{"channel":"spot.tickers","event":"subscribe","payload":["NOPE_USDT"],"error":{"code":2,"message":"unknown currency pair"},"result":{"status":"fail"}}"#,
            r#"{"channel":"spot.pong","event":"","result":null}"#,
            r#"{"channel":"spot.candlesticks","event":"update","result":{"n":"1m_BTC_USDT"}}"#,
            r#"{"channel":"spot.tickers","event":"update","result":{"currency_pair":"BTC_USDT","last":"0"}}"#,
            "not json",
        ] {
            assert!(parse_tick(ignored).is_none(), "{ignored}");
        }
    }
}
