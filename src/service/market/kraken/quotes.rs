//! Kraken's websocket v2 `ticker` channel: the last trade, the rolling 24h change, volume and
//! vwap of every market asked for. One subscribe message names them all; the server sends a
//! `heartbeat` every second.

use super::Kraken;
use super::ticker::kraken_volume;
use crate::format::parse_iso8601;
use crate::model::DataSource;
use crate::model::watchlist::quote::SourceQuote;
use crate::service::net::ws::{Decode, Keepalive, Session};
use crate::service::watchlist_quotes::quote_stream::{QuoteProtocol, StreamTick};
use serde_json::{Value, json};
use std::time::Duration;

const HOST: &str = "ws.kraken.com";

fn session(products: &[String]) -> Session {
    let subscribe = json!({
        "method": "subscribe",
        "params": {"channel": "ticker", "symbol": products},
    });
    Session {
        url: format!("wss://{HOST}/v2"),
        host: HOST.to_owned(),
        port: 443,
        subscribe: vec![subscribe.to_string()],
        max_data_silence: Duration::from_secs(30),
        decode: Decode::None,
        keepalive: Keepalive::Frame,
    }
}

/// `{"channel":"ticker","type":"update","data":[{"symbol":"BTC/USD","last":64268.8,
/// "change":-114.0,"volume":4527.03,"vwap":64719.4,"timestamp":"2026-11-17T10:17:54.974820Z",…}]}`.
/// The price a day ago is the last trade less the 24h change.
fn parse_tick(text: &str) -> Option<StreamTick> {
    let value: Value = serde_json::from_str(text).ok()?;
    if value.get("channel")?.as_str()? != "ticker" {
        return None;
    }
    let row = value.get("data")?.as_array()?.first()?;
    let number = |key: &str| row.get(key)?.as_f64();
    let price = number("last").filter(|price| *price > 0.0)?;
    let product_id = row.get("symbol")?.as_str()?.to_uppercase();
    let mut quote = SourceQuote {
        price,
        previous_day_price: number("change")
            .map(|change| price - change)
            .filter(|open| *open > 0.0),
        timestamp_ms: row
            .get("timestamp")
            .and_then(Value::as_str)
            .and_then(parse_iso8601),
        ..SourceQuote::default()
    };
    if let (Some(base), Some(vwap)) = (number("volume"), number("vwap")) {
        let (volume, kind) = kraken_volume(&product_id, base, vwap);
        quote.volume_24h = Some(volume);
        quote.volume_kind = Some(kind);
    }
    Some(StreamTick {
        source: DataSource::Kraken,
        product_id,
        quote,
    })
}

impl QuoteProtocol for Kraken {
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
        let session = session(&["BTC/USD".into(), "ETH/EUR".into()]);
        assert_eq!(session.url, "wss://ws.kraken.com/v2");
        let message: Value = serde_json::from_str(&session.subscribe[0]).unwrap();
        assert_eq!(
            message,
            json!({"method": "subscribe", "params": {
                "channel": "ticker",
                "symbol": ["BTC/USD", "ETH/EUR"],
            }})
        );
    }

    #[test]
    fn parses_a_ticker_with_the_change_as_the_reference_and_ignores_the_rest() {
        let text = r#"{"channel":"ticker","type":"update","data":[{"symbol":"BTC/USD","bid":64268.3,"ask":64268.4,"last":64268.8,"volume":4527.0,"vwap":64719.4,"low":63840.0,"high":65633.5,"change":-114.0,"change_pct":-0.18,"trades":183747,"timestamp":"2026-11-17T10:17:54.974820Z"}]}"#;
        let tick = parse_tick(text).expect("tick");
        assert_eq!(tick.source, DataSource::Kraken);
        assert_eq!(tick.product_id, "BTC/USD");
        assert_eq!(tick.quote.price, 64_268.8);
        assert_eq!(tick.quote.previous_day_price, Some(64_382.8));
        assert_eq!(tick.quote.timestamp_ms, Some(1_794_910_674_974));
        assert_eq!(
            (tick.quote.volume_24h, tick.quote.volume_kind),
            (Some(4527.0 * 64_719.4), Some(VolumeKind::QuoteCurrency))
        );

        // A pair quoted in a coin reports the units traded.
        let coin = text.replace("BTC/USD", "ETH/BTC");
        assert_eq!(
            parse_tick(&coin).unwrap().quote.volume_kind,
            Some(VolumeKind::Base)
        );

        for ignored in [
            r#"{"channel":"heartbeat"}"#,
            r#"{"channel":"status","type":"update","data":[{"system":"online"}]}"#,
            r#"{"method":"subscribe","result":{"channel":"ticker","symbol":"BTC/USD"},"success":true}"#,
            r#"{"error":"Currency pair not supported XBT/USD","method":"subscribe","success":false}"#,
            r#"{"channel":"ticker","type":"update","data":[{"symbol":"BTC/USD","last":0.0}]}"#,
            r#"{"channel":"ticker","type":"update","data":[]}"#,
            "not json",
        ] {
            assert!(parse_tick(ignored).is_none(), "{ignored}");
        }
    }
}
