use crate::lock::lock;
use crate::model::market::live::Klines;
use crate::model::{DataSource, Kline, Timeframe};
use crate::service::market::venue;
use crate::service::net::ws::{Flow, Frame, Session};
use eframe::egui;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// The most bars the inbox keeps. A feed nobody reads for days (a background tab, a hidden
/// window) still sees one new bar per symbol per interval, so this is only a backstop.
const INBOX_CAPACITY: usize = 4096;

/// What a live feed has received and the workspace has not applied yet: the latest update of each
/// `(bar, symbol)`. A kline stream resends the forming bar every second or two, and applying the
/// last of those is the same as applying all of them, so the queue never grows with time. A plain
/// channel did: it is only drained while the workspace is painted, and a tab left in the
/// background kept every update for as long as the app ran.
#[derive(Default)]
pub struct LiveInbox {
    latest: Mutex<BTreeMap<(i64, String), Kline>>,
}

impl LiveInbox {
    fn push(&self, symbol: String, kline: Kline) {
        let mut latest = lock(&self.latest);
        latest.insert((kline.open_time, symbol), kline);
        while latest.len() > INBOX_CAPACITY {
            latest.pop_first();
        }
    }

    /// Everything received since the last call, oldest bar first.
    pub fn drain(&self) -> Vec<(String, Kline)> {
        let mut latest = lock(&self.latest);
        std::mem::take(&mut *latest)
            .into_iter()
            .map(|((_, symbol), kline)| (symbol, kline))
            .collect()
    }
}

/// How a venue's candle socket is opened and read. The loop, the reconnecting and the inbox are
/// [`LiveFeed`]'s alone.
pub trait CandleProtocol: Sync {
    /// The connections that stream the candles of length `base` for `symbols` (an exchange that
    /// caps the topics of a connection gets several); `None` when the venue has no such candle.
    fn sessions(&self, symbols: &[String], base: Timeframe) -> Option<Vec<Session>>;

    /// What one message says on its own: the market as the venue names it, and its candle. Nothing
    /// for a message that is not one (an acknowledgement, a heartbeat, another topic). Every venue
    /// answers it, so none is left that reads nothing by forgetting both this and [`Self::reader`]:
    /// a venue whose messages cannot be read alone (they name a subscription, not a market)
    /// answers `None` here, as it is never called, and reads through [`Self::reader`].
    fn parse(&self, frame: Frame<'_>, base: Timeframe) -> Option<(String, Kline)>;

    /// For a venue that has to remember across messages of one connection (which market a channel
    /// number is): the reader [`LiveFeed`] makes of each connection's messages. `None` for a venue
    /// that reads each message by [`Self::parse`].
    fn reader(&self, _base: Timeframe) -> Option<Box<dyn FrameReader>> {
        None
    }

    /// What the klines of this venue are to the bars they name: the bar so far, as a candle
    /// channel sends it, or (for a venue with no candle channel) what trades built since the socket
    /// connected, which the chart adds to the candle it holds ([`Klines::Trades`]).
    fn klines(&self) -> Klines {
        Klines::Bars
    }
}

/// What a connection's messages have told so far, and the reading of the next one by it.
pub trait FrameReader {
    /// A new connection is up: what the last one told is void.
    fn reset(&mut self);

    /// What one message says; see [`CandleProtocol::parse`].
    fn read(&mut self, frame: Frame<'_>) -> Option<(String, Kline)>;

    /// Whether the message just read asked for a new connection (the server is restarting, or
    /// going into maintenance and dropping every subscription): asked after each message, and
    /// `true` once for each such message.
    fn reconnect(&mut self) -> bool {
        false
    }
}

/// How one connection's messages are read: by the protocol alone, or through the memory it asks for.
struct Reading {
    protocol: &'static dyn CandleProtocol,
    base: Timeframe,
    memory: Option<Box<dyn FrameReader>>,
}

impl Reading {
    fn new(protocol: &'static dyn CandleProtocol, base: Timeframe) -> Self {
        Self {
            protocol,
            base,
            memory: protocol.reader(base),
        }
    }

    fn link_up(&mut self) {
        if let Some(memory) = &mut self.memory {
            memory.reset();
        }
    }

    fn read(&mut self, frame: Frame<'_>) -> Option<(String, Kline)> {
        match &mut self.memory {
            Some(memory) => memory.read(frame),
            None => self.protocol.parse(frame, self.base),
        }
    }

    fn reconnect(&mut self) -> bool {
        self.memory
            .as_mut()
            .is_some_and(|memory| memory.reconnect())
    }
}

/// A kline feed for one workspace's charts of one source: background threads (one per connection:
/// an exchange that caps the topics of a connection gets several) that reconnect with backoff and
/// put `(symbol, kline)` updates in the inbox.
pub struct LiveFeed {
    pub inbox: Arc<LiveInbox>,
    signature: (Vec<String>, Timeframe),
    klines: Klines,
    shutdown: Arc<AtomicBool>,
}

impl LiveFeed {
    /// Opens the stream of `base` candles for `symbols`; `None` when `source` streams none.
    pub fn connect(
        source: DataSource,
        symbols: Vec<String>,
        base: Timeframe,
        ctx: egui::Context,
    ) -> Option<Self> {
        let protocol = venue::of(source).live()?;
        let sessions = protocol.sessions(&symbols, base)?;
        let inbox = Arc::new(LiveInbox::default());
        let shutdown = Arc::new(AtomicBool::new(false));
        for session in sessions {
            let thread_shutdown = Arc::clone(&shutdown);
            let thread_inbox = Arc::clone(&inbox);
            let ctx = ctx.clone();
            std::thread::spawn(move || {
                // Told when the link comes up and read at each message, by the one loop.
                let reading = RefCell::new(Reading::new(protocol, base));
                session.run_flow(
                    &thread_shutdown,
                    |up| {
                        if up {
                            reading.borrow_mut().link_up();
                        }
                    },
                    |frame| {
                        let mut reading = reading.borrow_mut();
                        if let Some((symbol, kline)) = reading.read(frame) {
                            thread_inbox.push(symbol, kline);
                            ctx.request_repaint();
                        }
                        if reading.reconnect() {
                            Flow::Reconnect
                        } else {
                            Flow::Read
                        }
                    },
                );
            });
        }
        Some(Self {
            inbox,
            signature: (symbols, base),
            klines: protocol.klines(),
            shutdown,
        })
    }

    /// What the klines of the inbox are to the bars they name.
    pub fn klines(&self) -> Klines {
        self.klines
    }

    /// The symbols and candle length this feed was opened for, so the owner can tell when a
    /// reconnect is needed.
    pub fn signature(&self) -> &(Vec<String>, Timeframe) {
        &self.signature
    }
}

impl Drop for LiveFeed {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kline(open_time: i64, close: f64) -> Kline {
        Kline {
            open_time,
            close_time: open_time + 3_599_999,
            open: 1.0,
            high: 2.0,
            low: 0.5,
            close,
            volume: 1.0,
        }
    }

    /// Reads `name` frames only on a connection that was first told `hello`: a stand-in for a
    /// venue whose messages name a subscription, not a market.
    #[derive(Default)]
    struct Introduced {
        market: Option<String>,
    }

    impl FrameReader for Introduced {
        fn reset(&mut self) {
            self.market = None;
        }

        fn read(&mut self, frame: Frame<'_>) -> Option<(String, Kline)> {
            let text = frame.text()?;
            match text.strip_prefix("hello ") {
                Some(market) => {
                    self.market = Some(market.to_owned());
                    None
                }
                None => Some((self.market.clone()?, kline(1_000, text.parse().ok()?))),
            }
        }
    }

    struct Remembering;
    struct Alone;

    impl CandleProtocol for Remembering {
        fn sessions(&self, _: &[String], _: Timeframe) -> Option<Vec<Session>> {
            None
        }

        fn parse(&self, _: Frame<'_>, _: Timeframe) -> Option<(String, Kline)> {
            None
        }

        fn reader(&self, _: Timeframe) -> Option<Box<dyn FrameReader>> {
            Some(Box::new(Introduced::default()))
        }
    }

    impl CandleProtocol for Alone {
        fn sessions(&self, _: &[String], _: Timeframe) -> Option<Vec<Session>> {
            None
        }

        fn parse(&self, frame: Frame<'_>, _: Timeframe) -> Option<(String, Kline)> {
            Some((
                "BTCUSDT".to_owned(),
                kline(1_000, frame.text()?.parse().ok()?),
            ))
        }
    }

    #[test]
    fn a_connection_is_read_through_the_memory_its_venue_asks_for_and_forgets_it_when_it_reconnects()
     {
        let mut reading = Reading::new(&Remembering, Timeframe::HOUR_1);
        reading.link_up();
        assert!(
            reading.read(Frame::Text("7")).is_none(),
            "no market told yet"
        );
        assert!(reading.read(Frame::Text("hello BTCUSD")).is_none());
        let (symbol, bar) = reading.read(Frame::Text("7")).expect("a candle");
        assert_eq!((symbol.as_str(), bar.close), ("BTCUSD", 7.0));

        // A new connection starts from nothing: what the last told is void.
        reading.link_up();
        assert!(reading.read(Frame::Text("8")).is_none());
        assert!(reading.read(Frame::Text("hello ETHUSD")).is_none());
        assert_eq!(reading.read(Frame::Text("8")).unwrap().0, "ETHUSD");
    }

    #[test]
    fn a_connection_the_server_asked_to_end_is_told_once() {
        /// Asks for a new connection at the message `restart`.
        #[derive(Default)]
        struct Restarting {
            asked: bool,
        }

        impl FrameReader for Restarting {
            fn reset(&mut self) {}

            fn read(&mut self, frame: Frame<'_>) -> Option<(String, Kline)> {
                self.asked = frame.text() == Some("restart");
                None
            }

            fn reconnect(&mut self) -> bool {
                std::mem::take(&mut self.asked)
            }
        }

        struct Venue;

        impl CandleProtocol for Venue {
            fn sessions(&self, _: &[String], _: Timeframe) -> Option<Vec<Session>> {
                None
            }

            fn parse(&self, _: Frame<'_>, _: Timeframe) -> Option<(String, Kline)> {
                None
            }

            fn reader(&self, _: Timeframe) -> Option<Box<dyn FrameReader>> {
                Some(Box::new(Restarting::default()))
            }
        }

        let mut reading = Reading::new(&Venue, Timeframe::HOUR_1);
        reading.read(Frame::Text("tick"));
        assert!(!reading.reconnect());
        reading.read(Frame::Text("restart"));
        assert!(reading.reconnect());
        assert!(!reading.reconnect(), "asked for once");
        // A venue that reads message by message has nothing to ask.
        let mut alone = Reading::new(&Alone, Timeframe::HOUR_1);
        alone.read(Frame::Text("restart"));
        assert!(!alone.reconnect());
    }

    #[test]
    fn a_venue_that_asks_for_no_memory_is_parsed_message_by_message() {
        let mut reading = Reading::new(&Alone, Timeframe::HOUR_1);
        reading.link_up();
        assert_eq!(reading.read(Frame::Text("3")).unwrap().1.close, 3.0);
        assert!(reading.read(Frame::Text("not a number")).is_none());
        assert!(reading.read(Frame::Binary(&[1])).is_none());
    }

    #[test]
    fn the_inbox_keeps_only_the_latest_update_of_a_bar() {
        let inbox = LiveInbox::default();
        for tick in 0..10_000 {
            inbox.push("BTCUSDT".into(), kline(1_000, f64::from(tick)));
        }
        let drained = inbox.drain();
        assert_eq!(drained.len(), 1);
        assert_eq!(drained[0].1.close, 9_999.0);
        assert!(inbox.drain().is_empty());
    }

    #[test]
    fn the_inbox_hands_over_older_bars_first() {
        let inbox = LiveInbox::default();
        inbox.push("ETHUSDT".into(), kline(2_000, 1.0));
        inbox.push("BTCUSDT".into(), kline(2_000, 2.0));
        inbox.push("BTCUSDT".into(), kline(1_000, 3.0));
        let order: Vec<_> = inbox
            .drain()
            .into_iter()
            .map(|(symbol, kline)| (kline.open_time, symbol))
            .collect();
        assert_eq!(
            order,
            [
                (1_000, "BTCUSDT".to_string()),
                (2_000, "BTCUSDT".to_string()),
                (2_000, "ETHUSDT".to_string())
            ]
        );
    }

    #[test]
    fn an_unread_inbox_stops_growing_at_its_capacity() {
        let inbox = LiveInbox::default();
        for bar in 0..(INBOX_CAPACITY as i64 + 500) {
            inbox.push("BTCUSDT".into(), kline(bar * 3_600_000, 1.0));
        }
        let drained = inbox.drain();
        assert_eq!(drained.len(), INBOX_CAPACITY);
        // The newest bars survive.
        assert_eq!(
            drained.last().unwrap().1.open_time,
            (INBOX_CAPACITY as i64 + 499) * 3_600_000
        );
    }

    /// Run with `cargo test --lib live::tests::live_okx -- --ignored --nocapture`.
    #[test]
    #[ignore = "needs the network"]
    fn live_okx_streams_the_forming_candle() {
        let feed = LiveFeed::connect(
            DataSource::Okx,
            vec!["BTC-USDT".to_owned()],
            Timeframe::MINUTE_1,
            egui::Context::default(),
        )
        .expect("OKX streams one-minute candles");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        loop {
            if let Some((symbol, kline)) = feed.inbox.drain().pop() {
                assert_eq!(symbol, "BTC-USDT");
                assert!(kline.close > 0.0 && kline.close_time > kline.open_time);
                return;
            }
            assert!(std::time::Instant::now() < deadline, "no candle in 30 s");
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
    }

    /// Run with `cargo test --lib live::tests::live_kraken -- --ignored --nocapture`.
    #[test]
    #[ignore = "needs the network"]
    fn live_kraken_streams_the_forming_candle() {
        let feed = LiveFeed::connect(
            DataSource::Kraken,
            vec!["BTC/USD".to_owned()],
            Timeframe::MINUTE_1,
            egui::Context::default(),
        )
        .expect("Kraken streams one-minute candles");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        loop {
            if let Some((symbol, kline)) = feed.inbox.drain().pop() {
                assert_eq!(symbol, "BTC/USD");
                assert!(kline.close > 0.0 && kline.close_time > kline.open_time);
                return;
            }
            assert!(std::time::Instant::now() < deadline, "no candle in 60 s");
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
    }

    /// Run with `cargo test --lib live::tests::live_bybit -- --ignored --nocapture`.
    #[test]
    #[ignore = "needs the network"]
    fn live_bybit_streams_the_forming_candle_of_every_market_asked_for_even_beside_an_unknown_one()
    {
        let feed = LiveFeed::connect(
            DataSource::Bybit,
            vec![
                "NOPEUSDT".to_owned(),
                "BTCUSDT".to_owned(),
                "eth/usdt".to_owned(),
            ],
            Timeframe::MINUTE_1,
            egui::Context::default(),
        )
        .expect("Bybit streams one-minute candles");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        let mut seen = std::collections::BTreeSet::new();
        while seen.len() < 2 {
            for (symbol, kline) in feed.inbox.drain() {
                assert!(kline.close > 0.0 && kline.close_time > kline.open_time);
                seen.insert(symbol);
            }
            assert!(
                std::time::Instant::now() < deadline,
                "only {seen:?} in 60 s"
            );
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
        assert_eq!(seen.into_iter().collect::<Vec<_>>(), ["BTCUSDT", "ETHUSDT"]);
    }

    /// Run with `cargo test --lib live::tests::live_gate -- --ignored --nocapture`.
    #[test]
    #[ignore = "needs the network"]
    fn live_gate_streams_the_forming_candle_of_every_market_asked_for_even_beside_an_unknown_one() {
        let feed = LiveFeed::connect(
            DataSource::Gate,
            vec![
                "NOPE_USDT".to_owned(),
                "BTC_USDT".to_owned(),
                "eth/usdt".to_owned(),
            ],
            Timeframe::MINUTE_1,
            egui::Context::default(),
        )
        .expect("Gate.io streams one-minute candles");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        let mut seen = std::collections::BTreeSet::new();
        while seen.len() < 2 {
            for (symbol, kline) in feed.inbox.drain() {
                assert!(kline.close > 0.0 && kline.close_time > kline.open_time);
                seen.insert(symbol);
            }
            assert!(
                std::time::Instant::now() < deadline,
                "only {seen:?} in 60 s"
            );
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
        assert_eq!(
            seen.into_iter().collect::<Vec<_>>(),
            ["BTC_USDT", "ETH_USDT"]
        );
    }

    /// Run with `cargo test --lib live::tests::live_mexc -- --ignored --nocapture`.
    #[test]
    #[ignore = "needs the network"]
    fn live_mexc_streams_the_forming_candle_of_every_market_asked_for_even_beside_an_unknown_one() {
        let feed = LiveFeed::connect(
            DataSource::Mexc,
            vec![
                "NOPEUSDT".to_owned(),
                "BTCUSDT".to_owned(),
                "eth/usdt".to_owned(),
            ],
            Timeframe::MINUTE_1,
            egui::Context::default(),
        )
        .expect("MEXC streams one-minute candles");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        let mut seen = std::collections::BTreeSet::new();
        while seen.len() < 2 {
            for (symbol, kline) in feed.inbox.drain() {
                assert!(kline.close > 0.0 && kline.close_time > kline.open_time);
                seen.insert(symbol);
            }
            assert!(
                std::time::Instant::now() < deadline,
                "only {seen:?} in 60 s"
            );
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
        assert_eq!(seen.into_iter().collect::<Vec<_>>(), ["BTCUSDT", "ETHUSDT"]);
    }

    #[test]
    fn a_source_without_a_candle_stream_opens_none() {
        assert!(
            LiveFeed::connect(
                DataSource::Coinbase,
                vec!["BTC-USD".to_owned()],
                Timeframe::MINUTE_1,
                egui::Context::default()
            )
            .is_none()
        );
        // OKX has no 8-hour bar to stream.
        assert!(
            LiveFeed::connect(
                DataSource::Okx,
                vec!["BTC-USDT".to_owned()],
                Timeframe::parse("8h").unwrap(),
                egui::Context::default()
            )
            .is_none()
        );
    }
}
