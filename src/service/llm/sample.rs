//! Provider failures made up for tests and pictures. Providers word their errors as they like and
//! change them, so nothing here is a provider's real message and no test depends on one: a screen
//! has to cope with every *shape* of failure, and these are the shapes (short, long, one huge
//! word, many lines, non-ASCII, no request id).

use super::LlmError;

const WORDS: [&str; 13] = [
    "upstream", "service", "failed", "while", "handling", "the", "request", "please", "retry",
    "later", "or", "contact", "support",
];

/// `count` words, separated by spaces, with a full stop every ninth word.
pub fn prose(count: usize) -> String {
    let mut text = String::new();
    for index in 0..count {
        if index > 0 {
            text.push(' ');
        }
        text.push_str(WORDS[index % WORDS.len()]);
        if index % 9 == 8 {
            text.push('.');
        }
    }
    text
}

/// A request id, as providers print them.
pub fn request_id() -> String {
    "req_0a1b2c3d-4e5f-6071-8293-a4b5c6d7e8f9".to_owned()
}

/// A server error with a long message and a request id: the case that broke the layouts.
pub fn server_error() -> LlmError {
    let id = request_id();
    LlmError::http(500, format!("{} ({id} is the request id.)", prose(40)))
        .with_request_id(Some(id))
}

/// Every shape of failure a screen has to hold, by name.
pub fn shapes() -> Vec<(&'static str, LlmError)> {
    let id = Some(request_id());
    vec![
        (
            "short",
            LlmError::http(500, "boom").with_request_id(id.clone()),
        ),
        ("empty", LlmError::http(502, "")),
        ("long", server_error()),
        (
            "one-word",
            LlmError::http(500, "x".repeat(500)).with_request_id(id.clone()),
        ),
        (
            "many-lines",
            LlmError::http(
                500,
                (0..40)
                    .map(|n| format!("line {n}"))
                    .collect::<Vec<_>>()
                    .join("\n"),
            ),
        ),
        (
            "non-ascii",
            LlmError::rate_limited("é漢字🙂".repeat(300)).with_request_id(id.clone()),
        ),
        (
            "huge",
            LlmError::http(503, prose(20_000)).with_request_id(id),
        ),
        ("no-id", LlmError::auth(prose(60))),
        ("network", LlmError::Network(prose(30))),
        ("config", LlmError::Config(prose(30))),
    ]
}
