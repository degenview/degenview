//! A local Model Context Protocol server.
//!
//! Disabled by default; when enabled it listens on `127.0.0.1` only and every request must carry
//! a bearer token issued in Settings. Tools never touch app state directly: they queue commands
//! through [`bridge`] and the app thread executes them.
//!
//! This is the one place with an async runtime. The server owns a single thread running a
//! current-thread tokio runtime; the rest of the app stays on threads and channels.

pub mod auth;
pub mod bridge;
pub mod chart_data;
pub mod drawings;
pub mod pine;
pub mod state;
pub mod tools;
pub mod transport;
pub mod types;

#[cfg(test)]
mod tests;

use crate::mcp::bridge::BridgeClient;
use crate::mcp::state::{McpShared, McpStatus};
use std::net::{Ipv4Addr, TcpListener};
use std::sync::Arc;
use std::thread::JoinHandle;
use tokio_util::sync::CancellationToken;

/// A running listener. Dropping it (or [`McpServer::stop`]) closes the port immediately.
pub struct McpServer {
    shared: Arc<McpShared>,
    cancel: CancellationToken,
    thread: Option<JoinHandle<()>>,
}

impl McpServer {
    /// Binds `127.0.0.1:port` (0 picks a free port) and serves until stopped. Binding happens
    /// here so a port conflict is reported to the caller instead of lost on the thread.
    pub fn start(shared: Arc<McpShared>, bridge: BridgeClient, port: u16) -> Result<Self, String> {
        shared.set_status(McpStatus::Starting);
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, port)).map_err(|error| {
            let message = if error.kind() == std::io::ErrorKind::AddrInUse {
                format!("Port {port} is already in use")
            } else {
                format!("Could not listen on port {port}: {error}")
            };
            shared.set_status(McpStatus::Failed(message.clone()));
            message
        })?;
        let fail = |message: String| {
            shared.set_status(McpStatus::Failed(message.clone()));
            message
        };
        let port = listener
            .local_addr()
            .map_err(|error| fail(format!("Could not read the listening address: {error}")))?
            .port();
        listener
            .set_nonblocking(true)
            .map_err(|error| fail(format!("Could not configure the listener: {error}")))?;

        let cancel = CancellationToken::new();
        let thread = {
            let shared = shared.clone();
            let cancel = cancel.clone();
            std::thread::Builder::new()
                .name("mcp-server".into())
                .spawn(move || run(listener, shared, bridge, port, cancel))
                .map_err(|error| fail(format!("Could not start the server thread: {error}")))?
        };
        shared.set_status(McpStatus::Running { port });
        Ok(Self {
            shared,
            cancel,
            thread: Some(thread),
        })
    }

    /// Stops accepting and drops open connections at once, and invalidates every queued request
    /// so nothing still waiting can execute afterwards. A request the app already started has
    /// committed and keeps its real outcome.
    pub fn stop(&mut self) {
        let Some(thread) = self.thread.take() else {
            return;
        };
        self.shared.set_status(McpStatus::Stopping);
        self.shared.bump_generation();
        self.cancel.cancel();
        let _ = thread.join();
        self.shared.set_status(McpStatus::Disabled);
    }
}

impl Drop for McpServer {
    fn drop(&mut self) {
        self.stop();
    }
}

fn run(
    listener: TcpListener,
    shared: Arc<McpShared>,
    bridge: BridgeClient,
    port: u16,
    cancel: CancellationToken,
) {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            shared.set_status(McpStatus::Failed(format!(
                "Could not start the runtime: {error}"
            )));
            return;
        }
    };
    runtime.block_on(async {
        let listener = match tokio::net::TcpListener::from_std(listener) {
            Ok(listener) => listener,
            Err(error) => {
                shared.set_status(McpStatus::Failed(format!("Listener error: {error}")));
                return;
            }
        };
        let app = transport::router(shared.clone(), bridge, port, cancel.clone());
        tokio::select! {
            result = axum::serve(listener, app) => {
                if let Err(error) = result && !cancel.is_cancelled() {
                    shared.set_status(McpStatus::Failed(format!("Server stopped: {error}")));
                }
            }
            () = cancel.cancelled() => {}
        }
    });
    // Do not wait on anything still parked in a connection: the port is already closed.
    runtime.shutdown_background();
}
