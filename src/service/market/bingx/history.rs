//! BingX's candles and how a chart pages back through them.
//!
//! Two endpoints answer the same rows (`[open_ms, open, high, low, close, volume, close_ms,
//! quote_volume]`, newest first, `limit` at most 1,000, `endTime` the latest open time wanted,
//! inclusive), and neither alone serves a chart:
//!
//! - `/openApi/spot/v2/market/kline` is the recent one. It answers the forming candle, but only as
//!   far back as a short window of the current time: `endTime` older than 7 days for `1m`, 63 for
//!   `3m`, 105 for `5m`, 315 for `15m`, 630 for `30m`, 1,260 for `1h` and 2,520 for `2h` is refused
//!   (`maximum query range`; measured on 2026-11-21).
//! - `/openApi/market/his/v1/kline` is the history. It reaches a market's first candle (BTC-USDT
//!   from 2024-02-24 at `30m` and up) but not the last day: its newest candle is 24 hours old, and
//!   it answers `no data found` for an `endTime` newer than that. `1m` to `15m` begin on
//!   2025-01-01 (an `endTime` past 689 days is refused as `exceeds the valid period`). Its prices
//!   have a digit more than the tick (`precision.rs`).
//!
//! So a page that ends in the last 36 hours is the recent endpoint's and an older one the
//! history's; the two meet without a gap and the chart cannot tell them apart.
//!
//! **`limit` counts the slots of the length, not the rows**: BingX has no candle for the minutes
//! its own service was down (2025-11-25 04:03 to 04:12 UTC, in every market), so a page across them
//! holds fewer rows than it asked for, and a short page is not the start of the market. A market
//! nobody trades in is filled with flat candles. The start of a market is the page that is empty.

use super::markets::Pair;
use super::parse::parse_candles;
use super::precision::cut_candle;
use super::rest::get;
use crate::model::{DataSource, Kline, Timeframe};
use crate::service::ProviderError;
use crate::service::market::paging::Paging;
use std::time::Duration;

/// The most slots one request holds.
const PAGE: usize = 1_000;

/// A chart reaches back at most 30,000 base candles in one request (thirty pages); older ones come
/// as the chart scrolls to them. 500 requests are allowed every 10 seconds per endpoint (the
/// `x-ratelimit-requests-remain` header counts them down), so there is no lane.
pub(super) const PAGING: Paging = Paging {
    page: PAGE,
    max_candles: 30 * PAGE,
    pause: Duration::ZERO,
};

/// The most requests one page takes: one, and the ones that step over a hole in the market's
/// candles.
const REQUESTS_PER_PAGE: usize = 4;

/// How old a page's end must be for the history endpoint to have it: its newest candle is 24 hours
/// old, and the margin covers a refresh that lags.
const HISTORY_LAG_MS: i64 = 36 * 3_600_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Endpoint {
    Recent,
    History,
}

impl Endpoint {
    /// The endpoint that has candles up to `end_ms`, when it is `now_ms`.
    fn serving(end_ms: i64, now_ms: i64) -> Self {
        if end_ms <= now_ms - HISTORY_LAG_MS {
            Self::History
        } else {
            Self::Recent
        }
    }

    fn path(self) -> &'static str {
        match self {
            Self::Recent => "/openApi/spot/v2/market/kline",
            Self::History => "/openApi/market/his/v1/kline",
        }
    }
}

/// BingX's name for a candle length it serves itself (`1m`, `4h`).
pub(super) fn interval(timeframe: Timeframe) -> Option<String> {
    DataSource::Bingx
        .natives()
        .contains(&timeframe)
        .then(|| timeframe.interval())
}

/// The path that asks `endpoint` for the `slots` newest slots of `interval` that open at or before
/// `end_ms`.
fn candles_path(endpoint: Endpoint, id: &str, interval: &str, end_ms: i64, slots: usize) -> String {
    format!(
        "{}?symbol={id}&interval={interval}&limit={}&endTime={}",
        endpoint.path(),
        slots.clamp(1, PAGE),
        end_ms.max(0)
    )
}

/// The newest `wanted` candles of `base` that open before `after_ms`, oldest first; fewer only when
/// the market began later. `ask` answers a GET of the path it is given; `now_ms` says which
/// endpoint has the page.
pub(super) fn page_before_with(
    mut ask: impl FnMut(&str) -> Result<String, ProviderError>,
    now_ms: i64,
    market: &Pair,
    base: Timeframe,
    after_ms: i64,
    wanted: usize,
) -> Result<Vec<Kline>, ProviderError> {
    let interval =
        interval(base).ok_or_else(|| ProviderError::unsupported(DataSource::Bingx, base))?;
    let wanted = wanted.min(PAGE);
    let mut candles: Vec<Kline> = Vec::new();
    let mut before = after_ms;
    for _ in 0..REQUESTS_PER_PAGE {
        if candles.len() >= wanted {
            break;
        }
        let end_ms = before - 1;
        let endpoint = Endpoint::serving(end_ms, now_ms);
        // Asking for the whole page again, not what is missing, so that a hole at the edge of the
        // window does not look like the start of the market.
        let path = candles_path(endpoint, &market.id, &interval, end_ms, wanted);
        let page: Vec<Kline> = parse_candles(&ask(&path)?, base)?
            .into_iter()
            .filter(|candle| candle.open_time < before)
            .map(|candle| match endpoint {
                Endpoint::History => cut_candle(candle, market.decimals),
                Endpoint::Recent => candle,
            })
            .collect();
        let Some(oldest) = page.first().map(|candle| candle.open_time) else {
            break;
        };
        before = oldest;
        candles.splice(0..0, page);
    }
    if candles.len() > wanted {
        candles.drain(..candles.len() - wanted);
    }
    Ok(candles)
}

/// [`page_before_with`] over HTTP, now.
pub(super) fn page_before(
    market: &Pair,
    base: Timeframe,
    after_ms: i64,
    wanted: usize,
) -> Result<Vec<Kline>, ProviderError> {
    page_before_with(get, crate::clock::now_ms(), market, base, after_ms, wanted)
}
