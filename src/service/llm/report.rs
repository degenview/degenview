//! A provider's failure as a person reads it: a headline, what the provider said, and the request
//! id a support desk asks for.
//!
//! It is built from the structure of an [`LlmError`] (the variant, the HTTP status, the body's
//! `error.message`, the request-id response header), never by reading the provider's prose: a
//! provider can reword its messages, translate them or add new kinds, and the report still holds.
//! The only thing done to the message is to cut it to a length a card can show.

use super::LlmError;

/// The most characters of the provider's message a report keeps.
const DETAIL_LIMIT: usize = 400;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ErrorReport {
    /// One line: what went wrong.
    pub headline: String,
    /// What the provider said. Empty when there is nothing beyond the headline.
    pub detail: String,
    /// The provider's id for the request, from its response header.
    pub request_id: Option<String>,
}

impl ErrorReport {
    /// A message of our own (not a provider's): all headline.
    pub fn message(text: impl Into<String>) -> Self {
        Self {
            headline: text.into(),
            ..Self::default()
        }
    }

    fn provider(headline: impl Into<String>, message: &str, request_id: Option<&str>) -> Self {
        Self {
            headline: headline.into(),
            detail: limited(&without_id_note(message.trim(), request_id)),
            request_id: request_id.map(str::to_owned),
        }
    }

    /// Whether there is more to show than the headline.
    pub fn has_more(&self) -> bool {
        !self.detail.is_empty() || self.request_id.is_some()
    }

    /// Everything, as text for the clipboard.
    pub fn copy_text(&self) -> String {
        let mut text = self.headline.clone();
        if !self.detail.is_empty() {
            text.push('\n');
            text.push_str(&self.detail);
        }
        if let Some(id) = &self.request_id {
            text.push_str("\nRequest ID: ");
            text.push_str(id);
        }
        text
    }
}

impl LlmError {
    /// This error as a report.
    pub fn report(&self) -> ErrorReport {
        match self {
            Self::Cancelled => ErrorReport::message("Cancelled"),
            Self::Auth {
                message,
                request_id,
            } => ErrorReport::provider("The API key was rejected", message, request_id.as_deref()),
            Self::RateLimited {
                message,
                request_id,
            } => ErrorReport::provider("Rate limited", message, request_id.as_deref()),
            // An error the provider sent inside an answer it had started.
            Self::Http {
                status: 200,
                message,
                request_id,
            } => ErrorReport::provider(
                "The provider reported an error",
                message,
                request_id.as_deref(),
            ),
            Self::Http {
                status,
                message,
                request_id,
            } => ErrorReport::provider(http_headline(*status), message, request_id.as_deref()),
            Self::Network(message) => {
                ErrorReport::provider("Could not reach the provider", message, None)
            }
            Self::Protocol(message) | Self::Config(message) => {
                ErrorReport::message(limited(message.trim()))
            }
        }
    }
}

fn http_headline(status: u16) -> String {
    match status {
        408 | 504 => format!("The provider timed out (HTTP {status})"),
        500..=599 => format!("The provider had a problem (HTTP {status})"),
        400..=499 => format!("The provider refused the request (HTTP {status})"),
        _ => format!("Unexpected answer from the provider (HTTP {status})"),
    }
}

/// Drops a closing parenthetical that holds `id` word for word ("(Please include the request ID
/// … in your email.)"): the report shows the id on a line of its own. Only an exact match of the id
/// we were given counts, and only when the parenthesis is balanced.
fn without_id_note(message: &str, id: Option<&str>) -> String {
    let Some(id) = id.filter(|id| !id.is_empty()) else {
        return message.to_owned();
    };
    if message.ends_with(')')
        && let Some(open) = message.rfind('(')
    {
        let note = &message[open..];
        let balanced = note.matches('(').count() == note.matches(')').count();
        if balanced && note.contains(id) {
            return message[..open].trim_end().to_owned();
        }
    }
    message.to_owned()
}

fn limited(text: &str) -> String {
    if text.chars().count() <= DETAIL_LIMIT {
        return text.to_owned();
    }
    let mut cut: String = text.chars().take(DETAIL_LIMIT).collect();
    cut.truncate(cut.trim_end().len());
    cut.push('…');
    cut
}

#[cfg(test)]
mod tests {
    use super::*;

    fn http(status: u16, message: &str, id: Option<&str>) -> LlmError {
        LlmError::Http {
            status,
            message: message.to_owned(),
            request_id: id.map(str::to_owned),
        }
    }

    #[test]
    fn the_headline_follows_the_error_kind_not_the_wording() {
        for (error, headline) in [
            (
                http(500, "x", None),
                "The provider had a problem (HTTP 500)",
            ),
            (http(503, "", None), "The provider had a problem (HTTP 503)"),
            (http(504, "", None), "The provider timed out (HTTP 504)"),
            (
                http(404, "", None),
                "The provider refused the request (HTTP 404)",
            ),
            (http(200, "", None), "The provider reported an error"),
            (LlmError::auth("anything"), "The API key was rejected"),
            (LlmError::rate_limited("anything"), "Rate limited"),
            (
                LlmError::Network("anything".into()),
                "Could not reach the provider",
            ),
            (LlmError::Config("Add your key.".into()), "Add your key."),
        ] {
            assert_eq!(error.report().headline, headline);
        }
        // The message is whatever the provider wrote, in any language: it is detail, untouched.
        let report = http(500, "Der Server hat einen Fehler gemeldet.", None).report();
        assert_eq!(report.detail, "Der Server hat einen Fehler gemeldet.");
    }

    #[test]
    fn the_request_id_is_the_one_given_and_its_note_is_dropped_only_on_an_exact_match() {
        let report = http(
            500,
            "It broke. (Please include the request ID abc-123 in your email.)",
            Some("abc-123"),
        )
        .report();
        assert_eq!(report.detail, "It broke.");
        assert_eq!(report.request_id.as_deref(), Some("abc-123"));
        // A different id in the prose is not ours: the message stays whole.
        let other = http(
            500,
            "It broke. (Please include the request ID zzz-999 in your email.)",
            Some("abc-123"),
        )
        .report();
        assert!(other.detail.contains("zzz-999"));
        // Without an id nothing is guessed from the text.
        let none = http(500, "It broke. (request ID zzz-999)", None).report();
        assert_eq!(none.request_id, None);
        assert!(none.detail.contains("zzz-999"));
    }

    #[test]
    fn the_detail_is_cut_on_a_character_boundary() {
        let huge = "é".repeat(5000);
        let report = http(502, &huge, None).report();
        assert_eq!(report.detail.chars().count(), DETAIL_LIMIT + 1);
        assert!(report.detail.ends_with('…'));
        let own = LlmError::Protocol("ü".repeat(900)).report();
        assert!(own.headline.chars().count() <= DETAIL_LIMIT + 1);
        assert_eq!(LlmError::Config(String::new()).report().headline, "");
    }

    #[test]
    fn copy_gives_everything() {
        let report = http(500, "Server said no.", Some("req_1")).report();
        assert_eq!(
            report.copy_text(),
            "The provider had a problem (HTTP 500)\nServer said no.\nRequest ID: req_1"
        );
        assert_eq!(ErrorReport::message("Only this.").copy_text(), "Only this.");
        assert!(!ErrorReport::message("Only this.").has_more());
    }
}
