//! Read deadlines, heartbeat checks, and reconnect pacing for live market feeds. How a connection
//! is kept alive ([`Keepalive`]) and how its binary messages are read ([`Decode`]) are data of the
//! session, decided by pure functions in their own files.

use super::proxy;
use std::io;
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use tungstenite::{Message, WebSocket, stream::MaybeTlsStream};

mod decode;
mod keepalive;

pub use decode::Decode;
pub use keepalive::Keepalive;

use keepalive::Inbound;

const READ_TIMEOUT: Duration = Duration::from_secs(5);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(8);
const PING_AFTER: Duration = Duration::from_secs(15);
const PONG_TIMEOUT: Duration = Duration::from_secs(10);

/// How often a connected feed asks whether the proxy changed under it.
const ROUTE_CHECK: Duration = Duration::from_secs(1);

/// The proxy generation a connection was made under, to hand to [`Health::new`]: the feed then
/// reconnects when the setting changes instead of keeping the old route.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Route(u64);

/// Bound TCP connect and TLS/WebSocket handshakes before entering the read loop. The connection
/// goes through the proxy in force, unless it skips `host` (see `net::proxy`).
pub fn connect(
    url: &str,
    host: &str,
    port: u16,
) -> Result<(WebSocket<MaybeTlsStream<TcpStream>>, Route), String> {
    // Refreshed and read first: a change while dialling then counts as a change after.
    proxy::refresh_if_stale();
    let route = Route(proxy::generation());
    let stream = proxy::dial(host, port, CONNECT_TIMEOUT).map_err(|error| error.to_string())?;
    let (mut socket, _) =
        tungstenite::client_tls(url, stream).map_err(|error| error.to_string())?;
    configure(&mut socket).map_err(|error| error.to_string())?;
    Ok((socket, route))
}

pub fn configure(socket: &mut WebSocket<MaybeTlsStream<TcpStream>>) -> io::Result<()> {
    let stream = match socket.get_mut() {
        MaybeTlsStream::Plain(stream) => stream,
        MaybeTlsStream::Rustls(stream) => &mut stream.sock,
        _ => {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "unsupported TLS stream",
            ));
        }
    };
    stream.set_read_timeout(Some(READ_TIMEOUT))?;
    stream.set_write_timeout(Some(READ_TIMEOUT))
}

pub struct Health {
    route: Route,
    route_checked: Instant,
    last_message: Instant,
    last_data: Instant,
    ping_sent: Option<Instant>,
    max_data_silence: Duration,
    keepalive: Keepalive,
    decode: Decode,
}

impl Health {
    pub fn new(max_data_silence: Duration, keepalive: Keepalive, route: Route) -> Self {
        let now = Instant::now();
        Self {
            route,
            route_checked: now,
            last_message: now,
            last_data: now,
            ping_sent: None,
            max_data_silence,
            keepalive,
            decode: Decode::None,
        }
    }

    /// Reads binary messages the way `decode` says.
    pub fn decoding(mut self, decode: Decode) -> Self {
        self.decode = decode;
        self
    }

    /// A timeout lets the owning thread check shutdown and probe a quiet socket.
    // `tungstenite::Error` is large, but it is the error type the socket API hands back.
    #[allow(clippy::result_large_err)]
    pub fn read(
        &mut self,
        socket: &mut WebSocket<MaybeTlsStream<TcpStream>>,
    ) -> tungstenite::Result<Option<Message>> {
        if self.route_changed() {
            return Err(tungstenite::Error::Io(io::Error::new(
                io::ErrorKind::ConnectionAborted,
                "the proxy setting changed",
            )));
        }
        match socket.read() {
            Ok(Message::Pong(_)) => {
                self.heard();
                Ok(None)
            }
            Ok(message) => self.on_message(socket, message),
            Err(tungstenite::Error::Io(error))
                if matches!(
                    error.kind(),
                    io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
                ) =>
            {
                self.on_timeout(socket)
            }
            Err(error) => Err(error),
        }
    }

    /// The peer was heard from, though not with data: a probe is answered.
    fn heard(&mut self) {
        self.last_message = Instant::now();
        self.ping_sent = None;
    }

    /// How long the peer has been silent, by the measure the keepalive takes: for a stream, since
    /// the last data; for a feed that may be quiet for hours, since the last message of any kind
    /// ([`Keepalive::heard_is_alive`]).
    fn silence(&self, now: Instant) -> Duration {
        let since = if self.keepalive.heard_is_alive() {
            self.last_message
        } else {
            self.last_data
        };
        now.duration_since(since)
    }

    /// A message that is not a pong frame: unpacked as the session says, then either heartbeat
    /// traffic (answered if it asks, never handed on) or the feed's data.
    #[allow(clippy::result_large_err)]
    fn on_message(
        &mut self,
        socket: &mut WebSocket<MaybeTlsStream<TcpStream>>,
        message: Message,
    ) -> tungstenite::Result<Option<Message>> {
        let message = match message {
            Message::Binary(bytes) => match self.decode.text(&bytes) {
                None => Message::Binary(bytes),
                Some(Ok(text)) => Message::Text(text.into()),
                // A message that cannot be read is no data, but the peer is there.
                Some(Err(_)) => {
                    self.heard();
                    return Ok(None);
                }
            },
            other => other,
        };
        if let Message::Text(text) = &message {
            match self.keepalive.inspect(text.as_str()) {
                Inbound::Data => {}
                Inbound::Absorbed => {
                    self.heard();
                    return Ok(None);
                }
                Inbound::Answer(reply) => {
                    socket.send(Message::Text(reply.into()))?;
                    self.heard();
                    return Ok(None);
                }
            }
        }
        let now = Instant::now();
        self.last_message = now;
        self.last_data = now;
        self.ping_sent = None;
        Ok(Some(message))
    }

    /// Whether the proxy in force is no longer the one this connection was made under. Asked
    /// once a second at most, not on every message; the feed's own thread, so it may refresh the
    /// system's proxy.
    fn route_changed(&mut self) -> bool {
        if self.route_checked.elapsed() < ROUTE_CHECK {
            return false;
        }
        self.route_checked = Instant::now();
        proxy::refresh_if_stale();
        Route(proxy::generation()) != self.route
    }

    #[allow(clippy::result_large_err)]
    fn on_timeout(
        &mut self,
        socket: &mut WebSocket<MaybeTlsStream<TcpStream>>,
    ) -> tungstenite::Result<Option<Message>> {
        let now = Instant::now();
        if self.silence(now) >= self.max_data_silence
            || self
                .ping_sent
                .is_some_and(|sent| now.duration_since(sent) >= PONG_TIMEOUT)
        {
            return Err(tungstenite::Error::Io(io::Error::new(
                io::ErrorKind::TimedOut,
                "websocket heartbeat timed out",
            )));
        }
        if self.ping_sent.is_none()
            && now.duration_since(self.last_message) >= PING_AFTER
            && let Some(probe) = self.keepalive.probe()
        {
            socket.send(probe)?;
            self.ping_sent = Some(now);
        }
        Ok(None)
    }
}

pub struct Backoff {
    next: Duration,
}

impl Backoff {
    pub fn new() -> Self {
        Self {
            next: Duration::from_secs(1),
        }
    }

    pub fn reset(&mut self) {
        self.next = Duration::from_secs(1);
    }

    pub fn wait(&mut self, shutdown: &AtomicBool) {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.subsec_nanos());
        let delay = jittered(self.next, nanos);
        self.next = (self.next * 2).min(Duration::from_secs(30));
        let started = Instant::now();
        while !shutdown.load(Ordering::Acquire) {
            let remaining = delay.saturating_sub(started.elapsed());
            if remaining.is_zero() {
                break;
            }
            std::thread::sleep(remaining.min(Duration::from_millis(250)));
        }
    }
}

/// One feed's connection: where it connects, what it subscribes to and how it is kept alive.
pub struct Session {
    pub url: String,
    pub host: String,
    pub port: u16,
    /// Sent in order, one message each, after every (re)connect; empty for a feed that subscribes
    /// through its URL.
    pub subscribe: Vec<String>,
    /// How long the connection may be silent before it is replaced; see [`Keepalive::heard_is_alive`]
    /// for what counts as silence.
    pub max_data_silence: Duration,
    pub keepalive: Keepalive,
    /// What becomes of a binary message: handed over as it is, or unpacked to text first.
    pub decode: Decode,
}

/// One data message of a feed (a heartbeat answer or a close is not one).
#[derive(Clone, Copy, Debug)]
pub enum Frame<'a> {
    Text(&'a str),
    /// A binary message: Protocol Buffers for MEXC.
    Binary(&'a [u8]),
}

impl<'a> Frame<'a> {
    /// The message as text; nothing for a binary one.
    pub fn text(self) -> Option<&'a str> {
        match self {
            Self::Text(text) => Some(text),
            Self::Binary(_) => None,
        }
    }
}

impl Session {
    /// Connects, subscribes and reads until `shutdown`, reconnecting with backoff whenever the
    /// socket fails. Every text message goes to `on_text`; it answers whether to keep reading
    /// (`false` when nobody is listening any more).
    pub fn run(&self, shutdown: &AtomicBool, mut on_text: impl FnMut(&str) -> bool) {
        self.run_frames(shutdown, |frame| match frame {
            Frame::Text(text) => on_text(text),
            Frame::Binary(_) => true,
        });
    }

    /// Like [`Self::run`], for a feed that also sends binary messages.
    pub fn run_frames(&self, shutdown: &AtomicBool, on_frame: impl FnMut(Frame<'_>) -> bool) {
        self.run_linked(shutdown, |_| {}, on_frame);
    }

    /// Like [`Self::run_frames`], and `on_link` hears when the socket is up (`true`, once
    /// connected and subscribed) and when it is down again (`false`, however it ended): for a feed
    /// that has to tell a quiet market from a socket that was not there.
    pub fn run_linked(
        &self,
        shutdown: &AtomicBool,
        mut on_link: impl FnMut(bool),
        mut on_frame: impl FnMut(Frame<'_>) -> bool,
    ) {
        let mut backoff = Backoff::new();
        while !shutdown.load(Ordering::Acquire) {
            if let Ok((mut socket, route)) = connect(&self.url, &self.host, self.port) {
                // The keepalive's own opening message (a heartbeat request) goes before the feed's.
                let subscribed = self
                    .keepalive
                    .greeting()
                    .iter()
                    .chain(&self.subscribe)
                    .all(|message| socket.send(Message::Text(message.as_str().into())).is_ok());
                if subscribed {
                    let mut health = Health::new(self.max_data_silence, self.keepalive, route)
                        .decoding(self.decode);
                    on_link(true);
                    let mut listening = true;
                    while listening && !shutdown.load(Ordering::Acquire) {
                        match health.read(&mut socket) {
                            Ok(Some(Message::Text(text))) => {
                                backoff.reset();
                                listening = on_frame(Frame::Text(text.as_str()));
                            }
                            Ok(Some(Message::Binary(bytes))) => {
                                backoff.reset();
                                listening = on_frame(Frame::Binary(&bytes));
                            }
                            Ok(Some(Message::Close(_))) | Err(_) => break,
                            Ok(_) => {}
                        }
                    }
                    on_link(false);
                    if !listening {
                        return;
                    }
                }
            }
            if shutdown.load(Ordering::Acquire) {
                return;
            }
            backoff.wait(shutdown);
        }
    }
}

fn jittered(delay: Duration, random: u32) -> Duration {
    delay.mul_f64((75 + random % 51) as f64 / 100.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    #[test]
    fn reconnect_delay_has_bounded_jitter() {
        assert_eq!(jittered(Duration::from_secs(4), 0), Duration::from_secs(3));
        assert_eq!(jittered(Duration::from_secs(4), 50), Duration::from_secs(5));
    }

    /// A server that reads `messages` messages, answers `replies`, then lingers so the client stays
    /// up. It returns what it read.
    fn serve_many(
        messages: usize,
        replies: &'static [&'static str],
    ) -> (u16, std::thread::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut socket = tungstenite::accept(stream).unwrap();
            let received: Vec<String> = (0..messages)
                .map(|_| socket.read().unwrap().into_text().unwrap().to_string())
                .collect();
            for reply in replies {
                socket.send(Message::Text((*reply).into())).unwrap();
            }
            std::thread::sleep(Duration::from_millis(300));
            received
        });
        (port, handle)
    }

    fn serve(replies: &'static [&'static str]) -> (u16, std::thread::JoinHandle<String>) {
        let (port, server) = serve_many(1, replies);
        let handle = std::thread::spawn(move || server.join().unwrap().remove(0));
        (port, handle)
    }

    #[test]
    fn a_session_sends_every_subscribe_message_in_order_after_each_connect() {
        let (port, server) = serve_many(3, &["tick"]);
        let session = Session {
            url: format!("ws://127.0.0.1:{port}"),
            host: "127.0.0.1".to_owned(),
            port,
            subscribe: vec!["one".to_owned(), "two".to_owned(), "three".to_owned()],
            max_data_silence: Duration::from_secs(60),
            keepalive: Keepalive::Frame,
            decode: Decode::None,
        };
        session.run(&AtomicBool::new(false), |_| false);
        assert_eq!(server.join().unwrap(), ["one", "two", "three"]);
    }

    #[test]
    fn a_session_subscribes_and_hands_over_what_it_reads() {
        let (port, server) = serve(&["tick"]);
        let session = Session {
            url: format!("ws://127.0.0.1:{port}"),
            host: "127.0.0.1".to_owned(),
            port,
            subscribe: vec!["subscribe".to_owned()],
            max_data_silence: Duration::from_secs(60),
            keepalive: Keepalive::Frame,
            decode: Decode::None,
        };
        let mut seen = Vec::new();
        session.run(&AtomicBool::new(false), |text| {
            seen.push(text.to_owned());
            false
        });
        assert_eq!(seen, ["tick"]);
        assert_eq!(server.join().unwrap(), "subscribe");
    }

    /// A server that sends a binary message, then lingers.
    fn serve_binary(frame: &'static [u8]) -> (u16, std::thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut socket = tungstenite::accept(stream).unwrap();
            let _ = socket.read().unwrap();
            socket.send(Message::Binary(frame.to_vec().into())).unwrap();
            std::thread::sleep(Duration::from_millis(300));
        });
        (port, handle)
    }

    #[test]
    fn a_binary_message_is_a_frame_and_text_only_callers_skip_it() {
        let (port, server) = serve_binary(&[1, 2, 3]);
        let session = Session {
            url: format!("ws://127.0.0.1:{port}"),
            host: "127.0.0.1".to_owned(),
            port,
            subscribe: vec!["subscribe".to_owned()],
            max_data_silence: Duration::from_secs(60),
            keepalive: Keepalive::Frame,
            decode: Decode::None,
        };
        let mut seen = Vec::new();
        session.run_frames(&AtomicBool::new(false), |frame| {
            if let Frame::Binary(bytes) = frame {
                seen.push(bytes.to_vec());
            }
            false
        });
        assert_eq!(seen, [vec![1, 2, 3]]);
        server.join().unwrap();

        // `run` hands over text only: the binary message ends nothing and reaches no callback.
        let (port, server) = serve_binary(&[9]);
        let session = Session {
            url: format!("ws://127.0.0.1:{port}"),
            host: "127.0.0.1".to_owned(),
            port,
            subscribe: vec!["subscribe".to_owned()],
            max_data_silence: Duration::from_millis(400),
            keepalive: Keepalive::Frame,
            decode: Decode::None,
        };
        let shutdown = AtomicBool::new(false);
        let mut texts = 0;
        std::thread::scope(|scope| {
            scope.spawn(|| {
                std::thread::sleep(Duration::from_millis(250));
                shutdown.store(true, Ordering::Release);
            });
            session.run(&shutdown, |_| {
                texts += 1;
                true
            });
        });
        assert_eq!(texts, 0);
        server.join().unwrap();
    }

    #[test]
    fn a_text_heartbeat_answer_is_not_data() {
        let (port, server) = serve(&["pong", "tick"]);
        let session = Session {
            url: format!("ws://127.0.0.1:{port}"),
            host: "127.0.0.1".to_owned(),
            port,
            subscribe: vec!["subscribe".to_owned()],
            max_data_silence: Duration::from_secs(60),
            keepalive: Keepalive::Text {
                ping: "ping",
                pong: "pong",
            },
            decode: Decode::None,
        };
        let mut seen = Vec::new();
        session.run(&AtomicBool::new(false), |text| {
            seen.push(text.to_owned());
            false
        });
        assert_eq!(seen, ["tick"]);
        server.join().unwrap();
    }

    #[test]
    fn a_text_heartbeat_answer_is_recognised_by_its_start() {
        // Bybit's pong carries the connection id, so only its beginning is fixed.
        let (port, server) = serve(&[
            r#"{"success":true,"ret_msg":"pong","conn_id":"abc-1","op":"ping"}"#,
            r#"{"topic":"tickers.BTCUSDT","data":{}}"#,
        ]);
        let session = Session {
            url: format!("ws://127.0.0.1:{port}"),
            host: "127.0.0.1".to_owned(),
            port,
            subscribe: vec!["subscribe".to_owned()],
            max_data_silence: Duration::from_secs(60),
            keepalive: Keepalive::Text {
                ping: r#"{"op":"ping"}"#,
                pong: r#"{"success":true,"ret_msg":"pong""#,
            },
            decode: Decode::None,
        };
        let mut seen = Vec::new();
        session.run(&AtomicBool::new(false), |text| {
            seen.push(text.to_owned());
            false
        });
        assert_eq!(seen, [r#"{"topic":"tickers.BTCUSDT","data":{}}"#]);
        server.join().unwrap();
    }

    /// A server that reads `greeted` messages, sends `frames`, then collects what the client sends
    /// until the client goes away.
    fn serve_frames(
        greeted: usize,
        frames: Vec<Message>,
    ) -> (u16, std::thread::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut socket = tungstenite::accept(stream).unwrap();
            let mut received: Vec<String> = (0..greeted)
                .map(|_| socket.read().unwrap().into_text().unwrap().to_string())
                .collect();
            for frame in frames {
                socket.send(frame).unwrap();
            }
            while let Ok(message) = socket.read() {
                if let Message::Text(text) = message {
                    received.push(text.to_string());
                }
            }
            received
        });
        (port, handle)
    }

    fn gzip(text: &str) -> Message {
        use std::io::Write;
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        encoder.write_all(text.as_bytes()).unwrap();
        Message::Binary(encoder.finish().unwrap().into())
    }

    /// Runs a session against `frames` until the feed has been handed one data message; returns
    /// what the feed saw and what the server received.
    fn run_against(
        keepalive: Keepalive,
        decode: Decode,
        greeted: usize,
        frames: Vec<Message>,
    ) -> (Vec<String>, Vec<String>) {
        let (port, server) = serve_frames(greeted, frames);
        let session = Session {
            url: format!("ws://127.0.0.1:{port}"),
            host: "127.0.0.1".to_owned(),
            port,
            subscribe: vec!["subscribe".to_owned()],
            max_data_silence: Duration::from_secs(60),
            keepalive,
            decode,
        };
        let mut seen = Vec::new();
        session.run(&AtomicBool::new(false), |text| {
            seen.push(text.to_owned());
            !text.contains("tick")
        });
        (seen, server.join().unwrap())
    }

    #[test]
    fn a_gzip_session_hands_unpacked_text_to_the_feed_and_a_plain_one_skips_it() {
        let frames = || vec![gzip(r#"{"tick":1}"#)];
        let (seen, _) = run_against(Keepalive::Frame, Decode::Gzip, 1, frames());
        assert_eq!(seen, [r#"{"tick":1}"#]);

        // Not gzip: the message is dropped, the socket stays up and the next one is read.
        let (seen, _) = run_against(
            Keepalive::Frame,
            Decode::Gzip,
            1,
            vec![
                Message::Binary(b"garbage".to_vec().into()),
                gzip(r#"{"tick":2}"#),
            ],
        );
        assert_eq!(seen, [r#"{"tick":2}"#]);
    }

    #[test]
    fn htx_pings_are_echoed_and_never_reach_the_feed() {
        let htx = Keepalive::EchoJson {
            ask: "ping",
            answer: "pong",
        };
        let (seen, received) = run_against(
            htx,
            Decode::Gzip,
            1,
            vec![gzip(r#"{"ping":1492420473027}"#), gzip(r#"{"tick":1}"#)],
        );
        assert_eq!(seen, [r#"{"tick":1}"#]);
        assert_eq!(received, ["subscribe", r#"{"pong":1492420473027}"#]);
    }

    #[test]
    fn bingx_text_pings_are_answered_with_pong_and_never_reach_the_feed() {
        let bingx = Keepalive::AnswerText {
            ask: "Ping",
            answer: "Pong",
        };
        let (seen, received) = run_against(
            bingx,
            Decode::Gzip,
            1,
            vec![gzip("Ping"), gzip(r#"{"tick":1}"#)],
        );
        assert_eq!(seen, [r#"{"tick":1}"#]);
        assert_eq!(received, ["subscribe", "Pong"]);
    }

    #[test]
    fn deribit_is_asked_for_a_heartbeat_first_and_answers_its_test_requests() {
        let deribit = Keepalive::JsonRpcHeartbeat { interval_secs: 30 };
        let (seen, received) = run_against(
            deribit,
            Decode::None,
            2,
            vec![
                Message::Text(
                    r#"{"jsonrpc":"2.0","method":"heartbeat","params":{"type":"heartbeat"}}"#
                        .into(),
                ),
                Message::Text(
                    r#"{"jsonrpc":"2.0","method":"heartbeat","params":{"type":"test_request"}}"#
                        .into(),
                ),
                Message::Text(r#"{"tick":1}"#.into()),
            ],
        );
        assert_eq!(seen, [r#"{"tick":1}"#]);
        assert!(received[0].contains("public/set_heartbeat"), "{received:?}");
        assert_eq!(received[1], "subscribe");
        assert!(received[2].contains("public/test"), "{received:?}");
    }

    /// A feed that was connected under one proxy setting reconnects when it changes, instead of
    /// keeping the route it was dialled with.
    #[test]
    fn a_feed_notices_that_the_proxy_setting_changed() {
        use crate::service::net::proxy::{ProxyConfig, ProxyMode};
        let _applied = proxy::AppliedProxy::new(ProxyConfig {
            mode: ProxyMode::Off,
            ..ProxyConfig::default()
        });
        let mut health = Health::new(
            Duration::from_secs(60),
            Keepalive::Frame,
            Route(proxy::generation()),
        );
        let due = |health: &mut Health| health.route_checked -= ROUTE_CHECK * 2;

        due(&mut health);
        assert!(!health.route_changed(), "nothing changed");
        proxy::apply(
            ProxyConfig {
                mode: ProxyMode::Manual,
                host: "127.0.0.1".to_owned(),
                port: Some(9),
                ..ProxyConfig::default()
            },
            None,
        );
        assert!(
            !health.route_changed(),
            "asked once a second at most, not on every message"
        );
        due(&mut health);
        assert!(health.route_changed(), "the proxy changed under the feed");
    }

    /// Reads a connection whose server sends `beat` every 30 ms for about half a second and then
    /// hangs up, with `max_data_silence` of 150 ms; what ended it.
    fn end_of_a_connection_that_only_beats(
        keepalive: Keepalive,
        beat: &'static str,
    ) -> tungstenite::Error {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut socket = tungstenite::accept(stream).unwrap();
            for _ in 0..16 {
                socket.send(Message::Text(beat.into())).unwrap();
                std::thread::sleep(Duration::from_millis(30));
            }
        });
        let (mut socket, route) =
            connect(&format!("ws://127.0.0.1:{port}"), "127.0.0.1", port).unwrap();
        if let MaybeTlsStream::Plain(stream) = socket.get_mut() {
            stream
                .set_read_timeout(Some(Duration::from_millis(20)))
                .unwrap();
        }
        let mut health = Health::new(Duration::from_millis(150), keepalive, route);
        let error = loop {
            if let Err(error) = health.read(&mut socket) {
                break error;
            }
        };
        server.join().unwrap();
        error
    }

    #[test]
    fn a_quiet_feed_that_hears_its_peer_is_not_cut_for_want_of_data() {
        let quiet = Keepalive::QuietText {
            ping: "ping",
            pong: "pong",
        };
        let error = end_of_a_connection_that_only_beats(quiet, "pong");
        assert!(
            !matches!(&error, tungstenite::Error::Io(io) if io.kind() == io::ErrorKind::TimedOut),
            "the heartbeat answers were heard the whole time: {error:?}"
        );
        let htx = Keepalive::EchoJson {
            ask: "ping",
            answer: "pong",
        };
        let error = end_of_a_connection_that_only_beats(htx, r#"{"ping":1}"#);
        assert!(
            !matches!(&error, tungstenite::Error::Io(io) if io.kind() == io::ErrorKind::TimedOut),
            "the server's pings were heard the whole time: {error:?}"
        );
    }

    #[test]
    fn a_stream_that_only_answers_its_heartbeat_is_cut_for_want_of_data() {
        let stream = Keepalive::Text {
            ping: "ping",
            pong: "pong",
        };
        let error = end_of_a_connection_that_only_beats(stream, "pong");
        assert!(
            matches!(&error, tungstenite::Error::Io(io) if io.kind() == io::ErrorKind::TimedOut),
            "{error:?}"
        );
    }

    #[test]
    fn silent_socket_reaches_heartbeat_deadline() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let _socket = tungstenite::accept(stream).unwrap();
            std::thread::sleep(Duration::from_millis(100));
        });
        let (mut socket, route) =
            connect(&format!("ws://127.0.0.1:{port}"), "127.0.0.1", port).unwrap();
        if let MaybeTlsStream::Plain(stream) = socket.get_mut() {
            stream
                .set_read_timeout(Some(Duration::from_millis(20)))
                .unwrap();
        }
        let mut health = Health::new(Duration::ZERO, Keepalive::Frame, route);
        assert!(matches!(
            health.read(&mut socket),
            Err(tungstenite::Error::Io(error)) if error.kind() == io::ErrorKind::TimedOut
        ));
        server.join().unwrap();
    }
}
