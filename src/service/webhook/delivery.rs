//! Sends rendered alert messages to webhook endpoints. The only code that performs
//! webhook HTTP: price alerts, Pine alerts and the Settings test button all come through
//! `deliver`, so validation, Content-Type, timeout and result classification are one path.
//!
//! One call is one attempt. There is deliberately no retry: an endpoint is often a
//! trading bot, and a retried "BUY" can become a second order. Redirects are never
//! followed (a 3xx is a failed delivery), so a validated URL cannot be redirected
//! somewhere the policy never saw.

use crate::model::watchlist::now_ms;
use crate::model::webhook;
use crate::model::webhook::policy::WebhookPayload;
use crate::model::webhook::{
    EndpointId, WebhookDeliveryError, WebhookDeliveryResult, WebhookEndpoint, WebhookHeader,
    WebhookHttpMethod,
};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// TradingView cancels a request that takes longer than three seconds.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Mode {
    /// Alert delivery: a disabled endpoint is skipped.
    Production,
    /// The Settings Test button: the user may be testing before enabling, so the enabled
    /// flag is bypassed. The URL is still validated like any other.
    Test,
}

/// A request ready to go on the wire. Holds the secret, so it is never logged or stored.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TransportRequest {
    pub method: WebhookHttpMethod,
    pub url: String,
    /// Custom headers, then `Content-Type` last: the body's type is DegenView's to decide.
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    pub timeout: Duration,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransportError {
    Timeout,
    Network,
}

/// The network seam of webhook delivery: sends one request and reports the HTTP status.
/// Tests substitute a recording fake, so nothing in the suite touches the Internet.
pub trait WebhookTransport: Send + Sync {
    fn send(&self, request: &TransportRequest) -> Result<u16, TransportError>;
}

/// Why a secret could not be saved. Carries the platform's code, never the secret.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
#[error("Couldn't save the secret: {0}")]
pub struct SecretStoreError(pub String);

/// Where an endpoint's secret lives (the Keychain in the app). The value never goes in
/// SQLite; the endpoint row only holds templates that say where it is used.
pub trait WebhookSecretStore: Send + Sync {
    /// `None` when nothing is stored or it cannot be read; delivery then fails with
    /// `SecretUnavailable`. Never prompts.
    fn secret(&self, endpoint: EndpointId) -> Option<String>;

    /// Whether a secret is stored, without reading its value.
    fn has_secret(&self, endpoint: EndpointId) -> bool {
        self.secret(endpoint).is_some()
    }

    fn set_secret(&self, endpoint: EndpointId, secret: &str) -> Result<(), SecretStoreError>;

    fn remove(&self, endpoint: EndpointId);
}

/// A store for tests and for builds with no Keychain.
#[derive(Default)]
pub struct InMemoryWebhookSecretStore {
    secrets: Mutex<HashMap<EndpointId, String>>,
    /// Set to make `set_secret` fail, for testing rollback.
    pub fails_to_write: std::sync::atomic::AtomicBool,
}

impl InMemoryWebhookSecretStore {
    pub fn set(&self, endpoint: EndpointId, secret: &str) {
        if let Ok(mut secrets) = self.secrets.lock() {
            secrets.insert(endpoint, secret.to_owned());
        }
    }
}

impl WebhookSecretStore for InMemoryWebhookSecretStore {
    fn secret(&self, endpoint: EndpointId) -> Option<String> {
        self.secrets.lock().ok()?.get(&endpoint).cloned()
    }

    fn set_secret(&self, endpoint: EndpointId, secret: &str) -> Result<(), SecretStoreError> {
        if self
            .fails_to_write
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            return Err(SecretStoreError("write refused".to_owned()));
        }
        self.set(endpoint, secret);
        Ok(())
    }

    fn remove(&self, endpoint: EndpointId) {
        if let Ok(mut secrets) = self.secrets.lock() {
            secrets.remove(&endpoint);
        }
    }
}

/// `ureq` transport with no cookies, no cache and no redirects: a 3xx therefore surfaces
/// as a failed delivery.
pub struct UreqTransport {
    agent: ureq::Agent,
}

impl Default for UreqTransport {
    fn default() -> Self {
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(REQUEST_TIMEOUT))
            .max_redirects(0)
            .max_redirects_will_error(false)
            .http_status_as_error(false)
            .build();
        Self {
            agent: ureq::Agent::new_with_config(config),
        }
    }
}

impl WebhookTransport for UreqTransport {
    fn send(&self, request: &TransportRequest) -> Result<u16, TransportError> {
        let mut builder = match request.method {
            WebhookHttpMethod::Post => self.agent.post(&request.url),
            WebhookHttpMethod::Put => self.agent.put(&request.url),
        };
        for (name, value) in &request.headers {
            builder = builder.header(name, value);
        }
        match builder.send(&request.body[..]) {
            Ok(response) => Ok(response.status().as_u16()),
            Err(ureq::Error::Timeout(_)) => Err(TransportError::Timeout),
            Err(_) => Err(TransportError::Network),
        }
    }
}

/// One attempt at one endpoint. Never fails: every outcome is in the result.
pub fn deliver(
    transport: &dyn WebhookTransport,
    secrets: &dyn WebhookSecretStore,
    message: &str,
    endpoint: &WebhookEndpoint,
    mode: Mode,
) -> WebhookDeliveryResult {
    let started = Instant::now();
    let timestamp_ms = now_ms();
    let result = |error: Option<WebhookDeliveryError>, status: Option<u16>| WebhookDeliveryResult {
        endpoint_id: endpoint.id,
        timestamp_ms,
        duration_secs: started.elapsed().as_secs_f64(),
        status_code: status,
        error,
    };

    if mode == Mode::Production && !endpoint.is_enabled {
        return result(Some(WebhookDeliveryError::DisabledEndpoint), None);
    }
    // A row the user added but never filled in is ignored, not an error.
    let custom: Vec<WebhookHeader> = endpoint
        .headers
        .iter()
        .filter(|header| !header.is_blank())
        .cloned()
        .collect();
    // The Keychain is read per attempt, and only when something uses the secret, so an
    // edit applies at once and an endpoint without one never touches it.
    let secret = webhook::policy::references_secret(&endpoint.url, &custom)
        .then(|| secrets.secret(endpoint.id))
        .flatten();
    let resolved = match webhook::policy::resolve(&endpoint.url, &custom, secret.as_deref()) {
        Ok(resolved) => resolved,
        Err(error) => return result(Some(error), None),
    };

    let payload = WebhookPayload::new(message);
    let mut headers: Vec<(String, String)> = resolved
        .headers
        .into_iter()
        .map(|header| (header.name, header.value))
        .collect();
    headers.push(("Content-Type".to_owned(), payload.content_type.to_owned()));
    let request = TransportRequest {
        method: endpoint.method,
        url: resolved.url,
        headers,
        body: payload.body,
        timeout: REQUEST_TIMEOUT,
    };

    let outcome = match transport.send(&request) {
        Ok(status) if (200..300).contains(&status) => result(None, Some(status)),
        Ok(status) => result(Some(WebhookDeliveryError::HttpFailure), Some(status)),
        Err(TransportError::Timeout) => result(Some(WebhookDeliveryError::Timeout), None),
        Err(TransportError::Network) => result(Some(WebhookDeliveryError::NetworkFailure), None),
    };
    #[cfg(debug_assertions)]
    if !outcome.succeeded() {
        eprintln!("[Webhook] delivery failed {}", outcome.log_line());
    }
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::Arc;

    /// Records every request and answers from a script.
    #[derive(Default)]
    struct RecordingTransport {
        requests: Mutex<Vec<TransportRequest>>,
        answers: Mutex<Vec<Result<u16, TransportError>>>,
    }

    impl RecordingTransport {
        fn answering(answers: Vec<Result<u16, TransportError>>) -> Self {
            Self {
                requests: Mutex::default(),
                answers: Mutex::new(answers),
            }
        }
        fn requests(&self) -> Vec<TransportRequest> {
            self.requests.lock().unwrap().clone()
        }
    }

    impl WebhookTransport for RecordingTransport {
        fn send(&self, request: &TransportRequest) -> Result<u16, TransportError> {
            self.requests.lock().unwrap().push(request.clone());
            let mut answers = self.answers.lock().unwrap();
            if answers.is_empty() {
                Ok(200)
            } else {
                answers.remove(0)
            }
        }
    }

    fn endpoint(url: &str) -> WebhookEndpoint {
        WebhookEndpoint::new("Bot", url, 0)
    }

    #[test]
    fn a_message_is_posted_as_is_with_the_content_type_chosen_last() {
        let transport = RecordingTransport::default();
        let secrets = InMemoryWebhookSecretStore::default();
        let mut hook = endpoint("https://example.com/hook");
        hook.headers.push(WebhookHeader::new("X-Trace", "abc"));
        hook.headers.push(WebhookHeader::new("", "")); // a blank row is ignored
        let result = deliver(
            &transport,
            &secrets,
            r#"{"event":"buy"}"#,
            &hook,
            Mode::Production,
        );
        assert!(result.succeeded());
        assert_eq!(result.status_code, Some(200));

        let sent = transport.requests();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].method, WebhookHttpMethod::Post);
        assert_eq!(sent[0].body, br#"{"event":"buy"}"#);
        assert_eq!(
            sent[0].headers,
            vec![
                ("X-Trace".to_owned(), "abc".to_owned()),
                (
                    "Content-Type".to_owned(),
                    "application/json; charset=utf-8".to_owned()
                )
            ]
        );
        assert_eq!(sent[0].timeout, Duration::from_secs(3));

        let text = deliver(
            &transport,
            &secrets,
            "BTC alert: 1",
            &hook,
            Mode::Production,
        );
        assert!(text.succeeded());
        assert_eq!(
            transport.requests()[1].headers.last().unwrap().1,
            "text/plain; charset=utf-8"
        );
    }

    #[test]
    fn put_endpoints_send_put() {
        let transport = RecordingTransport::default();
        let mut hook = endpoint("https://example.com/hook");
        hook.method = WebhookHttpMethod::Put;
        deliver(
            &transport,
            &InMemoryWebhookSecretStore::default(),
            "x",
            &hook,
            Mode::Production,
        );
        assert_eq!(transport.requests()[0].method, WebhookHttpMethod::Put);
    }

    #[test]
    fn the_secret_is_read_per_attempt_and_only_when_used() {
        let transport = RecordingTransport::default();
        let secrets = InMemoryWebhookSecretStore::default();
        let mut hook = endpoint("https://example.com/hook?token={{secret}}");
        hook.headers
            .push(WebhookHeader::new("Authorization", "Bearer {{secret}}"));

        // No secret stored: a failed delivery, and nothing is sent.
        let missing = deliver(&transport, &secrets, "m", &hook, Mode::Production);
        assert_eq!(missing.error, Some(WebhookDeliveryError::SecretUnavailable));
        assert!(transport.requests().is_empty());

        secrets.set(hook.id, "a b&c");
        let sent = deliver(&transport, &secrets, "m", &hook, Mode::Production);
        assert!(sent.succeeded());
        let request = &transport.requests()[0];
        assert_eq!(request.url, "https://example.com/hook?token=a%20b%26c");
        assert_eq!(
            request.headers[0],
            ("Authorization".to_owned(), "Bearer a b&c".to_owned())
        );
        // The secret never reaches the body or the log line.
        assert!(!String::from_utf8_lossy(&request.body).contains("a b"));
        assert!(!sent.log_line().contains("a b"));

        // An endpoint that does not use the secret never asks the store.
        struct Forbidden;
        impl WebhookSecretStore for Forbidden {
            fn secret(&self, _: EndpointId) -> Option<String> {
                panic!("the store must not be read");
            }
            fn set_secret(&self, _: EndpointId, _: &str) -> Result<(), SecretStoreError> {
                Ok(())
            }
            fn remove(&self, _: EndpointId) {}
        }
        let plain = endpoint("https://example.com/plain");
        assert!(deliver(&transport, &Forbidden, "m", &plain, Mode::Production).succeeded());
    }

    #[test]
    fn statuses_and_transport_errors_are_classified() {
        let secrets = InMemoryWebhookSecretStore::default();
        let hook = endpoint("https://example.com/hook");
        let cases = [
            (Ok(200), None),
            (Ok(204), None),
            (Ok(299), None),
            (Ok(301), Some(WebhookDeliveryError::HttpFailure)),
            (Ok(302), Some(WebhookDeliveryError::HttpFailure)),
            (Ok(404), Some(WebhookDeliveryError::HttpFailure)),
            (Ok(500), Some(WebhookDeliveryError::HttpFailure)),
            (
                Err(TransportError::Timeout),
                Some(WebhookDeliveryError::Timeout),
            ),
            (
                Err(TransportError::Network),
                Some(WebhookDeliveryError::NetworkFailure),
            ),
        ];
        for (answer, expected) in cases {
            let transport = RecordingTransport::answering(vec![answer]);
            let result = deliver(&transport, &secrets, "m", &hook, Mode::Production);
            assert_eq!(result.error, expected, "{answer:?}");
            assert_eq!(result.status_code, answer.ok(), "{answer:?}");
        }
    }

    #[test]
    fn there_is_exactly_one_attempt_even_when_it_fails() {
        let transport = RecordingTransport::answering(vec![Ok(500), Ok(200)]);
        let hook = endpoint("https://example.com/hook");
        let result = deliver(
            &transport,
            &InMemoryWebhookSecretStore::default(),
            "BUY",
            &hook,
            Mode::Production,
        );
        assert!(!result.succeeded());
        assert_eq!(
            transport.requests().len(),
            1,
            "a failed order webhook is never retried"
        );
    }

    #[test]
    fn disabled_endpoints_are_skipped_in_production_but_testable() {
        let transport = RecordingTransport::default();
        let secrets = InMemoryWebhookSecretStore::default();
        let mut hook = endpoint("https://example.com/hook");
        hook.is_enabled = false;
        let skipped = deliver(&transport, &secrets, "m", &hook, Mode::Production);
        assert_eq!(skipped.error, Some(WebhookDeliveryError::DisabledEndpoint));
        assert!(transport.requests().is_empty());
        assert!(deliver(&transport, &secrets, "m", &hook, Mode::Test).succeeded());
        // Test mode still validates the URL.
        hook.url = "ftp://example.com".into();
        assert_eq!(
            deliver(&transport, &secrets, "m", &hook, Mode::Test).error,
            Some(WebhookDeliveryError::UnsupportedScheme)
        );
    }

    #[test]
    fn invalid_urls_and_headers_never_reach_the_transport() {
        let transport = RecordingTransport::default();
        let secrets = InMemoryWebhookSecretStore::default();
        for (url, expected) in [
            ("", WebhookDeliveryError::InvalidUrl),
            ("not a url", WebhookDeliveryError::InvalidUrl),
            ("file:///etc/hosts", WebhookDeliveryError::UnsupportedScheme),
            ("http://host:0/x", WebhookDeliveryError::UnsupportedPort),
            (
                "https://{{secret}}.example.com",
                WebhookDeliveryError::SecretInHost,
            ),
        ] {
            assert_eq!(
                deliver(&transport, &secrets, "m", &endpoint(url), Mode::Production).error,
                Some(expected),
                "{url}"
            );
        }
        let mut bad_header = endpoint("https://example.com");
        bad_header
            .headers
            .push(WebhookHeader::new("Content-Type", "text/evil"));
        assert_eq!(
            deliver(&transport, &secrets, "m", &bad_header, Mode::Production).error,
            Some(WebhookDeliveryError::InvalidHeader)
        );
        assert!(transport.requests().is_empty());
    }

    /// A one-shot local HTTP server: records the raw request, answers with `response`.
    fn serve_once(response: &'static str) -> (String, std::thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        let url = format!("http://{}/hook", listener.local_addr().unwrap());
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream.set_read_timeout(Some(Duration::from_secs(2))).ok();
            let mut raw = Vec::new();
            let mut buffer = [0u8; 4096];
            // Read the head, then as many body bytes as Content-Length says.
            loop {
                let read = stream.read(&mut buffer).unwrap_or(0);
                if read == 0 {
                    break;
                }
                raw.extend_from_slice(&buffer[..read]);
                let text = String::from_utf8_lossy(&raw).to_string();
                if let Some(head_end) = text.find("\r\n\r\n") {
                    let length = text
                        .lines()
                        .find_map(|line| {
                            line.to_lowercase()
                                .strip_prefix("content-length:")
                                .map(|v| v.trim().parse::<usize>().unwrap_or(0))
                        })
                        .unwrap_or(0);
                    if raw.len() >= head_end + 4 + length {
                        break;
                    }
                }
            }
            stream.write_all(response.as_bytes()).unwrap();
            String::from_utf8_lossy(&raw).to_string()
        });
        (url, handle)
    }

    #[test]
    fn the_real_transport_posts_the_exact_bytes_and_headers_to_a_local_server() {
        let (url, server) =
            serve_once("HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        let mut hook = endpoint(&url);
        hook.headers.push(WebhookHeader::new("X-Key", "abc"));
        let result = deliver(
            &UreqTransport::default(),
            &InMemoryWebhookSecretStore::default(),
            r#"{"event":"DegenView webhook test"}"#,
            &hook,
            Mode::Test,
        );
        assert!(result.succeeded(), "{}", result.log_line());
        assert_eq!(result.status_code, Some(204));
        let raw = server.join().unwrap();
        assert!(raw.starts_with("POST /hook HTTP/1.1\r\n"), "{raw}");
        let lower = raw.to_lowercase();
        assert!(lower.contains("content-type: application/json; charset=utf-8"));
        assert!(lower.contains("x-key: abc"));
        assert!(
            raw.ends_with(r#"{"event":"DegenView webhook test"}"#),
            "no newline added: {raw:?}"
        );
    }

    #[test]
    fn the_real_transport_does_not_follow_redirects() {
        let (url, server) = serve_once(
            "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:9/elsewhere\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        );
        let result = deliver(
            &UreqTransport::default(),
            &InMemoryWebhookSecretStore::default(),
            "m",
            &endpoint(&url),
            Mode::Production,
        );
        server.join().unwrap();
        assert_eq!(result.status_code, Some(302));
        assert_eq!(result.error, Some(WebhookDeliveryError::HttpFailure));
    }

    #[test]
    fn the_real_transport_reports_a_dead_port_as_a_network_failure() {
        // Bind then drop to find a port nothing listens on.
        let port = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let result = deliver(
            &UreqTransport::default(),
            &InMemoryWebhookSecretStore::default(),
            "m",
            &endpoint(&format!("http://127.0.0.1:{port}/")),
            Mode::Production,
        );
        assert_eq!(result.error, Some(WebhookDeliveryError::NetworkFailure));
    }

    #[test]
    fn the_real_transport_times_out_a_silent_server() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/", listener.local_addr().unwrap());
        let holder = std::thread::spawn(move || {
            let (_stream, _) = listener.accept().unwrap();
            std::thread::sleep(Duration::from_secs(4));
        });
        let started = Instant::now();
        let result = deliver(
            &UreqTransport::default(),
            &InMemoryWebhookSecretStore::default(),
            "m",
            &endpoint(&url),
            Mode::Production,
        );
        assert_eq!(result.error, Some(WebhookDeliveryError::Timeout));
        assert!(
            started.elapsed() < Duration::from_millis(3_800),
            "gave up at ~3 s"
        );
        drop(holder);
    }

    #[test]
    fn the_in_memory_store_sets_checks_and_removes() {
        let store = InMemoryWebhookSecretStore::default();
        assert!(!store.has_secret(1));
        store.set_secret(1, "token").unwrap();
        assert!(store.has_secret(1));
        assert_eq!(store.secret(1).as_deref(), Some("token"));
        store.set_secret(1, "newer").unwrap();
        assert_eq!(store.secret(1).as_deref(), Some("newer"));
        store.remove(1);
        assert!(!store.has_secret(1));
        store
            .fails_to_write
            .store(true, std::sync::atomic::Ordering::Relaxed);
        let error = store.set_secret(2, "x").unwrap_err();
        assert!(!error.to_string().contains('x') || error.to_string().contains("write refused"));
        assert!(!store.has_secret(2), "a failed write stores nothing");
    }

    #[test]
    fn stores_share_across_threads() {
        let store = Arc::new(InMemoryWebhookSecretStore::default());
        store.set(7, "s");
        let reader = Arc::clone(&store);
        assert_eq!(
            std::thread::spawn(move || reader.secret(7))
                .join()
                .unwrap()
                .as_deref(),
            Some("s")
        );
        assert_eq!(store.secret(8), None);
    }
}
