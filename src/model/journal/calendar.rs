//! Civil-date arithmetic and the P&L calendar. Pure: the caller supplies the time zone as a
//! function from an instant to its UTC offset, so a daylight-saving day is 23 or 25 hours long and
//! tests need no time-zone database.
//!
//! **Day rule (used everywhere in the journal).** A trade belongs to the day it *closed*, in the
//! journal's time zone, at the offset in force at the closing instant. Open trades belong to no
//! day. A trade that spans midnight counts on its close day.

use std::collections::BTreeMap;

use rust_decimal::Decimal;

use super::model::{Trade, TradeStatus};

/// Offset from UTC in seconds at an instant (epoch milliseconds).
pub type Offset<'a> = &'a dyn Fn(i64) -> i64;

pub fn utc(_: i64) -> i64 {
    0
}

/// Days since 1970-01-01 of the local calendar day containing `ts_ms`.
pub fn day_number(ts_ms: i64, offset: Offset<'_>) -> i64 {
    (ts_ms.div_euclid(1000) + offset(ts_ms)).div_euclid(86_400)
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Civil {
    pub year: i32,
    pub month: u8,
    pub day: u8,
}

/// Howard Hinnant's `civil_from_days`: proleptic Gregorian, exact for every day number.
pub fn civil_from_days(days: i64) -> Civil {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u8;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u8;
    Civil {
        year: (year + i64::from(month <= 2)) as i32,
        month,
        day,
    }
}

pub fn days_from_civil(civil: Civil) -> i64 {
    let year = i64::from(civil.year) - i64::from(civil.month <= 2);
    let era = year.div_euclid(400);
    let yoe = year.rem_euclid(400);
    let month = i64::from(civil.month);
    let doy =
        (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + i64::from(civil.day) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// 0 = Sunday … 6 = Saturday.
pub fn weekday(days: i64) -> u8 {
    (days + 4).rem_euclid(7) as u8
}

pub fn is_leap(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

pub fn days_in_month(year: i32, month: u8) -> u8 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ if is_leap(year) => 29,
        _ => 28,
    }
}

/// The first instant of local day `day` (epoch ms). The offset is read at the guessed instant and
/// once more at the corrected one, which lands on the right side of a daylight-saving change.
pub fn local_midnight_ms(day: i64, offset: Offset<'_>) -> i64 {
    let guess = day * 86_400_000;
    let first = guess - offset(guess) * 1_000;
    guess - offset(first) * 1_000
}

/// Net P&L, count and outcome tally of one local day.
#[derive(Clone, Debug, PartialEq)]
pub struct DayCell {
    pub day: i64,
    pub date: Civil,
    pub net: Decimal,
    pub trades: usize,
    pub wins: usize,
    pub losses: usize,
    pub breakevens: usize,
}

impl DayCell {
    pub fn win_rate(&self) -> Option<f64> {
        (self.trades > 0).then(|| self.wins as f64 / self.trades as f64)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct WeekTotal {
    pub net: Decimal,
    pub trades: usize,
    pub trading_days: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MonthView {
    pub year: i32,
    pub month: u8,
    /// One entry per calendar day of the month that had a closed trade, by day of month.
    pub days: BTreeMap<u8, DayCell>,
    pub net: Decimal,
    pub trades: usize,
    pub trading_days: usize,
    pub green_days: usize,
    pub red_days: usize,
    pub best_day: Option<DayCell>,
    pub worst_day: Option<DayCell>,
}

/// Per-day totals of closed trades, keyed by local day number. `outcome` classifies a trade.
pub fn daily(
    trades: &[&Trade],
    offset: Offset<'_>,
    outcome: &dyn Fn(&Trade) -> Outcome,
) -> BTreeMap<i64, DayCell> {
    let mut days: BTreeMap<i64, DayCell> = BTreeMap::new();
    for trade in trades {
        let (TradeStatus::Closed, Some(close)) = (trade.status, trade.close_ts_ms) else {
            continue;
        };
        let day = day_number(close, offset);
        let cell = days.entry(day).or_insert_with(|| DayCell {
            day,
            date: civil_from_days(day),
            net: Decimal::ZERO,
            trades: 0,
            wins: 0,
            losses: 0,
            breakevens: 0,
        });
        cell.net += trade.net_pnl();
        cell.trades += 1;
        match outcome(trade) {
            Outcome::Win => cell.wins += 1,
            Outcome::Loss => cell.losses += 1,
            Outcome::Breakeven => cell.breakevens += 1,
        }
    }
    days
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Outcome {
    Win,
    Loss,
    Breakeven,
}

pub fn month_view(
    trades: &[&Trade],
    year: i32,
    month: u8,
    offset: Offset<'_>,
    outcome: &dyn Fn(&Trade) -> Outcome,
) -> MonthView {
    let first = days_from_civil(Civil {
        year,
        month,
        day: 1,
    });
    let last = first + i64::from(days_in_month(year, month)) - 1;
    let mut view = MonthView {
        year,
        month,
        days: BTreeMap::new(),
        net: Decimal::ZERO,
        trades: 0,
        trading_days: 0,
        green_days: 0,
        red_days: 0,
        best_day: None,
        worst_day: None,
    };
    for (day, cell) in daily(trades, offset, outcome) {
        if day < first || day > last {
            continue;
        }
        view.net += cell.net;
        view.trades += cell.trades;
        view.trading_days += 1;
        if cell.net > Decimal::ZERO {
            view.green_days += 1;
        } else if cell.net < Decimal::ZERO {
            view.red_days += 1;
        }
        if view
            .best_day
            .as_ref()
            .is_none_or(|best| cell.net > best.net)
        {
            view.best_day = Some(cell.clone());
        }
        if view
            .worst_day
            .as_ref()
            .is_none_or(|worst| cell.net < worst.net)
        {
            view.worst_day = Some(cell.clone());
        }
        view.days.insert(cell.date.day, cell);
    }
    view
}

impl MonthView {
    /// Rows of seven columns (`week_start`: 0 = Sunday, 1 = Monday); each is a day of the month or
    /// blank.
    pub fn grid(&self, week_start: u8) -> Vec<[Option<u8>; 7]> {
        let first = days_from_civil(Civil {
            year: self.year,
            month: self.month,
            day: 1,
        });
        let lead = (usize::from(weekday(first)) + 7 - usize::from(week_start % 7)) % 7;
        let count = usize::from(days_in_month(self.year, self.month));
        let mut rows = Vec::new();
        let mut row = [None; 7];
        for slot in 0..(lead + count) {
            if slot >= lead {
                row[slot % 7] = Some((slot - lead + 1) as u8);
            }
            if slot % 7 == 6 {
                rows.push(row);
                row = [None; 7];
            }
        }
        if (lead + count) % 7 != 0 {
            rows.push(row);
        }
        rows
    }

    /// Totals of each row of [`grid`](Self::grid).
    pub fn week_totals(&self, week_start: u8) -> Vec<WeekTotal> {
        self.grid(week_start)
            .iter()
            .map(|row| {
                let mut total = WeekTotal {
                    net: Decimal::ZERO,
                    trades: 0,
                    trading_days: 0,
                };
                for day in row.iter().flatten() {
                    if let Some(cell) = self.days.get(day) {
                        total.net += cell.net;
                        total.trades += cell.trades;
                        total.trading_days += 1;
                    }
                }
                total
            })
            .collect()
    }
}

/// The month before (`delta = -1`) or after (`1`) a month.
pub fn shift_month(year: i32, month: u8, delta: i32) -> (i32, u8) {
    let index = year * 12 + i32::from(month) - 1 + delta;
    (index.div_euclid(12), (index.rem_euclid(12) + 1) as u8)
}

#[cfg(test)]
mod tests;
