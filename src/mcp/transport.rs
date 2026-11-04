//! The loopback HTTP surface: one `/mcp` endpoint (MCP Streamable HTTP, served statelessly by
//! `rmcp`) behind a bearer-token guard, a request timeout and a concurrency cap.
//!
//! `rmcp` itself validates the `Host` header against an explicit loopback list, rejects every
//! request carrying an `Origin` (a browser page has no business here), and caps the body size.

use crate::mcp::auth::require_bearer;
use crate::mcp::bridge::BridgeClient;
use crate::mcp::state::McpShared;
use crate::mcp::tools::DegenViewMcp;
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

pub const MAX_BODY_BYTES: usize = 256 * 1024;
const MAX_CONCURRENT_REQUESTS: usize = 16;
/// Requests in flight before any token is looked at. Bounds what a flood of bad credentials can
/// occupy (hashing, scanning the client list, building responses) on this loopback service.
const MAX_UNAUTHENTICATED_REQUESTS: usize = 64;
const REQUEST_DEADLINE: Duration = Duration::from_secs(30);

async fn deadline(request: Request, next: Next) -> Response {
    match tokio::time::timeout(REQUEST_DEADLINE, next.run(request)).await {
        Ok(response) => response,
        Err(_) => StatusCode::REQUEST_TIMEOUT.into_response(),
    }
}

/// The full application for a server listening on 127.0.0.1:`port`.
pub fn router(
    shared: Arc<McpShared>,
    bridge: BridgeClient,
    port: u16,
    cancel: CancellationToken,
) -> Router {
    let config = StreamableHttpServerConfig::default()
        .with_allowed_hosts([format!("127.0.0.1:{port}"), format!("localhost:{port}")])
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
        // Nothing, not even a 404, is answered without a valid token.
        .layer(middleware::from_fn_with_state(shared, require_bearer))
        // Outermost: a cap that holds before authentication, so bad tokens cannot pile up.
        .layer(ConcurrencyLimitLayer::new(MAX_UNAUTHENTICATED_REQUESTS))
}
