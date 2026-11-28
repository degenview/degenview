//! How a venue asks the network: one GET, and what to do when it fails.

use crate::model::Listing;
use crate::service::ProviderError;
use crate::service::market::error::Statuses;
use crate::service::net::http::{Error, Retry};

/// A host with a schedule of its own: its requests keep a minimum gap, whoever asks.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Pace {
    CoinGecko,
    GeckoTerminal,
    Kraken,
}

/// A GET and the terms it is made on.
#[derive(Clone, Copy, Debug)]
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
