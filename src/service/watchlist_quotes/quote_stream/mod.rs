//! One shared ticker socket per streamed source, for every market a visible watchlist shows of
//! it. The ticker carries the last trade plus the 24h open and volume, which is all a row needs.

use crate::model::DataSource;
use crate::model::watchlist::quote::SourceQuote;
use crate::service::market::venue;
use crate::service::net::ws::{Flow, Frame, Session};
use std::cell::RefCell;
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

/// How a venue's ticker socket is opened and read. The socket loop is [`QuoteSocket`]'s alone.
pub trait QuoteProtocol: Sync {
    /// The socket for `products`, as the venue names them: its url, what to send once connected
    /// and how to keep it alive.
    fn session(&self, products: &[String]) -> Session;

    /// What one text message of the socket says on its own; nothing for a message that is not a
    /// price (an acknowledgement, a heartbeat, another topic). Every venue answers it, so none is
    /// left that reads nothing by forgetting both this and [`Self::reader`]: a venue whose messages
    /// cannot be read alone (they name a subscription, not a market) answers `None` here, as it is
    /// never called, and reads through [`Self::reader`].
    fn parse(&self, text: &str) -> Option<StreamTick>;

    /// Whether a tick says the price 24 hours ago (`StreamTick::quote`'s `previous_day_price`). A
    /// venue whose socket sends only trades says `false`, and the quote engine then takes that
    /// price, and the volume, from the REST answer and keeps asking for it as it ages.
    fn ticks_carry_reference(&self) -> bool {
        true
    }

    /// For a venue that has to remember across messages of one connection (which market a channel
    /// number is): the reader [`QuoteSocket`] makes of the connection's messages. `None` for a
    /// venue that reads each message by [`Self::parse`].
    fn reader(&self) -> Option<Box<dyn TickReader>> {
        None
    }
}

/// What a connection's messages have told so far, and the reading of the next one by it.
pub trait TickReader {
    /// A new connection is up: what the last one told is void.
    fn reset(&mut self);

    /// What one text message says; see [`QuoteProtocol::parse`].
    fn read(&mut self, text: &str) -> Option<StreamTick>;

    /// Whether the message just read asked for a new connection (the server is restarting, or
    /// going into maintenance and dropping every subscription): asked after each message, and
    /// `true` once for each such message.
    fn reconnect(&mut self) -> bool {
        false
    }
}

/// How one connection's messages are read: by the protocol alone, or through the memory it asks for.
struct Reading {
    protocol: &'static dyn QuoteProtocol,
    memory: Option<Box<dyn TickReader>>,
}

impl Reading {
    fn new(protocol: &'static dyn QuoteProtocol) -> Self {
        Self {
            protocol,
            memory: protocol.reader(),
        }
    }

    fn link_up(&mut self) {
        if let Some(memory) = &mut self.memory {
            memory.reset();
        }
    }

    fn read(&mut self, text: &str) -> Option<StreamTick> {
        match &mut self.memory {
            Some(memory) => memory.read(text),
            None => self.protocol.parse(text),
        }
    }

    fn reconnect(&mut self) -> bool {
        self.memory
            .as_mut()
            .is_some_and(|memory| memory.reconnect())
    }
}

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
        let protocol = venue::of(source).quotes()?;
        let session = protocol.session(&products);
        let shutdown = Arc::new(AtomicBool::new(false));
        let thread_shutdown = Arc::clone(&shutdown);
        std::thread::spawn(move || {
            let reading = RefCell::new(Reading::new(protocol));
            session.run_flow(
                &thread_shutdown,
                |up| {
                    if up {
                        reading.borrow_mut().link_up();
                    }
                },
                |frame| match frame {
                    Frame::Text(text) => {
                        let mut reading = reading.borrow_mut();
                        if reading
                            .read(text)
                            .is_some_and(|tick| sender.send(tick).is_err())
                        {
                            Flow::Stop
                        } else if reading.reconnect() {
                            Flow::Reconnect
                        } else {
                            Flow::Read
                        }
                    }
                    Frame::Binary(_) => Flow::Read,
                },
            );
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

    /// Reads `price` messages only on a connection that was first told `hello`: a stand-in for a
    /// venue whose messages name a subscription, not a market.
    #[derive(Default)]
    struct Introduced {
        product: Option<String>,
    }

    impl TickReader for Introduced {
        fn reset(&mut self) {
            self.product = None;
        }

        fn read(&mut self, text: &str) -> Option<StreamTick> {
            match text.strip_prefix("hello ") {
                Some(product) => {
                    self.product = Some(product.to_owned());
                    None
                }
                None => Some(StreamTick {
                    source: DataSource::Binance,
                    product_id: self.product.clone()?,
                    quote: SourceQuote {
                        price: text.parse().ok()?,
                        ..SourceQuote::default()
                    },
                }),
            }
        }
    }

    struct Remembering;
    struct Alone;

    impl QuoteProtocol for Remembering {
        fn session(&self, _: &[String]) -> Session {
            unreachable!("not opened by this test")
        }

        fn parse(&self, _: &str) -> Option<StreamTick> {
            None
        }

        fn reader(&self) -> Option<Box<dyn TickReader>> {
            Some(Box::new(Introduced::default()))
        }
    }

    impl QuoteProtocol for Alone {
        fn session(&self, _: &[String]) -> Session {
            unreachable!("not opened by this test")
        }

        fn parse(&self, text: &str) -> Option<StreamTick> {
            Introduced {
                product: Some("BTCUSDT".to_owned()),
            }
            .read(text)
        }
    }

    #[test]
    fn a_connection_is_read_through_the_memory_its_venue_asks_for_and_forgets_it_when_it_reconnects()
     {
        let mut reading = Reading::new(&Remembering);
        reading.link_up();
        assert!(reading.read("7").is_none(), "no product told yet");
        assert!(reading.read("hello BTCUSD").is_none());
        let tick = reading.read("7").expect("a price");
        assert_eq!(
            (tick.product_id.as_str(), tick.quote.price),
            ("BTCUSD", 7.0)
        );

        reading.link_up();
        assert!(
            reading.read("8").is_none(),
            "the last connection's channels are void"
        );
    }

    #[test]
    fn a_connection_the_server_asked_to_end_is_told_once() {
        /// Asks for a new connection at the message `restart`.
        #[derive(Default)]
        struct Restarting {
            asked: bool,
        }

        impl TickReader for Restarting {
            fn reset(&mut self) {}

            fn read(&mut self, text: &str) -> Option<StreamTick> {
                self.asked = text == "restart";
                None
            }

            fn reconnect(&mut self) -> bool {
                std::mem::take(&mut self.asked)
            }
        }

        struct Venue;

        impl QuoteProtocol for Venue {
            fn session(&self, _: &[String]) -> Session {
                unreachable!("not opened by this test")
            }

            fn parse(&self, _: &str) -> Option<StreamTick> {
                None
            }

            fn reader(&self) -> Option<Box<dyn TickReader>> {
                Some(Box::new(Restarting::default()))
            }
        }

        let mut reading = Reading::new(&Venue);
        reading.read("tick");
        assert!(!reading.reconnect());
        reading.read("restart");
        assert!(reading.reconnect());
        assert!(!reading.reconnect(), "asked for once");
        // A venue that reads message by message has nothing to ask.
        let mut alone = Reading::new(&Alone);
        alone.read("restart");
        assert!(!alone.reconnect());
    }

    #[test]
    fn a_venue_that_asks_for_no_memory_is_parsed_message_by_message() {
        let mut reading = Reading::new(&Alone);
        reading.link_up();
        assert_eq!(reading.read("3").unwrap().quote.price, 3.0);
        assert!(reading.read("not a number").is_none());
    }

    #[test]
    fn a_source_without_a_ticker_socket_opens_none() {
        let (sender, _receiver) = std::sync::mpsc::channel();
        assert!(
            QuoteSocket::connect(DataSource::Binance, vec!["BTCUSDT".to_owned()], sender).is_none()
        );
    }
}
