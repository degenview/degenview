//! The HTTP surface: one `/mcp` endpoint (MCP Streamable HTTP, served statelessly by
//! `rmcp`) behind a bearer-token guard, a request timeout and a concurrency cap.
//!
//! `rmcp` itself validates the `Host` header against an explicit list (the loopback names, or the
//! one interface address the server was told to listen on), rejects every
//! request carrying an `Origin` (a browser page has no business here), and caps the body size.
//!
//! Connections are accepted here rather than by `axum::serve`, which offers no way to bound what
//! a connection may send before a request exists: [`serve`] caps the number of headers, their
//! size, the time a client may take to send them, and the number of open connections.

pub mod auth;
pub mod cancel;
pub mod limits;
pub mod seal;
pub mod tls;
pub(in crate::mcp) mod tls_protect;

use crate::mcp::bridge::BridgeClient;
use crate::mcp::state::McpShared;
use crate::mcp::tools::DegenViewMcp;
use crate::mcp::transport::auth::require_bearer;
use crate::mcp::transport::cancel::honour_cancellations;
use crate::model::mcp::Peer;
use axum::Router;
use axum::extract::Request;
use axum::http::StatusCode;
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use rmcp::transport::streamable_http_server::session::never::NeverSessionManager;
use rmcp::transport::{StreamableHttpServerConfig, StreamableHttpService};
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use tower::limit::ConcurrencyLimitLayer;

/// Where a request came from, put in its extensions by [`serve_with`] for the bearer check. The
/// TCP peer (and whether that is this machine), not anything the request says about itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PeerAddr(pub Peer);

pub const MAX_BODY_BYTES: usize = 256 * 1024;
const MAX_CONCURRENT_REQUESTS: usize = 16;
/// Requests in flight before any token is looked at. Bounds what a flood of bad credentials can
/// occupy (hashing, scanning the client list, building responses) on this loopback service.
const MAX_UNAUTHENTICATED_REQUESTS: usize = 64;
pub(crate) const REQUEST_DEADLINE: Duration = Duration::from_secs(30);

async fn deadline(request: Request, next: Next) -> Response {
    match tokio::time::timeout(REQUEST_DEADLINE, next.run(request)).await {
        Ok(response) => response,
        Err(_) => StatusCode::REQUEST_TIMEOUT.into_response(),
    }
}

/// The full application for a server that answers requests whose `Host` is one of
/// `allowed_hosts` (`BindMode::allowed_hosts`) and no other.
pub fn router(
    shared: Arc<McpShared>,
    bridge: BridgeClient,
    allowed_hosts: Vec<String>,
    cancel: CancellationToken,
) -> Router {
    let config = StreamableHttpServerConfig::default()
        .with_allowed_hosts(allowed_hosts)
        .enforce_origin_validation()
        // Every request stands alone: the bearer token authorizes each one, no session to hijack.
        .with_legacy_session_mode(false)
        .with_json_response(true)
        .with_sse_keep_alive(None)
        .with_max_request_body_bytes(MAX_BODY_BYTES)
        .with_cancellation_token(cancel.child_token());
    let tools_shared = shared.clone();
    let service = StreamableHttpService::new(
        move || Ok(DegenViewMcp::new(bridge.clone(), tools_shared.clone())),
        Arc::new(NeverSessionManager::default()),
        config,
    );
    Router::new()
        .route_service("/mcp", service)
        .layer(middleware::from_fn(deadline))
        .layer(ConcurrencyLimitLayer::new(MAX_CONCURRENT_REQUESTS))
        // Outside the cap above, so a cancellation gets through even when the calls it cancels
        // hold every permit. Inside the bearer check, so a client can only cancel its own calls.
        .layer(middleware::from_fn_with_state(
            shared.clone(),
            honour_cancellations,
        ))
        // Nothing, not even a 404, is answered without a valid token.
        .layer(middleware::from_fn_with_state(shared, require_bearer))
        // Outermost: a cap that holds before authentication, so bad tokens cannot pile up.
        .layer(ConcurrencyLimitLayer::new(MAX_UNAUTHENTICATED_REQUESTS))
}

/// Headers a request may carry. Real clients send under a dozen.
pub const MAX_HEADERS: usize = 64;
/// Bytes of request line and headers together; a larger request is answered 431.
pub const MAX_HEADER_BYTES: usize = 16 * 1024;
/// Bytes hyper may buffer while reading a request's head. Above [`MAX_HEADER_BYTES`] so that
/// limit is the one that answers, and the hard bound on a head that never ends.
const MAX_READ_BUFFER: usize = 32 * 1024;
/// How long a client may take to send a request's headers, which also bounds how long an idle
/// keep-alive connection is kept for.
pub const HEADER_READ_TIMEOUT: Duration = Duration::from_secs(10);
/// Connections open at once. More are closed as they arrive. A client needs one or two.
pub const MAX_CONNECTIONS: usize = 128;
/// How long a client may take to finish the TLS handshake. It holds a connection slot meanwhile,
/// so without a bound a client that connects and says nothing would occupy one for good.
pub const TLS_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

/// Limits [`serve`] puts on a connection. A struct so tests can shorten the timeout.
#[derive(Clone, Copy, Debug)]
pub struct ConnectionLimits {
    pub max_headers: usize,
    pub max_header_bytes: usize,
    pub header_read_timeout: Duration,
    pub max_connections: usize,
    pub tls_handshake_timeout: Duration,
}

impl Default for ConnectionLimits {
    fn default() -> Self {
        Self {
            max_headers: MAX_HEADERS,
            max_header_bytes: MAX_HEADER_BYTES,
            header_read_timeout: HEADER_READ_TIMEOUT,
            max_connections: MAX_CONNECTIONS,
            tls_handshake_timeout: TLS_HANDSHAKE_TIMEOUT,
        }
    }
}

/// Serves `app` on `listener` until `cancel` fires, then drops every open connection.
///
/// HTTP/1 only. Each connection is its own task holding one of `max_connections` permits; a
/// connection that arrives with none free is closed at once. Dropping this future (or cancelling)
/// stops accepting and ends every connection task.
#[cfg(test)]
pub async fn serve(
    listener: tokio::net::TcpListener,
    app: Router,
    limits: ConnectionLimits,
    cancel: CancellationToken,
) {
    serve_with(listener, app, limits, None, cancel).await;
}

/// [`serve`], with every connection wrapped in TLS first when `tls` is given. A client that does
/// not finish the handshake within `limits.tls_handshake_timeout` is dropped, and its slot freed.
pub async fn serve_with(
    listener: tokio::net::TcpListener,
    app: Router,
    limits: ConnectionLimits,
    tls: Option<tokio_rustls::TlsAcceptor>,
    cancel: CancellationToken,
) {
    let permits = Arc::new(tokio::sync::Semaphore::new(limits.max_connections));
    let mut connections = tokio::task::JoinSet::new();
    loop {
        let accepted = tokio::select! {
            () = cancel.cancelled() => break,
            accepted = listener.accept() => accepted,
        };
        // Reap finished connections so the set does not grow with every request.
        while connections.try_join_next().is_some() {}
        let stream = match accepted {
            Ok((stream, peer)) => {
                // Local when loopback or the very address the connection arrived on; if that
                // cannot be read, only loopback is.
                let peer = match stream.local_addr() {
                    Ok(own) => Peer::of_connection(peer.ip(), own.ip()),
                    Err(_) => Peer::from_ip(peer.ip()),
                };
                (stream, peer)
            }
            Err(_) => {
                // Out of descriptors or a connection reset before accept: not fatal. Back off
                // briefly so a persistent error does not spin.
                tokio::time::sleep(Duration::from_millis(50)).await;
                continue;
            }
        };
        let (stream, peer) = stream;
        let Ok(permit) = permits.clone().try_acquire_owned() else {
            drop(stream);
            continue;
        };
        let _ = stream.set_nodelay(true);
        let app = app.clone();
        let cancel = cancel.clone();
        let tls = tls.clone();
        connections.spawn(async move {
            let _permit = permit;
            match tls {
                None => serve_connection(stream, peer, app, limits, cancel).await,
                Some(acceptor) => {
                    let handshake =
                        tokio::time::timeout(limits.tls_handshake_timeout, acceptor.accept(stream));
                    let stream = tokio::select! {
                        () = cancel.cancelled() => return,
                        handshake = handshake => match handshake {
                            Ok(Ok(stream)) => stream,
                            // A failed or stalled handshake sends nothing back: not even an
                            // HTTP error, which a plain-text client would otherwise get to read.
                            _ => return,
                        },
                    };
                    serve_connection(stream, peer, app, limits, cancel).await;
                }
            }
        });
    }
    connections.shutdown().await;
}

async fn serve_connection<S>(
    stream: S,
    peer: Peer,
    app: Router,
    limits: ConnectionLimits,
    cancel: CancellationToken,
) where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    use hyper_util::rt::{TokioIo, TokioTimer};
    use tower::Service;

    let service = hyper::service::service_fn(move |request: axum::http::Request<_>| {
        let mut request = request.map(axum::body::Body::new);
        // Set here, over anything a client could have put in the request's extensions (it cannot:
        // they are not part of HTTP), so the bearer check reads the real socket peer.
        request.extensions_mut().insert(PeerAddr(peer));
        app.clone().call(request)
    });
    let mut builder = hyper::server::conn::http1::Builder::new();
    builder
        .timer(TokioTimer::new())
        .header_read_timeout(limits.header_read_timeout)
        .max_headers(limits.max_headers)
        .max_header_size(limits.max_header_bytes)
        .max_buf_size(MAX_READ_BUFFER);
    let connection = builder.serve_connection(TokioIo::new(stream), service);
    tokio::select! {
        _ = connection => {}
        () = cancel.cancelled() => {}
    }
}
