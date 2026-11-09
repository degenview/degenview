//! Everything the Overview page shows, computed once from the selected trades. Pure.
//!
//! The page describes **one currency at a time**: a selection that spans several (an "all
//! accounts" view over a USD and a USDT account) offers the currencies and shows the chosen one,
//! because amounts are never added across currencies.
//!
//! **Equity.** The curve covers the trades closed inside the range. When the range starts after
//! the first trade, the equity it is measured from includes everything those accounts earned
//! before the range (`baseline`), so a percentage drawdown is a fraction of the real balance, not
//! of a balance that restarted at the range's first day. It is *realised* equity (see
//! [`super::equity`]).
//!
//! **Previous period.** The window of the same length just before the range, shifted forward so
//! the two curves line up on elapsed time, for the "vs previous" overlay and the card deltas.

use std::collections::{BTreeSet, HashMap};

use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;

use super::analytics::{Options, Summary, currency_of, split_by_currency, summarize_by_currency};
use super::calendar::{DayCell, Offset, daily};
use super::edge::{EdgeScore, edge_score};
use super::equity::{DrawdownPoint, EquityPoint, drawdown_series, equity_curve};
use super::filters::OutcomeFilter;
use super::metric::Metric;
use super::model::{Direction, JournalAccount, Trade, TradeStatus};
use super::reviews::Review;
use super::risk;

#[derive(Clone, Debug, PartialEq)]
pub struct Bin {
    pub low: f64,
    pub high: f64,
    pub count: usize,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Histogram {
    pub bins: Vec<Bin>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SideStats {
    pub trades: usize,
    pub wins: usize,
    pub losses: usize,
    pub net: Decimal,
    pub win_rate: Metric<f64>,
}

#[derive(Clone, Debug)]
pub struct Overview {
    pub currency: Option<String>,
    /// Every currency in the selection, for the picker; one entry means there is nothing to pick.
    pub currencies: Vec<Option<String>>,
    pub summary: Summary,
    pub previous: Option<Summary>,
    pub initial_balance: Option<Decimal>,
    pub equity: Vec<EquityPoint>,
    pub previous_equity: Vec<EquityPoint>,
    pub drawdown: Vec<DrawdownPoint>,
    pub previous_drawdown: Vec<DrawdownPoint>,
    /// Trading days in order.
    pub days: Vec<DayCell>,
    pub histogram: Histogram,
    /// Realised R of closed trades that have one, in bins of a round width.
    pub r_histogram: Histogram,
    pub long: SideStats,
    pub short: SideStats,
    pub edge: EdgeScore,
    /// Keys of the most recently closed trades, newest first.
    pub recent: Vec<String>,
}

pub struct Input<'a> {
    /// Trades in the range and filters.
    pub trades: &'a [&'a Trade],
    /// The previous period's trades, when the range has one.
    pub previous: Option<&'a [&'a Trade]>,
    /// The same filters without the range's start: what was earned before it.
    pub before: &'a [&'a Trade],
    pub accounts: &'a [&'a JournalAccount],
    /// The range, to shift the previous period onto it.
    pub from_ms: Option<i64>,
    pub to_ms: Option<i64>,
    pub currency: Option<Option<String>>,
    /// The user's reviews: a stop entered there overrides the one the source attached.
    pub reviews: &'a HashMap<String, Review>,
    pub breakeven_tolerance: Decimal,
    pub offset: Offset<'a>,
}

/// The currencies present, in display order (no currency last).
pub fn currencies_of(trades: &[&Trade]) -> Vec<Option<String>> {
    split_by_currency(trades)
        .into_iter()
        .map(|(currency, _)| currency)
        .collect()
}

/// `initial balances of the accounts in this currency + what they earned before the range`, or
/// `None` when any such account has no initial balance (the total would be a guess).
pub fn starting_balance(
    accounts: &[&JournalAccount],
    currency: &Option<String>,
    before: &[&Trade],
) -> Option<Decimal> {
    let mine: Vec<&&JournalAccount> = accounts
        .iter()
        .filter(|account| {
            currency
                .as_deref()
                .is_none_or(|code| account.currency.eq_ignore_ascii_case(code))
        })
        .collect();
    if mine.is_empty() {
        return None;
    }
    let mut total = Decimal::ZERO;
    for account in mine {
        total += account.initial_balance?;
    }
    let earned: Decimal = before
        .iter()
        .filter(|trade| {
            trade.status == TradeStatus::Closed && currency_of(trade) == currency.as_deref()
        })
        .map(|trade| trade.net_pnl())
        .sum();
    Some(total + earned)
}

pub fn compute(input: &Input<'_>) -> Overview {
    let currencies = currencies_of(input.trades);
    let currency = input
        .currency
        .clone()
        .filter(|chosen| currencies.contains(chosen))
        .or_else(|| currencies.first().cloned())
        .unwrap_or(None);
    let mine: Vec<&Trade> = input
        .trades
        .iter()
        .copied()
        .filter(|trade| currency_of(trade) == currency.as_deref())
        .collect();
    let before: Vec<&Trade> = input
        .before
        .iter()
        .copied()
        .filter(|trade| currency_of(trade) == currency.as_deref())
        .collect();
    let initial = starting_balance(input.accounts, &currency, &before);
    let r_of =
        |trade: &Trade| risk::r_multiple(trade, input.reviews.get(&trade.key).and_then(|r| r.stop));
    let options = Options {
        initial_balance: initial,
        breakeven_tolerance: input.breakeven_tolerance,
        offset: input.offset,
        r_of: &r_of,
    };
    let summary = summarize_by_currency(&mine, &options)
        .into_iter()
        .next()
        .map(|(_, summary)| summary)
        .unwrap_or_else(|| {
            super::analytics::summarize(&[], &options).expect("an empty selection has one currency")
        });

    let equity = equity_curve(&mine, initial);
    let drawdown = drawdown_series(&equity, initial);

    // The previous period, on the same footing.
    let (previous, previous_equity, previous_drawdown) = match input.previous {
        Some(previous) => {
            let prev_mine: Vec<&Trade> = previous
                .iter()
                .copied()
                .filter(|trade| currency_of(trade) == currency.as_deref())
                .collect();
            let prev_options = Options {
                // What it started from is the same baseline less this period's own earnings is
                // not known here; percentages for the previous period use the same balance.
                initial_balance: initial,
                breakeven_tolerance: input.breakeven_tolerance,
                offset: input.offset,
                r_of: &r_of,
            };
            let summary = summarize_by_currency(&prev_mine, &prev_options)
                .into_iter()
                .next()
                .map(|(_, summary)| summary);
            let shift = match (input.from_ms, input.to_ms) {
                (Some(from), Some(to)) if to > from => to - from,
                _ => 0,
            };
            let mut curve = equity_curve(&prev_mine, initial);
            for point in &mut curve {
                point.ts_ms += shift;
            }
            let series = drawdown_series(&curve, initial);
            (summary, curve, series)
        }
        None => (None, Vec::new(), Vec::new()),
    };

    let days: Vec<DayCell> = daily(
        &mine,
        input.offset,
        &|trade| match super::filters::outcome_of(trade, input.breakeven_tolerance) {
            OutcomeFilter::Win => super::calendar::Outcome::Win,
            OutcomeFilter::Loss => super::calendar::Outcome::Loss,
            _ => super::calendar::Outcome::Breakeven,
        },
    )
    .into_values()
    .collect();

    let closed_nets: Vec<f64> = mine
        .iter()
        .filter(|trade| trade.status == TradeStatus::Closed)
        .filter_map(|trade| trade.net_pnl().to_f64())
        .collect();
    let histogram = histogram(&closed_nets);
    let r_values: Vec<f64> = mine
        .iter()
        .filter(|trade| trade.status == TradeStatus::Closed)
        .filter_map(|trade| r_of(trade).into_value().and_then(|r| r.to_f64()))
        .collect();
    let r_histogram = histogram_of_r(&r_values);

    let side = |direction: Direction| {
        let side_trades: Vec<&&Trade> = mine
            .iter()
            .filter(|trade| trade.direction == direction && trade.status == TradeStatus::Closed)
            .collect();
        let (mut wins, mut losses) = (0, 0);
        let mut net = Decimal::ZERO;
        for trade in &side_trades {
            net += trade.net_pnl();
            match super::filters::outcome_of(trade, input.breakeven_tolerance) {
                OutcomeFilter::Win => wins += 1,
                OutcomeFilter::Loss => losses += 1,
                _ => {}
            }
        }
        SideStats {
            trades: side_trades.len(),
            wins,
            losses,
            net,
            win_rate: if side_trades.is_empty() {
                Metric::Unavailable(super::metric::Reason::NoTrades)
            } else {
                Metric::Value(wins as f64 / side_trades.len() as f64)
            },
        }
    };

    let mut recent: Vec<&&Trade> = mine
        .iter()
        .filter(|trade| trade.status == TradeStatus::Closed && trade.close_ts_ms.is_some())
        .collect();
    recent.sort_by(|a, b| (b.close_ts_ms, &b.key).cmp(&(a.close_ts_ms, &a.key)));
    let recent = recent
        .into_iter()
        .take(6)
        .map(|trade| trade.key.clone())
        .collect();

    let edge = edge_score(&summary, initial);
    Overview {
        currency,
        currencies,
        summary,
        previous,
        initial_balance: initial,
        equity,
        previous_equity,
        drawdown,
        previous_drawdown,
        days,
        histogram,
        r_histogram,
        long: side(Direction::Long),
        short: side(Direction::Short),
        edge,
        recent,
    }
}

fn histogram_of_r(values: &[f64]) -> Histogram {
    histogram(values)
}

/// A step of `1, 2, 2.5 or 5` times a power of ten that is at least `raw`.
pub fn nice_step(raw: f64) -> f64 {
    if !raw.is_finite() || raw <= 0.0 {
        return 1.0;
    }
    let magnitude = 10f64.powf(raw.log10().floor());
    for factor in [1.0, 2.0, 2.5, 5.0, 10.0] {
        if factor * magnitude >= raw * (1.0 - 1e-12) {
            return factor * magnitude;
        }
    }
    10.0 * magnitude
}

/// Bins of equal width whose edges are multiples of the width, so zero is always an edge: a bin is
/// wholly a loss or wholly a win. Empty input has no bins.
pub fn histogram(values: &[f64]) -> Histogram {
    let values: Vec<f64> = values.iter().copied().filter(|v| v.is_finite()).collect();
    if values.is_empty() {
        return Histogram::default();
    }
    let low = values.iter().copied().fold(f64::INFINITY, f64::min);
    let high = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let target = ((values.len() as f64).sqrt().ceil() as usize).clamp(5, 21);
    let width = if high > low {
        nice_step((high - low) / target as f64)
    } else {
        nice_step(low.abs().max(1.0) / 5.0)
    };
    let first = (low / width).floor();
    let last = (high / width).floor();
    let count = (last - first) as usize + 1;
    let mut bins: Vec<Bin> = (0..count)
        .map(|index| Bin {
            low: (first + index as f64) * width,
            high: (first + index as f64 + 1.0) * width,
            count: 0,
        })
        .collect();
    for value in values {
        let index = (((value / width).floor() - first) as usize).min(bins.len() - 1);
        bins[index].count += 1;
    }
    Histogram { bins }
}

/// The accounts a selection covers: the chosen one, or all.
pub fn accounts_in_scope<'a>(
    accounts: &'a [JournalAccount],
    chosen: Option<&str>,
    trades: &[&Trade],
) -> Vec<&'a JournalAccount> {
    match chosen {
        Some(uid) => accounts
            .iter()
            .filter(|account| account.uid == uid)
            .collect(),
        None => {
            let present: BTreeSet<&str> = trades.iter().map(|t| t.account_uid.as_str()).collect();
            accounts
                .iter()
                .filter(|account| present.contains(account.uid.as_str()))
                .collect()
        }
    }
}

#[cfg(test)]
mod tests;
