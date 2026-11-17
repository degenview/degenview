//! Read deadlines, heartbeat checks, and reconnect pacing for live market feeds.

use std::io;
use std::net::TcpStream;
use std::net::ToSocketAddrs;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use tungstenite::{Message, WebSocket, stream::MaybeTlsStream};

const READ_TIMEOUT: Duration = Duration::from_secs(5);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(8);
const PING_AFTER: Duration = Duration::from_secs(15);
const PONG_TIMEOUT: Duration = Duration::from_secs(10);

/// Bound TCP connect and TLS/WebSocket handshakes before entering the read loop.
pub fn connect(
    url: &str,
    host: &str,
    port: u16,
) -> Result<WebSocket<MaybeTlsStream<TcpStream>>, String> {
    let addresses = (host, port)
        .to_socket_addrs()
        .map_err(|error| error.to_string())?;
    let mut last_error = None;
    for address in addresses.take(4) {
        match TcpStream::connect_timeout(&address, CONNECT_TIMEOUT) {
            Ok(stream) => {
                stream
                    .set_read_timeout(Some(CONNECT_TIMEOUT))
                    .map_err(|error| error.to_string())?;
                stream
                    .set_write_timeout(Some(CONNECT_TIMEOUT))
                    .map_err(|error| error.to_string())?;
                let (mut socket, _) =
                    tungstenite::client_tls(url, stream).map_err(|error| error.to_string())?;
                configure(&mut socket).map_err(|error| error.to_string())?;
                return Ok(socket);
            }
            Err(error) => last_error = Some(error),
        }
    }
    Err(last_error.map_or_else(
        || "no address for WebSocket host".to_owned(),
        |error| error.to_string(),
    ))
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

/// How a quiet socket is probed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Keepalive {
    /// A WebSocket ping frame, answered by a pong frame.
    Frame,
    /// A text message answered by a text message, for servers that define their own heartbeat
    /// (OKX: `"ping"` and `"pong"`). The answer is not data.
    Text {
        ping: &'static str,
        pong: &'static str,
    },
}

pub struct Health {
    last_message: Instant,
    last_data: Instant,
    ping_sent: Option<Instant>,
    max_data_silence: Duration,
    keepalive: Keepalive,
}

impl Health {
    pub fn new(max_data_silence: Duration, keepalive: Keepalive) -> Self {
        let now = Instant::now();
        Self {
            last_message: now,
            last_data: now,
            ping_sent: None,
            max_data_silence,
            keepalive,
        }
    }

    /// A timeout lets the owning thread check shutdown and probe a quiet socket.
    // `tungstenite::Error` is large, but it is the error type the socket API hands back.
    #[allow(clippy::result_large_err)]
    pub fn read(
        &mut self,
        socket: &mut WebSocket<MaybeTlsStream<TcpStream>>,
    ) -> tungstenite::Result<Option<Message>> {
        match socket.read() {
            Ok(Message::Pong(_)) => {
                self.last_message = Instant::now();
                self.ping_sent = None;
                Ok(None)
            }
            Ok(Message::Text(text)) if matches!(self.keepalive, Keepalive::Text { pong, .. } if text.as_str() == pong) =>
            {
                self.last_message = Instant::now();
                self.ping_sent = None;
                Ok(None)
            }
            Ok(message) => {
                let now = Instant::now();
                self.last_message = now;
                self.last_data = now;
                self.ping_sent = None;
                Ok(Some(message))
            }
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

    #[allow(clippy::result_large_err)]
    fn on_timeout(
        &mut self,
        socket: &mut WebSocket<MaybeTlsStream<TcpStream>>,
    ) -> tungstenite::Result<Option<Message>> {
        let now = Instant::now();
        if now.duration_since(self.last_data) >= self.max_data_silence
            || self
                .ping_sent
                .is_some_and(|sent| now.duration_since(sent) >= PONG_TIMEOUT)
        {
            return Err(tungstenite::Error::Io(io::Error::new(
                io::ErrorKind::TimedOut,
                "websocket heartbeat timed out",
            )));
        }
        if self.ping_sent.is_none() && now.duration_since(self.last_message) >= PING_AFTER {
            socket.send(match self.keepalive {
                Keepalive::Frame => Message::Ping(Vec::new().into()),
                Keepalive::Text { ping, .. } => Message::Text(ping.into()),
            })?;
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
    /// Sent once after every (re)connect; `None` for a feed that subscribes through its URL.
    pub subscribe: Option<String>,
    pub max_data_silence: Duration,
    pub keepalive: Keepalive,
}

impl Session {
    /// Connects, subscribes and reads until `shutdown`, reconnecting with backoff whenever the
    /// socket fails. Every text message goes to `on_text`; it answers whether to keep reading
    /// (`false` when nobody is listening any more).
    pub fn run(&self, shutdown: &AtomicBool, mut on_text: impl FnMut(&str) -> bool) {
        let mut backoff = Backoff::new();
        while !shutdown.load(Ordering::Acquire) {
            if let Ok(mut socket) = connect(&self.url, &self.host, self.port) {
                let subscribed = self.subscribe.as_ref().is_none_or(|message| {
                    socket.send(Message::Text(message.as_str().into())).is_ok()
                });
                if subscribed {
                    let mut health = Health::new(self.max_data_silence, self.keepalive);
                    while !shutdown.load(Ordering::Acquire) {
                        match health.read(&mut socket) {
                            Ok(Some(Message::Text(text))) => {
                                backoff.reset();
                                if !on_text(text.as_str()) {
                                    return;
                                }
                            }
                            Ok(Some(Message::Close(_))) | Err(_) => break,
                            Ok(_) => {}
                        }
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

    /// A server that reads one message, answers `replies`, then lingers so the client stays up.
    fn serve(replies: &'static [&'static str]) -> (u16, std::thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut socket = tungstenite::accept(stream).unwrap();
            let received = socket.read().unwrap().into_text().unwrap().to_string();
            for reply in replies {
                socket.send(Message::Text((*reply).into())).unwrap();
            }
            std::thread::sleep(Duration::from_millis(300));
            received
        });
        (port, handle)
    }

    #[test]
    fn a_session_subscribes_and_hands_over_what_it_reads() {
        let (port, server) = serve(&["tick"]);
        let session = Session {
            url: format!("ws://127.0.0.1:{port}"),
            host: "127.0.0.1".to_owned(),
            port,
            subscribe: Some("subscribe".to_owned()),
            max_data_silence: Duration::from_secs(60),
            keepalive: Keepalive::Frame,
        };
        let mut seen = Vec::new();
        session.run(&AtomicBool::new(false), |text| {
            seen.push(text.to_owned());
            false
        });
        assert_eq!(seen, ["tick"]);
        assert_eq!(server.join().unwrap(), "subscribe");
    }

    #[test]
    fn a_text_heartbeat_answer_is_not_data() {
        let (port, server) = serve(&["pong", "tick"]);
        let session = Session {
            url: format!("ws://127.0.0.1:{port}"),
            host: "127.0.0.1".to_owned(),
            port,
            subscribe: Some("subscribe".to_owned()),
            max_data_silence: Duration::from_secs(60),
            keepalive: Keepalive::Text {
                ping: "ping",
                pong: "pong",
            },
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
    fn silent_socket_reaches_heartbeat_deadline() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let _socket = tungstenite::accept(stream).unwrap();
            std::thread::sleep(Duration::from_millis(100));
        });
        let mut socket = connect(&format!("ws://127.0.0.1:{port}"), "127.0.0.1", port).unwrap();
        if let MaybeTlsStream::Plain(stream) = socket.get_mut() {
            stream
                .set_read_timeout(Some(Duration::from_millis(20)))
                .unwrap();
        }
        let mut health = Health::new(Duration::ZERO, Keepalive::Frame);
        assert!(matches!(
            health.read(&mut socket),
            Err(tungstenite::Error::Io(error)) if error.kind() == io::ErrorKind::TimedOut
        ));
        server.join().unwrap();
    }
}
