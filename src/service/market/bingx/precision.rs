//! The prices of a market at the precision BingX quotes them in.
//!
//! BingX's history endpoint and its candle socket keep one digit more than the market's tick: a
//! bar of BTC-USDT (tick 0.01) opens at `64655.599`, and the float noise of that digit shows as
//! `61702.823000000004`. Its recent-candles endpoint, and so the price everyone sees, is the same
//! number cut off at the tick (`64655.59`, never rounded up: measured on 2026-11-21 over 4,000
//! prices of each of five markets, the cut matched every one and rounding only half). So a price is
//! cut at the tick as well, which makes the two endpoints, and the candle the socket pushes, one
//! bar.

use crate::model::Kline;

/// The digits a market's prices have after the decimal point, from its tick (`0.01` is 2, `1e-8` is
/// 8, `1` is 0). `None` for a tick that is not a positive number.
pub(super) fn decimals_of_tick(tick: f64) -> Option<u32> {
    (tick.is_finite() && tick > 0.0).then(|| (-tick.log10()).round().max(0.0) as u32)
}

/// `price` cut at `decimals` digits, as BingX cuts it: the decimal text of the number is shortened,
/// so `64441.509999999995` is `64441.5` (the double is a hair under `64441.51`, and the recent
/// endpoint says `64441.50`), not the `64441.51` that arithmetic with an allowance would give. This
/// takes the number exactly as the answer's text says it, which is why `serde_json` is built with
/// `float_roundtrip`.
pub(super) fn cut(price: f64, decimals: u32) -> f64 {
    let text = price.to_string();
    let Some((whole, fraction)) = text.split_once('.') else {
        return price;
    };
    let kept = &fraction[..fraction.len().min(decimals as usize)];
    format!("{whole}.{kept}0").parse().unwrap_or(price)
}

/// `candle` with its four prices cut at `decimals`; the volume is untouched.
pub(super) fn cut_candle(mut candle: Kline, decimals: u32) -> Kline {
    candle.open = cut(candle.open, decimals);
    candle.high = cut(candle.high, decimals);
    candle.low = cut(candle.low, decimals);
    candle.close = cut(candle.close, decimals);
    candle
}
