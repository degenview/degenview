//! A source and which of its markets: what decides the candle lengths, the stream and the request
//! a market is served by. Only Binance lists perpetuals; every other source is spot.

use crate::i18n::tr;
use crate::model::exchange::Market;
use crate::model::market::bucket::FoldPlan;
use crate::model::market::natives::BINANCE_PERPETUAL;
use crate::model::{DataSource, Timeframe};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Listing {
    pub source: DataSource,
    pub market: Market,
}

impl From<DataSource> for Listing {
    /// The source's spot markets.
    fn from(source: DataSource) -> Self {
        Self::new(source, Market::Spot)
    }
}

impl Listing {
    pub const BINANCE_PERPETUAL: Self = Self::new(DataSource::Binance, Market::Perpetual);

    pub const fn new(source: DataSource, market: Market) -> Self {
        Self { source, market }
    }

    /// The listings whose candles a socket follows.
    pub fn candle_streams() -> impl Iterator<Item = Self> {
        DataSource::CANDLE_STREAMS
            .into_iter()
            .map(Self::from)
            .chain([Self::BINANCE_PERPETUAL])
    }

    /// How a message names it: `Binance`, `Binance perp`.
    pub fn label(self) -> String {
        let source = self.source.display_name();
        match self.market {
            Market::Spot => source.to_owned(),
            Market::Perpetual => tr!("{exchange} perp", exchange = source),
        }
    }

    /// As [`DataSource::natives`].
    pub fn natives(self) -> &'static [Timeframe] {
        match (self.source, self.market) {
            (DataSource::Binance, Market::Perpetual) => &BINANCE_PERPETUAL,
            _ => self.source.natives(),
        }
    }

    /// As [`DataSource::plan`].
    pub fn plan(self, timeframe: Timeframe) -> Option<FoldPlan> {
        FoldPlan::in_grid(self.source.grid(), timeframe, self.natives())
    }

    /// As [`DataSource::serves`].
    pub fn serves(self, timeframe: Timeframe) -> bool {
        match (self.source, self.market) {
            (DataSource::Binance, Market::Perpetual) => self.plan(timeframe).is_some(),
            _ => self.source.serves(timeframe),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::InstrumentId;

    #[test]
    fn a_binance_perpetual_has_no_second_candles_and_spot_does() {
        let spot = InstrumentId::new(DataSource::Binance, "BTCUSDT").listing();
        let perp = InstrumentId::new(DataSource::Binance, "BTCUSDT.P").listing();
        assert_eq!(perp, Listing::BINANCE_PERPETUAL);
        assert!(spot.serves(Timeframe::SECOND_1));
        assert!(!perp.serves(Timeframe::SECOND_1));
        assert!(!perp.serves(Timeframe::parse("45s").unwrap()));
        let folded = Timeframe::parse("22m").unwrap();
        assert_eq!(perp.plan(folded), spot.plan(folded));
        assert_eq!(perp.label(), "Binance perp");
        assert_eq!(spot.label(), "Binance");
    }

    #[test]
    fn only_binance_reads_the_perpetual_suffix() {
        for source in DataSource::ALL {
            let listing = InstrumentId::new(source, "BTCUSDT.P").listing();
            let expected = match source {
                DataSource::Binance => Market::Perpetual,
                _ => Market::Spot,
            };
            assert_eq!(listing.market, expected, "{source:?}");
        }
    }
}
