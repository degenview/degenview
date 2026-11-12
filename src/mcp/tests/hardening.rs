//! Rate limits, connection and header limits, and cancellation, over real loopback sockets.

use super::{BOTH, Harness, READ, call, connect_local, raw_request, rpc, status_line};
use crate::mcp::transport::limits::RateLimits;
use crate::mcp::transport::{ConnectionLimits, serve};
use serde_json::json;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;

const LIST: &str = r#"{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}"#;

pub(super) fn limits(failed_burst: u32, client_burst: u32) -> RateLimits {
    RateLimits {
        failed_burst,
        // Slow enough that nothing refills while a test runs.
        failed_per_sec: 0.001,
        client_burst,
        client_per_sec: 0.001,
    }
}

pub(super) fn head(port: u16, token: &str) -> String {
    format!("Authorization: Bearer {token}\r\nHost: 127.0.0.1:{port}\r\n")
}

/// Sends a request and reads whatever comes back, tolerating a server that answers and closes
/// before the whole request is written (an oversized head).
fn lenient_request(port: u16, head: &str, body: &str) -> String {
    let mut stream = connect_local(port);
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
    let request = format!(
        "POST /mcp HTTP/1.1\r\n{head}Content-Type: application/json\r\nAccept: application/json, text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(request.as_bytes());
    let mut response = String::new();
    let _ = stream.read_to_string(&mut response);
    response
}

#[test]
fn bad_tokens_are_slowed_down_but_a_valid_token_never_is() {
    let harness = Harness::start(&[("c1", "good", READ)]);
    harness.shared.set_rate_limits(limits(5, 1000));
    let port = harness.port;
    let url = harness.url();

    for attempt in 0..5 {
        let response = raw_request(port, &head(port, "wrong"), LIST);
        assert!(
            status_line(&response).contains("401"),
            "attempt {attempt}: {response}"
        );
    }
    // The bucket is spent: junk is now refused with a retry time, and not looked at further.
    let refused = raw_request(port, &head(port, "wrong"), LIST);
    assert!(status_line(&refused).contains("429"), "{refused}");
    let lower = refused.to_ascii_lowercase();
    assert!(lower.contains("retry-after: "), "{refused}");
    // A request with no token at all counts the same way.
    let none = raw_request(port, &format!("Host: 127.0.0.1:{port}\r\n"), LIST);
    assert!(status_line(&none).contains("429"), "{none}");

    // The real credential is untouched by any of that.
    for _ in 0..3 {
        assert_eq!(rpc(&url, Some("good"), "tools/list", json!({})).status, 200);
    }
}

#[test]
fn one_client_over_its_limit_does_not_slow_another() {
    let harness = Harness::start(&[("c1", "one", READ), ("c2", "two", READ)]);
    harness.shared.set_rate_limits(limits(1000, 3));
    let port = harness.port;
    let url = harness.url();

    for _ in 0..3 {
        assert_eq!(rpc(&url, Some("one"), "tools/list", json!({})).status, 200);
    }
    let refused = raw_request(port, &head(port, "one"), LIST);
    assert!(status_line(&refused).contains("429"), "{refused}");
    assert!(
        refused.to_ascii_lowercase().contains("retry-after: "),
        "{refused}"
    );

    for _ in 0..3 {
        assert_eq!(rpc(&url, Some("two"), "tools/list", json!({})).status, 200);
    }
}

#[test]
fn a_client_recovers_as_its_bucket_refills() {
    let harness = Harness::start(&[("c1", "tok", READ)]);
    harness.shared.set_rate_limits(RateLimits {
        client_burst: 1,
        // One token every 500 ms: slow enough that the second call still finds the bucket empty
        // on a loaded machine.
        client_per_sec: 2.0,
        ..limits(1000, 1)
    });
    let url = harness.url();
    let first = rpc(&url, Some("tok"), "tools/list", json!({}));
    assert_eq!(first.status, 200, "{}", first.body);
    let second = rpc(&url, Some("tok"), "tools/list", json!({}));
    assert_eq!(second.status, 429, "{}", second.body);
    std::thread::sleep(Duration::from_millis(700));
    let third = rpc(&url, Some("tok"), "tools/list", json!({}));
    assert_eq!(third.status, 200, "{}", third.body);
}

#[test]
fn a_revoked_and_recreated_client_starts_with_a_full_bucket() {
    let harness = Harness::start(&[("c1", "tok", READ)]);
    harness.shared.set_rate_limits(limits(1000, 1));
    let url = harness.url();
    assert_eq!(rpc(&url, Some("tok"), "tools/list", json!({})).status, 200);
    assert_eq!(rpc(&url, Some("tok"), "tools/list", json!({})).status, 429);

    let clients = harness.shared.clients();
    harness.shared.replace(Vec::new(), harness.shared.global());
    harness.shared.replace(clients, harness.shared.global());
    assert_eq!(rpc(&url, Some("tok"), "tools/list", json!({})).status, 200);
}

#[test]
fn too_many_or_too_large_headers_are_refused_and_the_server_carries_on() {
    let harness = Harness::start(&[("c1", "tok", READ)]);
    let port = harness.port;
    let url = harness.url();

    let many: String = (0..80).map(|n| format!("X-Filler-{n}: 1\r\n")).collect();
    let response = lenient_request(port, &format!("{}{many}", head(port, "tok")), LIST);
    assert!(
        status_line(&response).contains("431"),
        "many headers: {response}"
    );

    let large = format!("X-Big: {}\r\n", "a".repeat(20 * 1024));
    let response = lenient_request(port, &format!("{}{large}", head(port, "tok")), LIST);
    assert!(
        status_line(&response).contains("431"),
        "large header: {response}"
    );

    // A normal request, and one with a handful of ordinary headers, still work.
    assert_eq!(rpc(&url, Some("tok"), "tools/list", json!({})).status, 200);
    let few: String = (0..20).map(|n| format!("X-Filler-{n}: 1\r\n")).collect();
    let ok = lenient_request(port, &format!("{}{few}", head(port, "tok")), LIST);
    assert!(status_line(&ok).contains("200"), "{ok}");
}

/// A server that answers `/` with "ok", behind `serve` with the given limits.
struct Plain {
    port: u16,
    cancel: CancellationToken,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Plain {
    fn start(limits: ConnectionLimits) -> Self {
        let std_listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        std_listener.set_nonblocking(true).unwrap();
        let port = std_listener.local_addr().unwrap().port();
        let cancel = CancellationToken::new();
        let token = cancel.clone();
        let thread = std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            runtime.block_on(async {
                let listener = tokio::net::TcpListener::from_std(std_listener).unwrap();
                let app = axum::Router::new().route("/", axum::routing::get(|| async { "ok" }));
                serve(listener, app, limits, token).await;
            });
        });
        Self {
            port,
            cancel,
            thread: Some(thread),
        }
    }

    fn connect(&self) -> TcpStream {
        let stream = connect_local(self.port);
        stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
        stream
    }

    fn get(&self) -> String {
        let mut stream = self.connect();
        stream
            .write_all(b"GET / HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n")
            .unwrap();
        let mut response = String::new();
        let _ = stream.read_to_string(&mut response);
        response
    }
}

impl Drop for Plain {
    fn drop(&mut self) {
        self.cancel.cancel();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Whether the server has closed `stream` (a read returns end-of-file or an error other than
/// the read timeout) within `wait`.
fn closed_within(stream: &mut TcpStream, wait: Duration) -> bool {
    // A socket the server has already closed can refuse the option (EINVAL on macOS): closed.
    if stream.set_read_timeout(Some(wait)).is_err() {
        return true;
    }
    let mut buffer = [0u8; 256];
    match stream.read(&mut buffer) {
        Ok(0) => true,
        Ok(_) => true, // an error response, then the close
        Err(error) => !matches!(
            error.kind(),
            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
        ),
    }
}

#[test]
fn every_request_carries_the_address_of_the_socket_it_arrived_on() {
    use crate::mcp::transport::PeerAddr;
    // A handler that answers with the peer the server recorded.
    let std_listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    std_listener.set_nonblocking(true).unwrap();
    let port = std_listener.local_addr().unwrap().port();
    let cancel = CancellationToken::new();
    let token = cancel.clone();
    let thread = std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let listener = tokio::net::TcpListener::from_std(std_listener).unwrap();
            let app = axum::Router::new().route(
                "/",
                axum::routing::get(|request: axum::extract::Request| async move {
                    match request.extensions().get::<PeerAddr>() {
                        Some(peer) => format!("{} {}", peer.0.ip, peer.0.local),
                        None => "none".into(),
                    }
                }),
            );
            serve(listener, app, ConnectionLimits::default(), token).await;
        });
    });
    let mut stream = connect_local(port);
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
    stream
        .write_all(b"GET / HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n")
        .unwrap();
    let mut response = String::new();
    let _ = stream.read_to_string(&mut response);
    cancel.cancel();
    let _ = thread.join();
    assert!(response.ends_with("127.0.0.1 true"), "{response}");
}

#[test]
fn the_bearer_guard_judges_the_recorded_peer_not_anything_in_the_request() {
    use crate::mcp::state::McpShared;
    use crate::mcp::transport::PeerAddr;
    use crate::mcp::transport::auth::require_bearer;
    use tower::Service;

    let shared = McpShared::new();
    let mut remote = super::client("remote", "tok-remote", READ);
    remote.remote_allowed = true;
    remote.allowed_cidrs = vec![crate::model::mcp::Cidr::parse("192.168.1.0/24").unwrap()];
    shared.replace(
        vec![super::client("local", "tok-local", READ), remote],
        crate::model::mcp::PermissionSet::everything(),
    );
    let app = axum::Router::new()
        .route("/", axum::routing::get(|| async { "in" }))
        .layer(axum::middleware::from_fn_with_state(shared, require_bearer));

    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    let status = |token: &str, peer: Option<&str>, spoof: &str| {
        let mut request = axum::http::Request::builder()
            .uri("/")
            .header("authorization", format!("Bearer {token}"))
            // What a client might send to look like it is local; nothing reads it.
            .header("x-forwarded-for", spoof)
            .header("host", "127.0.0.1")
            .body(axum::body::Body::empty())
            .unwrap();
        if let Some(peer) = peer {
            request
                .extensions_mut()
                .insert(PeerAddr(peer.parse().unwrap()));
        }
        let mut app = app.clone();
        runtime.block_on(async move { app.call(request).await.unwrap().status().as_u16() })
    };
    assert_eq!(status("tok-local", Some("127.0.0.1"), ""), 200);
    assert_eq!(status("tok-local", Some("192.168.1.9"), "127.0.0.1"), 401);
    assert_eq!(status("tok-remote", Some("192.168.1.9"), ""), 200);
    assert_eq!(
        status("tok-remote", Some("203.0.113.9"), "192.168.1.9"),
        401
    );
    // No recorded peer: refused, not assumed local.
    assert_eq!(status("tok-local", None, "127.0.0.1"), 401);
}

#[test]
fn one_remote_address_guessing_tokens_is_cut_off_without_slowing_this_machine() {
    use crate::mcp::state::McpShared;
    use crate::mcp::transport::PeerAddr;
    use crate::mcp::transport::auth::require_bearer;
    use crate::mcp::transport::limits::PEER_FAILED_BURST;
    use tower::Service;

    let shared = McpShared::new();
    shared.replace(
        vec![super::client("local", "tok-local", READ)],
        crate::model::mcp::PermissionSet::everything(),
    );
    let app = axum::Router::new()
        .route("/", axum::routing::get(|| async { "in" }))
        .layer(axum::middleware::from_fn_with_state(
            shared.clone(),
            require_bearer,
        ));
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    let status = |token: &str, peer: &str| {
        let mut request = axum::http::Request::builder()
            .uri("/")
            .header("authorization", format!("Bearer {token}"))
            .body(axum::body::Body::empty())
            .unwrap();
        request
            .extensions_mut()
            .insert(PeerAddr(peer.parse().unwrap()));
        let mut app = app.clone();
        runtime.block_on(async move { app.call(request).await.unwrap().status().as_u16() })
    };
    for _ in 0..PEER_FAILED_BURST {
        assert_eq!(status("guess", "203.0.113.9"), 401);
    }
    assert_eq!(
        status("guess", "203.0.113.9"),
        429,
        "its own bucket is spent"
    );
    // Another address, and this machine, are not slowed by it...
    assert_eq!(status("guess", "198.51.100.4"), 401);
    assert_eq!(status("guess", "127.0.0.1"), 401);
    // ...and a valid token from this machine is never refused for anyone's failures.
    assert_eq!(status("tok-local", "127.0.0.1"), 200);

    // Only the first PEER_FAILED_BURST (plus the two above) reached the log, as one row.
    let rows = shared.audit().take(0, true);
    let refused: Vec<_> = rows
        .iter()
        .filter(|r| r.kind == crate::model::mcp::McpAuditKind::AuthRefused)
        .collect();
    assert_eq!(refused.len(), 2, "one row per remote address: {refused:?}");
    let first = refused.iter().find(|r| r.peer == "203.0.113.9").unwrap();
    assert_eq!(first.count, PEER_FAILED_BURST + 1);
}

#[test]
fn a_peer_the_server_recorded_as_this_machine_is_this_machine_whatever_its_address() {
    use crate::mcp::state::McpShared;
    use crate::mcp::transport::PeerAddr;
    use crate::mcp::transport::auth::require_bearer;
    use crate::model::mcp::Peer;
    use tower::Service;

    let shared = McpShared::new();
    shared.replace(
        vec![super::client("local", "tok-local", READ)],
        crate::model::mcp::PermissionSet::everything(),
    );
    let app = axum::Router::new()
        .route("/", axum::routing::get(|| async { "in" }))
        .layer(axum::middleware::from_fn_with_state(shared, require_bearer));
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    let status = |peer: Peer| {
        let mut request = axum::http::Request::builder()
            .uri("/")
            .header("authorization", "Bearer tok-local")
            .body(axum::body::Body::empty())
            .unwrap();
        request.extensions_mut().insert(PeerAddr(peer));
        let mut app = app.clone();
        runtime.block_on(async move { app.call(request).await.unwrap().status().as_u16() })
    };
    let own: std::net::IpAddr = "192.168.1.20".parse().unwrap();
    // A client on this Mac using the address the server listens on arrives from that address.
    assert_eq!(status(Peer::of_connection(own, own)), 200);
    assert_eq!(
        status(Peer::of_connection("192.168.1.21".parse().unwrap(), own)),
        401
    );
}

/// This machine's address on the local network, when it has one (the system's answer to "which
/// address would reach 192.0.2.1"; nothing is sent).
fn lan_address() -> Option<std::net::IpAddr> {
    let socket = std::net::UdpSocket::bind(("0.0.0.0", 0)).ok()?;
    socket.connect(("192.0.2.1", 9)).ok()?;
    let address = socket.local_addr().ok()?.ip();
    (!address.is_unspecified() && !address.is_loopback()).then_some(address)
}

#[test]
fn a_connection_to_this_machines_own_address_is_recorded_as_this_machine() {
    use crate::mcp::transport::PeerAddr;
    // Needs an address of its own on a network; a machine without one has nothing to check.
    let Some(own) = lan_address() else {
        return;
    };
    let std_listener = std::net::TcpListener::bind((own, 0)).unwrap();
    std_listener.set_nonblocking(true).unwrap();
    let port = std_listener.local_addr().unwrap().port();
    let cancel = CancellationToken::new();
    let token = cancel.clone();
    let thread = std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let listener = tokio::net::TcpListener::from_std(std_listener).unwrap();
            let app = axum::Router::new().route(
                "/",
                axum::routing::get(|request: axum::extract::Request| async move {
                    match request.extensions().get::<PeerAddr>() {
                        Some(peer) => format!("{} {}", peer.0.ip, peer.0.local),
                        None => "none".into(),
                    }
                }),
            );
            serve(listener, app, ConnectionLimits::default(), token).await;
        });
    });
    let mut stream = TcpStream::connect((own, port)).unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
    stream
        .write_all(b"GET / HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n")
        .unwrap();
    let mut response = String::new();
    let _ = stream.read_to_string(&mut response);
    cancel.cancel();
    let _ = thread.join();
    assert!(
        response.ends_with(&format!("{own} true")),
        "a client on this machine, using the machine's own address, is local: {response}"
    );
}

#[test]
fn a_client_that_never_finishes_its_headers_is_dropped() {
    let server = Plain::start(ConnectionLimits {
        header_read_timeout: Duration::from_millis(300),
        ..ConnectionLimits::default()
    });
    let mut slow = server.connect();
    slow.write_all(b"GET / HTTP/1.1\r\nHost: x\r\n").unwrap();
    assert!(
        closed_within(&mut slow, Duration::from_secs(3)),
        "the half-sent request was never cut off"
    );
    assert!(server.get().contains("ok"));
}

#[test]
fn connections_past_the_cap_are_closed_and_slots_come_back() {
    let server = Plain::start(ConnectionLimits {
        max_connections: 3,
        header_read_timeout: Duration::from_secs(30),
        ..ConnectionLimits::default()
    });
    // Three connections, each part-way through a request, hold every slot.
    let mut held: Vec<TcpStream> = (0..3)
        .map(|_| {
            let mut stream = server.connect();
            stream.write_all(b"GET / HTTP/1.1\r\nHost: x\r\n").unwrap();
            stream
        })
        .collect();
    std::thread::sleep(Duration::from_millis(200));

    let mut extra = server.connect();
    let _ = extra.write_all(b"GET / HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n");
    assert!(
        closed_within(&mut extra, Duration::from_secs(3)),
        "a fourth connection was served"
    );
    let mut answered = String::new();
    let _ = extra.read_to_string(&mut answered);
    assert!(!answered.contains("ok"), "{answered}");

    // Free a slot and the next connection is served again.
    held.pop();
    std::thread::sleep(Duration::from_millis(300));
    assert!(server.get().contains("ok"));
}

#[test]
fn stopping_with_connections_held_open_is_prompt() {
    let mut harness = Harness::start(&[("c1", "tok", READ)]);
    let port = harness.port;
    let _held: Vec<TcpStream> = (0..8)
        .map(|_| {
            let mut stream = connect_local(port);
            stream
                .write_all(b"POST /mcp HTTP/1.1\r\nHost: x\r\n")
                .unwrap();
            stream
        })
        .collect();
    std::thread::sleep(Duration::from_millis(100));
    let started = Instant::now();
    harness.stop_server();
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "stop took {:?}",
        started.elapsed()
    );
}

#[test]
fn a_call_dropped_by_the_client_never_runs() {
    let harness = Harness::start(&[("c1", "tok", BOTH)]);
    let port = harness.port;
    harness.pause_app(true);

    // Send an add-chart call and hang up without waiting for the answer.
    let body = json!({
        "jsonrpc": "2.0", "id": 9, "method": "tools/call",
        "params": {"name": "degenview_add_chart",
                   "arguments": {"workspace_id": "1", "source": "binance", "symbol": "ETHUSDT"}}
    })
    .to_string();
    {
        let mut stream = connect_local(port);
        let request = format!(
            "POST /mcp HTTP/1.1\r\n{}MCP-Protocol-Version: 2025-06-18\r\nContent-Type: application/json\r\nAccept: application/json, text/event-stream\r\nContent-Length: {}\r\n\r\n{body}",
            head(port, "tok"),
            body.len()
        );
        stream.write_all(request.as_bytes()).unwrap();
        // Let the call reach the bridge queue, then hang up.
        std::thread::sleep(Duration::from_millis(300));
    }
    std::thread::sleep(Duration::from_millis(300));
    harness.pause_app(false);
    std::thread::sleep(Duration::from_millis(500));
    assert!(
        harness.added.lock().unwrap().is_empty(),
        "a call the client had given up on still changed the app"
    );

    // The server is unharmed: the same call from a client that waits goes through.
    let added = call(
        &harness.url(),
        "tok",
        "degenview_add_chart",
        json!({"workspace_id": "1", "source": "binance", "symbol": "ETHUSDT"}),
    );
    assert_eq!(added["isError"], json!(false), "{added}");
    assert_eq!(harness.added.lock().unwrap().len(), 1);
}

pub(super) fn add_chart_call(id: serde_json::Value) -> String {
    json!({
        "jsonrpc": "2.0", "id": id, "method": "tools/call",
        "params": {"name": "degenview_add_chart",
                   "arguments": {"workspace_id": "1", "source": "binance", "symbol": "ETHUSDT"}}
    })
    .to_string()
}

pub(super) fn cancel_message(request_id: serde_json::Value) -> String {
    json!({"jsonrpc": "2.0", "method": "notifications/cancelled",
           "params": {"requestId": request_id, "reason": "test"}})
    .to_string()
}

/// Starts `body` as a call that waits on a paused app, and returns what it eventually gets.
pub(super) fn start_call(
    url: &str,
    token: &str,
    body: String,
) -> std::thread::JoinHandle<super::Reply> {
    let url = url.to_owned();
    let token = token.to_owned();
    std::thread::spawn(move || super::post(&url, Some(&token), &[super::PROTOCOL], &body))
}

pub(super) fn wait_for_calls(harness: &Harness, running: usize) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while harness.shared.calls().running() != running {
        assert!(
            Instant::now() < deadline,
            "{} calls registered, expected {running}",
            harness.shared.calls().running()
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn a_cancel_notification_stops_the_queued_call() {
    let harness = Harness::start(&[("c1", "tok", BOTH)]);
    let url = harness.url();
    harness.pause_app(true);
    let waiting = start_call(&url, "tok", add_chart_call(json!(9)));
    wait_for_calls(&harness, 1);

    let reply = super::post(
        &url,
        Some("tok"),
        &[super::PROTOCOL],
        &cancel_message(json!(9)),
    );
    assert_eq!(reply.status, 202, "{}", reply.body);
    let started = Instant::now();
    let original = waiting.join().unwrap();
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "the call was not released by the cancel"
    );
    assert!(original.body.contains("cancelled"), "{}", original.body);

    harness.pause_app(false);
    std::thread::sleep(Duration::from_millis(500));
    assert!(
        harness.added.lock().unwrap().is_empty(),
        "a cancelled call still changed the app"
    );
    wait_for_calls(&harness, 0);

    // The server is unharmed.
    let added = call(
        &url,
        "tok",
        "degenview_add_chart",
        json!({"workspace_id": "1", "source": "binance", "symbol": "ETHUSDT"}),
    );
    assert_eq!(added["isError"], json!(false), "{added}");
}

#[test]
fn a_cancel_names_a_string_id_and_only_that_id() {
    let harness = Harness::start(&[("c1", "tok", BOTH)]);
    let url = harness.url();
    harness.pause_app(true);
    let kept = start_call(&url, "tok", add_chart_call(json!("7")));
    let dropped = start_call(&url, "tok", add_chart_call(json!("job-a")));
    wait_for_calls(&harness, 2);

    // The number 7 is not the string "7"; "job-a" is.
    super::post(
        &url,
        Some("tok"),
        &[super::PROTOCOL],
        &cancel_message(json!(7)),
    );
    super::post(
        &url,
        Some("tok"),
        &[super::PROTOCOL],
        &cancel_message(json!("job-a")),
    );
    let dropped = dropped.join().unwrap();
    assert!(dropped.body.contains("cancelled"), "{}", dropped.body);

    harness.pause_app(false);
    let kept = kept.join().unwrap();
    assert!(kept.body.contains("created"), "{}", kept.body);
    assert_eq!(harness.added.lock().unwrap().len(), 1);
}

#[test]
fn one_client_cannot_cancel_anothers_call() {
    let harness = Harness::start(&[("c1", "one", BOTH), ("c2", "two", BOTH)]);
    let url = harness.url();
    harness.pause_app(true);
    let waiting = start_call(&url, "one", add_chart_call(json!(9)));
    wait_for_calls(&harness, 1);

    // c2 names the same request id; it is answered the same way and changes nothing for c1.
    let reply = super::post(
        &url,
        Some("two"),
        &[super::PROTOCOL],
        &cancel_message(json!(9)),
    );
    assert_eq!(reply.status, 202);
    // An unknown id, and one with no token at all, do nothing either.
    super::post(
        &url,
        Some("one"),
        &[super::PROTOCOL],
        &cancel_message(json!(12345)),
    );
    let anonymous = super::post(&url, None, &[super::PROTOCOL], &cancel_message(json!(9)));
    assert_eq!(anonymous.status, 401);

    harness.pause_app(false);
    let original = waiting.join().unwrap();
    assert!(original.body.contains("created"), "{}", original.body);
    assert_eq!(harness.added.lock().unwrap().len(), 1);
}

#[test]
fn a_cancel_gets_through_while_every_permit_is_held() {
    let harness = Harness::start(&[("c1", "tok", BOTH)]);
    let url = harness.url();
    harness.shared.set_rate_limits(limits(1000, 1000));
    harness.pause_app(true);
    // Sixteen queued calls hold every request permit.
    let calls: Vec<_> = (0..16)
        .map(|n| start_call(&url, "tok", add_chart_call(json!(n))))
        .collect();
    wait_for_calls(&harness, 16);

    let started = Instant::now();
    let reply = super::post(
        &url,
        Some("tok"),
        &[super::PROTOCOL],
        &cancel_message(json!(0)),
    );
    assert_eq!(reply.status, 202, "{}", reply.body);
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "the cancel waited for a permit ({:?})",
        started.elapsed()
    );
    // The 202 only says the cancel was noted. Wait until the server has dropped the call before
    // letting the app run, or the app could take the request in the gap and run it.
    wait_for_calls(&harness, 15);

    harness.pause_app(false);
    let replies: Vec<_> = calls.into_iter().map(|call| call.join().unwrap()).collect();
    assert!(replies[0].body.contains("cancelled"), "{}", replies[0].body);
    assert_eq!(harness.added.lock().unwrap().len(), 15);
    wait_for_calls(&harness, 0);
}

#[test]
fn finished_calls_leave_nothing_registered() {
    let harness = Harness::start(&[("c1", "tok", BOTH)]);
    let url = harness.url();
    for _ in 0..3 {
        let reply = call(&url, "tok", "degenview_list_workspaces", json!({}));
        assert_eq!(reply["isError"], json!(false));
    }
    let _ = rpc(&url, Some("tok"), "tools/list", json!({}));
    wait_for_calls(&harness, 0);
}
