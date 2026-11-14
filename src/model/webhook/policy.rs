//! The rules for what a webhook may be and how a request is built: which URLs are
//! acceptable, what a custom header may say, where `{{secret}}` may go and how it is
//! encoded, how a URL is redacted for display, and which Content-Type the body gets.
//! One place, so the editor, the store and the request builder cannot disagree.

use crate::model::alert::{AlertEventOrigin, AlertTriggerEvent};
use crate::model::webhook::{WebhookDeliveryError, WebhookHeader};

// MARK: URL

/// A URL split into the parts the policy and the redactor care about. Not a general URL
/// parser: it understands `scheme://[userinfo@]host[:port][/path][?query][#fragment]`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParsedUrl {
    pub scheme: String,
    pub userinfo: Option<String>,
    pub host: String,
    pub port: Option<u32>,
    pub path: String,
    pub query: Option<String>,
    pub fragment: Option<String>,
}

/// Characters that make a string no URL at all (RFC 3986 forbids them unescaped).
fn is_forbidden_in_url(character: char) -> bool {
    character.is_control() || character.is_whitespace() || "<>\"\\^`{|}".contains(character)
}

/// Splits `text`, or `None` when it is not a URL with a scheme and an authority.
pub fn parse_url(text: &str) -> Option<ParsedUrl> {
    let text = text.trim();
    if text.is_empty() || text.chars().any(is_forbidden_in_url) {
        return None;
    }
    let (scheme, rest) = text.split_once("://")?;
    if scheme.is_empty()
        || !scheme.starts_with(|c: char| c.is_ascii_alphabetic())
        || !scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c))
    {
        return None;
    }
    let (rest, fragment) = match rest.split_once('#') {
        Some((before, fragment)) => (before, Some(fragment.to_owned())),
        None => (rest, None),
    };
    let (rest, query) = match rest.split_once('?') {
        Some((before, query)) => (before, Some(query.to_owned())),
        None => (rest, None),
    };
    let (authority, path) = match rest.find('/') {
        Some(index) => (&rest[..index], rest[index..].to_owned()),
        None => (rest, String::new()),
    };
    let (userinfo, host_port) = match authority.rsplit_once('@') {
        Some((userinfo, host_port)) => (Some(userinfo.to_owned()), host_port),
        None => (None, authority),
    };
    let (host, port) = split_host_port(host_port)?;
    Some(ParsedUrl {
        scheme: scheme.to_lowercase(),
        userinfo,
        host,
        port,
        path,
        query,
        fragment,
    })
}

/// `host`, `host:port`, `[v6]`, `[v6]:port`. A port that is not all digits is no URL;
/// its range is the policy's business (`unsupportedPort`).
fn split_host_port(text: &str) -> Option<(String, Option<u32>)> {
    let (host, port_text) = if let Some(stripped) = text.strip_prefix('[') {
        let (inside, after) = stripped.split_once(']')?;
        let port = match after {
            "" => None,
            _ => Some(after.strip_prefix(':')?),
        };
        (inside.to_owned(), port)
    } else {
        match text.rsplit_once(':') {
            Some((host, port)) => (host.to_owned(), Some(port)),
            None => (text.to_owned(), None),
        }
    };
    let port = match port_text {
        None | Some("") => None,
        Some(digits) if digits.chars().all(|c| c.is_ascii_digit()) => {
            Some(digits.parse::<u32>().unwrap_or(u32::MAX))
        }
        Some(_) => return None,
    };
    Some((host, port))
}

pub const ALLOWED_SCHEMES: [&str; 2] = ["http", "https"];

/// The one place that decides which webhook URLs are acceptable.
///
/// Policy: `http` and `https` only, a host is required, any port. Loopback, link-local and
/// private-network hosts are allowed on purpose: DegenView is a desktop app and the endpoints
/// are configured by its owner, and local trading bridges and dev servers are a legitimate
/// target. This is looser than TradingView, which only reaches ports 80 and 443.
pub fn validate_url(text: &str) -> Result<ParsedUrl, WebhookDeliveryError> {
    let parsed = parse_url(text).ok_or(WebhookDeliveryError::InvalidUrl)?;
    if !ALLOWED_SCHEMES.contains(&parsed.scheme.as_str()) {
        return Err(WebhookDeliveryError::UnsupportedScheme);
    }
    if parsed.host.is_empty() {
        return Err(WebhookDeliveryError::InvalidUrl);
    }
    if parsed
        .port
        .is_some_and(|port| !(1..=65_535).contains(&port))
    {
        return Err(WebhookDeliveryError::UnsupportedPort);
    }
    Ok(parsed)
}

/// A message for the editor. Never includes the URL.
pub fn url_message(error: WebhookDeliveryError) -> &'static str {
    match error {
        WebhookDeliveryError::UnsupportedScheme => "Use an http:// or https:// address.",
        WebhookDeliveryError::UnsupportedPort => "The port must be between 1 and 65535.",
        WebhookDeliveryError::SecretInHost => {
            "The secret can't go in the host. Use it in the path or after a ?."
        }
        _ => "Enter a full address, such as https://example.com/hook.",
    }
}

/// Webhook URLs can be credentials. A log line goes through here first: `https://example.com/•••`.
/// The scheme and host stay so endpoints can be told apart; path, query, fragment and userinfo
/// never appear. Unparseable input yields a bare marker. The Settings row shows the address in
/// full, as the user typed it: a secret in it is a `{{secret}}` placeholder, its value is in
/// the secret store.
#[allow(
    dead_code,
    reason = "the webhook guide sends any logged webhook URL through here; nothing logs one yet"
)]
pub fn redact(url: &str) -> String {
    let Some(parsed) = parse_url(url).filter(|parsed| !parsed.host.is_empty()) else {
        return "•••".to_owned();
    };
    let host = if parsed.host.contains(':') {
        format!("[{}]", parsed.host)
    } else {
        parsed.host.clone()
    };
    let port = parsed
        .port
        .map_or_else(String::new, |port| format!(":{port}"));
    let has_more = !(parsed.path.is_empty() || parsed.path == "/") || parsed.query.is_some();
    format!(
        "{}://{host}{port}/{}",
        parsed.scheme,
        if has_more { "•••" } else { "" }
    )
}

// MARK: Headers

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WebhookHeaderError {
    EmptyName,
    InvalidName,
    NameTooLong,
    Reserved,
    Duplicate,
    InvalidValue,
    ValueTooLong,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WebhookSecretError {
    Empty,
    TooLong,
    ControlCharacters,
}

pub const MAXIMUM_HEADERS: usize = 10;
pub const MAXIMUM_NAME_LENGTH: usize = 64;
pub const MAXIMUM_VALUE_LENGTH: usize = 2048;
pub const MAXIMUM_SECRET_LENGTH: usize = 2048;

/// Headers DegenView or the HTTP stack own. Letting a user override them could break the
/// body's Content-Type detection or the connection itself.
const RESERVED: [&str; 10] = [
    "content-type",
    "content-length",
    "host",
    "connection",
    "transfer-encoding",
    "expect",
    "upgrade",
    "te",
    "trailer",
    "keep-alive",
];

/// RFC 7230 `token` characters.
fn is_token_character(character: char) -> bool {
    character.is_ascii_alphanumeric() || "!#$%&'*+-.^_`|~".contains(character)
}

pub fn validate_header_name(raw: &str) -> Option<WebhookHeaderError> {
    let name = raw.trim_matches([' ', '\t']);
    if name.is_empty() {
        return Some(WebhookHeaderError::EmptyName);
    }
    if name.chars().count() > MAXIMUM_NAME_LENGTH {
        return Some(WebhookHeaderError::NameTooLong);
    }
    if !name.chars().all(is_token_character) {
        return Some(WebhookHeaderError::InvalidName);
    }
    if RESERVED.contains(&name.to_lowercase().as_str()) {
        return Some(WebhookHeaderError::Reserved);
    }
    None
}

/// Printable ASCII and tabs only: no line breaks (header injection) and nothing a server
/// might mis-decode. The check is made on the template and again after the secret is in.
pub fn validate_header_value(raw: &str) -> Option<WebhookHeaderError> {
    let value = raw.trim_matches([' ', '\t']);
    if value.chars().count() > MAXIMUM_VALUE_LENGTH {
        return Some(WebhookHeaderError::ValueTooLong);
    }
    if !value.chars().all(is_header_value_character) {
        return Some(WebhookHeaderError::InvalidValue);
    }
    None
}

fn is_header_value_character(character: char) -> bool {
    character == '\t' || ('\u{20}'..='\u{7E}').contains(&character)
}

/// The first problem with each row, by header id. Empty means every row is fine.
pub fn header_errors(headers: &[WebhookHeader]) -> Vec<(u64, WebhookHeaderError)> {
    let mut errors = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for header in headers {
        if let Some(problem) =
            validate_header_name(&header.name).or_else(|| validate_header_value(&header.value))
        {
            errors.push((header.id, problem));
            continue;
        }
        if !seen.insert(header.name.trim().to_lowercase()) {
            errors.push((header.id, WebhookHeaderError::Duplicate));
        }
    }
    errors
}

pub fn headers_are_valid(headers: &[WebhookHeader]) -> bool {
    headers.len() <= MAXIMUM_HEADERS && header_errors(headers).is_empty()
}

pub fn validate_secret(secret: &str) -> Option<WebhookSecretError> {
    if secret.is_empty() {
        return Some(WebhookSecretError::Empty);
    }
    if secret.chars().count() > MAXIMUM_SECRET_LENGTH {
        return Some(WebhookSecretError::TooLong);
    }
    if secret.chars().any(char::is_control) {
        return Some(WebhookSecretError::ControlCharacters);
    }
    None
}

pub fn header_message(error: WebhookHeaderError) -> String {
    match error {
        WebhookHeaderError::EmptyName => "Enter a header name.".to_owned(),
        WebhookHeaderError::InvalidName => "Use letters, numbers and - _ . only.".to_owned(),
        WebhookHeaderError::NameTooLong => {
            format!("Header names are limited to {MAXIMUM_NAME_LENGTH} characters.")
        }
        WebhookHeaderError::Reserved => "DegenView sets this header itself.".to_owned(),
        WebhookHeaderError::Duplicate => "This header is already set.".to_owned(),
        WebhookHeaderError::InvalidValue => {
            "Use plain letters, numbers and symbols (no line breaks).".to_owned()
        }
        WebhookHeaderError::ValueTooLong => {
            format!("Header values are limited to {MAXIMUM_VALUE_LENGTH} characters.")
        }
    }
}

pub fn secret_message(error: WebhookSecretError) -> String {
    match error {
        WebhookSecretError::Empty => "Enter the secret.".to_owned(),
        WebhookSecretError::TooLong => {
            format!("Secrets are limited to {MAXIMUM_SECRET_LENGTH} characters.")
        }
        WebhookSecretError::ControlCharacters => {
            "The secret can't contain line breaks or control characters.".to_owned()
        }
    }
}

/// Whether a header of this name normally carries a credential, for the "save it as the
/// secret" nudge.
pub fn looks_sensitive(name: &str) -> bool {
    let lowered = name.to_lowercase();
    [
        "authorization",
        "key",
        "token",
        "secret",
        "auth",
        "password",
        "bearer",
    ]
    .iter()
    .any(|word| lowered.contains(word))
}

// MARK: The `{{secret}}` token

/// What the editor inserts.
pub const SECRET_TOKEN: &str = "{{secret}}";

/// Byte ranges of every `{{secret}}` in `text`, tolerant of spacing and case
/// (`{{ Secret }}`), because it is typed by people, not generated.
fn secret_ranges(text: &str) -> Vec<std::ops::Range<usize>> {
    let bytes: Vec<(usize, char)> = text.char_indices().collect();
    let mut ranges = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        if text[bytes[index].0..].starts_with("{{")
            && let Some(end) = match_secret(text, &bytes, index + 2)
        {
            let end_byte = bytes.get(end).map_or(text.len(), |(offset, _)| *offset);
            ranges.push(bytes[index].0..end_byte);
            index = end;
            continue;
        }
        index += 1;
    }
    ranges
}

/// After `{{`: optional spaces, `secret` in any case, optional spaces, `}}`. Returns the
/// index just past the closing braces.
fn match_secret(text: &str, chars: &[(usize, char)], mut index: usize) -> Option<usize> {
    while chars.get(index).is_some_and(|(_, c)| c.is_whitespace()) {
        index += 1;
    }
    for expected in "secret".chars() {
        let (_, found) = chars.get(index)?;
        if !found.to_lowercase().eq(expected.to_lowercase()) {
            return None;
        }
        index += 1;
    }
    while chars.get(index).is_some_and(|(_, c)| c.is_whitespace()) {
        index += 1;
    }
    let (offset, _) = chars.get(index)?;
    text[*offset..].starts_with("}}").then_some(index + 2)
}

pub fn contains_secret(text: &str) -> bool {
    !secret_ranges(text).is_empty()
}

/// Replaces every placeholder with `replacement`, literally: nothing in the replacement
/// means anything special.
pub fn replace_secret(text: &str, replacement: &str) -> String {
    let ranges = secret_ranges(text);
    if ranges.is_empty() {
        return text.to_owned();
    }
    let mut result = String::with_capacity(text.len());
    let mut cursor = 0;
    for range in ranges {
        result.push_str(&text[cursor..range.start]);
        result.push_str(replacement);
        cursor = range.end;
    }
    result.push_str(&text[cursor..]);
    result
}

/// Percent-encodes `secret` for use inside a URL with the RFC 3986 unreserved set only
/// (`A-Z a-z 0-9 - . _ ~`), so one rule is right in a path segment, a query value or a
/// fragment: `/`, `?`, `&`, `=`, `#`, `+`, `%`, spaces and non-ASCII are all escaped.
pub fn url_encode(secret: &str) -> String {
    let mut encoded = String::with_capacity(secret.len());
    for byte in secret.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
            encoded.push(byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

// MARK: Resolving a request

/// A request ready to send: the final URL and headers, with the secret already in. It
/// holds the secret, so it must never be logged or stored.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedRequest {
    pub url: String,
    pub headers: Vec<WebhookHeader>,
}

/// Checks everything that does not need the secret value: the URL's shape and where the
/// secret sits in it, and the headers. The editor and the store use this before saving.
pub fn validate_templates(url: &str, headers: &[WebhookHeader]) -> Option<WebhookDeliveryError> {
    let trimmed = url.trim();
    if secret_is_in_authority(trimmed) {
        return Some(WebhookDeliveryError::SecretInHost);
    }
    if let Err(error) = validate_url(&replace_secret(trimmed, "x")) {
        return Some(error);
    }
    if !headers_are_valid(headers) {
        return Some(WebhookDeliveryError::InvalidHeader);
    }
    None
}

/// Whether the URL or a header asks for the Keychain secret.
pub fn references_secret(url: &str, headers: &[WebhookHeader]) -> bool {
    contains_secret(url) || headers.iter().any(|header| contains_secret(&header.value))
}

/// `secret` is required only when the URL or a header uses `{{secret}}`; an unused secret
/// is ignored. `None` means it could not be read.
pub fn resolve(
    url: &str,
    headers: &[WebhookHeader],
    secret: Option<&str>,
) -> Result<ResolvedRequest, WebhookDeliveryError> {
    if let Some(problem) = validate_templates(url, headers) {
        return Err(problem);
    }
    let trimmed = url.trim();
    let secret_text = secret.unwrap_or("");
    if references_secret(trimmed, headers) && secret_text.is_empty() {
        return Err(WebhookDeliveryError::SecretUnavailable);
    }
    let resolved_url = replace_secret(trimmed, &url_encode(secret_text));
    validate_url(&resolved_url)?;

    let mut resolved = Vec::with_capacity(headers.len());
    for header in headers {
        let value = replace_secret(header.value.trim_matches([' ', '\t']), secret_text);
        // The secret may add characters the template did not have.
        if validate_header_value(&value).is_some() {
            return Err(WebhookDeliveryError::InvalidHeader);
        }
        resolved.push(WebhookHeader {
            id: header.id,
            name: header.name.trim().to_owned(),
            value,
        });
    }
    Ok(ResolvedRequest {
        url: resolved_url,
        headers: resolved,
    })
}

/// The URL as the editor previews it: the secret shown as bullets, nothing sensitive.
pub fn masked_url(template: &str) -> String {
    replace_secret(template.trim(), "••••••")
}

/// Whether the placeholder sits in the scheme, user info, host or port — before the path.
pub fn secret_is_in_authority(template: &str) -> bool {
    if !contains_secret(template) {
        return false;
    }
    // Without a scheme the URL policy reports the real problem.
    let Some((scheme, rest)) = template.split_once("://") else {
        return false;
    };
    let authority_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    contains_secret(scheme) || contains_secret(&rest[..authority_end])
}

// MARK: "Use secret…" shortcuts

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SecretPlacement {
    /// Add `?name={{secret}}` (or `&name=…`) to the URL.
    UrlParameter(String),
    /// `Authorization: Bearer {{secret}}`
    BearerToken,
    /// `X-API-Key: {{secret}}`
    ApiKey,
    /// A new header whose name the user fills in.
    CustomHeader,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlacementEdit {
    pub url: String,
    pub headers: Vec<WebhookHeader>,
    /// Whether the change lands in the headers section, so the editor can open it.
    pub touches_headers: bool,
}

/// Each shortcut edits the URL or headers so a non-technical user never has to type
/// `{{secret}}`. Pure and idempotent: pressing one twice changes nothing the second time.
pub fn apply_placement(
    choice: &SecretPlacement,
    url: &str,
    headers: &[WebhookHeader],
) -> PlacementEdit {
    match choice {
        SecretPlacement::UrlParameter(name) => PlacementEdit {
            url: adding_parameter(name, url),
            headers: headers.to_vec(),
            touches_headers: false,
        },
        SecretPlacement::BearerToken => PlacementEdit {
            url: url.to_owned(),
            headers: setting_header("Authorization", &format!("Bearer {SECRET_TOKEN}"), headers),
            touches_headers: true,
        },
        SecretPlacement::ApiKey => PlacementEdit {
            url: url.to_owned(),
            headers: setting_header("X-API-Key", SECRET_TOKEN, headers),
            touches_headers: true,
        },
        SecretPlacement::CustomHeader => {
            let already_there = headers
                .iter()
                .any(|header| header.name.trim().is_empty() && contains_secret(&header.value));
            let mut added = headers.to_vec();
            if !already_there {
                added.push(WebhookHeader::new("", SECRET_TOKEN));
            }
            PlacementEdit {
                url: url.to_owned(),
                headers: added,
                touches_headers: true,
            }
        }
    }
}

/// Appends the parameter before any `#fragment`, with `?` or `&` as the URL needs. A URL
/// that already uses the secret, or is empty, is returned unchanged.
pub fn adding_parameter(name: &str, url: &str) -> String {
    let trimmed = url.trim();
    if trimmed.is_empty() || contains_secret(trimmed) {
        return url.to_owned();
    }
    let (base, fragment) = match trimmed.find('#') {
        Some(index) => (&trimmed[..index], &trimmed[index..]),
        None => (trimmed, ""),
    };
    let separator = if base.contains('?') {
        if base.ends_with('?') || base.ends_with('&') {
            ""
        } else {
            "&"
        }
    } else {
        "?"
    };
    format!("{base}{separator}{name}={SECRET_TOKEN}{fragment}")
}

/// Sets `name` (case-insensitively) if a header of that name exists, else appends it. A
/// header that already carries the secret is left as is.
fn setting_header(name: &str, value: &str, headers: &[WebhookHeader]) -> Vec<WebhookHeader> {
    let mut result = headers.to_vec();
    match result
        .iter_mut()
        .find(|header| header.name.trim().eq_ignore_ascii_case(name))
    {
        Some(existing) => {
            if !contains_secret(&existing.value) {
                existing.value = value.to_owned();
            }
        }
        None => result.push(WebhookHeader::new(name, value)),
    }
    result
}

/// A header that looks like it carries a credential typed in plain, which would otherwise
/// be saved in the database instead of the Keychain.
pub fn plain_credential_header(headers: &[WebhookHeader]) -> Option<&WebhookHeader> {
    headers.iter().find(|header| {
        looks_sensitive(&header.name)
            && !header.value.trim().is_empty()
            && !contains_secret(&header.value)
    })
}

/// Moves such a header's value into the secret: `Bearer abc` becomes secret `abc` and
/// value `Bearer {{secret}}`; anything else moves whole.
pub fn moving_to_secret(header: &WebhookHeader) -> (String, String) {
    let value = header.value.trim();
    for prefix in ["Bearer ", "Token ", "Basic "] {
        if value.len() >= prefix.len() && value[..prefix.len()].eq_ignore_ascii_case(prefix) {
            return (
                value[prefix.len()..].trim().to_owned(),
                format!("{prefix}{SECRET_TOKEN}"),
            );
        }
    }
    (value.to_owned(), SECRET_TOKEN.to_owned())
}

// MARK: Body

pub const JSON_CONTENT_TYPE: &str = "application/json; charset=utf-8";
pub const TEXT_CONTENT_TYPE: &str = "text/plain; charset=utf-8";

/// The HTTP body and Content-Type for a rendered alert message. The message is the body,
/// byte for byte: nothing is wrapped, reformatted or terminated with a newline. Only the
/// Content-Type depends on the content, as on TradingView: JSON when the whole message
/// parses as a JSON object or array, plain text otherwise.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WebhookPayload {
    pub body: Vec<u8>,
    pub content_type: &'static str,
}

impl WebhookPayload {
    pub fn new(message: &str) -> Self {
        Self {
            body: message.as_bytes().to_vec(),
            content_type: if is_json(message) {
                JSON_CONTENT_TYPE
            } else {
                TEXT_CONTENT_TYPE
            },
        }
    }
}

/// Real parsing, not a prefix check. Bare scalars such as `123` or `true` are not treated
/// as JSON: nobody configures a webhook to receive a lone number, and a plain word like
/// `BUY` must stay text either way.
pub fn is_json(message: &str) -> bool {
    matches!(
        serde_json::from_str::<serde_json::Value>(message),
        Ok(serde_json::Value::Object(_) | serde_json::Value::Array(_))
    )
}

// MARK: Which price-alert triggers may be sent

/// A trigger older than this is never sent. If an owner died with a webhook unsent, the next
/// owner must not post a stale "BUY" a minute later.
pub const MAXIMUM_EVENT_AGE_MS: i64 = 60_000;

/// Whether a price-alert trigger's webhooks may go out now. Checked before any delivery row is
/// claimed: delivery switched off, a catch-up event (the app was closed when it happened) and an
/// event past its minute all send nothing and record nothing.
pub fn price_event_is_sendable(
    event: &AlertTriggerEvent,
    delivery_enabled: bool,
    now_ms: i64,
) -> bool {
    delivery_enabled
        && event.origin == AlertEventOrigin::Live
        && now_ms - event.timestamp_ms <= MAXIMUM_EVENT_AGE_MS
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(name: &str, value: &str) -> WebhookHeader {
        WebhookHeader::new(name, value)
    }

    #[test]
    fn urls_are_http_or_https_with_a_host_and_any_valid_port() {
        for ok in [
            "https://example.com/hook",
            "http://localhost:8080/x",
            "http://127.0.0.1:1",
            "http://192.168.1.10:65535/bridge",
            "https://[::1]:8443/hook",
            "HTTPS://Example.com",
            "  https://example.com/hook  ",
        ] {
            assert!(validate_url(ok).is_ok(), "{ok}");
        }
        assert_eq!(
            validate_url("ftp://example.com"),
            Err(WebhookDeliveryError::UnsupportedScheme)
        );
        assert_eq!(
            validate_url("javascript://x"),
            Err(WebhookDeliveryError::UnsupportedScheme)
        );
        assert_eq!(
            validate_url("file:///etc/passwd"),
            Err(WebhookDeliveryError::UnsupportedScheme)
        );
        assert_eq!(
            validate_url("http://host:0"),
            Err(WebhookDeliveryError::UnsupportedPort)
        );
        assert_eq!(
            validate_url("http://host:70000"),
            Err(WebhookDeliveryError::UnsupportedPort)
        );
        assert_eq!(
            validate_url("http://host:99999999999999999999"),
            Err(WebhookDeliveryError::UnsupportedPort)
        );
        for bad in [
            "",
            "   ",
            "example.com/hook",
            "http://",
            "http:///path",
            "https://exa mple.com",
            "https://x.com/a b",
            "http://host:abc",
        ] {
            assert_eq!(
                validate_url(bad),
                Err(WebhookDeliveryError::InvalidUrl),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn url_parts_are_split_correctly() {
        let parsed = parse_url("https://user:pw@Example.com:8443/a/b?x=1#frag").unwrap();
        assert_eq!(parsed.scheme, "https");
        assert_eq!(parsed.userinfo.as_deref(), Some("user:pw"));
        assert_eq!(parsed.host, "Example.com");
        assert_eq!(parsed.port, Some(8443));
        assert_eq!(parsed.path, "/a/b");
        assert_eq!(parsed.query.as_deref(), Some("x=1"));
        assert_eq!(parsed.fragment.as_deref(), Some("frag"));
        let bare = parse_url("http://host").unwrap();
        assert_eq!(
            (bare.path.as_str(), bare.query, bare.port),
            ("", None, None)
        );
        assert_eq!(
            parse_url("https://[2001:db8::1]:9/x").unwrap().host,
            "2001:db8::1"
        );
    }

    #[test]
    fn redaction_keeps_scheme_and_host_and_hides_everything_else() {
        assert_eq!(
            redact("https://example.com/hook/abc?token=s3cret#f"),
            "https://example.com/•••"
        );
        assert_eq!(
            redact("https://user:pw@example.com:8443/"),
            "https://example.com:8443/"
        );
        assert_eq!(redact("http://localhost:8080"), "http://localhost:8080/");
        assert_eq!(
            redact("http://h/?token=x"),
            "http://h/•••",
            "a query alone is hidden"
        );
        assert_eq!(redact("https://[::1]:8443/x"), "https://[::1]:8443/•••");
        assert_eq!(redact("not a url"), "•••");
        assert_eq!(redact(""), "•••");
        for secret in ["s3cret", "pw", "token"] {
            assert!(!redact("https://user:pw@example.com/p?token=s3cret").contains(secret));
        }
    }

    #[test]
    fn header_names_follow_the_rfc_token_rule() {
        assert_eq!(validate_header_name("X-API-Key"), None);
        assert_eq!(validate_header_name("  Authorization "), None);
        assert_eq!(
            validate_header_name(""),
            Some(WebhookHeaderError::EmptyName)
        );
        assert_eq!(
            validate_header_name("Bad Name"),
            Some(WebhookHeaderError::InvalidName)
        );
        assert_eq!(
            validate_header_name("X:Y"),
            Some(WebhookHeaderError::InvalidName)
        );
        assert_eq!(
            validate_header_name("é"),
            Some(WebhookHeaderError::InvalidName)
        );
        assert_eq!(
            validate_header_name(&"a".repeat(65)),
            Some(WebhookHeaderError::NameTooLong)
        );
        assert_eq!(validate_header_name(&"a".repeat(64)), None);
        for reserved in [
            "Content-Type",
            "content-length",
            "HOST",
            "Transfer-Encoding",
            "Keep-Alive",
            "TE",
        ] {
            assert_eq!(
                validate_header_name(reserved),
                Some(WebhookHeaderError::Reserved),
                "{reserved}"
            );
        }
    }

    #[test]
    fn header_values_are_printable_ascii_without_line_breaks() {
        assert_eq!(validate_header_value("Bearer abc.def~1"), None);
        assert_eq!(validate_header_value("tab\tok"), None);
        for bad in ["a\r\nX-Evil: 1", "a\nb", "naïve", "emoji 🙂", "\u{7f}"] {
            assert_eq!(
                validate_header_value(bad),
                Some(WebhookHeaderError::InvalidValue),
                "{bad:?}"
            );
        }
        assert_eq!(
            validate_header_value(&"v".repeat(2049)),
            Some(WebhookHeaderError::ValueTooLong)
        );
        assert_eq!(validate_header_value(&"v".repeat(2048)), None);
    }

    #[test]
    fn header_sets_are_checked_for_duplicates_and_count() {
        let ok = vec![header("A", "1"), header("B", "2")];
        assert!(headers_are_valid(&ok));
        let dup = vec![header("X-Key", "1"), header("x-key ", "2")];
        let errors = header_errors(&dup);
        assert_eq!(errors, vec![(dup[1].id, WebhookHeaderError::Duplicate)]);
        assert!(!headers_are_valid(&dup));
        let eleven: Vec<WebhookHeader> = (0..11).map(|n| header(&format!("H{n}"), "v")).collect();
        assert!(!headers_are_valid(&eleven));
        assert!(headers_are_valid(&eleven[..10]));
        assert_eq!(header_errors(&[header("", "v")]).len(), 1);
    }

    #[test]
    fn secrets_have_their_own_limits() {
        assert_eq!(validate_secret(""), Some(WebhookSecretError::Empty));
        assert_eq!(
            validate_secret("a\nb"),
            Some(WebhookSecretError::ControlCharacters)
        );
        assert_eq!(
            validate_secret("a\u{0}b"),
            Some(WebhookSecretError::ControlCharacters)
        );
        assert_eq!(
            validate_secret(&"s".repeat(2049)),
            Some(WebhookSecretError::TooLong)
        );
        assert_eq!(validate_secret("ünïcode ok"), None);
        assert!(secret_message(WebhookSecretError::Empty).contains("Enter"));
        assert!(header_message(WebhookHeaderError::Reserved).contains("DegenView"));
    }

    #[test]
    fn credential_looking_headers_are_flagged() {
        for name in [
            "Authorization",
            "X-API-Key",
            "x-auth-token",
            "Proxy-Password",
        ] {
            assert!(looks_sensitive(name), "{name}");
        }
        assert!(!looks_sensitive("Content-Language"));
        assert!(!looks_sensitive("X-Trace"));
    }

    #[test]
    fn the_secret_token_forgives_spacing_and_case_but_nothing_else() {
        for ok in [
            "{{secret}}",
            "{{ secret }}",
            "{{Secret}}",
            "{{ SECRET\t}}",
            "a{{secret}}b",
        ] {
            assert!(contains_secret(ok), "{ok}");
        }
        for no in [
            "{secret}",
            "{{secrets}}",
            "{{ sec ret }}",
            "{{secret}",
            "secret",
            "{{}}",
            "{{ secre",
        ] {
            assert!(!contains_secret(no), "{no}");
        }
        // Overlapping braces still find the real token.
        assert!(contains_secret("{{{{secret}}"));
        assert_eq!(
            replace_secret("a={{secret}}&b={{ Secret }}", "X"),
            "a=X&b=X"
        );
        assert_eq!(replace_secret("{{{{secret}}", "X"), "{{X");
    }

    #[test]
    fn replacement_is_literal() {
        assert_eq!(
            replace_secret("{{secret}}", "$1\\0{{secret}}"),
            "$1\\0{{secret}}"
        );
        assert_eq!(replace_secret("no token", "x"), "no token");
        assert_eq!(replace_secret("é{{secret}}é", "S"), "éSé");
    }

    #[test]
    fn url_encoding_escapes_everything_but_the_unreserved_set() {
        assert_eq!(url_encode("AZaz09-._~"), "AZaz09-._~");
        assert_eq!(
            url_encode("a b/c?d&e=f#g+h%i"),
            "a%20b%2Fc%3Fd%26e%3Df%23g%2Bh%25i"
        );
        assert_eq!(url_encode("é"), "%C3%A9");
        assert_eq!(url_encode(""), "");
    }

    #[test]
    fn the_secret_may_not_sit_in_the_host_part() {
        for bad in [
            "https://{{secret}}.example.com/hook",
            "https://example.com:{{secret}}/hook",
            "https://user:{{secret}}@example.com/hook",
            "{{secret}}://example.com",
            "https://example.com{{secret}}",
        ] {
            assert!(secret_is_in_authority(bad), "{bad}");
            assert_eq!(
                validate_templates(bad, &[]),
                Some(WebhookDeliveryError::SecretInHost),
                "{bad}"
            );
        }
        for ok in [
            "https://example.com/{{secret}}",
            "https://example.com/hook?token={{secret}}",
            "https://example.com/#{{secret}}",
            "https://example.com?k={{secret}}",
        ] {
            assert!(!secret_is_in_authority(ok), "{ok}");
            assert_eq!(validate_templates(ok, &[]), None, "{ok}");
        }
    }

    #[test]
    fn templates_without_a_secret_resolve_as_written() {
        let resolved =
            resolve("https://example.com/hook", &[header(" X-A ", " 1 ")], None).unwrap();
        assert_eq!(resolved.url, "https://example.com/hook");
        assert_eq!(
            (
                resolved.headers[0].name.as_str(),
                resolved.headers[0].value.as_str()
            ),
            ("X-A", "1")
        );
        // An unused secret is ignored.
        assert!(resolve("https://example.com/hook", &[], Some("ignored")).is_ok());
    }

    #[test]
    fn the_secret_lands_encoded_in_the_url_and_raw_in_headers() {
        let resolved = resolve(
            "https://example.com/hook/{{secret}}?token={{ SECRET }}",
            &[
                header("Authorization", "Bearer {{secret}}"),
                header("X-Plain", "keep"),
            ],
            Some("a b/c&d"),
        )
        .unwrap();
        assert_eq!(
            resolved.url,
            "https://example.com/hook/a%20b%2Fc%26d?token=a%20b%2Fc%26d"
        );
        assert_eq!(resolved.headers[0].value, "Bearer a b/c&d");
        assert_eq!(resolved.headers[1].value, "keep");
    }

    #[test]
    fn a_missing_or_empty_secret_fails_only_when_something_uses_it() {
        for secret in [None, Some("")] {
            assert_eq!(
                resolve("https://example.com/?t={{secret}}", &[], secret),
                Err(WebhookDeliveryError::SecretUnavailable)
            );
            assert_eq!(
                resolve(
                    "https://example.com/",
                    &[header("X-K", "{{secret}}")],
                    secret
                ),
                Err(WebhookDeliveryError::SecretUnavailable)
            );
        }
        assert!(resolve("https://example.com/", &[], None).is_ok());
    }

    #[test]
    fn a_secret_cannot_smuggle_characters_into_a_header() {
        let headers = [header("X-Key", "{{secret}}")];
        assert_eq!(
            resolve("https://example.com/", &headers, Some("ünï")),
            Err(WebhookDeliveryError::InvalidHeader),
            "non-ASCII is fine in a URL (encoded) but not in a header"
        );
        assert_eq!(
            resolve("https://example.com/", &headers, Some("a\r\nX-Evil: 1")),
            Err(WebhookDeliveryError::InvalidHeader)
        );
        assert!(resolve("https://example.com/?k={{secret}}", &[], Some("ünï")).is_ok());
    }

    #[test]
    fn resolution_reports_template_problems_before_touching_the_secret() {
        assert_eq!(
            resolve("", &[], Some("s")),
            Err(WebhookDeliveryError::InvalidUrl)
        );
        assert_eq!(
            resolve("ftp://x.com", &[], Some("s")),
            Err(WebhookDeliveryError::UnsupportedScheme)
        );
        assert_eq!(
            resolve("https://x.com", &[header("Content-Type", "x")], Some("s")),
            Err(WebhookDeliveryError::InvalidHeader)
        );
        assert_eq!(
            resolve("https://{{secret}}.x.com", &[], Some("s")),
            Err(WebhookDeliveryError::SecretInHost)
        );
    }

    #[test]
    fn the_preview_masks_the_secret() {
        assert_eq!(
            masked_url(" https://x.com/h?t={{ secret }} "),
            "https://x.com/h?t=••••••"
        );
        assert!(references_secret("https://x.com/{{secret}}", &[]));
        assert!(references_secret(
            "https://x.com/",
            &[header("A", "{{Secret}}")]
        ));
        assert!(
            !references_secret("https://x.com/", &[header("{{secret}}", "v")]),
            "only values count"
        );
    }

    #[test]
    fn use_secret_shortcuts_are_idempotent() {
        let edit = apply_placement(
            &SecretPlacement::UrlParameter("token".into()),
            "https://x.com/h",
            &[],
        );
        assert_eq!(edit.url, "https://x.com/h?token={{secret}}");
        assert!(!edit.touches_headers);
        let again = apply_placement(
            &SecretPlacement::UrlParameter("token".into()),
            &edit.url,
            &[],
        );
        assert_eq!(again.url, edit.url);

        assert_eq!(
            adding_parameter("k", "https://x.com/h?a=1"),
            "https://x.com/h?a=1&k={{secret}}"
        );
        assert_eq!(
            adding_parameter("k", "https://x.com/h?"),
            "https://x.com/h?k={{secret}}"
        );
        assert_eq!(
            adding_parameter("k", "https://x.com/h?a=1&"),
            "https://x.com/h?a=1&k={{secret}}"
        );
        assert_eq!(
            adding_parameter("k", "https://x.com/h#frag"),
            "https://x.com/h?k={{secret}}#frag"
        );
        assert_eq!(adding_parameter("k", ""), "");

        let bearer = apply_placement(&SecretPlacement::BearerToken, "https://x.com", &[]);
        assert_eq!(bearer.headers.len(), 1);
        assert_eq!(bearer.headers[0].value, "Bearer {{secret}}");
        assert!(bearer.touches_headers);
        let twice = apply_placement(
            &SecretPlacement::BearerToken,
            "https://x.com",
            &bearer.headers,
        );
        assert_eq!(twice.headers, bearer.headers);
        // An existing header of that name (any case) is repointed rather than duplicated.
        let existing = [header("authorization", "Basic old")];
        let replaced = apply_placement(&SecretPlacement::BearerToken, "https://x.com", &existing);
        assert_eq!(replaced.headers.len(), 1);
        assert_eq!(replaced.headers[0].value, "Bearer {{secret}}");

        let key = apply_placement(&SecretPlacement::ApiKey, "https://x.com", &[]);
        assert_eq!(
            (key.headers[0].name.as_str(), key.headers[0].value.as_str()),
            ("X-API-Key", "{{secret}}")
        );

        let custom = apply_placement(&SecretPlacement::CustomHeader, "https://x.com", &[]);
        assert_eq!(custom.headers.len(), 1);
        assert!(custom.headers[0].name.is_empty());
        let custom_twice = apply_placement(
            &SecretPlacement::CustomHeader,
            "https://x.com",
            &custom.headers,
        );
        assert_eq!(custom_twice.headers.len(), 1);
    }

    #[test]
    fn plain_credentials_can_be_moved_into_the_secret() {
        let headers = [
            header("X-Trace", "abc"),
            header("Authorization", "Bearer abc123"),
            header("X-API-Key", "{{secret}}"),
        ];
        let found = plain_credential_header(&headers).unwrap();
        assert_eq!(found.name, "Authorization");
        assert_eq!(
            moving_to_secret(found),
            ("abc123".to_owned(), "Bearer {{secret}}".to_owned())
        );
        assert_eq!(
            moving_to_secret(&header("X-Token", "raw-value")),
            ("raw-value".to_owned(), "{{secret}}".to_owned())
        );
        assert_eq!(
            moving_to_secret(&header("Authorization", "basic dXNlcjpwdw==")),
            ("dXNlcjpwdw==".to_owned(), "Basic {{secret}}".to_owned())
        );
        assert!(plain_credential_header(&headers[..1]).is_none());
        assert!(plain_credential_header(&[header("Authorization", "  ")]).is_none());
    }

    #[test]
    fn the_body_is_the_message_and_only_the_content_type_depends_on_it() {
        let json = WebhookPayload::new(r#"{"event":"x"}"#);
        assert_eq!(json.content_type, JSON_CONTENT_TYPE);
        assert_eq!(json.body, br#"{"event":"x"}"#);
        assert_eq!(WebhookPayload::new("[1,2]").content_type, JSON_CONTENT_TYPE);
        assert_eq!(
            WebhookPayload::new("  {\"a\":1}\n").content_type,
            JSON_CONTENT_TYPE
        );
        for text in [
            "BUY",
            "123",
            "true",
            "null",
            "\"str\"",
            "BTC alert: 1",
            "{not json}",
            "{\"a\":1} trailing",
            "",
        ] {
            let payload = WebhookPayload::new(text);
            assert_eq!(payload.content_type, TEXT_CONTENT_TYPE, "{text:?}");
            assert_eq!(
                payload.body,
                text.as_bytes(),
                "byte for byte, no newline added"
            );
        }
    }

    #[test]
    fn messages_for_the_editor_never_echo_the_url() {
        assert!(url_message(WebhookDeliveryError::UnsupportedScheme).contains("http"));
        assert!(url_message(WebhookDeliveryError::UnsupportedPort).contains("65535"));
        assert!(url_message(WebhookDeliveryError::SecretInHost).contains("host"));
        assert!(url_message(WebhookDeliveryError::InvalidUrl).contains("https://example.com/hook"));
    }

    fn sendable_trigger(origin: AlertEventOrigin, timestamp_ms: i64) -> AlertTriggerEvent {
        use crate::model::alert::{
            AlertAsset, AlertCandleSnapshot, AlertCurrency, AlertDeliveryRecord,
        };
        use crate::model::{DataSource, InstrumentId};
        AlertTriggerEvent {
            id: 1,
            alert_id: 2,
            asset: AlertAsset::new(
                InstrumentId::new(DataSource::Binance, "BTCUSDT"),
                "BTC",
                "Bitcoin",
            ),
            observed_value: 70_050.0,
            target: 70_000.0,
            currency: AlertCurrency::Usd,
            timestamp_ms,
            quote_fingerprint: "f".into(),
            origin,
            delivery: AlertDeliveryRecord::default(),
            candle: Some(AlertCandleSnapshot {
                open_time_ms: 1_714_571_130_000,
                open: 69_900.0,
                high: 70_100.0,
                low: 69_800.0,
                close: 70_020.5,
                volume: 12.0,
                interval: "1h".into(),
            }),
        }
    }

    #[test]
    fn only_fresh_live_triggers_with_delivery_on_may_be_sent() {
        let now = 1_000_000;
        let live = sendable_trigger(AlertEventOrigin::Live, now - 59_000);
        assert!(price_event_is_sendable(&live, true, now));
        assert!(
            price_event_is_sendable(
                &sendable_trigger(AlertEventOrigin::Live, now - 60_000),
                true,
                now
            ),
            "exactly a minute old still goes"
        );
        assert!(!price_event_is_sendable(
            &sendable_trigger(AlertEventOrigin::Live, now - 60_001),
            true,
            now
        ));
        assert!(
            !price_event_is_sendable(&live, false, now),
            "delivery switched off"
        );
        let catch_up = sendable_trigger(AlertEventOrigin::CatchUp, now);
        assert!(
            !price_event_is_sendable(&catch_up, true, now),
            "the app was closed when it fired"
        );
        let future = sendable_trigger(AlertEventOrigin::Live, now + 5_000);
        assert!(
            price_event_is_sendable(&future, true, now),
            "clock skew is not staleness"
        );
    }
}
