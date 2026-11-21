//! How Hyperliquid names a market, and the part of the name that tells where to ask for it.

use crate::model::{DataSource, InstrumentId};

/// The name the API takes for `symbol`: exactly as Hyperliquid lists it (`BTC`, `kPEPE`,
/// `xyz:SP500`, `PURR/USDC`, `@107`). Case matters, so nothing is folded.
pub(super) fn coin_id(symbol: &str) -> String {
    InstrumentId::new(DataSource::Hyperliquid, symbol).api_symbol()
}

/// Whether `id` names a spot pair: `PURR/USDC`, or `@107` for a pair Hyperliquid numbers instead of
/// naming. Everything else is a perpetual.
pub(super) fn is_spot(id: &str) -> bool {
    id.starts_with('@') || id.contains('/')
}

/// The dex a perpetual belongs to: `xyz` for `xyz:SP500`, none for a market of the main dex.
pub(super) fn dex_of(id: &str) -> Option<&str> {
    id.split_once(':')
        .map(|(dex, _)| dex)
        .filter(|dex| !dex.is_empty())
}
