//! Protocol cancellation: honouring `notifications/cancelled` for a call still in flight.
//!
//! The server is stateless: every POST is served on its own, so rmcp cannot match a cancellation
//! notification (a separate POST) to the call it names. This module keeps its own record. Each
//! tool call registers its request id under the calling client for as long as it runs;
//! [`honour_cancellations`] reads a cancellation notification before rmcp does and cancels the
//! matching calls of *that client only*.
//!
//! What cancelling a call means is the same as for a hang-up: the call's future is dropped, so
//! work still queued never runs. Work the app has already started keeps its real outcome.

use crate::lock::lock;
use crate::mcp::state::{ClientIdentity, McpShared};
use crate::mcp::transport::{MAX_BODY_BYTES, REQUEST_DEADLINE};
use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use rmcp::model::NumberOrString;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tokio_util::sync::CancellationToken;

type Key = (String, NumberOrString);

/// The tool calls in flight, by calling client and JSON-RPC request id.
///
/// Stateless mode has no session, so two simultaneous calls from one client may share an id;
/// each is kept, and a cancellation for that id cancels all of them.
#[derive(Default)]
pub struct CallRegistry {
    next_serial: AtomicU64,
    calls: Mutex<HashMap<Key, Vec<(u64, CancellationToken)>>>,
}

impl CallRegistry {
    fn calls(&self) -> std::sync::MutexGuard<'_, HashMap<Key, Vec<(u64, CancellationToken)>>> {
        lock(&self.calls)
    }

    /// Records a call as running. Dropping the guard forgets it.
    pub fn register(self: &Arc<Self>, client_id: &str, id: NumberOrString) -> CallGuard {
        let serial = self.next_serial.fetch_add(1, Ordering::Relaxed);
        let token = CancellationToken::new();
        let key = (client_id.to_owned(), id);
        self.calls()
            .entry(key.clone())
            .or_default()
            .push((serial, token.clone()));
        CallGuard {
            registry: self.clone(),
            key,
            serial,
            token,
        }
    }

    /// Cancels every running call `client_id` made with request id `id`. Returns how many.
    pub fn cancel(&self, client_id: &str, id: &NumberOrString) -> usize {
        let calls = self.calls();
        let Some(running) = calls.get(&(client_id.to_owned(), id.clone())) else {
            return 0;
        };
        for (_, token) in running {
            token.cancel();
        }
        running.len()
    }

    #[cfg(test)]
    pub fn running(&self) -> usize {
        self.calls().values().map(Vec::len).sum()
    }
}

/// A call registered as running; forgets it when dropped, however the call ends.
pub struct CallGuard {
    registry: Arc<CallRegistry>,
    key: Key,
    serial: u64,
    token: CancellationToken,
}

impl CallGuard {
    /// Cancelled when the client sends `notifications/cancelled` for this call.
    pub fn token(&self) -> CancellationToken {
        self.token.clone()
    }
}

impl Drop for CallGuard {
    fn drop(&mut self) {
        let mut calls = self.registry.calls();
        if let Some(running) = calls.get_mut(&self.key) {
            running.retain(|(serial, _)| *serial != self.serial);
            if running.is_empty() {
                calls.remove(&self.key);
            }
        }
    }
}

/// The request id a `notifications/cancelled` message names, or `None` if `body` is anything
/// else (another message, a batch, malformed JSON).
pub fn cancelled_request_id(body: &[u8]) -> Option<NumberOrString> {
    const METHOD: &[u8] = b"notifications/cancelled";
    // Almost every body is an ordinary call: skip the JSON parse unless the method name appears.
    if !body.windows(METHOD.len()).any(|window| window == METHOD) {
        return None;
    }
    let message: serde_json::Value = serde_json::from_slice(body).ok()?;
    if message.get("method")?.as_str()? != "notifications/cancelled" {
        return None;
    }
    match message.get("params")?.get("requestId")? {
        serde_json::Value::Number(number) => Some(NumberOrString::Number(number.as_i64()?)),
        serde_json::Value::String(text) => Some(NumberOrString::String(text.as_str().into())),
        _ => None,
    }
}

/// Answers a `notifications/cancelled` itself and passes everything else on untouched.
///
/// It sits outside the concurrency cap on purpose: a cancellation must get through even when
/// every permit is held by the very calls it is cancelling. It runs after the bearer check, so
/// the caller is known and can only cancel its own calls. The notification is answered 202 (what
/// rmcp would have answered) whether or not it matched anything.
pub async fn honour_cancellations(
    State(shared): State<Arc<McpShared>>,
    request: Request,
    next: Next,
) -> Response {
    if request.method() != Method::POST {
        return next.run(request).await;
    }
    let client = request.extensions().get::<ClientIdentity>().cloned();
    let (parts, body) = request.into_parts();
    // The body is read here, outside the request deadline further in, so it gets its own.
    let bytes =
        match tokio::time::timeout(REQUEST_DEADLINE, axum::body::to_bytes(body, MAX_BODY_BYTES))
            .await
        {
            Ok(Ok(bytes)) => bytes,
            Ok(Err(_)) => return StatusCode::PAYLOAD_TOO_LARGE.into_response(),
            Err(_) => return StatusCode::REQUEST_TIMEOUT.into_response(),
        };
    if let (Some(client), Some(id)) = (client, cancelled_request_id(&bytes)) {
        shared.calls().cancel(&client.id, &id);
        return StatusCode::ACCEPTED.into_response();
    }
    next.run(Request::from_parts(parts, Body::from(bytes)))
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: i64) -> NumberOrString {
        NumberOrString::Number(n)
    }

    #[test]
    fn a_cancel_names_its_request_id_as_a_number_or_a_string() {
        let number =
            br#"{"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":9}}"#;
        assert_eq!(cancelled_request_id(number), Some(id(9)));
        let text = br#"{"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":"abc","reason":"x"}}"#;
        assert_eq!(
            cancelled_request_id(text),
            Some(NumberOrString::String("abc".into()))
        );
        // 9 and "9" are different ids.
        let quoted = br#"{"method":"notifications/cancelled","params":{"requestId":"9"}}"#;
        assert_ne!(cancelled_request_id(quoted), Some(id(9)));
    }

    #[test]
    fn anything_else_is_not_a_cancel() {
        for body in [
            &br#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#[..],
            br#"not json notifications/cancelled"#,
            br#"{"method":"notifications/cancelled","params":{}}"#,
            br#"{"method":"notifications/cancelled","params":{"requestId":1.5}}"#,
            br#"{"method":"notifications/cancelled","params":{"requestId":null}}"#,
            br#"{"method":"tools/call","params":{"name":"notifications/cancelled","requestId":3}}"#,
            // A batch is passed on to rmcp, not read here.
            br#"[{"method":"notifications/cancelled","params":{"requestId":1}}]"#,
            b"",
        ] {
            assert_eq!(cancelled_request_id(body), None, "{body:?}");
        }
    }

    #[test]
    fn a_cancel_reaches_only_its_own_clients_call() {
        let registry = Arc::new(CallRegistry::default());
        let mine = registry.register("a", id(1));
        let theirs = registry.register("b", id(1));
        assert_eq!(registry.cancel("a", &id(1)), 1);
        assert!(mine.token().is_cancelled());
        assert!(!theirs.token().is_cancelled());
        assert_eq!(registry.cancel("a", &id(2)), 0);
        assert_eq!(registry.cancel("c", &id(1)), 0);
    }

    #[test]
    fn calls_sharing_an_id_are_all_cancelled_and_each_forgets_itself() {
        let registry = Arc::new(CallRegistry::default());
        let first = registry.register("a", id(1));
        let second = registry.register("a", id(1));
        assert_eq!(registry.running(), 2);
        assert_eq!(registry.cancel("a", &id(1)), 2);
        assert!(first.token().is_cancelled() && second.token().is_cancelled());
        drop(first);
        assert_eq!(registry.running(), 1);
        drop(second);
        assert_eq!(registry.running(), 0);
        // Nothing left to cancel, and the map did not keep an empty entry.
        assert_eq!(registry.cancel("a", &id(1)), 0);
    }
}
