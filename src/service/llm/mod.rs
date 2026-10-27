//! Chat completion against the providers a user can bring an API key for: OpenAI, Anthropic, and
//! any host that speaks OpenAI's wire format (OpenRouter, Ollama, LM Studio, ...). The key is the
//! user's own, read from `keychain_ai_keys`; nothing here logs a request, and every error message
//! is scrubbed of the key before it leaves.
//!
//! Outside `src/mcp/` there is no async runtime, so a reply streams on a worker thread: the
//! caller hands `stream_chat` a callback that forwards each piece of text over a channel. The wire
//! formats are pure (`openai`, `anthropic`) and tested against recorded frames; `HttpProvider` is
//! the only part that touches the network.

// Transient: the Settings page and the Script Manager assistant wire these up next.
#![allow(dead_code)]

pub mod anthropic;
pub mod openai;
pub mod prefs;
pub mod sse;
pub mod summary;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sse::{Event, Flow};
use std::io::BufReader;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

// MARK: Types

/// The providers the assistant can talk to.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ProviderKind {
    #[default]
    OpenAi,
    Anthropic,
    /// Any host that copies OpenAI's chat completions API, at a base URL the user gives.
    Compatible,
}

impl ProviderKind {
    pub const ALL: [Self; 3] = [Self::OpenAi, Self::Anthropic, Self::Compatible];

    pub fn title(self) -> &'static str {
        match self {
            Self::OpenAi => "OpenAI",
            Self::Anthropic => "Anthropic",
            Self::Compatible => "OpenAI-compatible",
        }
    }

    /// The Keychain account and the settings key for this provider.
    pub fn key(self) -> &'static str {
        match self {
            Self::OpenAi => "openai",
            Self::Anthropic => "anthropic",
            Self::Compatible => "compatible",
        }
    }

    /// Where a developer gives the key when nothing is saved in the app.
    pub fn env_var(self) -> &'static str {
        match self {
            Self::OpenAi => "DEGENVIEW_OPENAI_KEY",
            Self::Anthropic => "DEGENVIEW_ANTHROPIC_KEY",
            Self::Compatible => "DEGENVIEW_LLM_KEY",
        }
    }

    pub fn default_base_url(self) -> &'static str {
        match self {
            Self::OpenAi => "https://api.openai.com/v1",
            Self::Anthropic => "https://api.anthropic.com",
            Self::Compatible => "",
        }
    }

    /// A starting model name. Names change often, so the field stays editable.
    pub fn default_model(self) -> &'static str {
        match self {
            Self::OpenAi => "gpt-5",
            Self::Anthropic => "claude-sonnet-5-5",
            Self::Compatible => "",
        }
    }

    /// A local server needs no key, so the compatible provider may go without one.
    pub fn needs_key(self) -> bool {
        self != Self::Compatible
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.key() == key)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Role {
    User,
    Assistant,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChatMessage {
    pub role: Role,
    pub content: String,
}

impl ChatMessage {
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: content.into(),
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            content: content.into(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct ChatRequest {
    pub model: String,
    pub system: String,
    pub messages: Vec<ChatMessage>,
    pub max_tokens: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Done {
    /// The reply stopped at the token limit, so a script in it may be cut off.
    pub truncated: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LlmError {
    Cancelled,
    /// 401 / 403: the key is wrong, expired, or not allowed to use the model.
    Auth(String),
    RateLimited(String),
    Http {
        status: u16,
        message: String,
    },
    Network(String),
    /// The server answered, but not in the format expected.
    Protocol(String),
    /// Nothing was sent: the settings are incomplete.
    Config(String),
}

impl std::fmt::Display for LlmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Cancelled => f.write_str("Cancelled"),
            Self::Auth(message) => write!(f, "The provider rejected the API key: {message}"),
            Self::RateLimited(message) => write!(f, "Rate limited: {message}"),
            Self::Http {
                status: 200,
                message,
            } => f.write_str(message),
            Self::Http { status, message } => write!(f, "HTTP {status}: {message}"),
            Self::Network(message) => write!(f, "Network error: {message}"),
            Self::Protocol(message) | Self::Config(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for LlmError {}

/// A provider that streams a reply. `on_text` gets each piece as it arrives; `cancel` is checked
/// between pieces.
pub trait LlmProvider: Send + Sync {
    fn stream_chat(
        &self,
        request: &ChatRequest,
        cancel: &AtomicBool,
        on_text: &mut dyn FnMut(&str),
    ) -> Result<Done, LlmError>;

    /// The model ids the key can use. Doubles as a cheap "does this key work" check, since it
    /// spends no tokens.
    fn list_models(&self) -> Result<Vec<String>, LlmError>;
}

// MARK: Wire formats

/// A request ready to send. Holds the key in a header, so it is deliberately not `Debug`.
pub struct WireRequest {
    pub url: String,
    pub headers: Vec<(String, String)>,
    /// Empty for a GET.
    pub body: Vec<u8>,
}

pub(crate) enum Step {
    Text(String),
    /// The provider's closing frame.
    End,
    Skip,
}

/// Turns a provider's stream frames into text.
pub(crate) trait Decoder {
    fn feed(&mut self, event: &Event) -> Result<Step, LlmError>;
    /// `Some(truncated)` once the provider said the reply is complete.
    fn finished(&self) -> Option<bool>;
}

/// Drives a decoder over a stream; shared by every provider.
pub(crate) fn run_stream<R: std::io::BufRead>(
    reader: R,
    mut decoder: impl Decoder,
    cancel: &AtomicBool,
    on_text: &mut dyn FnMut(&str),
) -> Result<Done, LlmError> {
    let mut any_text = false;
    sse::read_events(reader, cancel, |event| match decoder.feed(&event)? {
        Step::Text(text) => {
            any_text = true;
            on_text(&text);
            Ok(Flow::Continue)
        }
        Step::End => Ok(Flow::Stop),
        Step::Skip => Ok(Flow::Continue),
    })?;
    let Some(truncated) = decoder.finished() else {
        return Err(LlmError::Protocol(
            "The reply ended before the model finished it.".to_owned(),
        ));
    };
    if !any_text {
        return Err(LlmError::Protocol("The model returned no text.".to_owned()));
    }
    Ok(Done { truncated })
}

// MARK: Base URL

/// A base URL the key may be sent to: HTTPS, or plain HTTP to this machine (a local model server).
/// Anything else would put the key on the network in the clear, or hand it to the wrong host.
pub fn validate_base_url(text: &str) -> Result<String, String> {
    let text = text.trim();
    let (scheme, rest) = text
        .split_once("://")
        .ok_or("Enter a full URL such as https://openrouter.ai/api/v1.")?;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    if authority.is_empty() || authority.contains('@') || authority.contains(char::is_whitespace) {
        return Err("The URL needs a host and no user name.".to_owned());
    }
    let host = match authority.strip_prefix('[') {
        Some(v6) => v6.split(']').next().unwrap_or(""),
        None => authority.split(':').next().unwrap_or(""),
    };
    let local = matches!(host, "localhost" | "127.0.0.1" | "::1");
    match scheme {
        "https" => {}
        "http" if local => {}
        "http" => return Err("Use https://, or http:// only for a server on this Mac.".to_owned()),
        _ => return Err("The URL must start with https://.".to_owned()),
    }
    Ok(text.trim_end_matches('/').to_owned())
}

// MARK: HTTP provider

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// A reasoning model can think a long while before the first byte.
const FIRST_BYTE_TIMEOUT: Duration = Duration::from_secs(180);
const BODY_TIMEOUT: Duration = Duration::from_secs(15 * 60);
const MAX_STREAM_BYTES: u64 = 32 * 1024 * 1024;
const MAX_ERROR_BYTES: u64 = 64 * 1024;

pub struct HttpProvider {
    kind: ProviderKind,
    base_url: String,
    api_key: Option<String>,
    agent: ureq::Agent,
}

impl HttpProvider {
    /// `base_url` is ignored for OpenAI and Anthropic unless given (a proxy), and required for
    /// the compatible provider.
    pub fn new(
        kind: ProviderKind,
        base_url: &str,
        api_key: Option<String>,
    ) -> Result<Self, LlmError> {
        let base = if base_url.trim().is_empty() {
            kind.default_base_url()
        } else {
            base_url
        };
        if base.is_empty() {
            return Err(LlmError::Config(
                "Enter the base URL of the server in Settings ▸ AI.".to_owned(),
            ));
        }
        let base_url = validate_base_url(base).map_err(LlmError::Config)?;
        let api_key = api_key
            .map(|key| key.trim().to_owned())
            .filter(|key| !key.is_empty());
        if kind.needs_key() && api_key.is_none() {
            return Err(LlmError::Config(format!(
                "Add your {} API key in Settings ▸ AI.",
                kind.title()
            )));
        }
        // No redirects: a redirect could carry the key to another host.
        let config = ureq::Agent::config_builder()
            .timeout_connect(Some(CONNECT_TIMEOUT))
            .timeout_recv_response(Some(FIRST_BYTE_TIMEOUT))
            .timeout_recv_body(Some(BODY_TIMEOUT))
            .max_redirects(0)
            .max_redirects_will_error(false)
            .http_status_as_error(false)
            .build();
        Ok(Self {
            kind,
            base_url,
            api_key,
            agent: ureq::Agent::new_with_config(config),
        })
    }

    fn chat_request(&self, request: &ChatRequest) -> WireRequest {
        let key = self.api_key.as_deref();
        match self.kind {
            ProviderKind::OpenAi => {
                openai::build(openai::Flavor::OpenAi, &self.base_url, key, request)
            }
            ProviderKind::Compatible => {
                openai::build(openai::Flavor::Compatible, &self.base_url, key, request)
            }
            ProviderKind::Anthropic => {
                anthropic::build(&self.base_url, key.unwrap_or_default(), request)
            }
        }
    }

    fn models_request(&self) -> WireRequest {
        let key = self.api_key.as_deref();
        match self.kind {
            ProviderKind::Anthropic => {
                anthropic::models_request(&self.base_url, key.unwrap_or_default())
            }
            _ => openai::models_request(&self.base_url, key),
        }
    }

    /// Sends the request; a non-2xx answer becomes the error, a 2xx one the open body.
    fn send(&self, wire: &WireRequest) -> Result<ureq::Body, LlmError> {
        let response = if wire.body.is_empty() {
            let mut builder = self.agent.get(&wire.url);
            for (name, value) in &wire.headers {
                builder = builder.header(name, value);
            }
            builder.call()
        } else {
            let mut builder = self.agent.post(&wire.url);
            for (name, value) in &wire.headers {
                builder = builder.header(name, value);
            }
            builder.send(&wire.body[..])
        }
        .map_err(|error| LlmError::Network(self.scrub(&error.to_string())))?;
        let status = response.status().as_u16();
        let (_, mut body) = response.into_parts();
        if (200..300).contains(&status) {
            return Ok(body);
        }
        let text = body
            .with_config()
            .limit(MAX_ERROR_BYTES)
            .lossy_utf8(true)
            .read_to_string()
            .unwrap_or_default();
        Err(self.scrub_error(error_for_status(status, &text)))
    }

    fn scrub(&self, text: &str) -> String {
        match &self.api_key {
            Some(key) => text.replace(key.as_str(), "[key]"),
            None => text.to_owned(),
        }
    }

    fn scrub_error(&self, error: LlmError) -> LlmError {
        match error {
            LlmError::Auth(m) => LlmError::Auth(self.scrub(&m)),
            LlmError::RateLimited(m) => LlmError::RateLimited(self.scrub(&m)),
            LlmError::Http { status, message } => LlmError::Http {
                status,
                message: self.scrub(&message),
            },
            LlmError::Network(m) => LlmError::Network(self.scrub(&m)),
            LlmError::Protocol(m) => LlmError::Protocol(self.scrub(&m)),
            other => other,
        }
    }
}

impl LlmProvider for HttpProvider {
    fn stream_chat(
        &self,
        request: &ChatRequest,
        cancel: &AtomicBool,
        on_text: &mut dyn FnMut(&str),
    ) -> Result<Done, LlmError> {
        if request.model.trim().is_empty() {
            return Err(LlmError::Config(
                "Choose a model in Settings ▸ AI.".to_owned(),
            ));
        }
        let wire = self.chat_request(request);
        let mut body = self.send(&wire)?;
        let reader = BufReader::new(body.with_config().limit(MAX_STREAM_BYTES).reader());
        let result = match self.kind {
            ProviderKind::Anthropic => run_stream(
                reader,
                anthropic::AnthropicDecoder::default(),
                cancel,
                on_text,
            ),
            _ => run_stream(reader, openai::OpenAiDecoder::default(), cancel, on_text),
        };
        result.map_err(|error| self.scrub_error(error))
    }

    fn list_models(&self) -> Result<Vec<String>, LlmError> {
        let wire = self.models_request();
        let mut body = self.send(&wire)?;
        let text = body
            .with_config()
            .limit(4 * 1024 * 1024)
            .lossy_utf8(true)
            .read_to_string()
            .map_err(|error| LlmError::Network(self.scrub(&error.to_string())))?;
        parse_models(&text)
    }
}

/// `{"data": [{"id": "..."}]}` in both OpenAI's and Anthropic's lists.
pub(crate) fn parse_models(text: &str) -> Result<Vec<String>, LlmError> {
    let value: Value = serde_json::from_str(text)
        .map_err(|_| LlmError::Protocol("The server's model list was not JSON.".to_owned()))?;
    let data = value
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| LlmError::Protocol("The server's model list had no data.".to_owned()))?;
    let mut ids: Vec<String> = data
        .iter()
        .filter_map(|model| model.get("id").and_then(Value::as_str))
        .map(str::to_owned)
        .collect();
    ids.sort();
    Ok(ids)
}

/// Maps a failed HTTP answer to an error, using the provider's own message when the body has one
/// (`{"error": {"message": ...}}` in both formats).
pub(crate) fn error_for_status(status: u16, body: &str) -> LlmError {
    let message = serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|value| {
            value
                .pointer("/error/message")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .unwrap_or_else(|| {
            let snippet: String = body.trim().chars().take(200).collect();
            if snippet.is_empty() {
                "no details".to_owned()
            } else {
                snippet
            }
        });
    match status {
        401 | 403 => LlmError::Auth(message),
        429 => LlmError::RateLimited(message),
        _ => LlmError::Http { status, message },
    }
}

/// A scripted provider for tests: each call to `stream_chat` takes the next canned reply.
#[cfg(test)]
pub mod mock {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::Mutex;

    pub struct MockProvider {
        replies: Mutex<VecDeque<Result<Vec<String>, LlmError>>>,
        /// Every request seen, for assertions about what was sent.
        pub requests: Mutex<Vec<ChatRequest>>,
        pub models: Result<Vec<String>, LlmError>,
    }

    impl MockProvider {
        pub fn replying(replies: Vec<Result<Vec<String>, LlmError>>) -> Self {
            Self {
                replies: Mutex::new(replies.into()),
                requests: Mutex::new(Vec::new()),
                models: Ok(vec!["mock-1".to_owned()]),
            }
        }

        pub fn text(pieces: &[&str]) -> Result<Vec<String>, LlmError> {
            Ok(pieces.iter().map(|piece| (*piece).to_owned()).collect())
        }
    }

    impl LlmProvider for MockProvider {
        fn stream_chat(
            &self,
            request: &ChatRequest,
            cancel: &AtomicBool,
            on_text: &mut dyn FnMut(&str),
        ) -> Result<Done, LlmError> {
            self.requests.lock().unwrap().push(request.clone());
            let reply = self
                .replies
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| Err(LlmError::Protocol("no scripted reply".to_owned())))?;
            for piece in reply {
                if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                    return Err(LlmError::Cancelled);
                }
                on_text(&piece);
            }
            Ok(Done { truncated: false })
        }

        fn list_models(&self) -> Result<Vec<String>, LlmError> {
            self.models.clone()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;

    #[test]
    fn base_urls_must_be_https_or_this_machine() {
        assert_eq!(
            validate_base_url(" https://openrouter.ai/api/v1/ ").as_deref(),
            Ok("https://openrouter.ai/api/v1")
        );
        assert!(validate_base_url("http://localhost:11434/v1").is_ok());
        assert!(validate_base_url("http://127.0.0.1:1234/v1").is_ok());
        assert!(validate_base_url("http://[::1]:8080").is_ok());
        assert!(validate_base_url("http://example.com/v1").is_err());
        assert!(validate_base_url("ftp://example.com").is_err());
        assert!(validate_base_url("example.com/v1").is_err());
        assert!(validate_base_url("https://").is_err());
    }

    #[test]
    fn a_user_name_cannot_disguise_the_host() {
        assert!(validate_base_url("http://localhost@evil.example/v1").is_err());
        assert!(validate_base_url("https://user:pw@example.com").is_err());
        assert!(validate_base_url("http://localhost.evil.example/v1").is_err());
    }

    #[test]
    fn a_provider_needs_its_key_but_a_local_server_does_not() {
        let missing = HttpProvider::new(ProviderKind::OpenAi, "", None);
        assert!(matches!(missing, Err(LlmError::Config(_))));
        let blank = HttpProvider::new(ProviderKind::Anthropic, "", Some("  ".to_owned()));
        assert!(matches!(blank, Err(LlmError::Config(_))));
        assert!(HttpProvider::new(ProviderKind::OpenAi, "", Some("k".to_owned())).is_ok());
        assert!(
            HttpProvider::new(ProviderKind::Compatible, "http://localhost:11434/v1", None).is_ok()
        );
        let no_url = HttpProvider::new(ProviderKind::Compatible, "", None);
        assert!(matches!(no_url, Err(LlmError::Config(_))));
    }

    #[test]
    fn errors_use_the_providers_message_and_never_the_key() {
        let body = r#"{"error":{"message":"Incorrect API key provided: sk-secret"}}"#;
        let provider =
            HttpProvider::new(ProviderKind::OpenAi, "", Some("sk-secret".to_owned())).unwrap();
        let error = provider.scrub_error(error_for_status(401, body));
        assert_eq!(
            error,
            LlmError::Auth("Incorrect API key provided: [key]".to_owned())
        );
        assert!(!error.to_string().contains("sk-secret"));
    }

    #[test]
    fn statuses_map_to_error_kinds() {
        assert!(matches!(error_for_status(401, "{}"), LlmError::Auth(_)));
        assert!(matches!(error_for_status(403, ""), LlmError::Auth(_)));
        assert!(matches!(
            error_for_status(429, ""),
            LlmError::RateLimited(_)
        ));
        assert_eq!(
            error_for_status(500, "<html>oops</html>"),
            LlmError::Http {
                status: 500,
                message: "<html>oops</html>".to_owned()
            }
        );
        assert_eq!(
            error_for_status(502, "  "),
            LlmError::Http {
                status: 502,
                message: "no details".to_owned()
            }
        );
    }

    #[test]
    fn model_lists_are_sorted_ids() {
        let ids = parse_models(r#"{"data":[{"id":"b"},{"id":"a"},{"nope":1}]}"#).unwrap();
        assert_eq!(ids, vec!["a", "b"]);
        assert!(parse_models("[]").is_err());
        assert!(parse_models("nope").is_err());
    }

    fn stream_openai(input: &str) -> (Result<Done, LlmError>, String) {
        let cancel = AtomicBool::new(false);
        let mut text = String::new();
        let result = run_stream(
            input.as_bytes(),
            openai::OpenAiDecoder::default(),
            &cancel,
            &mut |piece| text.push_str(piece),
        );
        (result, text)
    }

    #[test]
    fn a_whole_openai_stream_assembles_the_reply() {
        let (result, text) = stream_openai(
            "data: {\"choices\":[{\"delta\":{\"content\":\"Hel\"}}]}\n\n\
             data: {\"choices\":[{\"delta\":{\"content\":\"lo\"},\"finish_reason\":\"stop\"}]}\n\n\
             data: [DONE]\n\n",
        );
        assert_eq!(result, Ok(Done { truncated: false }));
        assert_eq!(text, "Hello");
    }

    #[test]
    fn a_stream_that_just_stops_is_an_error_not_a_reply() {
        let (result, text) =
            stream_openai("data: {\"choices\":[{\"delta\":{\"content\":\"Hel\"}}]}\n\n");
        assert!(matches!(result, Err(LlmError::Protocol(_))));
        assert_eq!(text, "Hel");
    }

    #[test]
    fn a_reply_with_no_text_is_an_error() {
        let (result, _) = stream_openai("data: [DONE]\n\n");
        assert!(matches!(result, Err(LlmError::Protocol(_))));
    }

    #[test]
    fn a_whole_anthropic_stream_assembles_the_reply() {
        let input = "event: message_start\ndata: {\"type\":\"message_start\"}\n\n\
             event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"Hi\"}}\n\n\
             event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"max_tokens\"}}\n\n\
             event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n";
        let cancel = AtomicBool::new(false);
        let mut text = String::new();
        let result = run_stream(
            input.as_bytes(),
            anthropic::AnthropicDecoder::default(),
            &cancel,
            &mut |piece| text.push_str(piece),
        );
        assert_eq!(result, Ok(Done { truncated: true }));
        assert_eq!(text, "Hi");
    }

    #[test]
    fn cancel_stops_a_stream_between_frames() {
        let cancel = AtomicBool::new(false);
        let mut pieces = 0;
        let input = "data: {\"choices\":[{\"delta\":{\"content\":\"a\"}}]}\n\n\
                     data: {\"choices\":[{\"delta\":{\"content\":\"b\"}}]}\n\n";
        let result = run_stream(
            input.as_bytes(),
            openai::OpenAiDecoder::default(),
            &cancel,
            &mut |_| {
                pieces += 1;
                cancel.store(true, Ordering::Relaxed);
            },
        );
        assert_eq!(result, Err(LlmError::Cancelled));
        assert_eq!(pieces, 1);
    }

    /// Serves one canned HTTP answer on a loopback port and returns the request it received.
    fn serve_once(
        status: &str,
        content_type: &str,
        body: &str,
    ) -> (String, std::thread::JoinHandle<String>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!(
            "http://127.0.0.1:{}/v1",
            listener.local_addr().unwrap().port()
        );
        let response = format!(
            "HTTP/1.1 {status}\r\ncontent-type: {content_type}\r\nconnection: close\r\n\r\n{body}"
        );
        let handle = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut received = Vec::new();
            let mut chunk = [0_u8; 4096];
            loop {
                let read = socket.read(&mut chunk).unwrap();
                received.extend_from_slice(&chunk[..read]);
                let text = String::from_utf8_lossy(&received).to_string();
                if let Some((head, rest)) = text.split_once("\r\n\r\n") {
                    let wanted = head
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .and_then(|value| value.trim().parse::<usize>().ok())
                        })
                        .unwrap_or(0);
                    if rest.len() >= wanted {
                        break;
                    }
                }
                if read == 0 {
                    break;
                }
            }
            socket.write_all(response.as_bytes()).unwrap();
            String::from_utf8_lossy(&received).to_string()
        });
        (base, handle)
    }

    fn hello_request() -> ChatRequest {
        ChatRequest {
            model: "local-model".to_owned(),
            system: "sys".to_owned(),
            messages: vec![ChatMessage::user("hi")],
            max_tokens: 64,
        }
    }

    #[test]
    fn a_real_request_carries_the_key_and_streams_the_reply_back() {
        let sse = "data: {\"choices\":[{\"delta\":{\"content\":\"Hel\"}}]}\n\n\
                   data: {\"choices\":[{\"delta\":{\"content\":\"lo\"},\"finish_reason\":\"stop\"}]}\n\n\
                   data: [DONE]\n\n";
        let (base, server) = serve_once("200 OK", "text/event-stream", sse);
        let provider =
            HttpProvider::new(ProviderKind::Compatible, &base, Some("sk-local".to_owned()))
                .unwrap();
        let mut text = String::new();
        let done = provider
            .stream_chat(&hello_request(), &AtomicBool::new(false), &mut |piece| {
                text.push_str(piece)
            })
            .unwrap();
        assert_eq!(text, "Hello");
        assert!(!done.truncated);
        let sent = server.join().unwrap().to_ascii_lowercase();
        assert!(sent.starts_with("post /v1/chat/completions "));
        assert!(sent.contains("authorization: bearer sk-local"));
        assert!(sent.contains("\"stream\":true"));
        assert!(sent.contains("\"model\":\"local-model\""));
    }

    #[test]
    fn a_rejected_key_comes_back_as_an_auth_error_without_the_key() {
        let body = r#"{"error":{"message":"bad key sk-local"}}"#;
        let (base, server) = serve_once("401 Unauthorized", "application/json", body);
        let provider =
            HttpProvider::new(ProviderKind::Compatible, &base, Some("sk-local".to_owned()))
                .unwrap();
        let error = provider
            .stream_chat(&hello_request(), &AtomicBool::new(false), &mut |_| {})
            .unwrap_err();
        server.join().unwrap();
        assert_eq!(error, LlmError::Auth("bad key [key]".to_owned()));
    }

    #[test]
    fn listing_models_is_a_get_that_spends_nothing() {
        let (base, server) = serve_once(
            "200 OK",
            "application/json",
            r#"{"data":[{"id":"m2"},{"id":"m1"}]}"#,
        );
        let provider = HttpProvider::new(ProviderKind::Compatible, &base, None).unwrap();
        assert_eq!(provider.list_models().unwrap(), vec!["m1", "m2"]);
        let sent = server.join().unwrap().to_ascii_lowercase();
        assert!(sent.starts_with("get /v1/models "));
        assert!(!sent.contains("authorization"));
    }

    #[test]
    fn a_redirect_is_an_error_not_a_second_hop_with_the_key() {
        let (base, server) = serve_once("302 Found", "text/plain", "");
        let provider =
            HttpProvider::new(ProviderKind::Compatible, &base, Some("sk-local".to_owned()))
                .unwrap();
        let error = provider
            .stream_chat(&hello_request(), &AtomicBool::new(false), &mut |_| {})
            .unwrap_err();
        server.join().unwrap();
        assert!(matches!(error, LlmError::Http { status: 302, .. }));
    }

    #[test]
    fn provider_keys_round_trip() {
        for kind in ProviderKind::ALL {
            assert_eq!(ProviderKind::from_key(kind.key()), Some(kind));
        }
        assert_eq!(ProviderKind::from_key("nope"), None);
    }
}
