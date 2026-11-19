//! Which Alpaca data feed serves bars of a timeframe, and what a chart says about it.
//!
//! IEX is a single venue: real time, but its volume is 2-4% of the consolidated one, which skews
//! Volume, VWAP and every volume-based script. SIP carries every US venue, 15 minutes late on the
//! free (Basic) plan. The delay is invisible on a daily bar, so bars of a day and longer come from
//! SIP and intraday bars stay on IEX. The app does not know the account's plan.

use crate::model::{DataSource, Timeframe};

/// An Alpaca stock data feed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AlpacaFeed {
    Iex,
    Sip,
}

impl AlpacaFeed {
    /// The feed bars of `bar` (the length asked of Alpaca, not a folded one) come from.
    pub const fn for_bars(bar: Timeframe) -> Self {
        if bar.is_intraday() {
            Self::Iex
        } else {
            Self::Sip
        }
    }

    /// The `feed` query parameter.
    pub const fn query(self) -> &'static str {
        match self {
            Self::Iex => "iex",
            Self::Sip => "sip",
        }
    }
}

impl DataSource {
    /// How a chart of `timeframe` names where its bars come from: the exchange, and for Alpaca
    /// the feed that carries the timeframe. [`DataSource::display_name`] stays the venue's stable
    /// name; portfolio and paper keys are built from it.
    pub fn chart_feed_label(self, timeframe: Timeframe) -> &'static str {
        match (self, AlpacaFeed::for_bars(timeframe)) {
            (Self::Alpaca, AlpacaFeed::Sip) => "Alpaca (SIP · 15 min delay on Basic)",
            (source, _) => source.display_name(),
        }
    }

    /// Whether the last close of a chart of `timeframe` is a price of this moment on every
    /// account. False for Alpaca bars from SIP, which a Basic account gets 15 minutes late.
    pub fn bars_are_real_time(self, timeframe: Timeframe) -> bool {
        !(self == Self::Alpaca && AlpacaFeed::for_bars(timeframe) == AlpacaFeed::Sip)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn days_and_longer_come_from_sip_minutes_and_hours_from_iex() {
        for timeframe in [Timeframe::DAY_1, Timeframe::WEEK_1, Timeframe::MONTH_1] {
            assert_eq!(AlpacaFeed::for_bars(timeframe).query(), "sip");
        }
        for timeframe in [Timeframe::MINUTE_1, Timeframe::HOUR_1] {
            assert_eq!(AlpacaFeed::for_bars(timeframe).query(), "iex");
        }
    }

    #[test]
    fn a_charts_feed_is_the_feed_of_the_bars_it_is_folded_from() {
        let timeframes = Timeframe::PRESETS.into_iter().chain(
            ["2d", "3h", "22m", "24h", "3M", "1Y"].map(|text| Timeframe::parse(text).unwrap()),
        );
        for timeframe in timeframes {
            let Some(plan) = DataSource::Alpaca.plan(timeframe) else {
                continue;
            };
            assert_eq!(
                AlpacaFeed::for_bars(timeframe),
                AlpacaFeed::for_bars(plan.base()),
                "{timeframe}"
            );
        }
    }

    #[test]
    fn a_chart_names_the_feed_that_carries_its_timeframe() {
        assert_eq!(
            DataSource::Alpaca.chart_feed_label(Timeframe::HOUR_1),
            "Alpaca (IEX)"
        );
        assert_eq!(
            DataSource::Alpaca.chart_feed_label(Timeframe::DAY_1),
            "Alpaca (SIP · 15 min delay on Basic)"
        );
        assert_eq!(
            DataSource::Binance.chart_feed_label(Timeframe::DAY_1),
            "Binance"
        );
    }

    #[test]
    fn the_venue_name_stays_put_because_portfolio_and_paper_keys_use_it() {
        assert_eq!(DataSource::Alpaca.display_name(), "Alpaca (IEX)");
    }

    #[test]
    fn only_alpaca_sip_bars_may_be_late() {
        assert!(DataSource::Alpaca.bars_are_real_time(Timeframe::HOUR_1));
        assert!(!DataSource::Alpaca.bars_are_real_time(Timeframe::DAY_1));
        assert!(DataSource::Binance.bars_are_real_time(Timeframe::DAY_1));
    }
}
