//! One shared ticker socket per streamed source, for every market a visible watchlist shows of
//! it. The ticker carries the last trade plus the 24h open and volume, which is all a row needs.

mod bybit;
mod coinbase;
mod gateio;
mod kraken;
mod okx;

use crate::model::DataSource;
use crate::model::watchlist::quote::SourceQuote;
use crate::service::net::ws::Session;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;

/// One update from a ticker socket: the market as the source names it, and its price.
#[derive(Clone, Debug, PartialEq)]
pub struct StreamTick {
    pub source: DataSource,
    pub product_id: String,
    pub quote: SourceQuote,
}

/// Turns one text message of a source's stream into a tick.
type Parser = fn(&str) -> Option<StreamTick>;

/// A live connection; dropping it closes the socket.
pub struct QuoteSocket {
    shutdown: Arc<AtomicBool>,
    products: Vec<String>,
}

impl QuoteSocket {
    /// Opens the ticker of `products` on `source`; `None` when the source has no ticker socket.
    pub fn connect(
        source: DataSource,
        products: Vec<String>,
        sender: Sender<StreamTick>,
    ) -> Option<Self> {
        let (session, parse): (Session, Parser) = match source {
            DataSource::Coinbase => (coinbase::session(&products), coinbase::parse_tick),
            DataSource::Okx => (okx::session(&products), okx::parse_tick),
            DataSource::Kraken => (kraken::session(&products), kraken::parse_tick),
            DataSource::Bybit => (bybit::session(&products), bybit::parse_tick),
            DataSource::Gate => (gateio::session(&products), gateio::parse_tick),
            _ => return None,
        };
        let shutdown = Arc::new(AtomicBool::new(false));
        let thread_shutdown = Arc::clone(&shutdown);
        std::thread::spawn(move || {
            session.run(&thread_shutdown, |text| {
                parse(text).is_none_or(|tick| sender.send(tick).is_ok())
            });
        });
        Some(Self { shutdown, products })
    }

    pub fn products(&self) -> &[String] {
        &self.products
    }
}

impl Drop for QuoteSocket {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Run with `cargo test --lib quote_stream::tests::live_okx -- --ignored --nocapture`.
    #[test]
    #[ignore = "needs the network"]
    fn live_okx_streams_a_ticker() {
        let (sender, receiver) = std::sync::mpsc::channel();
        let _socket = QuoteSocket::connect(
            DataSource::Okx,
            vec!["BTC-USDT".to_owned(), "ETH-USDT".to_owned()],
            sender,
        )
        .expect("OKX has a ticker socket");
        let tick = receiver
            .recv_timeout(std::time::Duration::from_secs(30))
            .expect("a tick in 30 s");
        assert_eq!(tick.source, DataSource::Okx);
        assert!(["BTC-USDT", "ETH-USDT"].contains(&tick.product_id.as_str()));
        assert!(tick.quote.price > 0.0);
    }

    /// Run with `cargo test --lib quote_stream::tests::live_kraken -- --ignored --nocapture`.
    #[test]
    #[ignore = "needs the network"]
    fn live_kraken_streams_a_ticker() {
        let (sender, receiver) = std::sync::mpsc::channel();
        let _socket = QuoteSocket::connect(
            DataSource::Kraken,
            vec!["BTC/USD".to_owned(), "ETH/USD".to_owned()],
            sender,
        )
        .expect("Kraken has a ticker socket");
        let tick = receiver
            .recv_timeout(std::time::Duration::from_secs(30))
            .expect("a tick in 30 s");
        assert_eq!(tick.source, DataSource::Kraken);
        assert!(["BTC/USD", "ETH/USD"].contains(&tick.product_id.as_str()));
        assert!(tick.quote.price > 0.0 && tick.quote.previous_day_price.is_some());
    }

    /// Run with `cargo test --lib quote_stream::tests::live_bybit -- --ignored --nocapture`.
    #[test]
    #[ignore = "needs the network"]
    fn live_bybit_streams_a_ticker_beside_an_unknown_product() {
        let (sender, receiver) = std::sync::mpsc::channel();
        let _socket = QuoteSocket::connect(
            DataSource::Bybit,
            vec!["NOPEUSDT".to_owned(), "BTCUSDT".to_owned()],
            sender,
        )
        .expect("Bybit has a ticker socket");
        let tick = receiver
            .recv_timeout(std::time::Duration::from_secs(30))
            .expect("a tick in 30 s");
        assert_eq!(tick.source, DataSource::Bybit);
        assert_eq!(tick.product_id, "BTCUSDT");
        assert!(tick.quote.price > 0.0 && tick.quote.previous_day_price.is_some());
    }

    /// Run with `cargo test --lib quote_stream::tests::live_gate -- --ignored --nocapture`.
    #[test]
    #[ignore = "needs the network"]
    fn live_gate_streams_a_ticker_beside_an_unknown_product() {
        let (sender, receiver) = std::sync::mpsc::channel();
        let _socket = QuoteSocket::connect(
            DataSource::Gate,
            vec!["NOPE_USDT".to_owned(), "BTC_USDT".to_owned()],
            sender,
        )
        .expect("Gate.io has a ticker socket");
        let tick = receiver
            .recv_timeout(std::time::Duration::from_secs(30))
            .expect("a tick in 30 s");
        assert_eq!(tick.source, DataSource::Gate);
        assert_eq!(tick.product_id, "BTC_USDT");
        assert!(tick.quote.price > 0.0 && tick.quote.previous_day_price.is_some());
    }

    #[test]
    fn a_source_without_a_ticker_socket_opens_none() {
        let (sender, _receiver) = std::sync::mpsc::channel();
        assert!(
            QuoteSocket::connect(DataSource::Binance, vec!["BTCUSDT".to_owned()], sender).is_none()
        );
    }
}
