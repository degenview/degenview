//! Read deadlines, heartbeat checks, and reconnect pacing for live market feeds.

use super::proxy;
use std::io;
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use tungstenite::{Message, WebSocket, stream::MaybeTlsStream};

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
    // Read first: a change while dialling then counts as a change after.
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
}

impl Health {
    pub fn new(max_data_silence: Duration, route: Route) -> Self {
        let now = Instant::now();
        Self {
            route,
            route_checked: now,
            last_message: now,
            last_data: now,
            ping_sent: None,
            max_data_silence,
        }
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

    /// Whether the proxy in force is no longer the one this connection was made under. Asked
    /// once a second at most, not on every message.
    fn route_changed(&mut self) -> bool {
        if self.route_checked.elapsed() < ROUTE_CHECK {
            return false;
        }
        self.route_checked = Instant::now();
        Route(proxy::generation()) != self.route
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
            socket.send(Message::Ping(Vec::new().into()))?;
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

    /// A feed that was connected under one proxy setting reconnects when it changes, instead of
    /// keeping the route it was dialled with.
    #[test]
    fn a_feed_notices_that_the_proxy_setting_changed() {
        use crate::service::net::proxy::{ProxyConfig, ProxyMode};
        let _applied = proxy::AppliedProxy::new(ProxyConfig {
            mode: ProxyMode::Off,
            ..ProxyConfig::default()
        });
        let mut health = Health::new(Duration::from_secs(60), Route(proxy::generation()));
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
        let mut health = Health::new(Duration::ZERO, route);
        assert!(matches!(
            health.read(&mut socket),
            Err(tungstenite::Error::Io(error)) if error.kind() == io::ErrorKind::TimedOut
        ));
        server.join().unwrap();
    }
}
