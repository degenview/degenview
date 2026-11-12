//! One shared Coinbase `ticker` socket for every product a visible watchlist shows.
//! Coinbase has no candle channel, but its ticker carries the last trade plus the 24h
//! open and volume, which is all a row needs.

use crate::model::watchlist::quote::SourceQuote;
use serde_json::{Value, json};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::time::Duration;

const FEED_URL: &str = "wss://ws-feed.exchange.coinbase.com";

/// One trade from the ticker channel.
#[derive(Clone, Debug, PartialEq)]
pub struct CoinbaseTick {
    pub product_id: String,
    pub price: f64,
    pub open_24h: Option<f64>,
    pub volume_24h: Option<f64>,
    pub time_ms: Option<i64>,
}

impl CoinbaseTick {
    /// The tick as a provider answer: change measured against the rolling 24h open.
    pub fn as_source_quote(&self) -> SourceQuote {
        let mut quote = SourceQuote {
            price: self.price,
            previous_day_price: self.open_24h.filter(|open| *open > 0.0),
            timestamp_ms: self.time_ms,
            ..SourceQuote::default()
        };
        if let Some(base) = self.volume_24h {
            let (volume, kind) =
                super::provider::coinbase_volume(&self.product_id, base, self.price);
            quote.volume_24h = Some(volume);
            quote.volume_kind = Some(kind);
        }
        quote
    }
}

/// A live connection; dropping it closes the socket.
pub struct CoinbaseSocket {
    shutdown: Arc<AtomicBool>,
    products: Vec<String>,
}

impl CoinbaseSocket {
    pub fn connect(products: Vec<String>, sender: Sender<CoinbaseTick>) -> Self {
        let shutdown = Arc::new(AtomicBool::new(false));
        let thread_shutdown = Arc::clone(&shutdown);
        let thread_products = products.clone();
        std::thread::spawn(move || run(&thread_products, &sender, &thread_shutdown));
        Self { shutdown, products }
    }

    pub fn products(&self) -> &[String] {
        &self.products
    }
}

impl Drop for CoinbaseSocket {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Release);
    }
}

fn run(products: &[String], sender: &Sender<CoinbaseTick>, shutdown: &AtomicBool) {
    let mut backoff = crate::service::net::ws::Backoff::new();
    while !shutdown.load(Ordering::Acquire) {
        if let Ok(mut socket) =
            crate::service::net::ws::connect(FEED_URL, "ws-feed.exchange.coinbase.com", 443)
        {
            let subscribe = subscription(products);
            if socket
                .send(tungstenite::Message::Text(subscribe.into()))
                .is_ok()
            {
                let mut health = crate::service::net::ws::Health::new(Duration::from_secs(15));
                while !shutdown.load(Ordering::Acquire) {
                    match health.read(&mut socket) {
                        Ok(Some(tungstenite::Message::Text(text))) => {
                            backoff.reset();
                            if let Some(tick) = parse_tick(&text)
                                && sender.send(tick).is_err()
                            {
                                return;
                            }
                        }
                        Ok(Some(tungstenite::Message::Close(_))) => break,
                        Ok(_) => {}
                        Err(_) => break,
                    }
                }
            }
        }
        backoff.wait(shutdown);
    }
}

pub fn subscription(products: &[String]) -> String {
    json!({
        "type": "subscribe",
        "product_ids": products,
        "channels": ["ticker", "heartbeat"],
    })
    .to_string()
}

/// `{"type":"ticker","product_id":"BTC-USD","price":"67000.1","open_24h":"66000","volume_24h":"123.4","time":"…Z"}`
pub fn parse_tick(text: &str) -> Option<CoinbaseTick> {
    let value: Value = serde_json::from_str(text).ok()?;
    if value.get("type")?.as_str()? != "ticker" {
        return None;
    }
    let number = |key: &str| -> Option<f64> { value.get(key)?.as_str()?.parse().ok() };
    let price = number("price").filter(|price| *price > 0.0)?;
    Some(CoinbaseTick {
        product_id: value.get("product_id")?.as_str()?.to_uppercase(),
        price,
        open_24h: number("open_24h"),
        volume_24h: number("volume_24h"),
        time_ms: value
            .get("time")
            .and_then(Value::as_str)
            .and_then(super::provider::parse_iso_ms),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::watchlist::quote::VolumeKind;

    #[test]
    fn parses_a_ticker_message_and_ignores_the_rest() {
        let text = r#"{"type":"ticker","product_id":"btc-usd","price":"67000.1","open_24h":"66000","volume_24h":"10","time":"2024-01-15T15:00:00.250000Z"}"#;
        let tick = parse_tick(text).unwrap();
        assert_eq!(tick.product_id, "BTC-USD");
        assert_eq!(tick.price, 67_000.1);
        assert_eq!(tick.open_24h, Some(66_000.0));
        assert_eq!(tick.time_ms, Some(1_705_330_800_250));

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
    fn a_tick_becomes_a_quote_with_the_24h_open_as_reference() {
        let tick = CoinbaseTick {
            product_id: "ETH-USD".to_owned(),
            price: 2_000.0,
            open_24h: Some(1_000.0),
            volume_24h: Some(5.0),
            time_ms: Some(7),
        };
        let quote = tick.as_source_quote();
        assert_eq!(quote.previous_day_price, Some(1_000.0));
        assert_eq!(
            (quote.volume_24h, quote.volume_kind),
            (Some(10_000.0), Some(VolumeKind::QuoteCurrency))
        );
        let no_open = CoinbaseTick {
            open_24h: Some(0.0),
            volume_24h: None,
            ..tick
        };
        let quote = no_open.as_source_quote();
        assert_eq!(
            quote.previous_day_price, None,
            "a zero open is no reference"
        );
        assert_eq!(quote.volume_24h, None);
    }

    #[test]
    fn subscribes_to_ticker_and_heartbeat() {
        let message: Value =
            serde_json::from_str(&subscription(&["BTC-USD".into(), "ETH-USD".into()])).unwrap();
        assert_eq!(message["type"], "subscribe");
        assert_eq!(message["product_ids"][1], "ETH-USD");
        assert_eq!(message["channels"], json!(["ticker", "heartbeat"]));
    }
}
