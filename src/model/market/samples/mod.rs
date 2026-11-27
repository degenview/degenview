//! CoinGecko's price samples. It has no candles: `/market_chart` returns one price per sample, and
//! how far apart the samples are depends on how many days are asked for. A candle is built from the
//! samples that fall in its bucket, so they must be no coarser than the candle.

use crate::model::Timeframe;

const DAY_MS: i64 = 86_400_000;

/// `(most days asked, spacing of the samples in milliseconds)`: a day or less comes every five
/// minutes, up to 90 days every hour, and up to the free API's 365 days every day.
const TIERS: [(i64, i64); 3] = [(1, 300_000), (90, 3_600_000), (365, 86_400_000)];

/// How far apart the samples of a `/market_chart` request for `days` are, in milliseconds.
pub fn spacing_ms(days: i64) -> i64 {
    let [.., (_, coarsest)] = TIERS;
    TIERS
        .iter()
        .find(|(most, _)| days <= *most)
        .map_or(coarsest, |(_, spacing)| *spacing)
}

/// The days to ask `/market_chart` for to chart `limit` candles of `timeframe`: as many as the
/// candles cover, up to the deepest tier whose sample spacing divides the candle, or `None` when
/// none does (one-minute candles, and any length that is not a whole number of five minutes). A
/// source that cannot reach `limit` candles gives fewer; a shorter candle is never made.
pub fn chart_days(timeframe: Timeframe, limit: u32) -> Option<i64> {
    let length = timeframe.nominal_ms();
    let wanted = ((i64::from(limit) * length + DAY_MS - 1) / DAY_MS).max(1);
    TIERS
        .iter()
        .filter(|(_, spacing)| length % spacing == 0)
        .map(|(days, _)| *days)
        .max()
        .map(|deepest| wanted.min(deepest))
}

#[cfg(test)]
mod tests;
