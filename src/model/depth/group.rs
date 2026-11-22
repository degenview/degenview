//! How many ticks a ladder row spans: the choices a viewer has, and the one picked for them.

use super::Price;

/// How many ticks a row can span.
const MULTIPLES: [i64; 7] = [1, 5, 10, 50, 100, 500, 1_000];
/// The share of the mid price that [`automatic`] fits in [`AUTOMATIC_ROWS`] rows.
const AUTOMATIC_SHARE: f64 = 0.005;
/// The automatic group spreads half a percent of the mid over this many rows, whatever the
/// panel's height: it is the coarseness of the default, not the number of rows on screen.
const AUTOMATIC_ROWS: f64 = 40.0;

/// Every row size on offer for a market that moves in steps of `tick`, smallest first.
pub fn choices(tick: Price) -> Vec<Price> {
    MULTIPLES
        .iter()
        .map(|multiple| tick.times(*multiple))
        .collect()
}

/// The smallest of [`choices`] whose [`AUTOMATIC_ROWS`] rows cover about half a percent of `mid`;
/// the largest when none does.
pub fn automatic(tick: Price, mid: Price) -> Price {
    let wanted = mid.as_f64() * AUTOMATIC_SHARE / AUTOMATIC_ROWS;
    let all = choices(tick);
    all.iter()
        .copied()
        .find(|group| group.as_f64() >= wanted)
        .unwrap_or(all[all.len() - 1])
}
