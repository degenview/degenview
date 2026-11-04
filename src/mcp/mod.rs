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
pub mod indicator_tools;
pub mod indicator_values;
pub mod market_tools;
pub mod markets;
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

/// How long [`McpServer::start`] waits for the server thread to own the listener.
const STARTUP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

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
        // The server thread reports once the runtime owns the listener (or why it never will).
        // Until then the status stays `Starting`: it is never `Running` on the parent's say-so.
        let (ready, started) = std::sync::mpsc::sync_channel::<Result<(), String>>(1);
        let thread = {
            let shared = shared.clone();
            let cancel = cancel.clone();
            std::thread::Builder::new()
                .name("mcp-server".into())
                .spawn(move || {
                    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        run(
                            listener,
                            shared.clone(),
                            bridge,
                            port,
                            cancel.clone(),
                            ready,
                        );
                    }));
                    // A panic is a failure, not a silent end; stopping on purpose is not.
                    if outcome.is_err() && !cancel.is_cancelled() {
                        shared.set_status(McpStatus::Failed(
                            "The MCP server stopped unexpectedly".into(),
                        ));
                    }
                })
                .map_err(|error| fail(format!("Could not start the server thread: {error}")))?
        };
        match started.recv_timeout(STARTUP_TIMEOUT) {
            Ok(Ok(())) => {}
            Ok(Err(message)) => {
                let _ = thread.join();
                return Err(fail(message));
            }
            Err(_) => {
                // Timed out, or the thread ended before it could answer.
                cancel.cancel();
                let _ = thread.join();
                return Err(fail("The MCP server did not start".into()));
            }
        }
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
    ready: std::sync::mpsc::SyncSender<Result<(), String>>,
) {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            let _ = ready.send(Err(format!("Could not start the runtime: {error}")));
            return;
        }
    };
    runtime.block_on(async {
        let listener = match tokio::net::TcpListener::from_std(listener) {
            Ok(listener) => listener,
            Err(error) => {
                let _ = ready.send(Err(format!("Listener error: {error}")));
                return;
            }
        };
        let app = transport::router(shared.clone(), bridge, port, cancel.clone());
        // The runtime owns the listener and the app is built: this is the moment the server is
        // running. Set before answering, so a later failure cannot be overwritten by the caller.
        shared.set_status(McpStatus::Running { port });
        let _ = ready.send(Ok(()));
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
