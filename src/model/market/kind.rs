//! How a market is traded: the one fact about a listing that the symbol alone does not tell.

use serde::{Deserialize, Serialize};

/// Whether a market settles the asset itself or tracks it with a perpetual contract. `BTC` on
/// Binance spot and `BTC` on Hyperliquid are different markets with the same name, so the picker
/// tags the second.
///
/// Spot is every market that is not a perpetual: coins and pairs, stocks, prediction markets and
/// index gauges. Not stored with a saved chart or watchlist row (their wire formats are unchanged):
/// the venue answers it from the id (`service::market::venue::Venue::kind_of`).
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum MarketKind {
    #[default]
    Spot,
    /// A perpetual contract: no expiry, priced to the spot market by a funding rate.
    Perp,
}

impl MarketKind {
    /// The short word a row's tag shows; nothing for spot, which is the unmarked case.
    pub const fn tag(self) -> Option<&'static str> {
        match self {
            Self::Spot => None,
            Self::Perp => Some("Perp"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spot_is_the_default_and_the_unmarked_case() {
        assert_eq!(MarketKind::default(), MarketKind::Spot);
        assert_eq!(MarketKind::Spot.tag(), None);
        assert_eq!(MarketKind::Perp.tag(), Some("Perp"));
    }

    #[test]
    fn the_wire_names_are_stable() {
        assert_eq!(
            serde_json::to_string(&MarketKind::Spot).unwrap(),
            "\"spot\""
        );
        assert_eq!(
            serde_json::to_string(&MarketKind::Perp).unwrap(),
            "\"perp\""
        );
        assert_eq!(
            serde_json::from_str::<MarketKind>("\"perp\"").unwrap(),
            MarketKind::Perp
        );
    }
}
