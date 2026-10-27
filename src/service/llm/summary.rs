//! A provider's error as a person reads it: a short headline, the rest as detail, and the request
//! id a support desk asks for. Errors reach the UI as the text of [`super::LlmError`], and
//! providers write whole paragraphs into it, so the screens that show one split it here first
//! instead of printing it on one line.

/// The most characters of detail kept; the full text is still what Copy gives.
const DETAIL_LIMIT: usize = 400;
/// Text this short, in one sentence, is its own headline.
const SHORT: usize = 90;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ErrorSummary {
    /// One line: what went wrong.
    pub headline: String,
    /// What the provider said besides, without the request id. Empty when the headline says it all.
    pub detail: String,
    pub request_id: Option<String>,
}

/// Splits an error message into its parts. Never fails: text it does not recognise becomes a
/// headline (its first sentence) and detail (the rest).
pub fn summarize(text: &str) -> ErrorSummary {
    let text = text.trim();
    let request_id = request_id(text);
    let text = without_request_note(text, request_id.as_deref());
    let (headline, rest) = split_known(&text).unwrap_or_else(|| split_sentence(&text));
    ErrorSummary {
        headline,
        detail: limited(rest.trim()),
        request_id,
    }
}

/// The headline and the rest for the message shapes `LlmError` writes.
fn split_known(text: &str) -> Option<(String, String)> {
    if let Some(rest) = text.strip_prefix("HTTP ")
        && let Some((code, message)) = rest.split_once(':')
        && let Ok(status) = code.trim().parse::<u16>()
    {
        let headline = match status {
            504 => format!("The provider timed out (HTTP {status})"),
            500..=599 => format!("The provider had a problem (HTTP {status})"),
            408 => format!("The provider timed out (HTTP {status})"),
            400..=499 => format!("The provider refused the request (HTTP {status})"),
            _ => format!("Unexpected answer (HTTP {status})"),
        };
        return Some((headline, message.trim().to_owned()));
    }
    for (prefix, headline) in [
        (
            "The provider rejected the API key:",
            "The API key was rejected",
        ),
        ("Rate limited:", "Rate limited"),
        ("Network error:", "Could not reach the provider"),
    ] {
        if let Some(rest) = text.strip_prefix(prefix) {
            return Some((headline.to_owned(), rest.trim().to_owned()));
        }
    }
    None
}

/// The first sentence as the headline, unless the whole text is short enough to be one.
fn split_sentence(text: &str) -> (String, String) {
    if text.chars().count() <= SHORT && !text.trim_end_matches('.').contains(". ") {
        return (text.to_owned(), String::new());
    }
    let end = text
        .match_indices(['.', '!', '?'])
        .map(|(index, _)| index + 1)
        .find(|end| {
            text[*end..].starts_with(char::is_whitespace) && text[..*end].chars().count() <= SHORT
        });
    match end {
        Some(end) => (
            text[..end].trim_end_matches('.').to_owned(),
            text[end..].to_owned(),
        ),
        None => (
            truncated(text, SHORT),
            if text.chars().count() > SHORT {
                text.to_owned()
            } else {
                String::new()
            },
        ),
    }
}

/// The id after "request ID" (any case), as providers print it: letters, digits and dashes.
fn request_id(text: &str) -> Option<String> {
    let lower = text.to_ascii_lowercase();
    let at = lower.find("request id")? + "request id".len();
    let id: String = text[at..]
        .trim_start_matches([' ', ':'])
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
        .collect();
    (id.len() >= 8).then_some(id)
}

/// The text without a trailing "(Please include the request ID … in your email.)" note, which the
/// request id line says better. Only a closing parenthetical holding the id is removed.
fn without_request_note(text: &str, id: Option<&str>) -> String {
    let Some(id) = id else {
        return text.to_owned();
    };
    if text.ends_with(')')
        && let Some(open) = text.rfind('(')
        && text[open..].contains(id)
    {
        return text[..open].trim_end().to_owned();
    }
    text.to_owned()
}

fn limited(text: &str) -> String {
    truncated(text, DETAIL_LIMIT)
}

fn truncated(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_owned();
    }
    let mut cut: String = text.chars().take(limit).collect();
    cut.truncate(cut.trim_end().len());
    cut.push('…');
    cut
}

#[cfg(test)]
mod tests {
    use super::*;

    const OPENAI_500: &str = "HTTP 500: The server had an error processing your request. Sorry about that! You can retry your request, or contact us through our help center at help.openai.com if you keep seeing this error. (Please include the request ID f3762b57-a48f-45e4-bd18-d3a34c646c59 in your email.)";

    #[test]
    fn a_server_error_is_split_into_headline_detail_and_request_id() {
        let summary = summarize(OPENAI_500);
        assert_eq!(summary.headline, "The provider had a problem (HTTP 500)");
        assert_eq!(
            summary.request_id.as_deref(),
            Some("f3762b57-a48f-45e4-bd18-d3a34c646c59")
        );
        assert!(summary.detail.starts_with("The server had an error"));
        assert!(summary.detail.ends_with("seeing this error."));
        assert!(!summary.detail.contains("f3762b57"), "{}", summary.detail);
    }

    #[test]
    fn the_other_error_shapes_get_plain_headlines() {
        let key = summarize("The provider rejected the API key: Incorrect API key provided.");
        assert_eq!(key.headline, "The API key was rejected");
        assert_eq!(key.detail, "Incorrect API key provided.");
        let rate = summarize("Rate limited: you exceeded your current quota");
        assert_eq!(rate.headline, "Rate limited");
        let network = summarize("Network error: connection refused");
        assert_eq!(network.headline, "Could not reach the provider");
        assert_eq!(network.detail, "connection refused");
        assert_eq!(
            summarize("HTTP 404: no such model").headline,
            "The provider refused the request (HTTP 404)"
        );
    }

    #[test]
    fn a_short_message_is_its_own_headline() {
        let summary = summarize("Add your OpenAI API key in Settings ▸ AI.");
        assert_eq!(
            summary.headline,
            "Add your OpenAI API key in Settings ▸ AI."
        );
        assert_eq!(summary.detail, "");
        assert_eq!(summary.request_id, None);
    }

    #[test]
    fn a_long_unknown_message_is_cut_at_its_first_sentence() {
        let summary = summarize(
            "The model replied with something else. It sent a frame this app has not seen before, so the reply was dropped.",
        );
        assert_eq!(summary.headline, "The model replied with something else");
        assert!(summary.detail.starts_with("It sent a frame"));
    }

    #[test]
    fn a_huge_message_is_limited_and_never_panics() {
        let huge = format!("HTTP 502: {}", "é".repeat(5000));
        let summary = summarize(&huge);
        assert!(summary.detail.chars().count() <= DETAIL_LIMIT + 1);
        assert!(summary.detail.ends_with('…'));
        assert_eq!(summarize("").headline, "");
        let unbroken = summarize(&"x".repeat(300));
        assert!(unbroken.headline.chars().count() <= SHORT + 1);
    }

    #[test]
    fn an_id_that_is_too_short_to_be_one_is_ignored() {
        assert_eq!(request_id("see the request id: abc"), None);
        assert_eq!(
            request_id("Request ID: req_0123456789 failed").as_deref(),
            Some("req_0123456789")
        );
    }
}
