//! A candle length: a count of minutes, hours, days, weeks or months.
//!
//! `Timeframe` is a value, not a closed list: `22m`, `3h` and `2D` are as real as `1D`. The
//! toolbar still offers [`Timeframe::LEGACY`]; [`Timeframe::PRESETS`] are the lengths providers
//! serve natively. Where a candle opens and closes is `model::market::bucket`'s business.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::cmp::Ordering;
use std::fmt;
use thiserror::Error;

const MINUTE_MS: i64 = 60_000;
const HOUR_MS: i64 = 3_600_000;
const DAY_MS: i64 = 86_400_000;

/// What a [`Timeframe`]'s count counts. Seconds are not offered.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum TimeUnit {
    Minute,
    Hour,
    Day,
    Week,
    Month,
}

impl TimeUnit {
    /// The largest count a timeframe of this unit takes.
    pub const fn max_count(self) -> u16 {
        match self {
            Self::Minute => 1_440,
            Self::Hour => 24,
            Self::Day => 365,
            Self::Week => 52,
            Self::Month => 12,
        }
    }

    /// The unit's name for a list of them, as in "Minutes go from 1 to 1440".
    pub const fn plural(self) -> &'static str {
        match self {
            Self::Minute => "Minutes",
            Self::Hour => "Hours",
            Self::Day => "Days",
            Self::Week => "Weeks",
            Self::Month => "Months",
        }
    }

    /// The unit's name for one candle, as in "22-minute candles".
    const fn singular(self) -> &'static str {
        match self {
            Self::Minute => "minute",
            Self::Hour => "hour",
            Self::Day => "day",
            Self::Week => "week",
            Self::Month => "month",
        }
    }
}

/// The group a timeframe is listed under in a picker.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Section {
    Minutes,
    Hours,
    DaysAndUp,
}

/// Why a count and unit, or some text, is not a timeframe. The messages are shown as they are.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum TimeframeParseError {
    #[error("Type a number and a unit: 22m, 3h, 2D, 1W, 3M")]
    Malformed,
    #[error("A timeframe can't be zero")]
    Zero,
    #[error("Seconds aren't supported")]
    SecondsUnsupported,
    #[error("{} go from 1 to {max}", .unit.plural())]
    OutOfRange { unit: TimeUnit, max: u16 },
    #[error("Only 1Y is a year; use up to 12M for anything shorter")]
    YearsUnsupported,
}

/// The length of one candle. Equal lengths are equal values: `60m` is `1h`, and `12M` is the year
/// (`1Y`); `24h` stays apart from `1D`, which is a calendar day.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Timeframe {
    count: u16,
    unit: TimeUnit,
}

impl Timeframe {
    pub const MINUTE_1: Self = Self::preset(1, TimeUnit::Minute);
    pub const MINUTE_5: Self = Self::preset(5, TimeUnit::Minute);
    pub const MINUTE_15: Self = Self::preset(15, TimeUnit::Minute);
    pub const MINUTE_30: Self = Self::preset(30, TimeUnit::Minute);
    pub const HOUR_1: Self = Self::preset(1, TimeUnit::Hour);
    pub const HOUR_4: Self = Self::preset(4, TimeUnit::Hour);
    pub const DAY_1: Self = Self::preset(1, TimeUnit::Day);
    pub const WEEK_1: Self = Self::preset(1, TimeUnit::Week);
    pub const MONTH_1: Self = Self::preset(1, TimeUnit::Month);
    pub const MONTH_3: Self = Self::preset(3, TimeUnit::Month);
    pub const YEAR_1: Self = Self::preset(12, TimeUnit::Month);

    /// The lengths the providers serve, shortest first.
    pub const PRESETS: [Self; 11] = [
        Self::MINUTE_1,
        Self::MINUTE_5,
        Self::MINUTE_15,
        Self::MINUTE_30,
        Self::HOUR_1,
        Self::HOUR_4,
        Self::DAY_1,
        Self::WEEK_1,
        Self::MONTH_1,
        Self::MONTH_3,
        Self::YEAR_1,
    ];

    /// The six lengths the toolbar, Pine Studio and drawing visibility have always offered; the
    /// ones saved data names with a variant name (see the serde impls).
    pub const LEGACY: [Self; 6] = [
        Self::HOUR_1,
        Self::DAY_1,
        Self::WEEK_1,
        Self::MONTH_1,
        Self::MONTH_3,
        Self::YEAR_1,
    ];

    /// A timeframe of `count` `unit`s, in its one canonical form: minutes that make whole hours
    /// (up to 24) become hours.
    pub const fn new(count: u16, unit: TimeUnit) -> Result<Self, TimeframeParseError> {
        if count == 0 {
            return Err(TimeframeParseError::Zero);
        }
        let max = unit.max_count();
        if count > max {
            return Err(TimeframeParseError::OutOfRange { unit, max });
        }
        if matches!(unit, TimeUnit::Minute) && count.is_multiple_of(60) {
            return Ok(Self {
                count: count / 60,
                unit: TimeUnit::Hour,
            });
        }
        Ok(Self { count, unit })
    }

    /// A compile-time `new` for the presets: a bad one fails the build.
    const fn preset(count: u16, unit: TimeUnit) -> Self {
        match Self::new(count, unit) {
            Ok(timeframe) => timeframe,
            Err(_) => panic!("a preset timeframe is out of range"),
        }
    }

    pub const fn count(self) -> u16 {
        self.count
    }

    pub const fn unit(self) -> TimeUnit {
        self.unit
    }

    pub const fn section(self) -> Section {
        match self.unit {
            TimeUnit::Minute => Section::Minutes,
            TimeUnit::Hour => Section::Hours,
            TimeUnit::Day | TimeUnit::Week | TimeUnit::Month => Section::DaysAndUp,
        }
    }

    /// Minutes and hours: the candles of one trading day.
    pub const fn is_intraday(self) -> bool {
        matches!(self.unit, TimeUnit::Minute | TimeUnit::Hour)
    }

    /// Whether the timeframe is a calendar year: `1Y` (`12M`).
    pub const fn is_year(self) -> bool {
        matches!(self.unit, TimeUnit::Month) && self.count == 12
    }

    /// Reads `22m`, `3h`, `2D`, `1W`, `3M` and `1Y`: digits, then an optional unit (spaces around
    /// and between are fine). With no unit the count is minutes, as on TradingView. `m` is minutes
    /// and `M` months; the other units take either case. Also reads the interval tokens providers
    /// take (`1h`, `1d`, `1w`, `1M`, `15m`).
    pub fn parse(text: &str) -> Result<Self, TimeframeParseError> {
        let text = text.trim();
        let digits_end = text
            .find(|character: char| !character.is_ascii_digit())
            .unwrap_or(text.len());
        let (digits, rest) = text.split_at(digits_end);
        if digits.is_empty() {
            return Err(TimeframeParseError::Malformed);
        }
        // A count too long for u16 is out of range for any unit.
        let count = digits.parse::<u16>().unwrap_or(u16::MAX);
        let mut letters = rest.trim_start().chars();
        let unit = letters.next();
        if letters.next().is_some() {
            return Err(TimeframeParseError::Malformed);
        }
        match unit {
            Some('s' | 'S') => Err(TimeframeParseError::SecondsUnsupported),
            None | Some('m') => Self::new(count, TimeUnit::Minute),
            Some('h' | 'H') => Self::new(count, TimeUnit::Hour),
            Some('d' | 'D') => Self::new(count, TimeUnit::Day),
            Some('w' | 'W') => Self::new(count, TimeUnit::Week),
            Some('M') => Self::new(count, TimeUnit::Month),
            Some('y' | 'Y') => match count {
                0 => Err(TimeframeParseError::Zero),
                1 => Ok(Self::YEAR_1),
                _ => Err(TimeframeParseError::YearsUnsupported),
            },
            Some(_) => Err(TimeframeParseError::Malformed),
        }
    }

    /// The text shown for the timeframe: `22m`, `3h`, `2D`, `1W`, `3M`, `1Y`.
    pub fn label(self) -> String {
        self.to_string()
    }

    /// A sentence-case name for tooltips: "Daily candles", "22-minute candles".
    pub fn description(self) -> String {
        match (self.unit, self.count) {
            (TimeUnit::Minute | TimeUnit::Hour, 1) => {
                format!("One-{} candles", self.unit.singular())
            }
            (TimeUnit::Day, 1) => "Daily candles".to_owned(),
            (TimeUnit::Week, 1) => "Weekly candles".to_owned(),
            (TimeUnit::Month, 1) => "Monthly candles".to_owned(),
            (TimeUnit::Month, 3) => "Quarterly candles".to_owned(),
            (TimeUnit::Month, 12) => "Yearly candles".to_owned(),
            (unit, count) => format!("{count}-{} candles", unit.singular()),
        }
    }

    /// The interval token every provider is asked for: `22m`, `3h`, `2d`, `1w`, `3M`, `1Y`. A
    /// source with no such candle folds finer ones (`model::market::bucket::fold`).
    pub fn interval(self) -> String {
        let count = self.count;
        match self.unit {
            TimeUnit::Minute => format!("{count}m"),
            TimeUnit::Hour => format!("{count}h"),
            TimeUnit::Day => format!("{count}d"),
            TimeUnit::Week => format!("{count}w"),
            TimeUnit::Month if self.is_year() => "1Y".to_owned(),
            TimeUnit::Month => format!("{count}M"),
        }
    }

    /// A candle's nominal length in milliseconds. Minutes to weeks are exact. Calendar months are
    /// approximate (30 days each, except a quarter at 91 and a year at 365): it is only used where a
    /// candle's span is capped by the next candle's open anyway, such as aligning one market's
    /// candles to another's.
    pub const fn nominal_ms(self) -> i64 {
        let count = self.count as i64;
        match self.unit {
            TimeUnit::Minute => count * MINUTE_MS,
            TimeUnit::Hour => count * HOUR_MS,
            TimeUnit::Day => count * DAY_MS,
            TimeUnit::Week => count * 7 * DAY_MS,
            TimeUnit::Month if self.count == 3 => 91 * DAY_MS,
            TimeUnit::Month if self.count == 12 => 365 * DAY_MS,
            TimeUnit::Month => count * 30 * DAY_MS,
        }
    }

    /// The name older builds saved a [`Timeframe::LEGACY`] value under.
    fn legacy_name(self) -> Option<&'static str> {
        Some(match self {
            Self::HOUR_1 => "Hour",
            Self::DAY_1 => "Day",
            Self::WEEK_1 => "Week",
            Self::MONTH_1 => "Month",
            Self::MONTH_3 => "Quarter",
            Self::YEAR_1 => "Year",
            _ => return None,
        })
    }

    fn from_legacy_name(name: &str) -> Option<Self> {
        Self::LEGACY
            .into_iter()
            .find(|timeframe| timeframe.legacy_name() == Some(name))
    }
}

impl fmt::Display for Timeframe {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let count = self.count;
        match self.unit {
            TimeUnit::Minute => write!(formatter, "{count}m"),
            TimeUnit::Hour => write!(formatter, "{count}h"),
            TimeUnit::Day => write!(formatter, "{count}D"),
            TimeUnit::Week => write!(formatter, "{count}W"),
            TimeUnit::Month if self.is_year() => formatter.write_str("1Y"),
            TimeUnit::Month => write!(formatter, "{count}M"),
        }
    }
}

/// Shorter candles first; two lengths of one duration (`24h`, `1D`) by unit.
impl Ord for Timeframe {
    fn cmp(&self, other: &Self) -> Ordering {
        self.nominal_ms()
            .cmp(&other.nominal_ms())
            .then(self.unit.cmp(&other.unit))
    }
}

impl PartialOrd for Timeframe {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Saved data wrote this as an enum: the six [`Timeframe::LEGACY`] lengths keep their variant
/// names (`"Hour"` … `"Year"`) so an older build, or a synced layout read by one, still decodes
/// them. Any other length is its [`Display`](fmt::Display) text (`"22m"`).
impl Serialize for Timeframe {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self.legacy_name() {
            Some(name) => serializer.serialize_str(name),
            None => serializer.collect_str(self),
        }
    }
}

impl<'de> Deserialize<'de> for Timeframe {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        match Self::from_legacy_name(&text) {
            Some(timeframe) => Ok(timeframe),
            None => Self::parse(&text).map_err(serde::de::Error::custom),
        }
    }
}

#[cfg(test)]
mod tests;
