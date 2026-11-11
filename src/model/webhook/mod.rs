//! Webhook domain: the destinations alerts can POST to and the outcome of each attempt.
//! Pure data. URL and header values are *templates* stored as plain text; a secret goes
//! in as `{{secret}}` and its value lives only in the Keychain. Anything written to a log
//! goes through `webhook::policy::redact` first.

// Transient: nothing calls this yet; the alert runtime and Settings wire it next.
#![allow(dead_code)]

pub mod policy;

use crate::model::watchlist::new_id;
use serde::{Deserialize, Serialize};

pub type EndpointId = u64;

/// The HTTP method a webhook is called with. Both carry the alert message as the body;
/// POST is TradingView's and the default.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub enum WebhookHttpMethod {
    #[default]
    #[serde(rename = "POST")]
    Post,
    #[serde(rename = "PUT")]
    Put,
}

impl WebhookHttpMethod {
    pub const ALL: [Self; 2] = [Self::Post, Self::Put];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Post => "POST",
            Self::Put => "PUT",
        }
    }
}

/// One custom HTTP header. The value is a template: `{{secret}}` is replaced with the
/// endpoint's Keychain secret when the request is built.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct WebhookHeader {
    pub id: u64,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub value: String,
}

impl WebhookHeader {
    pub fn new(name: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            id: new_id(),
            name: name.into(),
            value: value.into(),
        }
    }

    /// A row the user added but never filled in. It is ignored, not an error.
    pub fn is_blank(&self) -> bool {
        self.name.trim().is_empty() && self.value.trim().is_empty()
    }
}

/// A reusable webhook destination, configured once in Settings and referenced by id from
/// price alerts and Pine alert subscriptions. Renaming keeps the id, so everything that
/// points at it keeps working.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct WebhookEndpoint {
    pub id: EndpointId,
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// A row without a URL (older data) still loads, empty, and fails validation when used
    /// instead of making the whole endpoint list unreadable.
    #[serde(default)]
    pub url: String,
    #[serde(default, deserialize_with = "method_or_post")]
    pub method: WebhookHttpMethod,
    #[serde(default)]
    pub headers: Vec<WebhookHeader>,
    #[serde(default = "enabled")]
    pub is_enabled: bool,
    #[serde(default)]
    pub created_at_ms: i64,
    #[serde(default)]
    pub updated_at_ms: i64,
}

fn enabled() -> bool {
    true
}

/// A missing or unrecognised method falls back to POST rather than dropping the endpoint.
fn method_or_post<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<WebhookHttpMethod, D::Error> {
    Ok(Option::<serde_json::Value>::deserialize(deserializer)?
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_default())
}

impl WebhookEndpoint {
    pub fn new(name: impl Into<String>, url: impl Into<String>, now_ms: i64) -> Self {
        Self {
            id: new_id(),
            name: name.into(),
            description: String::new(),
            url: url.into(),
            method: WebhookHttpMethod::Post,
            headers: Vec::new(),
            is_enabled: true,
            created_at_ms: now_ms,
            updated_at_ms: now_ms,
        }
    }
}

/// Why a webhook delivery did not succeed. Domain state only: UI wording is derived elsewhere.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum WebhookDeliveryError {
    InvalidUrl,
    UnsupportedScheme,
    UnsupportedPort,
    DisabledEndpoint,
    MissingEndpoint,
    /// The URL or a header uses `{{secret}}` but the Keychain has no readable secret for it.
    SecretUnavailable,
    /// A header name or value is not allowed, including after the secret is substituted in.
    InvalidHeader,
    /// `{{secret}}` was placed in the host part of the URL.
    SecretInHost,
    Timeout,
    NetworkFailure,
    /// The server answered with a status outside 200…299. Redirects land here too: they
    /// are never followed.
    HttpFailure,
}

impl WebhookDeliveryError {
    /// Stable name for logs and the delivery table (no URL, no payload).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::InvalidUrl => "invalidURL",
            Self::UnsupportedScheme => "unsupportedScheme",
            Self::UnsupportedPort => "unsupportedPort",
            Self::DisabledEndpoint => "disabledEndpoint",
            Self::MissingEndpoint => "missingEndpoint",
            Self::SecretUnavailable => "secretUnavailable",
            Self::InvalidHeader => "invalidHeader",
            Self::SecretInHost => "secretInHost",
            Self::Timeout => "timeout",
            Self::NetworkFailure => "networkFailure",
            Self::HttpFailure => "httpFailure",
        }
    }

    pub fn from_str(value: &str) -> Option<Self> {
        match value {
            "invalidURL" => Some(Self::InvalidUrl),
            "unsupportedScheme" => Some(Self::UnsupportedScheme),
            "unsupportedPort" => Some(Self::UnsupportedPort),
            "disabledEndpoint" => Some(Self::DisabledEndpoint),
            "missingEndpoint" => Some(Self::MissingEndpoint),
            "secretUnavailable" => Some(Self::SecretUnavailable),
            "invalidHeader" => Some(Self::InvalidHeader),
            "secretInHost" => Some(Self::SecretInHost),
            "timeout" => Some(Self::Timeout),
            "networkFailure" => Some(Self::NetworkFailure),
            "httpFailure" => Some(Self::HttpFailure),
            _ => None,
        }
    }
}

/// What triggered a delivery. Part of the history key, so one trigger reaches an endpoint once.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum WebhookDeliverySource {
    Price,
    Pine,
    Test,
}

impl WebhookDeliverySource {
    /// Stable name for the delivery table's dedupe key (matches the Swift rawValue).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Price => "price",
            Self::Pine => "pine",
            Self::Test => "test",
        }
    }

    pub fn from_str(value: &str) -> Option<Self> {
        match value {
            "price" => Some(Self::Price),
            "pine" => Some(Self::Pine),
            "test" => Some(Self::Test),
            _ => None,
        }
    }
}

/// The outcome of one attempt at one endpoint.
#[derive(Clone, Debug, PartialEq)]
pub struct WebhookDeliveryResult {
    pub endpoint_id: EndpointId,
    pub timestamp_ms: i64,
    pub duration_secs: f64,
    pub status_code: Option<u16>,
    pub error: Option<WebhookDeliveryError>,
}

impl WebhookDeliveryResult {
    pub fn succeeded(&self) -> bool {
        self.error.is_none()
    }

    /// The log line for a finished attempt: ids, status and timing only. The resolved URL
    /// and header values contain the secret and never appear.
    pub fn log_line(&self) -> String {
        let mut parts = vec![format!("endpointID={}", self.endpoint_id)];
        if let Some(status) = self.status_code {
            parts.push(format!("status={status}"));
        }
        if let Some(error) = self.error {
            parts.push(format!("error={}", error.as_str()));
        }
        parts.push(format!("duration={:.2}", self.duration_secs));
        parts.join(" ")
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum WebhookDeliveryState {
    /// Claimed but not finished. A row that stays here was interrupted (the process quit
    /// mid-request); it is never retried, because the request may have reached the server.
    Pending,
    Delivered,
    Failed,
}

impl WebhookDeliveryState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Delivered => "delivered",
            Self::Failed => "failed",
        }
    }

    pub fn from_str(value: &str) -> Option<Self> {
        match value {
            "pending" => Some(Self::Pending),
            "delivered" => Some(Self::Delivered),
            "failed" => Some(Self::Failed),
            _ => None,
        }
    }
}

/// A persisted attempt. Holds no URL, payload or response body.
#[derive(Clone, Debug, PartialEq)]
pub struct WebhookDeliveryRecord {
    pub id: u64,
    pub endpoint_id: EndpointId,
    pub source: WebhookDeliverySource,
    /// The alert trigger it belongs to: an `AlertTriggerEvent` or Pine notification id.
    pub event_id: Option<u64>,
    pub timestamp_ms: i64,
    pub state: WebhookDeliveryState,
    pub status_code: Option<u16>,
    pub duration_secs: Option<f64>,
    pub error: Option<WebhookDeliveryError>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_endpoint_rows_load_with_safe_defaults() {
        let minimal: WebhookEndpoint = serde_json::from_str(r#"{"id":1,"name":"Bridge"}"#).unwrap();
        assert_eq!(minimal.url, "");
        assert_eq!(minimal.method, WebhookHttpMethod::Post);
        assert!(minimal.headers.is_empty());
        assert!(minimal.is_enabled);
    }

    #[test]
    fn an_unknown_method_falls_back_to_post_instead_of_dropping_the_endpoint() {
        for method in [r#""PATCH""#, "null", "7"] {
            let json = format!(r#"{{"id":1,"name":"x","method":{method}}}"#);
            let endpoint: WebhookEndpoint = serde_json::from_str(&json).unwrap();
            assert_eq!(endpoint.method, WebhookHttpMethod::Post, "{method}");
        }
        let put: WebhookEndpoint =
            serde_json::from_str(r#"{"id":1,"name":"x","method":"PUT"}"#).unwrap();
        assert_eq!(put.method, WebhookHttpMethod::Put);
        assert_eq!(WebhookHttpMethod::Put.as_str(), "PUT");
    }

    #[test]
    fn endpoints_round_trip() {
        let mut endpoint =
            WebhookEndpoint::new("Bot", "https://example.com/hook?token={{secret}}", 5);
        endpoint
            .headers
            .push(WebhookHeader::new("X-Key", "{{secret}}"));
        endpoint.method = WebhookHttpMethod::Put;
        let json = serde_json::to_string(&endpoint).unwrap();
        assert!(json.contains(r#""method":"PUT""#));
        assert_eq!(
            serde_json::from_str::<WebhookEndpoint>(&json).unwrap(),
            endpoint
        );
    }

    #[test]
    fn blank_headers_are_recognised() {
        assert!(WebhookHeader::new("  ", " ").is_blank());
        assert!(!WebhookHeader::new("X", "").is_blank());
        assert!(!WebhookHeader::new("", "v").is_blank());
    }

    #[test]
    fn log_lines_carry_only_ids_status_error_and_timing() {
        let result = WebhookDeliveryResult {
            endpoint_id: 42,
            timestamp_ms: 0,
            duration_secs: 0.5,
            status_code: Some(500),
            error: Some(WebhookDeliveryError::HttpFailure),
        };
        assert_eq!(
            result.log_line(),
            "endpointID=42 status=500 error=httpFailure duration=0.50"
        );
        assert!(!result.succeeded());
        let ok = WebhookDeliveryResult {
            endpoint_id: 1,
            timestamp_ms: 0,
            duration_secs: 1.234,
            status_code: Some(204),
            error: None,
        };
        assert_eq!(ok.log_line(), "endpointID=1 status=204 duration=1.23");
        assert!(ok.succeeded());
    }
}
