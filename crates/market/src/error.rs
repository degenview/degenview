use thiserror::Error;

/// Why an exchange's answer could not be read as candles.
#[derive(Debug, Error)]
pub enum ParseError {
    /// The text is not JSON.
    #[error("{0}")]
    Json(#[from] serde_json::Error),
    /// The JSON is not the shape the exchange documents.
    #[error("{0}")]
    Invalid(String),
    /// The exchange answered, but it has nothing for the request.
    #[error("{0}")]
    NoData(String),
    /// The exchange answered with its own error, in its own words.
    #[error("{exchange}: {message}")]
    Refused {
        exchange: &'static str,
        message: String,
        /// The error code says to slow down.
        rate_limited: bool,
    },
}

impl ParseError {
    pub(crate) fn invalid(message: impl Into<String>) -> Self {
        Self::Invalid(message.into())
    }

    /// Whether the exchange refused the request for asking too often.
    pub fn is_rate_limit(&self) -> bool {
        matches!(
            self,
            Self::Refused {
                rate_limited: true,
                ..
            }
        )
    }
}
