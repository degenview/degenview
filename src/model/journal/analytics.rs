//! Performance statistics over closed trades. Pure: trades in, [`Summary`] out.
//!
//! Definitions (shown as tooltips in the UI and repeated in `docs/trading-journal.md`):
//!
//! - **Closed trade**: flat to flat. Open trades are counted in `open` and excluded from every
//!   realised figure.
//! - **Outcome**: net P&L above `+tolerance` is a win, below `-tolerance` a loss, anything else a
//!   breakeven. The tolerance is an amount in the account currency, zero by default.
//! - **Win rate** = wins / closed trades (breakevens count in the denominator).
//! - **Gross profit / gross loss** = the sum of positive / negative net P&L of all closed trades
//!   (loss as a positive magnitude), whatever their outcome label.
//! - **Profit factor** = gross profit / gross loss. With no loss it is unavailable ("no
//!   losses"), never infinity.
//! - **Average win / loss** = mean net P&L of winning / losing trades (loss as a magnitude).
//!   **Payoff** = average win / average loss.
//! - **Expectancy** = net P&L per closed trade. **Expectancy (R)** = mean R over the trades that
//!   have an R.
//! - **Streaks** run in close order; a breakeven ends both kinds of streak.
//! - **Days**: see [`super::calendar`]; the **day win rate** is winning days / trading days and a
//!   *trading day* is one with at least one closed trade.
//! - **Sharpe, Sortino, Calmar** use daily returns of realised equity on trading days only (days
//!   without a closed trade are not zero-return days), are not annualised, take a zero risk-free
//!   rate, and need an initial balance and at least [`MIN_DAYS`] trading days. **SQN** is
//!   `sqrt(n) × mean R / stdev R` and needs at least [`MIN_R_TRADES`] trades with an R.
//! - Amounts in different currencies are never added: [`summarize`] refuses a mixed selection and
//!   [`summarize_by_currency`] returns one summary per currency.

use std::collections::BTreeMap;

use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;

use super::calendar::{self, DayCell, Offset, Outcome};
use super::equity::{DrawdownStats, drawdown_stats, equity_curve};
use super::metric::{Metric, Reason, finite};
use super::model::{Trade, TradeStatus};
use super::risk;

pub const MIN_DAYS: usize = 30;
pub const MIN_R_TRADES: usize = 30;

pub struct Options<'a> {
    pub initial_balance: Option<Decimal>,
    pub breakeven_tolerance: Decimal,
    pub offset: Offset<'a>,
    /// R of a trade. The default reads the trade's own stop; a caller with reviews supplies the
    /// manual stops.
    pub r_of: &'a dyn Fn(&Trade) -> Metric<Decimal>,
}

fn default_r(trade: &Trade) -> Metric<Decimal> {
    risk::r_multiple(trade, None)
}

impl Default for Options<'_> {
    fn default() -> Self {
        Self {
            initial_balance: None,
            breakeven_tolerance: Decimal::ZERO,
            offset: &calendar::utc,
            r_of: &default_r,
        }
    }
}

impl Options<'_> {
    pub fn outcome(&self, trade: &Trade) -> Outcome {
        let net = trade.net_pnl();
        if net > self.breakeven_tolerance {
            Outcome::Win
        } else if net < -self.breakeven_tolerance {
            Outcome::Loss
        } else {
            Outcome::Breakeven
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct RStats {
    /// Closed trades that have an R.
    pub count: usize,
    pub average: Metric<f64>,
    pub best: Metric<f64>,
    pub worst: Metric<f64>,
    pub total: Metric<f64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Streaks {
    pub max_wins: usize,
    pub max_losses: usize,
    /// Positive for a winning streak in progress, negative for a losing one, zero otherwise.
    pub current: i64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DayStats {
    pub trading_days: usize,
    pub winning: usize,
    pub losing: usize,
    pub flat: usize,
    pub day_win_rate: Metric<f64>,
    pub best: Metric<DayCell>,
    pub worst: Metric<DayCell>,
    pub average: Metric<Decimal>,
    /// Largest winning day / total of winning days (the Edge Score's consistency input).
    pub profit_concentration: Metric<f64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Durations {
    pub average_ms: Metric<i64>,
    pub average_win_ms: Metric<i64>,
    pub average_loss_ms: Metric<i64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Summary {
    /// The one currency every amount is in; `None` for trades that carry none.
    pub currency: Option<String>,
    pub closed: usize,
    pub open: usize,
    pub wins: usize,
    pub losses: usize,
    pub breakevens: usize,
    pub gross_pnl: Decimal,
    pub fees: Decimal,
    /// False when a closed trade had an execution with no fee figure: `fees` and `net_pnl` are
    /// then a floor, not final.
    pub fees_complete: bool,
    pub net_pnl: Decimal,
    pub gross_profit: Decimal,
    pub gross_loss: Decimal,
    pub win_rate: Metric<f64>,
    pub profit_factor: Metric<f64>,
    pub average_win: Metric<Decimal>,
    pub average_loss: Metric<Decimal>,
    pub payoff: Metric<f64>,
    pub expectancy: Metric<Decimal>,
    pub r: RStats,
    pub expectancy_r: Metric<f64>,
    pub drawdown: DrawdownStats,
    pub streaks: Streaks,
    pub days: DayStats,
    pub durations: Durations,
    pub sharpe: Metric<f64>,
    pub sortino: Metric<f64>,
    pub calmar: Metric<f64>,
    pub sqn: Metric<f64>,
}

fn f(value: Decimal) -> f64 {
    value.to_f64().unwrap_or(f64::NAN)
}

fn ratio(numerator: Decimal, denominator: Decimal, otherwise: Reason) -> Metric<f64> {
    if denominator.is_zero() {
        return Metric::Unavailable(otherwise);
    }
    finite(f(numerator) / f(denominator), Reason::NoSpread)
}

fn mean_decimal(values: &[Decimal], otherwise: Reason) -> Metric<Decimal> {
    if values.is_empty() {
        return Metric::Unavailable(otherwise);
    }
    Metric::Value(values.iter().copied().sum::<Decimal>() / Decimal::from(values.len()))
}

fn mean_ms(values: &[i64], otherwise: Reason) -> Metric<i64> {
    if values.is_empty() {
        return Metric::Unavailable(otherwise);
    }
    Metric::Value(values.iter().sum::<i64>() / values.len() as i64)
}

fn mean_std(values: &[f64]) -> Option<(f64, f64)> {
    if values.len() < 2 {
        return None;
    }
    let n = values.len() as f64;
    let mean = values.iter().sum::<f64>() / n;
    let variance = values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / (n - 1.0);
    Some((mean, variance.sqrt()))
}

/// Which currency a trade is in. Two trades with different (or one missing) currency are never
/// added together.
pub fn currency_of(trade: &Trade) -> Option<&str> {
    trade.currency.as_deref()
}

/// Splits a selection by currency, in currency order (no currency last).
pub fn split_by_currency<'a>(trades: &[&'a Trade]) -> Vec<(Option<String>, Vec<&'a Trade>)> {
    let mut groups: BTreeMap<Option<String>, Vec<&'a Trade>> = BTreeMap::new();
    for trade in trades {
        groups
            .entry(currency_of(trade).map(str::to_owned))
            .or_default()
            .push(trade);
    }
    let mut out: Vec<_> = groups.into_iter().collect();
    // `None` sorts first in an Option; put it last.
    out.sort_by_key(|(currency, _)| currency.is_none());
    out
}

pub fn summarize_by_currency(
    trades: &[&Trade],
    options: &Options<'_>,
) -> Vec<(Option<String>, Summary)> {
    split_by_currency(trades)
        .into_iter()
        .map(|(currency, group)| {
            let summary = summarize_one(&group, currency.clone(), options);
            (currency, summary)
        })
        .collect()
}

/// A summary of one currency's trades. `Err(MixedCurrency)` when the selection holds more than
/// one.
pub fn summarize(trades: &[&Trade], options: &Options<'_>) -> Result<Summary, Reason> {
    let mut groups = split_by_currency(trades);
    match groups.len() {
        0 => Ok(summarize_one(&[], None, options)),
        1 => {
            let (currency, group) = groups.remove(0);
            Ok(summarize_one(&group, currency, options))
        }
        _ => Err(Reason::MixedCurrency),
    }
}

fn summarize_one(trades: &[&Trade], currency: Option<String>, options: &Options<'_>) -> Summary {
    let open = trades
        .iter()
        .filter(|trade| trade.status == TradeStatus::Open)
        .count();
    let mut closed: Vec<&Trade> = trades
        .iter()
        .copied()
        .filter(|trade| trade.status == TradeStatus::Closed && trade.close_ts_ms.is_some())
        .collect();
    closed.sort_by(|a, b| (a.close_ts_ms, &a.key).cmp(&(b.close_ts_ms, &b.key)));

    let mut wins = Vec::new();
    let mut losses = Vec::new();
    let mut breakevens = 0usize;
    let mut gross_pnl = Decimal::ZERO;
    let mut fees = Decimal::ZERO;
    let mut fees_complete = true;
    let mut gross_profit = Decimal::ZERO;
    let mut gross_loss = Decimal::ZERO;
    let mut win_durations = Vec::new();
    let mut loss_durations = Vec::new();
    let mut durations = Vec::new();
    let mut streaks = Streaks {
        max_wins: 0,
        max_losses: 0,
        current: 0,
    };
    for trade in &closed {
        let net = trade.net_pnl();
        gross_pnl += trade.gross_pnl;
        fees += trade.fees;
        fees_complete &= trade.fees_known;
        if net > Decimal::ZERO {
            gross_profit += net;
        } else {
            gross_loss -= net;
        }
        let duration = trade.duration_ms();
        if let Some(duration) = duration {
            durations.push(duration);
        }
        match options.outcome(trade) {
            Outcome::Win => {
                wins.push(net);
                win_durations.extend(duration);
                streaks.current = streaks.current.max(0) + 1;
                streaks.max_wins = streaks.max_wins.max(streaks.current as usize);
            }
            Outcome::Loss => {
                losses.push(-net);
                loss_durations.extend(duration);
                streaks.current = streaks.current.min(0) - 1;
                streaks.max_losses = streaks
                    .max_losses
                    .max(streaks.current.unsigned_abs() as usize);
            }
            Outcome::Breakeven => {
                breakevens += 1;
                streaks.current = 0;
            }
        }
    }
    let count = closed.len();
    let net_pnl = gross_pnl - fees;

    let win_rate = if count == 0 {
        Metric::Unavailable(Reason::NoTrades)
    } else {
        Metric::Value(wins.len() as f64 / count as f64)
    };
    let profit_factor = if count == 0 {
        Metric::Unavailable(Reason::NoTrades)
    } else {
        ratio(gross_profit, gross_loss, Reason::NoLosses)
    };
    let average_win = mean_decimal(&wins, Reason::NoWins);
    let average_loss = mean_decimal(&losses, Reason::NoLosses);
    let payoff = match (&average_win, &average_loss) {
        (Metric::Value(win), Metric::Value(loss)) => ratio(*win, *loss, Reason::NoLosses),
        (Metric::Unavailable(reason), _) | (_, Metric::Unavailable(reason)) => {
            Metric::Unavailable(reason.clone())
        }
    };
    let expectancy = if count == 0 {
        Metric::Unavailable(Reason::NoTrades)
    } else {
        Metric::Value(net_pnl / Decimal::from(count))
    };

    let r_values: Vec<f64> = closed
        .iter()
        .filter_map(|trade| (options.r_of)(trade).into_value().and_then(|r| r.to_f64()))
        .collect();
    let r = r_stats(&r_values);
    let expectancy_r = r.average.clone();

    let curve = equity_curve(&closed, options.initial_balance);
    let drawdown = drawdown_stats(&curve, options.initial_balance);

    let day_cells = calendar::daily(&closed, options.offset, &|trade| options.outcome(trade));
    let days = day_stats(&day_cells);
    let (sharpe, sortino, calmar) =
        risk_adjusted(&day_cells, options.initial_balance, net_pnl, &drawdown);
    let sqn = sqn(&r_values);

    Summary {
        currency,
        closed: count,
        open,
        wins: wins.len(),
        losses: losses.len(),
        breakevens,
        gross_pnl,
        fees,
        fees_complete,
        net_pnl,
        gross_profit,
        gross_loss,
        win_rate,
        profit_factor,
        average_win,
        average_loss,
        payoff,
        expectancy,
        r,
        expectancy_r,
        drawdown,
        streaks,
        days,
        durations: Durations {
            average_ms: mean_ms(&durations, Reason::NoTrades),
            average_win_ms: mean_ms(&win_durations, Reason::NoWins),
            average_loss_ms: mean_ms(&loss_durations, Reason::NoLosses),
        },
        sharpe,
        sortino,
        calmar,
        sqn,
    }
}

fn r_stats(values: &[f64]) -> RStats {
    if values.is_empty() {
        let none = || Metric::Unavailable(Reason::MissingStop);
        return RStats {
            count: 0,
            average: none(),
            best: none(),
            worst: none(),
            total: none(),
        };
    }
    let total: f64 = values.iter().sum();
    RStats {
        count: values.len(),
        average: finite(total / values.len() as f64, Reason::NoSpread),
        best: finite(
            values.iter().copied().fold(f64::NEG_INFINITY, f64::max),
            Reason::NoSpread,
        ),
        worst: finite(
            values.iter().copied().fold(f64::INFINITY, f64::min),
            Reason::NoSpread,
        ),
        total: finite(total, Reason::NoSpread),
    }
}

fn day_stats(cells: &BTreeMap<i64, DayCell>) -> DayStats {
    let trading_days = cells.len();
    let winning = cells.values().filter(|c| c.net > Decimal::ZERO).count();
    let losing = cells.values().filter(|c| c.net < Decimal::ZERO).count();
    let best = cells
        .values()
        .max_by(|a, b| a.net.cmp(&b.net).then(b.day.cmp(&a.day)));
    let worst = cells
        .values()
        .min_by(|a, b| a.net.cmp(&b.net).then(a.day.cmp(&b.day)));
    let net: Decimal = cells.values().map(|c| c.net).sum();
    let winning_total: Decimal = cells
        .values()
        .filter(|c| c.net > Decimal::ZERO)
        .map(|c| c.net)
        .sum();
    let biggest = cells
        .values()
        .filter(|c| c.net > Decimal::ZERO)
        .map(|c| c.net)
        .max();
    DayStats {
        trading_days,
        winning,
        losing,
        flat: trading_days - winning - losing,
        day_win_rate: if trading_days == 0 {
            Metric::Unavailable(Reason::NoTrades)
        } else {
            Metric::Value(winning as f64 / trading_days as f64)
        },
        best: best.map_or(Metric::Unavailable(Reason::NoTrades), |c| {
            Metric::Value(c.clone())
        }),
        worst: worst.map_or(Metric::Unavailable(Reason::NoTrades), |c| {
            Metric::Value(c.clone())
        }),
        average: if trading_days == 0 {
            Metric::Unavailable(Reason::NoTrades)
        } else {
            Metric::Value(net / Decimal::from(trading_days))
        },
        profit_concentration: match biggest {
            Some(biggest) => ratio(biggest, winning_total, Reason::NotApplicable),
            None => Metric::Unavailable(Reason::NotApplicable),
        },
    }
}

fn risk_adjusted(
    cells: &BTreeMap<i64, DayCell>,
    initial_balance: Option<Decimal>,
    net_pnl: Decimal,
    drawdown: &DrawdownStats,
) -> (Metric<f64>, Metric<f64>, Metric<f64>) {
    let Some(initial) = initial_balance.filter(|initial| *initial > Decimal::ZERO) else {
        let none = || Metric::Unavailable(Reason::NoInitialBalance);
        return (none(), none(), none());
    };
    if cells.len() < MIN_DAYS {
        let few = || {
            Metric::Unavailable(Reason::Insufficient {
                have: cells.len(),
                need: MIN_DAYS,
            })
        };
        return (few(), few(), few());
    }
    let mut equity = initial;
    let mut returns = Vec::with_capacity(cells.len());
    for cell in cells.values() {
        if equity > Decimal::ZERO {
            returns.push(f(cell.net / equity));
        }
        equity += cell.net;
    }
    let Some((mean, std)) = mean_std(&returns) else {
        let none = || Metric::Unavailable(Reason::NoSpread);
        return (none(), none(), none());
    };
    let sharpe = if std > 0.0 {
        finite(mean / std, Reason::NoSpread)
    } else {
        Metric::Unavailable(Reason::NoSpread)
    };
    let downside =
        (returns.iter().map(|r| r.min(0.0).powi(2)).sum::<f64>() / returns.len() as f64).sqrt();
    let sortino = if downside > 0.0 {
        finite(mean / downside, Reason::NoLosses)
    } else {
        Metric::Unavailable(Reason::NoLosses)
    };
    let calmar = match drawdown.max_pct {
        Some(pct) if pct > 0.0 => finite(f(net_pnl / initial) / pct, Reason::NotApplicable),
        _ => Metric::Unavailable(Reason::NotApplicable),
    };
    (sharpe, sortino, calmar)
}

fn sqn(r_values: &[f64]) -> Metric<f64> {
    if r_values.len() < MIN_R_TRADES {
        return Metric::Unavailable(Reason::Insufficient {
            have: r_values.len(),
            need: MIN_R_TRADES,
        });
    }
    match mean_std(r_values) {
        Some((mean, std)) if std > 0.0 => finite(
            (r_values.len() as f64).sqrt() * mean / std,
            Reason::NoSpread,
        ),
        _ => Metric::Unavailable(Reason::NoSpread),
    }
}

#[cfg(test)]
mod tests;
