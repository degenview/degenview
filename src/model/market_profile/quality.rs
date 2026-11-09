//! Where a number on an order-flow chart came from.
//!
//! A footprint made from exchange trades, one made from a candle's taker-buy volume and one made
//! from nothing but a candle's colour are three different things, and the chart says which it is
//! showing. Nothing estimated is ever presented as exchange data, and a bar that cannot be built says
//! why instead of drawing something plausible.

/// The quality of one bar's (or session's) data, best first. A bar that cannot be built has no
/// quality: it carries an [`Unsupported`] reason instead.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DataQuality {
    /// Every trade of the bar, each with the exchange's aggressor side: exact per price.
    TradeDerived,
    /// Lower-timeframe candles with the exchange's own taker-buy volume: the buy/sell split of each
    /// candle is exact, which prices inside it traded is spread evenly (an estimate).
    ExchangeSplit,
    /// Lower-timeframe candles, each classed buy or sell by its own close versus open (TradingView's
    /// aggregated-intrabar rule), volume spread evenly over its range: an estimate.
    Intrabar,
    /// The chart's own candles only (no finer data): a coarse estimate.
    ChartBars,
}

/// Why a bar or chart cannot be built.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Unsupported {
    /// The market carries no volume (some providers send none).
    NoVolume,
    /// The chart's bar is too short to cut into anything finer.
    BarTooShort,
    /// The chart's bar is too long for this chart type.
    BarTooLong,
    /// The price step of the market is unknown and cannot be inferred.
    NoTickSize,
    /// The rows asked for would be more than the engine draws.
    TooManyRows,
}

impl DataQuality {
    /// Whether the numbers are what traded, not an estimate of it.
    pub fn is_exact(self) -> bool {
        matches!(self, Self::TradeDerived)
    }

    /// Whether per-price buy and sell volume is real, which is what imbalances compare. An even
    /// spread across a candle's range makes every row of a candle look alike, so imbalances on it
    /// would be artifacts and are not computed.
    pub fn has_real_rows(self) -> bool {
        matches!(self, Self::TradeDerived)
    }

    /// A short name for a status line or tooltip.
    pub fn label(self) -> &'static str {
        match self {
            Self::TradeDerived => "Trade data",
            Self::ExchangeSplit => "Exchange buy/sell split, prices estimated",
            Self::Intrabar => "Estimated from lower-timeframe candles",
            Self::ChartBars => "Estimated from chart candles",
        }
    }

    /// One or two sentences saying what the data is and is not.
    #[cfg(test)]
    pub fn explanation(self) -> &'static str {
        match self {
            Self::TradeDerived => {
                "Built from every trade, each with the exchange's own buyer or seller side."
            }
            Self::ExchangeSplit => {
                "Each candle's buy and sell volume comes from the exchange. Which prices inside the \
                 candle traded is spread evenly, so rows are an estimate."
            }
            Self::Intrabar => {
                "Built from finer candles: each counts as buying when it closed above its open, \
                 selling when below. Volume is spread evenly over each candle's range, so this is \
                 not the exchange's order flow."
            }
            Self::ChartBars => {
                "No finer data is available, so volume is spread over each chart candle's range."
            }
        }
    }
}

impl Unsupported {
    pub fn explanation(self) -> &'static str {
        match self {
            Self::NoVolume => "This market has no volume data.",
            Self::BarTooShort => "The chart's bars are too short to cut into finer data.",
            Self::BarTooLong => "The chart's bars are too long for this chart type.",
            Self::NoTickSize => "The price step of this market is not known.",
            Self::TooManyRows => "That row size would need too many rows. Use a larger row size.",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_trade_data_is_exact_and_has_real_rows() {
        for quality in [
            DataQuality::ExchangeSplit,
            DataQuality::Intrabar,
            DataQuality::ChartBars,
        ] {
            assert!(!quality.is_exact(), "{quality:?}");
            assert!(!quality.has_real_rows(), "{quality:?}");
        }
        assert!(DataQuality::TradeDerived.is_exact());
        assert!(DataQuality::TradeDerived.has_real_rows());
    }

    #[test]
    fn every_quality_explains_itself() {
        for quality in [
            DataQuality::TradeDerived,
            DataQuality::ExchangeSplit,
            DataQuality::Intrabar,
            DataQuality::ChartBars,
        ] {
            assert!(!quality.label().is_empty());
            assert!(quality.explanation().ends_with('.'), "{quality:?}");
        }
    }
}
