use crate::service::market::ports::http::{Http, Pace, Request};
use crate::service::net::http::{self, Error};
use crate::service::net::rate_limit;

/// `service::net::http`: its agent, proxy setting, timeout, body limit and retry, and the hosts'
/// schedules in `service::net::rate_limit`.
pub struct NetHttp;

impl Http for NetHttp {
    fn get(&self, request: &Request<'_>) -> Result<String, Error> {
        if let Some(pace) = request.pace {
            match pace {
                Pace::CoinGecko => rate_limit::coingecko(),
                Pace::GeckoTerminal => rate_limit::geckoterminal(),
                Pace::Kraken => rate_limit::kraken(),
            }
            .wait_for_slot();
        }
        match request.max_body_bytes {
            Some(limit) => {
                http::get_text_with_limit(request.url, request.headers, request.retry, limit)
            }
            None => http::get_text(request.url, request.headers, request.retry),
        }
    }
}
