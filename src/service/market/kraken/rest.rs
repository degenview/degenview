//! Kraken's candle requests over HTTP: one page, the paging around it.

use super::parse::parse_ohlc;
use super::{BASE_URL, interval, pair};
use crate::model::market::bucket::{FoldPlan, bucket_start_ms};
use crate::model::market::natives::KRAKEN_BARS;
use crate::model::{DataSource, Kline, Timeframe};
use crate::service::ProviderError;
use crate::service::net::{http, rate_limit};

/// The candles of `plan`'s target that open in `[start_ms, end_ms)` out of everything Kraken holds
/// of the base candle (`rows`), the newest `max`. A full window of `rows` was cut short of the
/// market's beginning, so its first fold is a fragment and goes, even when the range leaves fewer
/// candles than that: otherwise the page before what a chart shows would bring that fragment back
/// as a candle of its own.
pub(super) fn window(
    plan: FoldPlan,
    rows: Vec<Kline>,
    start_ms: i64,
    end_ms: i64,
    max: usize,
) -> Vec<Kline> {
    let cut = rows.len() >= KRAKEN_BARS;
    let requested = plan.base_count(max).min(KRAKEN_BARS);
    let mut candles = rows;
    candles.retain(|candle| candle.open_time >= start_ms && candle.open_time < end_ms);
    let candles = newest(candles, requested);
    let first_open = candles.first().map(|candle| candle.open_time);
    let mut shown = plan.fold(candles, cut);
    // A fold keeps a lone bucket, whatever it is: a lone fragment is not a candle.
    if cut
        && shown.len() == 1
        && first_open.is_some_and(|open| open > bucket_start_ms(plan.target(), open))
    {
        shown.clear();
    }
    let skip = shown.len().saturating_sub(max);
    shown.drain(..skip);
    shown
}

/// `text` with Kraken's legacy asset names in the common ones.
pub(super) fn alias(text: &str) -> String {
    text.replace("XBT", "BTC").replace("XDG", "DOGE")
}

/// The last `count` candles.
pub(super) fn newest(mut candles: Vec<Kline>, count: usize) -> Vec<Kline> {
    if candles.len() > count {
        candles.drain(..candles.len() - count);
    }
    candles
}

/// The candles Kraken holds for `symbol`, oldest first. `recent` asks only for that many of the
/// newest (and a little more), which keeps a tail refresh small; `None` asks for all 720.
pub(super) fn fetch_ohlc(
    symbol: &str,
    base: Timeframe,
    recent: Option<usize>,
) -> Result<Vec<Kline>, ProviderError> {
    let minutes =
        interval(base).ok_or_else(|| ProviderError::unsupported(DataSource::Kraken, base))?;
    let mut url = format!("{BASE_URL}/OHLC?pair={}&interval={minutes}", pair(symbol));
    if let Some(count) = recent.filter(|count| *count < KRAKEN_BARS) {
        let since = crate::clock::now_ms() / 1_000 - (count as i64 + 1) * i64::from(minutes) * 60;
        url.push_str(&format!("&since={since}"));
    }
    parse_ohlc(&get(&url)?, base)
}

/// A request on Kraken's one-a-second schedule.
pub(super) fn get(url: &str) -> Result<String, ProviderError> {
    rate_limit::kraken().wait_for_slot();
    Ok(http::get_text(url, &[], http::Retry::Transient)?)
}
