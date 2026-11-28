use crate::service::market::ports::http::{Error, Http, Pace, Request, Retry};
use crate::service::net::http::{self, MAX_BODY_BYTES};
use crate::service::net::rate_limit::{self, RateLimiter};

/// `service::net::http`: its agent, proxy setting, timeout, body limit and retry, and the hosts'
/// schedules in `service::net::rate_limit`.
pub struct NetHttp;

impl Http for NetHttp {
    fn get(&self, request: &Request<'_>) -> Result<String, Error> {
        send(
            request,
            |pace| limiter(pace).wait_for_slot(),
            http::get_text_with_limit,
        )
    }
}

/// The schedule a host keeps for its requests, whoever asks.
pub(super) fn limiter(pace: Pace) -> &'static RateLimiter {
    match pace {
        Pace::CoinGecko => rate_limit::coingecko(),
        Pace::GeckoTerminal => rate_limit::geckoterminal(),
        Pace::Kraken => rate_limit::kraken(),
    }
}

/// What the port's `Retry` means to the transport.
fn retry(retry: Retry) -> http::Retry {
    match retry {
        Retry::Never => http::Retry::Never,
        Retry::Transient => http::Retry::Transient,
    }
}

/// The request's terms handed to the transport: the wait for its host's turn first, then the GET
/// under the body limit it asked for, or the transport's usual one.
pub(super) fn send(
    request: &Request<'_>,
    wait: impl FnOnce(Pace),
    get: impl FnOnce(&str, &[(&str, &str)], http::Retry, u64) -> Result<String, http::Error>,
) -> Result<String, Error> {
    if let Some(pace) = request.pace {
        wait(pace);
    }
    get(
        request.url,
        request.headers,
        retry(request.retry),
        request.max_body_bytes.unwrap_or(MAX_BODY_BYTES),
    )
    .map_err(Error::from)
}

/// The transport's failure as the port tells it. What counts as transient is told twice, here and
/// in the transport's own retry (`http::Error::is_transient`): a test keeps the two together.
impl From<http::Error> for Error {
    fn from(error: http::Error) -> Self {
        match error {
            http::Error::Status(code) => Self::Status(code),
            http::Error::BodyTooLarge(limit) => Self::BodyTooLarge(limit),
            http::Error::Network(detail) => Self::Network(detail),
        }
    }
}
