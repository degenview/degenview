//! `TOTAL`, `TOTAL2`, `TOTAL3` and `BTC.D`: CoinMarketCap's crypto market-cap indices, read from the
//! global-metrics history behind its website (`api.coinmarketcap.com/data-api`).
//!
//! That API is unofficial and undocumented, so it can change or go away, and it takes no key: these
//! charts never read the Pro API key the gauges use. Its readings are price samples, folded into
//! candles the way CoinGecko's are (`model::market::samples`): five-minute readings serve candles
//! of a whole number of five minutes, hourly readings whole hours, daily readings whole days, the
//! newest five-minute readings following the last of the coarser ones.
//! There is no older-history paging: a chart asks for as many candles as it shows.

mod parse;
mod rest;
mod windows;

use crate::clock::now_ms;
use crate::model::market::cmc::global::{GlobalIndex, GlobalPoint};
use crate::model::market::cmc::interval::{Interval, Window};
use crate::model::market::samples;
use crate::model::{DataSource, Kline, Timeframe};
use crate::service::ProviderError;
use crate::service::cache::lookup;

/// Candles of `index`, oldest first, at most `limit` of them.
pub(super) fn fetch(
    index: GlobalIndex,
    timeframe: Timeframe,
    limit: u32,
) -> Result<Vec<Kline>, ProviderError> {
    let now = now_ms();
    candles(index, timeframe, limit, now, |interval, windows| {
        let cache = lookup::shared();
        let points = windows::readings(cache, interval, windows, now, rest::fetch_window)?;
        Ok(with_newest(points, interval, || {
            windows::newest(cache, now, rest::fetch_window)
        }))
    })
}

/// The readings of a spacing coarser than five minutes trail the clock by up to a spacing: the
/// day's reading is taken hours into the day (stamped 10:10 UTC on 2026-11-18 to 20) and a day's
/// candle sits still from then until the next, today's missing until then. So the newest readings
/// at the finest spacing, those after the last one, follow them and the candle moves with the market.
fn with_newest(
    mut points: Vec<GlobalPoint>,
    interval: Interval,
    newest: impl FnOnce() -> Vec<GlobalPoint>,
) -> Vec<GlobalPoint> {
    if interval == Interval::Minutes5 {
        return points;
    }
    let after = points.last().map_or(i64::MIN, |point| point.time_ms);
    points.extend(newest().into_iter().filter(|point| point.time_ms > after));
    points
}

/// The candles, given a way to load the readings of some windows.
fn candles(
    index: GlobalIndex,
    timeframe: Timeframe,
    limit: u32,
    now_ms: i64,
    load: impl FnOnce(Interval, &[Window]) -> Result<Vec<GlobalPoint>, ProviderError>,
) -> Result<Vec<Kline>, ProviderError> {
    let interval = Interval::for_timeframe(timeframe)
        .ok_or_else(|| ProviderError::unsupported(DataSource::CoinMarketCap, timeframe))?;
    let from = now_ms - i64::from(limit) * timeframe.nominal_ms();
    let points = load(interval, &interval.windows(from, now_ms))?;
    let candles = samples::fold_samples(&index.samples(&points), timeframe);
    if candles.is_empty() {
        return Err(ProviderError::other("No CoinMarketCap data"));
    }
    let start = candles.len().saturating_sub(limit as usize);
    Ok(candles[start..].to_vec())
}

#[cfg(test)]
mod tests;
