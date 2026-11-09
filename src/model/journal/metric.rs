//! A figure that may not exist. Every statistic the journal shows is a [`Metric`]: either a
//! value, or the reason there is none. "No losses" is not infinity, "no trades" is not zero, and
//! a missing stop is not a risk of zero.

#[derive(Clone, Debug, PartialEq)]
pub enum Metric<T> {
    Value(T),
    Unavailable(Reason),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Reason {
    NoTrades,
    NoWins,
    NoLosses,
    /// The account has no initial balance, so a percentage of it cannot exist.
    NoInitialBalance,
    /// The trade has no stop (planned or entered by the user), so risk and R cannot exist.
    MissingStop,
    /// The stop sits on the wrong side of an entry (above a long's entry, below a short's).
    InvalidStop,
    /// The trade is still open.
    OpenTrade,
    /// The selection mixes currencies; amounts are never added across currencies.
    MixedCurrency,
    /// A figure that needs more samples than the selection has.
    Insufficient {
        have: usize,
        need: usize,
    },
    /// The inputs have no spread (every value equal), so a ratio over the spread is undefined.
    NoSpread,
    /// Nothing happened that this figure measures (no drawdown episode, no winning day).
    NotApplicable,
    /// The provider returned no candles over the hold.
    NoCandles,
    /// The hold is shorter than one candle, or every candle in it contains a fill: nothing can be
    /// said without looking inside a bar.
    NoCompleteBar,
}

impl Reason {
    /// A short phrase for a tooltip or an empty cell.
    pub fn explain(&self) -> String {
        match self {
            Self::NoTrades => "No closed trades.".to_owned(),
            Self::NoWins => "No winning trades.".to_owned(),
            Self::NoLosses => "No losing trades.".to_owned(),
            Self::NoInitialBalance => "The account has no initial balance.".to_owned(),
            Self::MissingStop => "The trade has no stop.".to_owned(),
            Self::InvalidStop => "The stop is on the wrong side of the entry.".to_owned(),
            Self::OpenTrade => "The trade is still open.".to_owned(),
            Self::MixedCurrency => "The selection mixes currencies.".to_owned(),
            Self::Insufficient { have, need } => {
                format!("Needs at least {need} samples; there are {have}.")
            }
            Self::NoSpread => "Every value is the same.".to_owned(),
            Self::NotApplicable => "Nothing to measure.".to_owned(),
            Self::NoCandles => "No candles were available over this trade.".to_owned(),
            Self::NoCompleteBar => {
                "No complete candle sits between the fills; the hold is too short for this interval."
                    .to_owned()
            }
        }
    }
}

impl<T> Metric<T> {
    pub fn value(&self) -> Option<&T> {
        match self {
            Self::Value(value) => Some(value),
            Self::Unavailable(_) => None,
        }
    }

    pub fn into_value(self) -> Option<T> {
        match self {
            Self::Value(value) => Some(value),
            Self::Unavailable(_) => None,
        }
    }

    pub fn reason(&self) -> Option<&Reason> {
        match self {
            Self::Value(_) => None,
            Self::Unavailable(reason) => Some(reason),
        }
    }

    pub fn is_available(&self) -> bool {
        matches!(self, Self::Value(_))
    }

    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> Metric<U> {
        match self {
            Self::Value(value) => Metric::Value(f(value)),
            Self::Unavailable(reason) => Metric::Unavailable(reason),
        }
    }
}

impl<T: Copy> Metric<T> {
    pub fn copied(&self) -> Option<T> {
        self.value().copied()
    }
}

/// A finite `f64` as a metric; NaN and infinity never leave the analytics.
pub fn finite(value: f64, otherwise: Reason) -> Metric<f64> {
    if value.is_finite() {
        Metric::Value(value)
    } else {
        Metric::Unavailable(otherwise)
    }
}
