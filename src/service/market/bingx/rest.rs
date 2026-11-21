//! BingX's requests over HTTP. Public endpoints, no key.

use crate::model::DataSource;
use crate::service::ProviderError;
use crate::service::net::http;

pub(super) const BASE_URL: &str = "https://open-api.bingx.com";

/// The answer to a GET of `path` (with its query), with the refusal of a country told apart from a
/// network failure.
pub(super) fn get(path: &str) -> Result<String, ProviderError> {
    match http::get_text(&format!("{BASE_URL}{path}"), &[], http::Retry::Transient) {
        Ok(body) => Ok(body),
        Err(http::Error::Status(403)) => Err(ProviderError::refused_by_region(DataSource::Bingx)),
        Err(error) => Err(error.into()),
    }
}
