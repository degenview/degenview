//! The request for one window of the global-metrics history.

use super::parse;
use crate::model::market::cmc::global::GlobalPoint;
use crate::model::market::cmc::interval::{Interval, Window};
use crate::service::ProviderError;
use crate::service::net::{http, rate_limit};

const URL: &str = "https://api.coinmarketcap.com/data-api/v3/global-metrics/quotes/historical";

pub(super) fn url(interval: Interval, window: &Window) -> String {
    let (from, to) = window.query_seconds();
    format!(
        "{URL}?format=chart_altcoin&interval={}&timeStart={from}&timeEnd={to}",
        interval.query()
    )
}

/// The readings of `window`. No key is sent or needed. It answers 403 to a request with no
/// `User-Agent` at all and any other value is accepted, so the shared agent's own is enough.
pub(super) fn fetch_window(
    interval: Interval,
    window: &Window,
) -> Result<Vec<GlobalPoint>, ProviderError> {
    rate_limit::cmc_global().wait_for_slot();
    let body = http::get_text(
        &url(interval, window),
        &[("Accept", "application/json")],
        http::Retry::Never,
    )
    .map_err(ProviderError::CoinMarketCap)?;
    parse::readings(&body)
}
