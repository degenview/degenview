//! Performance cut by one dimension: weekday, hour, instrument, side, hold time, size, strategy,
//! tag, mistake, emotion or rating. Pure; the caller passes one currency's closed trades.
//!
//! - **Weekday** is the day the trade *closed*, and **hour** the hour it *opened*, in the journal's
//!   time zone (the same day rule as the calendar).
//! - **Hold time** buckets: under 1 minute, 1 to 5 minutes, 5 to 15, 15 to 60, 1 to 4 hours, 4 to
//!   24 hours, 1 to 7 days, over 7 days.
//! - **Size** is the entry notional (`entry quantity x average entry x multiplier`) in quartiles of
//!   the selection, so the buckets are relative to this selection and not to the account.
//! - **Strategy, tag, mistake, emotion, rating** come from the user's review. A trade with several
//!   tags counts once in each, so the slices of those dimensions overlap and their trade counts add
//!   up to more than the number of trades. A trade with no value is in "(none)".
//!
//! A slice says how much to trust it ([`Sample`]): fewer than 5 trades is thin and fewer than 30 is
//! low confidence. Nothing here claims a difference is significant, and the associations with tags,
//! mistakes and emotions are descriptive, not causal.

use std::collections::{BTreeMap, HashMap};

use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;

use super::calendar::{Offset, day_number, weekday};
use super::filters::{OutcomeFilter, outcome_of};
use super::format;
use super::metric::{Metric, Reason};
use super::model::{Direction, Trade, TradeStatus};
use super::reviews::Review;
use super::risk;
use super::table::return_fraction;

pub const THIN_BELOW: usize = 5;
pub const LOW_CONFIDENCE_BELOW: usize = 30;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Dimension {
    Weekday,
    Hour,
    Instrument,
    Side,
    HoldTime,
    Size,
    Strategy,
    Tag,
    Mistake,
    Emotion,
    Rating,
}

impl Dimension {
    pub const ALL: [Dimension; 11] = [
        Self::Weekday,
        Self::Hour,
        Self::Instrument,
        Self::Side,
        Self::HoldTime,
        Self::Size,
        Self::Strategy,
        Self::Tag,
        Self::Mistake,
        Self::Emotion,
        Self::Rating,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Weekday => "Weekday",
            Self::Hour => "Hour",
            Self::Instrument => "Instrument",
            Self::Side => "Side",
            Self::HoldTime => "Hold time",
            Self::Size => "Size",
            Self::Strategy => "Strategy",
            Self::Tag => "Tag",
            Self::Mistake => "Mistake",
            Self::Emotion => "Emotion",
            Self::Rating => "Rating",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::Weekday => "By the day the trade closed.",
            Self::Hour => "By the hour the trade opened.",
            Self::Instrument => "By market.",
            Self::Side => "Long against short.",
            Self::HoldTime => "By how long the trade was held.",
            Self::Size => "By entry value, in quartiles of this selection.",
            Self::Strategy => "By the playbook chosen in the review.",
            Self::Tag => "By review tag. A trade with several tags counts in each.",
            Self::Mistake => "By mistake noted in the review. A trade can have several.",
            Self::Emotion => "By emotion noted in the review. A trade can have several.",
            Self::Rating => "By the 1 to 5 rating given in the review.",
        }
    }

    /// Slices keep their natural order (a clock, a week) rather than being ranked by result.
    pub fn natural_order(self) -> bool {
        matches!(
            self,
            Self::Weekday | Self::Hour | Self::HoldTime | Self::Size | Self::Rating | Self::Side
        )
    }

    /// Slices of these overlap: one trade can sit in several.
    pub fn overlapping(self) -> bool {
        matches!(self, Self::Tag | Self::Mistake | Self::Emotion)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Sample {
    /// Fewer than [`THIN_BELOW`] trades: too few to read anything into.
    Thin,
    /// Fewer than [`LOW_CONFIDENCE_BELOW`]: a hint at most.
    Low,
    Enough,
}

pub fn sample_of(trades: usize) -> Sample {
    if trades < THIN_BELOW {
        Sample::Thin
    } else if trades < LOW_CONFIDENCE_BELOW {
        Sample::Low
    } else {
        Sample::Enough
    }
}

impl Sample {
    pub fn note(self) -> Option<&'static str> {
        match self {
            Self::Thin => Some("Too few trades to read anything into."),
            Self::Low => Some("A small sample: a hint at most."),
            Self::Enough => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Slice {
    pub label: String,
    /// Sort key for natural-order dimensions.
    pub order: i64,
    pub trades: usize,
    pub wins: usize,
    pub losses: usize,
    pub breakevens: usize,
    pub net: Decimal,
    pub gross_profit: Decimal,
    pub gross_loss: Decimal,
    pub win_rate: Metric<f64>,
    pub profit_factor: Metric<f64>,
    pub expectancy: Metric<Decimal>,
    /// Mean R over trades in the slice that have one.
    pub average_r: Metric<f64>,
    pub r_count: usize,
    pub average_return: Metric<f64>,
    pub sample: Sample,
}

pub struct Context<'a> {
    pub reviews: &'a HashMap<String, Review>,
    /// Playbook id to name.
    pub playbook_names: &'a HashMap<String, String>,
    pub offset: Offset<'a>,
    pub breakeven_tolerance: Decimal,
}

const NONE_LABEL: &str = "(none)";

fn hold_bucket(ms: i64) -> (i64, &'static str) {
    const MINUTE: i64 = 60_000;
    const HOUR: i64 = 60 * MINUTE;
    const DAY: i64 = 24 * HOUR;
    match ms {
        m if m < MINUTE => (0, "Under 1 min"),
        m if m < 5 * MINUTE => (1, "1 to 5 min"),
        m if m < 15 * MINUTE => (2, "5 to 15 min"),
        m if m < HOUR => (3, "15 to 60 min"),
        m if m < 4 * HOUR => (4, "1 to 4 hours"),
        m if m < DAY => (5, "4 to 24 hours"),
        m if m < 7 * DAY => (6, "1 to 7 days"),
        _ => (7, "Over 7 days"),
    }
}

/// Quartile boundaries of entry notional among `trades` (the 25th, 50th and 75th percentile).
pub fn notional_quartiles(trades: &[&Trade]) -> Option<[Decimal; 3]> {
    let mut values: Vec<Decimal> = trades
        .iter()
        .map(|t| t.entry_quantity * t.avg_entry * t.multiplier)
        .collect();
    if values.len() < 4 {
        return None;
    }
    values.sort();
    let at = |p: usize| values[(values.len() * p / 4).min(values.len() - 1)];
    Some([at(1), at(2), at(3)])
}

fn size_bucket(notional: Decimal, quartiles: &[Decimal; 3]) -> (i64, String) {
    let index = quartiles.iter().filter(|q| notional >= **q).count();
    (
        index as i64,
        match index {
            0 => format!(
                "Q1 (under {})",
                format::compact(quartiles[0].to_f64().unwrap_or(0.0))
            ),
            1 => "Q2".to_owned(),
            2 => "Q3".to_owned(),
            _ => format!(
                "Q4 ({}+)",
                format::compact(quartiles[2].to_f64().unwrap_or(0.0))
            ),
        },
    )
}

/// The slice labels (and sort keys) one trade belongs to under a dimension.
fn labels_for(
    dimension: Dimension,
    trade: &Trade,
    review: Option<&Review>,
    context: &Context<'_>,
    quartiles: Option<&[Decimal; 3]>,
) -> Vec<(i64, String)> {
    let list = |items: Option<&Vec<String>>| -> Vec<(i64, String)> {
        match items.filter(|items| !items.is_empty()) {
            Some(items) => items.iter().map(|item| (0, item.clone())).collect(),
            None => vec![(i64::MAX, NONE_LABEL.to_owned())],
        }
    };
    match dimension {
        Dimension::Weekday => {
            let at = trade.close_ts_ms.unwrap_or(trade.open_ts_ms);
            let day = weekday(day_number(at, context.offset));
            vec![(i64::from(day), format::weekday_name(day).to_owned())]
        }
        Dimension::Hour => {
            let seconds = (trade.open_ts_ms.div_euclid(1_000) + (context.offset)(trade.open_ts_ms))
                .rem_euclid(86_400);
            let hour = seconds / 3_600;
            vec![(hour, format!("{hour:02}:00"))]
        }
        Dimension::Instrument => vec![(0, trade.instrument.display_name.clone())],
        Dimension::Side => vec![match trade.direction {
            Direction::Long => (0, "Long".to_owned()),
            Direction::Short => (1, "Short".to_owned()),
        }],
        Dimension::HoldTime => {
            let (order, label) = hold_bucket(trade.duration_ms().unwrap_or(0));
            vec![(order, label.to_owned())]
        }
        Dimension::Size => match quartiles {
            Some(quartiles) => {
                let notional = trade.entry_quantity * trade.avg_entry * trade.multiplier;
                vec![size_bucket(notional, quartiles)]
            }
            None => vec![(0, "All sizes".to_owned())],
        },
        Dimension::Strategy => match review
            .and_then(|r| r.playbook.as_ref())
            .and_then(|id| context.playbook_names.get(id))
        {
            Some(name) => vec![(0, name.clone())],
            None => vec![(i64::MAX, NONE_LABEL.to_owned())],
        },
        Dimension::Tag => list(review.map(|r| &r.tags)),
        Dimension::Mistake => list(review.map(|r| &r.mistakes)),
        Dimension::Emotion => list(review.map(|r| &r.emotions)),
        Dimension::Rating => match review.and_then(|r| r.rating) {
            Some(rating) => vec![(
                i64::from(rating),
                format!("{rating} star{}", if rating == 1 { "" } else { "s" }),
            )],
            None => vec![(i64::MAX, NONE_LABEL.to_owned())],
        },
    }
}

#[derive(Default)]
struct Accumulator {
    order: i64,
    trades: usize,
    wins: usize,
    losses: usize,
    breakevens: usize,
    net: Decimal,
    gross_profit: Decimal,
    gross_loss: Decimal,
    r_values: Vec<f64>,
    returns: Vec<f64>,
}

/// Closed trades cut by `dimension`. Open trades are ignored.
pub fn breakdown(trades: &[&Trade], dimension: Dimension, context: &Context<'_>) -> Vec<Slice> {
    let closed: Vec<&Trade> = trades
        .iter()
        .copied()
        .filter(|t| t.status == TradeStatus::Closed)
        .collect();
    let quartiles = (dimension == Dimension::Size)
        .then(|| notional_quartiles(&closed))
        .flatten();
    let mut slices: BTreeMap<String, Accumulator> = BTreeMap::new();
    for trade in &closed {
        let review = context.reviews.get(&trade.key);
        let net = trade.net_pnl();
        let r = risk::r_multiple(trade, review.and_then(|r| r.stop))
            .into_value()
            .and_then(|r| r.to_f64());
        let outcome = outcome_of(trade, context.breakeven_tolerance);
        for (order, label) in labels_for(dimension, trade, review, context, quartiles.as_ref()) {
            let acc = slices.entry(label).or_default();
            acc.order = order;
            acc.trades += 1;
            acc.net += net;
            if net > Decimal::ZERO {
                acc.gross_profit += net;
            } else {
                acc.gross_loss -= net;
            }
            match outcome {
                OutcomeFilter::Win => acc.wins += 1,
                OutcomeFilter::Loss => acc.losses += 1,
                _ => acc.breakevens += 1,
            }
            acc.r_values.extend(r);
            acc.returns.extend(return_fraction(trade));
        }
    }
    let mut out: Vec<Slice> = slices
        .into_iter()
        .map(|(label, acc)| {
            let n = acc.trades;
            Slice {
                label,
                order: acc.order,
                trades: n,
                wins: acc.wins,
                losses: acc.losses,
                breakevens: acc.breakevens,
                net: acc.net,
                gross_profit: acc.gross_profit,
                gross_loss: acc.gross_loss,
                win_rate: Metric::Value(acc.wins as f64 / n as f64),
                profit_factor: if acc.gross_loss.is_zero() {
                    Metric::Unavailable(Reason::NoLosses)
                } else {
                    super::metric::finite(
                        acc.gross_profit.to_f64().unwrap_or(f64::NAN)
                            / acc.gross_loss.to_f64().unwrap_or(f64::NAN),
                        Reason::NoLosses,
                    )
                },
                expectancy: Metric::Value(acc.net / Decimal::from(n)),
                average_r: if acc.r_values.is_empty() {
                    Metric::Unavailable(Reason::MissingStop)
                } else {
                    Metric::Value(acc.r_values.iter().sum::<f64>() / acc.r_values.len() as f64)
                },
                r_count: acc.r_values.len(),
                average_return: if acc.returns.is_empty() {
                    Metric::Unavailable(Reason::NoTrades)
                } else {
                    Metric::Value(acc.returns.iter().sum::<f64>() / acc.returns.len() as f64)
                },
                sample: sample_of(n),
            }
        })
        .collect();
    if dimension.natural_order() {
        out.sort_by(|a, b| (a.order, &a.label).cmp(&(b.order, &b.label)));
    } else {
        // Ranked by result, with "(none)" last.
        out.sort_by(|a, b| {
            (a.label == NONE_LABEL)
                .cmp(&(b.label == NONE_LABEL))
                .then(b.net.cmp(&a.net))
                .then(a.label.cmp(&b.label))
        });
    }
    out
}

#[cfg(test)]
mod tests;
