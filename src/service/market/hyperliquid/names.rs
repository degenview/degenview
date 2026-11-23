//! The names traders search the index perpetuals of trade.xyz by. Hyperliquid lists them as
//! `xyz:SP500` and `xyz:XYZ100`; "S&P", "SPX" or "Nasdaq" would find neither without help.
//!
//! The name is shown beside the market (`xyz:SP500 — S&P 500 (SPX)`) and searched with it, like an
//! equity's company, so the shared ranking applies and nothing is hidden: the other dexes' index
//! perpetuals are found by their own names.

/// `(id, name)`: what a market is also called.
const NAMES: [(&str, &str); 2] = [
    ("xyz:SP500", "S&P 500 (SPX)"),
    ("xyz:XYZ100", "Nasdaq 100 (NDX)"),
];

/// The name `id` is also known by, when it has one.
pub(super) fn name_of(id: &str) -> Option<&'static str> {
    NAMES
        .iter()
        .find(|(listed, _)| *listed == id)
        .map(|(_, name)| *name)
}
