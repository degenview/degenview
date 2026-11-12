//! Remote access: a server on a named interface is served over TLS or not at all, and the
//! certificate a client pins is the one that is presented.
//!
//! No test can assume the machine has a LAN address, so a TLS listener is exercised on loopback
//! with a certificate made for 127.0.0.1; the refusals are checked with an address from the
//! documentation range, which would fail to bind if the check did not come first.

use super::hardening::head;
use super::{READ, client};
use crate::mcp::bridge;
use crate::mcp::state::{McpShared, McpStatus};
use crate::mcp::transport::tls::TlsIdentity;
use crate::mcp::transport::{ConnectionLimits, serve_with};
use crate::mcp::{Listen, McpServer};
use crate::model::mcp::{BindMode, Permission, PermissionSet};
use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, TcpStream};
use std::sync::Arc;
use std::time::Duration;
use tokio_rustls::rustls::pki_types::ServerName;
use tokio_rustls::rustls::{self, ClientConfig, ClientConnection, RootCertStore, StreamOwned};
use tokio_util::sync::CancellationToken;

const LIST: &str = r#"{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}"#;
const LOCAL: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);

fn documentation_address() -> IpAddr {
    "192.0.2.1".parse().unwrap()
}

/// A client that trusts exactly the certificate of `trusted`, as a pinning client does.
fn pinned(trusted: &TlsIdentity) -> Arc<ClientConfig> {
    let mut roots = RootCertStore::empty();
    roots.add(trusted.certificate_der()).unwrap();
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    Arc::new(
        ClientConfig::builder_with_provider(provider)
            .with_protocol_versions(&[&rustls::version::TLS13])
            .unwrap()
            .with_root_certificates(roots)
            .with_no_client_auth(),
    )
}

/// Sends `request` over TLS and returns everything read back; an error is a failed handshake.
fn tls_request(port: u16, config: Arc<ClientConfig>, request: &str) -> Result<String, String> {
    let tcp = TcpStream::connect(("127.0.0.1", port)).map_err(|e| e.to_string())?;
    tcp.set_read_timeout(Some(Duration::from_secs(5))).ok();
    tcp.set_write_timeout(Some(Duration::from_secs(5))).ok();
    let connection = ClientConnection::new(config, ServerName::IpAddress(LOCAL.into()))
        .map_err(|e| e.to_string())?;
    let mut stream = StreamOwned::new(connection, tcp);
    stream
        .write_all(request.as_bytes())
        .and_then(|()| stream.flush())
        .map_err(|e| e.to_string())?;
    // The server closes without a TLS close_notify, which reads as an error after the data.
    let mut bytes = Vec::new();
    let mut buffer = [0u8; 4096];
    loop {
        match stream.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(n) => bytes.extend_from_slice(&buffer[..n]),
        }
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

fn mcp_request(head: &str) -> String {
    format!(
        "POST /mcp HTTP/1.1\r\n{head}Content-Type: application/json\r\nAccept: application/json, text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{LIST}",
        LIST.len()
    )
}

struct Tls {
    _server: McpServer,
    identity: TlsIdentity,
    port: u16,
}

impl Tls {
    /// The real server and router, listening on loopback behind a certificate for 127.0.0.1.
    fn start() -> Self {
        let shared = McpShared::new();
        shared.replace(
            vec![client("c1", "tok", READ)],
            PermissionSet::of(Permission::ALL),
        );
        let (bridge, _inbox) = bridge::channel(shared.clone(), Arc::new(|| {}));
        let identity = TlsIdentity::generate(LOCAL).unwrap();
        let server = McpServer::start_listening(
            shared.clone(),
            bridge,
            Listen {
                bind: BindMode::Loopback,
                port: 0,
                tls: Some(identity.clone()),
            },
        )
        .expect("server starts");
        let port = match shared.status() {
            McpStatus::Running { port } => port,
            other => panic!("not running: {other:?}"),
        };
        Self {
            _server: server,
            identity,
            port,
        }
    }
}

#[test]
fn a_named_interface_without_tls_is_refused_before_any_socket_opens() {
    let shared = McpShared::new();
    let (bridge, _inbox) = bridge::channel(shared.clone(), Arc::new(|| {}));
    let error = McpServer::start_listening(
        shared.clone(),
        bridge,
        Listen {
            bind: BindMode::interface(documentation_address()).unwrap(),
            port: 0,
            tls: None,
        },
    )
    .err()
    .expect("must be refused");
    // The TLS check, not "could not listen": nothing was bound.
    assert!(error.contains("TLS certificate"), "{error}");
    assert!(error.contains("plain HTTP"), "{error}");
    assert!(matches!(shared.status(), McpStatus::Failed(message) if message == error));
}

#[test]
fn an_address_that_could_not_have_been_chosen_is_refused_even_with_a_matching_certificate() {
    // `BindMode::Interface` can be written down without going through `BindMode::interface`, so
    // the one place that opens the socket judges the address itself. Each of these has a
    // certificate made for exactly its address, which is all the other checks look at.
    for text in [
        "0.0.0.0",
        "::",
        "224.0.0.1",
        "ff02::1",
        "255.255.255.255",
        "fe80::1",
        // This machine's own address, which is `Loopback` and nothing else.
        "127.0.0.1",
        "::1",
    ] {
        let address: IpAddr = text.parse().unwrap();
        // A port nothing holds, so a socket opened after all would show.
        let port = {
            let probe = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
            probe.local_addr().unwrap().port()
        };
        let shared = McpShared::new();
        let (bridge, _inbox) = bridge::channel(shared.clone(), Arc::new(|| {}));
        let error = McpServer::start_listening(
            shared.clone(),
            bridge,
            Listen {
                bind: BindMode::Interface(address),
                port,
                tls: Some(TlsIdentity::generate(address).unwrap()),
            },
        )
        .err()
        .unwrap_or_else(|| panic!("{text} must be refused"));
        assert!(
            !error.contains("Could not listen") && !error.contains("already in use"),
            "{text} reached the socket: {error}"
        );
        assert!(matches!(shared.status(), McpStatus::Failed(message) if message == error));
        assert!(
            std::net::TcpListener::bind(("127.0.0.1", port)).is_ok(),
            "{text} left a listener on the port"
        );
    }
}

#[test]
fn a_certificate_made_for_another_address_is_refused() {
    let shared = McpShared::new();
    let (bridge, _inbox) = bridge::channel(shared.clone(), Arc::new(|| {}));
    let other = TlsIdentity::generate("192.0.2.2".parse().unwrap()).unwrap();
    let error = McpServer::start_listening(
        shared.clone(),
        bridge,
        Listen {
            bind: BindMode::interface(documentation_address()).unwrap(),
            port: 0,
            tls: Some(other),
        },
    )
    .err()
    .expect("must be refused");
    assert!(
        error.contains("192.0.2.2") && error.contains("192.0.2.1"),
        "{error}"
    );
    assert!(matches!(shared.status(), McpStatus::Failed(_)));
}

#[test]
fn the_endpoint_answers_over_tls_to_a_client_that_pins_the_certificate() {
    let server = Tls::start();
    let config = pinned(&server.identity);
    let port = server.port;

    let listed = tls_request(port, config.clone(), &mcp_request(&head(port, "tok"))).unwrap();
    assert!(listed.starts_with("HTTP/1.1 200"), "{listed}");
    assert!(listed.contains("degenview_list_charts"), "{listed}");

    // The bearer check and the Host check sit behind TLS exactly as they do without it.
    let no_token = tls_request(
        port,
        config.clone(),
        &mcp_request(&format!("Host: 127.0.0.1:{port}\r\n")),
    )
    .unwrap();
    assert!(no_token.starts_with("HTTP/1.1 401"), "{no_token}");
    let wrong_host = tls_request(
        port,
        config,
        &mcp_request("Authorization: Bearer tok\r\nHost: evil.example:80\r\n"),
    )
    .unwrap();
    assert!(!wrong_host.starts_with("HTTP/1.1 200"), "{wrong_host}");
}

#[test]
fn a_client_pinning_another_certificate_does_not_get_in() {
    let server = Tls::start();
    let impostor = TlsIdentity::generate(LOCAL).unwrap();
    assert_ne!(impostor.fingerprint(), server.identity.fingerprint());
    let port = server.port;
    let outcome = tls_request(port, pinned(&impostor), &mcp_request(&head(port, "tok")));
    // The client aborts the handshake, so it never sends the request or the token.
    let response = outcome.unwrap_or_default();
    assert!(!response.contains("HTTP/1.1"), "{response}");
}

#[test]
fn plain_http_to_a_tls_port_is_dropped_without_an_answer() {
    let server = Tls::start();
    let port = server.port;
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
    let request = mcp_request(&head(port, "tok"));
    let _ = stream.write_all(request.as_bytes());
    let mut response = Vec::new();
    let _ = stream.read_to_end(&mut response);
    let text = String::from_utf8_lossy(&response);
    // What may come back is a TLS alert, never an HTTP response: the token was not honoured.
    assert!(!text.contains("HTTP/1."), "{text}");
    assert!(!text.contains("degenview_"), "{text}");
}

#[test]
fn a_connection_that_never_finishes_the_handshake_is_dropped_and_its_slot_freed() {
    let identity = TlsIdentity::generate(LOCAL).unwrap();
    let acceptor = tokio_rustls::TlsAcceptor::from(identity.server_config().unwrap());
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
            let limits = ConnectionLimits {
                max_connections: 1,
                tls_handshake_timeout: Duration::from_millis(300),
                ..ConnectionLimits::default()
            };
            serve_with(listener, app, limits, Some(acceptor), token).await;
        });
    });

    // One slot, held by a client that connects and says nothing.
    let mut silent = TcpStream::connect(("127.0.0.1", port)).unwrap();
    silent.set_read_timeout(Some(Duration::from_secs(3))).ok();
    let mut buffer = [0u8; 16];
    let closed = match silent.read(&mut buffer) {
        Ok(_) => true,
        Err(error) => !matches!(
            error.kind(),
            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
        ),
    };
    assert!(closed, "the silent connection was never cut off");

    // The slot is back: a real client now gets through.
    let get = "GET / HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n";
    let served = tls_request(port, pinned(&identity), get).unwrap();
    assert!(served.starts_with("HTTP/1.1 200"), "{served}");

    cancel.cancel();
    let _ = thread.join();
}
