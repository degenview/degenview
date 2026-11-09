//! Selecting trades, and the date ranges the journal offers.
//!
//! One [`TradeFilter`] drives every widget, table and chart, so a number on the dashboard and the
//! rows beneath it always describe the same trades.
//!
//! **Range rule.** A closed trade is in a range by its *close* time (the same rule as the
//! calendar); an open trade, which has no close, by its *open* time. A range is half-open:
//! `from <= t < to`.

use std::collections::BTreeSet;

use rust_decimal::Decimal;

use super::calendar::{self, Civil, Offset, civil_from_days, day_number, days_from_civil};
use super::model::{Direction, Trade, TradeStatus};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OutcomeFilter {
    Open,
    Win,
    Loss,
    Breakeven,
}

/// Open, or by net P&L against the tolerance: above wins, below its negative loses, else breakeven.
pub fn outcome_of(trade: &Trade, breakeven_tolerance: Decimal) -> OutcomeFilter {
    match trade.status {
        TradeStatus::Open => OutcomeFilter::Open,
        TradeStatus::Closed => {
            let net = trade.net_pnl();
            if net > breakeven_tolerance {
                OutcomeFilter::Win
            } else if net < -breakeven_tolerance {
                OutcomeFilter::Loss
            } else {
                OutcomeFilter::Breakeven
            }
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TradeFilter {
    /// Journal account uids; empty means every account.
    pub accounts: BTreeSet<String>,
    pub from_ms: Option<i64>,
    pub to_ms: Option<i64>,
    /// `InstrumentRef::market_key`s; empty means every instrument.
    pub markets: BTreeSet<String>,
    pub direction: Option<Direction>,
    pub outcome: Option<OutcomeFilter>,
    /// Case-insensitive text matched against the symbol and display name.
    pub search: String,
}

impl TradeFilter {
    /// Whether anything narrows the selection.
    pub fn is_active(&self) -> bool {
        !self.accounts.is_empty()
            || self.from_ms.is_some()
            || self.to_ms.is_some()
            || !self.markets.is_empty()
            || self.direction.is_some()
            || self.outcome.is_some()
            || !self.search.trim().is_empty()
    }

    /// The names of what is narrowing, for the "active filters" chips.
    pub fn active_labels(&self) -> Vec<String> {
        let mut labels = Vec::new();
        if !self.accounts.is_empty() {
            labels.push(format!("{} account(s)", self.accounts.len()));
        }
        if self.from_ms.is_some() || self.to_ms.is_some() {
            labels.push("date range".to_owned());
        }
        if !self.markets.is_empty() {
            labels.push(format!("{} instrument(s)", self.markets.len()));
        }
        if let Some(direction) = self.direction {
            labels.push(
                match direction {
                    Direction::Long => "long",
                    Direction::Short => "short",
                }
                .to_owned(),
            );
        }
        if let Some(outcome) = self.outcome {
            labels.push(
                match outcome {
                    OutcomeFilter::Open => "open",
                    OutcomeFilter::Win => "wins",
                    OutcomeFilter::Loss => "losses",
                    OutcomeFilter::Breakeven => "breakeven",
                }
                .to_owned(),
            );
        }
        if !self.search.trim().is_empty() {
            labels.push(format!("\"{}\"", self.search.trim()));
        }
        labels
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }

    pub fn matches(&self, trade: &Trade, breakeven_tolerance: Decimal) -> bool {
        if !self.accounts.is_empty() && !self.accounts.contains(&trade.account_uid) {
            return false;
        }
        if !self.markets.is_empty() && !self.markets.contains(&trade.instrument.market_key) {
            return false;
        }
        if self
            .direction
            .is_some_and(|direction| direction != trade.direction)
        {
            return false;
        }
        let at = match (trade.status, trade.close_ts_ms) {
            (TradeStatus::Closed, Some(close)) => close,
            _ => trade.open_ts_ms,
        };
        if self.from_ms.is_some_and(|from| at < from) || self.to_ms.is_some_and(|to| at >= to) {
            return false;
        }
        if let Some(outcome) = self.outcome
            && outcome_of(trade, breakeven_tolerance) != outcome
        {
            return false;
        }
        let needle = self.search.trim().to_lowercase();
        if !needle.is_empty()
            && !trade.instrument.symbol.to_lowercase().contains(&needle)
            && !trade
                .instrument
                .display_name
                .to_lowercase()
                .contains(&needle)
        {
            return false;
        }
        true
    }

    /// Positions in `trades` of the ones that match, in order.
    pub fn matching_indices(&self, trades: &[Trade], breakeven_tolerance: Decimal) -> Vec<usize> {
        trades
            .iter()
            .enumerate()
            .filter(|(_, trade)| self.matches(trade, breakeven_tolerance))
            .map(|(index, _)| index)
            .collect()
    }

    pub fn apply<'a>(&self, trades: &'a [Trade], breakeven_tolerance: Decimal) -> Vec<&'a Trade> {
        trades
            .iter()
            .filter(|trade| self.matches(trade, breakeven_tolerance))
            .collect()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RangePreset {
    Today,
    Week,
    Month,
    /// The last 365 local days, today included.
    Year,
    /// Since 1 January of the current year.
    YearToDate,
    All,
}

/// The `(from, to)` of a preset around `now_ms`, in local days. `week_start`: 0 = Sunday.
pub fn range_for(
    preset: RangePreset,
    now_ms: i64,
    offset: Offset<'_>,
    week_start: u8,
) -> (Option<i64>, Option<i64>) {
    let today = day_number(now_ms, offset);
    let civil = civil_from_days(today);
    let tomorrow = today + 1;
    let (first, end) = match preset {
        RangePreset::All => return (None, None),
        RangePreset::Today => (today, tomorrow),
        RangePreset::Week => {
            let since =
                (i64::from(calendar::weekday(today)) - i64::from(week_start % 7)).rem_euclid(7);
            (today - since, today - since + 7)
        }
        RangePreset::Month => {
            let start = days_from_civil(Civil { day: 1, ..civil });
            let (year, month) = calendar::shift_month(civil.year, civil.month, 1);
            (
                start,
                days_from_civil(Civil {
                    year,
                    month,
                    day: 1,
                }),
            )
        }
        RangePreset::Year => (tomorrow - 365, tomorrow),
        RangePreset::YearToDate => (
            days_from_civil(Civil {
                year: civil.year,
                month: 1,
                day: 1,
            }),
            tomorrow,
        ),
    };
    (
        Some(calendar::local_midnight_ms(first, offset)),
        Some(calendar::local_midnight_ms(end, offset)),
    )
}

/// The window of the same length immediately before `(from, to)`, for "versus previous period".
/// `None` for an unbounded range, which has no previous period.
pub fn previous_window(from_ms: Option<i64>, to_ms: Option<i64>) -> Option<(i64, i64)> {
    let (from, to) = (from_ms?, to_ms?);
    (to > from).then(|| (from - (to - from), from))
}

#[cfg(test)]
mod tests;
