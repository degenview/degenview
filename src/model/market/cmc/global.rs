//! CoinMarketCap's crypto market-cap indices: `TOTAL`, `TOTAL2`, `TOTAL3` and `BTC.D`, derived from
//! one reading of the whole market (its global-metrics history). Pure: the service downloads the
//! readings, this decides what each index is.
//!
//! Market caps are in billions of US dollars and the dominance is a percent: a chart prints plain
//! numbers (`2,410` and `58.49`) and the symbol's title carries the unit.

use serde::{Deserialize, Serialize};

pub const TOTAL: &str = "TOTAL";
pub const TOTAL2: &str = "TOTAL2";
pub const TOTAL3: &str = "TOTAL3";
pub const BTC_DOMINANCE: &str = "BTC.D";

const BILLION: f64 = 1e9;

/// One reading of the whole crypto market.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub struct GlobalPoint {
    pub time_ms: i64,
    /// The market cap of every crypto asset, in dollars.
    pub total_cap: f64,
    /// The market cap of every asset but Bitcoin, in dollars.
    pub altcoin_cap: f64,
    /// Bitcoin's share of `total_cap`, in percent.
    pub btc_dominance: f64,
    /// Ether's share of `total_cap`, in percent; zero before Ether existed.
    pub eth_dominance: f64,
}

/// An index drawn from the readings.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GlobalIndex {
    Total,
    Total2,
    Total3,
    BtcDominance,
}

impl GlobalIndex {
    pub const ALL: [Self; 4] = [Self::Total, Self::Total2, Self::Total3, Self::BtcDominance];

    /// The index a chart symbol names, in any case.
    pub fn from_symbol(symbol: &str) -> Option<Self> {
        let symbol = symbol.trim();
        Self::ALL
            .into_iter()
            .find(|index| index.symbol().eq_ignore_ascii_case(symbol))
    }

    pub const fn symbol(self) -> &'static str {
        match self {
            Self::Total => TOTAL,
            Self::Total2 => TOTAL2,
            Self::Total3 => TOTAL3,
            Self::BtcDominance => BTC_DOMINANCE,
        }
    }

    /// The chart's title, unit included.
    pub const fn title(self) -> &'static str {
        match self {
            Self::Total => "TOTAL Cap (USD bn)",
            Self::Total2 => "TOTAL2 Cap (USD bn)",
            Self::Total3 => "TOTAL3 Cap (USD bn)",
            Self::BtcDominance => "BTC.D Dominance (%)",
        }
    }

    /// One line for the picker.
    pub const fn summary(self) -> &'static str {
        match self {
            Self::Total => "Whole crypto market cap",
            Self::Total2 => "Crypto market cap without Bitcoin",
            Self::Total3 => "Crypto market cap without Bitcoin and Ether",
            Self::BtcDominance => "Bitcoin's share of the crypto market cap",
        }
    }

    /// The index at `point`: billions of dollars, or a percent for the dominance.
    pub fn value(self, point: &GlobalPoint) -> f64 {
        match self {
            Self::Total => point.total_cap / BILLION,
            Self::Total2 => point.altcoin_cap / BILLION,
            Self::Total3 => {
                let share = 1.0 - (point.btc_dominance + point.eth_dominance) / 100.0;
                point.total_cap * share / BILLION
            }
            Self::BtcDominance => point.btc_dominance,
        }
    }

    /// The readings as the `(time, value)` samples candles are folded from.
    pub fn samples(self, points: &[GlobalPoint]) -> Vec<(i64, f64)> {
        points
            .iter()
            .map(|point| (point.time_ms, self.value(point)))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A reading from CoinMarketCap on 2026-11-21 07:10 UTC.
    const POINT: GlobalPoint = GlobalPoint {
        time_ms: 1_795_245_000_000,
        total_cap: 2_002_891_524_988.97,
        altcoin_cap: 831_413_723_206.62,
        btc_dominance: 58.4893,
        eth_dominance: 9.9848,
    };

    #[test]
    fn the_four_indices_are_derived_from_one_reading() {
        assert!((GlobalIndex::Total.value(&POINT) - 2_002.891_524_988_97).abs() < 1e-9);
        assert!((GlobalIndex::Total2.value(&POINT) - 831.413_723_206_62).abs() < 1e-9);
        assert!((GlobalIndex::BtcDominance.value(&POINT) - 58.4893).abs() < 1e-12);
        // 2,002.89 bn without Bitcoin's 58.4893 % and Ether's 9.9848 %.
        assert!((GlobalIndex::Total3.value(&POINT) - 631.43).abs() < 0.01);
    }

    #[test]
    fn the_altcoin_cap_is_the_total_without_bitcoins_share() {
        let without_bitcoin = POINT.total_cap * (1.0 - POINT.btc_dominance / 100.0);
        assert!((without_bitcoin / POINT.altcoin_cap - 1.0).abs() < 1e-5);
    }

    #[test]
    fn each_cap_is_larger_than_the_next() {
        let [total, total2, total3] =
            [GlobalIndex::Total, GlobalIndex::Total2, GlobalIndex::Total3]
                .map(|index| index.value(&POINT));
        assert!(total > total2 && total2 > total3 && total3 > 0.0);
    }

    #[test]
    fn before_ether_existed_the_third_index_is_the_second() {
        let early = GlobalPoint {
            eth_dominance: 0.0,
            ..POINT
        };
        let gap = GlobalIndex::Total3.value(&early) / GlobalIndex::Total2.value(&early) - 1.0;
        assert!(gap.abs() < 1e-5);
    }

    #[test]
    fn a_symbol_names_its_index_in_any_case() {
        for index in GlobalIndex::ALL {
            assert_eq!(GlobalIndex::from_symbol(index.symbol()), Some(index));
        }
        assert_eq!(
            GlobalIndex::from_symbol(" btc.d "),
            Some(GlobalIndex::BtcDominance)
        );
        assert_eq!(GlobalIndex::from_symbol("TOTAL4"), None);
        assert_eq!(GlobalIndex::from_symbol("cmc.fear_and_greed_latest"), None);
    }

    #[test]
    fn samples_keep_the_reading_times() {
        let later = GlobalPoint {
            time_ms: POINT.time_ms + 300_000,
            ..POINT
        };
        let samples = GlobalIndex::BtcDominance.samples(&[POINT, later]);
        assert_eq!(
            samples,
            vec![(POINT.time_ms, 58.4893), (later.time_ms, 58.4893)]
        );
    }
}
