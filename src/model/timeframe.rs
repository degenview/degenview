use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum Timeframe {
    Hour,
    Day,
    Week,
    Month,
    Quarter,
    Year,
}

impl Timeframe {
    pub const ALL: [Self; 6] = [
        Self::Hour,
        Self::Day,
        Self::Week,
        Self::Month,
        Self::Quarter,
        Self::Year,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Hour => "1H",
            Self::Day => "1D",
            Self::Week => "1W",
            Self::Month => "1M",
            Self::Quarter => "3M",
            Self::Year => "1Y",
        }
    }

    pub const fn description(self) -> &'static str {
        match self {
            Self::Hour => "One-hour candles",
            Self::Day => "Daily candles",
            Self::Week => "Weekly candles",
            Self::Month => "Monthly candles",
            Self::Quarter => "Quarterly candles",
            Self::Year => "Yearly candles",
        }
    }

    /// The interval token every provider is asked for. Quarters and years are real calendar
    /// buckets: a source with no such bar folds finer ones (`service::fold_calendar`).
    pub const fn interval(self) -> &'static str {
        match self {
            Self::Hour => "1h",
            Self::Day => "1d",
            Self::Week => "1w",
            Self::Month => "1M",
            Self::Quarter => "3M",
            Self::Year => "1Y",
        }
    }

    /// A bar's nominal length in milliseconds. Calendar timeframes are approximate (a month is 30
    /// days, a quarter 91, a year 365): it is only used where a bar's span is capped by the next
    /// bar's open anyway, such as aligning one market's candles to another's.
    pub const fn nominal_ms(self) -> i64 {
        const DAY: i64 = 86_400_000;
        match self {
            Self::Hour => 3_600_000,
            Self::Day => DAY,
            Self::Week => 7 * DAY,
            Self::Month => 30 * DAY,
            Self::Quarter => 91 * DAY,
            Self::Year => 365 * DAY,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_timeframe_has_its_own_interval_token() {
        let tokens: Vec<_> = Timeframe::ALL.iter().map(|tf| tf.interval()).collect();
        assert_eq!(tokens, ["1h", "1d", "1w", "1M", "3M", "1Y"]);
    }
}
