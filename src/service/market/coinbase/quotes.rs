//! Coinbase's `ticker` channel. It has no candle channel, but the ticker carries the last trade
//! plus the 24h open and volume.

use super::Coinbase;
use super::ticker::coinbase_volume;
use crate::model::DataSource;
use crate::model::watchlist::quote::SourceQuote;
use crate::service::net::ws::{Keepalive, Session};
use crate::service::watchlist_quotes::quote_stream::{QuoteProtocol, StreamTick};
use serde_json::{Value, json};
use std::time::Duration;

const HOST: &str = "ws-feed.exchange.coinbase.com";

fn session(products: &[String]) -> Session {
    Session {
        url: format!("wss://{HOST}"),
        host: HOST.to_owned(),
        port: 443,
        subscribe: vec![subscription(products)],
        max_data_silence: Duration::from_secs(15),
        keepalive: Keepalive::Frame,
    }
}

fn subscription(products: &[String]) -> String {
    json!({
        "type": "subscribe",
        "product_ids": products,
        "channels": ["ticker", "heartbeat"],
    })
    .to_string()
}

/// `{"type":"ticker","product_id":"BTC-USD","price":"67000.1","open_24h":"66000","volume_24h":"123.4","time":"…Z"}`
/// as a quote with the change measured against the rolling 24h open.
fn parse_tick(text: &str) -> Option<StreamTick> {
    let value: Value = serde_json::from_str(text).ok()?;
    if value.get("type")?.as_str()? != "ticker" {
        return None;
    }
    let number = |key: &str| -> Option<f64> { value.get(key)?.as_str()?.parse().ok() };
    let price = number("price").filter(|price| *price > 0.0)?;
    let product_id = value.get("product_id")?.as_str()?.to_uppercase();
    let mut quote = SourceQuote {
        price,
        previous_day_price: number("open_24h").filter(|open| *open > 0.0),
        timestamp_ms: value
            .get("time")
            .and_then(Value::as_str)
            .and_then(crate::format::parse_iso8601),
        ..SourceQuote::default()
    };
    if let Some(base) = number("volume_24h") {
        let (volume, kind) = coinbase_volume(&product_id, base, price);
        quote.volume_24h = Some(volume);
        quote.volume_kind = Some(kind);
    }
    Some(StreamTick {
        source: DataSource::Coinbase,
        product_id,
        quote,
    })
}

impl QuoteProtocol for Coinbase {
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
    fn parses_a_ticker_message_and_ignores_the_rest() {
        let text = r#"{"type":"ticker","product_id":"btc-usd","price":"67000.1","open_24h":"66000","volume_24h":"10","time":"2024-01-15T15:00:00.250000Z"}"#;
        let tick = parse_tick(text).unwrap();
        assert_eq!(tick.source, DataSource::Coinbase);
        assert_eq!(tick.product_id, "BTC-USD");
        assert_eq!(tick.quote.price, 67_000.1);
        assert_eq!(tick.quote.previous_day_price, Some(66_000.0));
        assert_eq!(tick.quote.timestamp_ms, Some(1_705_330_800_250));

        for ignored in [
            r#"{"type":"heartbeat","product_id":"BTC-USD"}"#,
            r#"{"type":"subscriptions","channels":[]}"#,
            r#"{"type":"ticker","product_id":"BTC-USD","price":"0"}"#,
            r#"{"type":"ticker","price":"1"}"#,
            "not json",
        ] {
            assert!(parse_tick(ignored).is_none(), "{ignored}");
        }
    }

    #[test]
    fn a_tick_is_a_quote_with_the_24h_open_as_reference() {
        let text = |open: &str, volume: &str| {
            format!(
                r#"{{"type":"ticker","product_id":"ETH-USD","price":"2000","open_24h":"{open}","volume_24h":"{volume}"}}"#
            )
        };
        let quote = parse_tick(&text("1000", "5")).unwrap().quote;
        assert_eq!(quote.previous_day_price, Some(1_000.0));
        assert_eq!(
            (quote.volume_24h, quote.volume_kind),
            (Some(10_000.0), Some(VolumeKind::QuoteCurrency))
        );
        let quote = parse_tick(&text("0", "x")).unwrap().quote;
        assert_eq!(
            quote.previous_day_price, None,
            "a zero open is no reference"
        );
        assert_eq!(quote.volume_24h, None);
    }

    #[test]
    fn subscribes_to_ticker_and_heartbeat() {
        let session = session(&["BTC-USD".into(), "ETH-USD".into()]);
        let message: Value = serde_json::from_str(&session.subscribe[0]).unwrap();
        assert_eq!(message["type"], "subscribe");
        assert_eq!(message["product_ids"][1], "ETH-USD");
        assert_eq!(message["channels"], json!(["ticker", "heartbeat"]));
    }
}
