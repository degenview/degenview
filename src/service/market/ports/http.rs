//! How a venue asks the network: one GET, and what to do when it fails.

use crate::model::Listing;
use crate::service::ProviderError;
use crate::service::market::error::Statuses;

/// Whether a failed request is asked once more.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Retry {
    /// The caller owns backoff, or another request would exceed a provider's limit.
    Never,
    /// One extra attempt for failures that may pass (see [`Error::is_transient`]).
    Transient,
}

/// Why a request got no body, as far as a venue tells its failures apart.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum Error {
    /// The host answered with this status.
    #[error("HTTP {0}")]
    Status(u16),
    /// The body held more than the request allowed.
    #[error("response exceeds {0} bytes")]
    BodyTooLarge(u64),
    /// No answer came: the connection failed, timed out, or broke while the body was read.
    #[error("{0}")]
    Network(String),
}

impl Error {
    /// A failure that asking again may not repeat: the connection failed, or the host said it was
    /// busy or broken (408, 425, 429, 5xx). A refusal of this request (4xx) or a body too big is
    /// not.
    pub fn is_transient(&self) -> bool {
        matches!(
            self,
            Self::Status(408 | 425 | 429 | 500..=599) | Self::Network(_)
        )
    }
}

/// A host with a schedule of its own: its requests keep a minimum gap, whoever asks.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Pace {
    CoinGecko,
    GeckoTerminal,
    Kraken,
}

/// A GET and the terms it is made on. Its `Debug` names the headers and leaves their values out:
/// they carry the user's keys.
#[derive(Clone, Copy)]
pub struct Request<'a> {
    pub url: &'a str,
    pub headers: &'a [(&'a str, &'a str)],
    pub retry: Retry,
    /// The most the body may hold; `None` is the transport's usual limit.
    pub max_body_bytes: Option<u64>,
    /// The host's schedule the request waits its turn on, before it is sent.
    pub pace: Option<Pace>,
    /// What the statuses of this kind of request mean to the venue beyond a rate limit: a status
    /// means what the endpoint makes of it (`ProviderError::http_with`).
    pub statuses: Statuses,
}

impl std::fmt::Debug for Request<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Request")
            .field("url", &self.url)
            .field(
                "headers",
                &self
                    .headers
                    .iter()
                    .map(|(name, _)| name)
                    .collect::<Vec<_>>(),
            )
            .field("retry", &self.retry)
            .field("max_body_bytes", &self.max_body_bytes)
            .field("pace", &self.pace)
            .field("statuses", &self.statuses)
            .finish()
    }
}

impl<'a> Request<'a> {
    /// A GET of `url` that is asked once more after a failure that may pass.
    pub fn get(url: &'a str) -> Self {
        Self {
            url,
            headers: &[],
            retry: Retry::Transient,
            max_body_bytes: None,
            pace: None,
            statuses: &[],
        }
    }

    pub fn headers(self, headers: &'a [(&'a str, &'a str)]) -> Self {
        Self { headers, ..self }
    }

    pub fn retry(self, retry: Retry) -> Self {
        Self { retry, ..self }
    }

    pub fn max_body_bytes(self, limit: u64) -> Self {
        Self {
            max_body_bytes: Some(limit),
            ..self
        }
    }

    pub fn paced(self, pace: Pace) -> Self {
        Self {
            pace: Some(pace),
            ..self
        }
    }

    /// The table that reads this request's statuses: one per kind of request a venue makes, since
    /// a 400 to a request that names a market says it is unknown, and to one that names none says
    /// nothing about any.
    pub fn statuses(self, statuses: Statuses) -> Self {
        Self { statuses, ..self }
    }
}

pub trait Http: Send + Sync {
    /// The body of a successful answer.
    fn get(&self, request: &Request<'_>) -> Result<String, Error>;
}

impl dyn Http + '_ {
    /// [`Http::get`] with a failure told as `venue`'s, its statuses read by the request's own table
    /// (`ProviderError::http_with`).
    pub fn fetch(
        &self,
        venue: impl Into<Listing>,
        request: &Request<'_>,
    ) -> Result<String, ProviderError> {
        self.get(request)
            .map_err(|error| ProviderError::http_with(venue, request.statuses, error))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_printed_for_debugging_names_its_headers_and_not_their_values() {
        let request = Request::get("https://example.test/bars")
            .headers(&[("APCA-API-SECRET-KEY", "s3cr3t-value")]);
        let printed = format!("{request:?} {request:#?}");
        assert!(printed.contains("APCA-API-SECRET-KEY"), "{printed}");
        assert!(!printed.contains("s3cr3t-value"), "{printed}");
    }

    #[test]
    fn only_a_failure_that_may_pass_is_transient() {
        for status in [408, 425, 429, 500, 503, 599] {
            assert!(Error::Status(status).is_transient(), "{status}");
        }
        assert!(Error::Network("reset".to_owned()).is_transient());
        for status in [400, 401, 403, 404, 410, 600] {
            assert!(!Error::Status(status).is_transient(), "{status}");
        }
        assert!(!Error::BodyTooLarge(1).is_transient());
    }

    #[test]
    fn a_failure_prints_what_happened_without_words_for_a_person() {
        assert_eq!(Error::Status(503).to_string(), "HTTP 503");
        assert_eq!(
            Error::BodyTooLarge(1_024).to_string(),
            "response exceeds 1024 bytes"
        );
        assert_eq!(Error::Network("reset".to_owned()).to_string(), "reset");
    }
}
