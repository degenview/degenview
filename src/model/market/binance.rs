//! How Binance's two markets share one namespace: `BTCUSDT` is the spot pair and `BTCUSDT.P` the
//! USDⓈ-M perpetual of it. The suffix is read and written here and nowhere else.

use crate::model::exchange::Market;

const PERPETUAL_SUFFIX: &str = ".P";

/// The pair `symbol` names, as Binance's APIs spell it when `symbol` is canonical, and which of
/// its two markets.
pub fn split_market(symbol: &str) -> (&str, Market) {
    let split = symbol.len().checked_sub(PERPETUAL_SUFFIX.len());
    match split.and_then(|at| symbol.split_at_checked(at)) {
        Some((pair, suffix)) if suffix.eq_ignore_ascii_case(PERPETUAL_SUFFIX) => {
            (pair, Market::Perpetual)
        }
        _ => (symbol, Market::Spot),
    }
}

/// The app's symbol for `pair` on `market`: the inverse of [`split_market`].
pub fn join_market(pair: &str, market: Market) -> String {
    match market {
        Market::Spot => pair.to_owned(),
        Market::Perpetual => format!("{pair}{PERPETUAL_SUFFIX}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_suffix_names_the_perpetual_and_round_trips() {
        for (symbol, pair, market) in [
            ("BTCUSDT", "BTCUSDT", Market::Spot),
            ("BTCUSDT.P", "BTCUSDT", Market::Perpetual),
            ("btc.p", "btc", Market::Perpetual),
            ("P", "P", Market::Spot),
            ("", "", Market::Spot),
            ("龙虾USDT.P", "龙虾USDT", Market::Perpetual),
            ("龙", "龙", Market::Spot),
        ] {
            assert_eq!(split_market(symbol), (pair, market), "{symbol}");
        }
        assert_eq!(join_market("BTCUSDT", Market::Perpetual), "BTCUSDT.P");
        assert_eq!(join_market("BTCUSDT", Market::Spot), "BTCUSDT");
    }
}
