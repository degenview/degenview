//! TradingView's `EXCHANGE:TICKER` names and how each exchange DegenView serves writes the same
//! market. A pair is split only at a known quote currency; a venue no provider serves, and a
//! derivative, are reported as such and left for the user to map.

use crate::model::DataSource;
use crate::model::market::source::{split_binance_pair, split_mexc_pair};
use crate::model::market::venue;
use std::fmt;

/// What a chart, drawing or watchlist row names, upper-cased: usually `COINBASE:BTCUSD`. Anything
/// is kept, so that a formula or a name without an exchange holds its place and is reported.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct TvSymbol(String);

impl TvSymbol {
    /// `None` only for blank text.
    pub fn read(text: &str) -> Option<Self> {
        let text = text.trim();
        (!text.is_empty()).then(|| Self(text.to_uppercase()))
    }

    /// A chart whose symbol could not be read at all.
    pub fn unknown() -> Self {
        Self("?".to_owned())
    }

    /// A formula over markets (`A:B/C:D`, `1/FX:EURUSD`) or something else that is not one market.
    pub fn is_expression(&self) -> bool {
        self.0 == "?"
            || self
                .ticker()
                .contains([':', '+', '*', '(', ')', ' ', '|', ',', '{', '='])
            || self
                .exchange()
                .contains(['+', '*', '(', ')', ' ', '/', '{', '='])
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn exchange(&self) -> &str {
        self.0.split_once(':').map_or("", |(exchange, _)| exchange)
    }

    pub fn ticker(&self) -> &str {
        self.0.split_once(':').map_or(&self.0, |(_, ticker)| ticker)
    }

    /// The provider that serves this exchange, if one does.
    pub fn source(&self) -> Option<DataSource> {
        venue::source_for_prefix(self.exchange())
    }

    /// A perpetual or a dated future: no provider here lists derivatives.
    pub fn is_derivative(&self) -> bool {
        let ticker = self.ticker();
        if ticker.ends_with(".P") || ticker.ends_with("PERP") || ticker.ends_with('!') {
            return true;
        }
        // A dated contract ends in a month code and a year: `BTCUSDZ2026`.
        let year = ticker.len().saturating_sub(5);
        ticker.get(year..).is_some_and(|tail| {
            let mut chars = tail.chars();
            chars
                .next()
                .is_some_and(|month| "FGHJKMNQUVXZ".contains(month))
                && tail[1..].starts_with("20")
                && tail[1..].chars().all(|c| c.is_ascii_digit())
        })
    }

    /// The traded asset and what it is priced in. Where a quote ends the ticker depends on the
    /// exchange: Kraken's `XBTUSD` is XBT in dollars, not XB in TrueUSD, which only exchanges
    /// that list that coin are read with.
    pub fn pair(&self) -> Option<(&str, &str)> {
        if self.is_expression() {
            return None;
        }
        let ticker = self.ticker();
        let split = match self.source() {
            Some(DataSource::Binance | DataSource::Bybit) => split_binance_pair(ticker),
            Some(DataSource::Mexc) => split_mexc_pair(ticker),
            _ => QUOTES.iter().find_map(|quote| {
                let base = ticker.strip_suffix(quote)?;
                (!base.is_empty()).then_some((base, *quote))
            }),
        };
        let (base, quote) = split?;
        let base = base.trim_end_matches(['-', '/', '_']);
        (!base.is_empty()).then_some((base, quote))
    }
}

impl fmt::Display for TvSymbol {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// What pairs are priced in away from Binance, Bybit and MEXC, longest first.
const QUOTES: [&str; 14] = [
    "PYUSD", "USDT", "USDC", "USDE", "USD", "EUR", "GBP", "JPY", "BRL", "AUD", "CAD", "CHF", "BTC",
    "ETH",
];

/// The exchanges a pair missing from its own venue is looked for on, in order: the ones that
/// trade real currencies first for a pair priced in one.
pub fn substitutes(quote: &str) -> [DataSource; 7] {
    use DataSource::{Binance, Bybit, Coinbase, Gate, Kraken, Mexc, Okx};
    if matches!(quote, "USD" | "EUR" | "GBP" | "CHF" | "CAD" | "AUD" | "JPY") {
        [Coinbase, Kraken, Binance, Okx, Bybit, Gate, Mexc]
    } else {
        [Binance, Coinbase, Okx, Kraken, Bybit, Gate, Mexc]
    }
}

/// How `source` writes the pair `base`/`quote`; `None` for a provider that has no pair names.
pub fn pair_on(source: DataSource, base: &str, quote: &str) -> Option<String> {
    match source {
        DataSource::Binance | DataSource::Bybit | DataSource::Mexc => {
            Some(format!("{base}{quote}"))
        }
        DataSource::Coinbase | DataSource::Okx => Some(format!("{base}-{quote}")),
        DataSource::Gate => Some(format!("{base}_{quote}")),
        DataSource::Kraken => Some(format!("{}/{quote}", kraken_base(base))),
        _ => None,
    }
}

/// Kraken's old codes, which TradingView keeps for it.
fn kraken_base(base: &str) -> &str {
    match base {
        "XBT" => "BTC",
        "XDG" => "DOGE",
        other => other,
    }
}

/// The name `symbol`'s own provider would give the market, when it can be told from the ticker.
pub fn native_name(symbol: &TvSymbol) -> Option<(DataSource, String)> {
    if symbol.is_derivative() {
        return None;
    }
    let source = symbol.source()?;
    match source {
        DataSource::Alpaca => Some((source, symbol.ticker().to_owned())),
        _ => {
            let (base, quote) = symbol.pair()?;
            Some((source, pair_on(source, base, quote)?))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn symbol(text: &str) -> TvSymbol {
        TvSymbol::read(text).expect(text)
    }

    #[test]
    fn each_exchange_gets_the_pair_the_way_it_writes_it() {
        for (tv, source, name) in [
            ("BINANCE:BTCUSDT", DataSource::Binance, "BTCUSDT"),
            ("coinbase:btcusd", DataSource::Coinbase, "BTC-USD"),
            ("OKX:OMIUSDT", DataSource::Okx, "OMI-USDT"),
            ("KRAKEN:XBTUSD", DataSource::Kraken, "BTC/USD"),
            ("GATEIO:PEPEUSDT", DataSource::Gate, "PEPE_USDT"),
            ("BINANCE:BTCFDUSD", DataSource::Binance, "BTCFDUSD"),
            ("COINBASE:ENAUSD", DataSource::Coinbase, "ENA-USD"),
            ("NASDAQ:AAPL", DataSource::Alpaca, "AAPL"),
        ] {
            assert_eq!(
                native_name(&symbol(tv)),
                Some((source, name.to_owned())),
                "{tv}"
            );
        }
    }

    #[test]
    fn derivatives_and_unserved_venues_are_not_translated() {
        for text in ["BINANCE:BTCUSDT.P", "CME_MINI:ES1!", "BYBIT:BTCUSDZ2026"] {
            assert!(symbol(text).is_derivative(), "{text}");
            assert_eq!(native_name(&symbol(text)), None, "{text}");
        }
        assert_eq!(native_name(&symbol("BITSTAMP:ETHUSD")), None);
        assert_eq!(symbol("BITSTAMP:ETHUSD").pair(), Some(("ETH", "USD")));
    }

    #[test]
    fn formulas_are_kept_and_known_for_what_they_are() {
        for text in [
            "BINANCE:BTCUSDT/BINANCE:ETHUSDT",
            "A:B+C",
            "1/FX:EURUSD",
            "={\"symbol\":\"A:B\"}",
        ] {
            assert!(symbol(text).is_expression(), "{text}");
        }
        for text in [
            "BINANCE:BTCUSDT",
            "KRAKEN:XBT/USD",
            "CRYPTOCAP:BTC.D",
            "BTCUSD",
        ] {
            assert!(!symbol(text).is_expression(), "{text}");
        }
        assert_eq!(TvSymbol::read("  "), None);
    }

    #[test]
    fn a_ticker_that_is_only_a_quote_is_not_a_pair() {
        assert_eq!(symbol("NASDAQ:EUR").pair(), None);
        assert_eq!(symbol("CRYPTOCAP:BTC.D").pair(), None);
    }
}
