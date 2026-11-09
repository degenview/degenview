//! The trade table's columns and ordering. Pure: the view asks for an ordered list of row indices
//! and formats the visible rows itself.

use std::cmp::Ordering;
use std::collections::HashMap;

use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;
use serde::{Deserialize, Serialize};

use super::filters::{OutcomeFilter, outcome_of};
use super::model::{Direction, Trade};
use super::reviews::{Review, status_of};
use super::risk;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Column {
    Open,
    Close,
    Symbol,
    Account,
    Side,
    Status,
    Quantity,
    Entry,
    Exit,
    Gross,
    Fees,
    Net,
    ReturnPct,
    R,
    Duration,
    Strategy,
    Tags,
    Review,
}

impl Column {
    pub const ALL: [Column; 18] = [
        Self::Open,
        Self::Close,
        Self::Symbol,
        Self::Account,
        Self::Side,
        Self::Status,
        Self::Quantity,
        Self::Entry,
        Self::Exit,
        Self::Gross,
        Self::Fees,
        Self::Net,
        Self::ReturnPct,
        Self::R,
        Self::Duration,
        Self::Strategy,
        Self::Tags,
        Self::Review,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Open => "Opened",
            Self::Close => "Closed",
            Self::Symbol => "Symbol",
            Self::Account => "Account",
            Self::Side => "Side",
            Self::Status => "Status",
            Self::Quantity => "Quantity",
            Self::Entry => "Avg entry",
            Self::Exit => "Avg exit",
            Self::Gross => "Gross P&L",
            Self::Fees => "Fees",
            Self::Net => "Net P&L",
            Self::ReturnPct => "Return",
            Self::R => "R",
            Self::Duration => "Duration",
            Self::Strategy => "Strategy",
            Self::Tags => "Tags",
            Self::Review => "Review",
        }
    }

    pub fn default_visible(self) -> bool {
        !matches!(
            self,
            Self::Account
                | Self::Gross
                | Self::Fees
                | Self::ReturnPct
                | Self::Strategy
                | Self::Tags
        )
    }

    pub fn default_width(self) -> f32 {
        match self {
            Self::Open | Self::Close => 150.0,
            Self::Symbol => 120.0,
            Self::Account => 130.0,
            Self::Side | Self::Status => 84.0,
            Self::Quantity | Self::Entry | Self::Exit => 104.0,
            Self::Gross | Self::Fees | Self::Net => 104.0,
            Self::ReturnPct | Self::R | Self::Duration => 84.0,
            Self::Strategy => 130.0,
            Self::Tags => 150.0,
            Self::Review => 90.0,
        }
    }

    pub fn min_width(self) -> f32 {
        56.0
    }

    pub fn right_aligned(self) -> bool {
        matches!(
            self,
            Self::Quantity
                | Self::Entry
                | Self::Exit
                | Self::Gross
                | Self::Fees
                | Self::Net
                | Self::ReturnPct
                | Self::R
                | Self::Duration
        )
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Sort {
    pub column: Column,
    pub descending: bool,
}

impl Default for Sort {
    fn default() -> Self {
        Self {
            column: Column::Open,
            descending: true,
        }
    }
}

impl Sort {
    /// A click on a header: the same column flips direction, another starts descending for
    /// figures and ascending for text.
    pub fn toggled(self, column: Column) -> Self {
        if self.column == column {
            Self {
                column,
                descending: !self.descending,
            }
        } else {
            Self {
                column,
                descending: column.right_aligned()
                    || matches!(column, Column::Open | Column::Close),
            }
        }
    }
}

/// Return on the entry notional: net P&L over `entry quantity x average entry x multiplier`. Not a
/// return on account equity or margin.
pub fn return_fraction(trade: &Trade) -> Option<f64> {
    let notional = trade.entry_quantity * trade.avg_entry * trade.multiplier;
    if notional <= Decimal::ZERO {
        return None;
    }
    (trade.net_pnl() / notional)
        .to_f64()
        .filter(|v| v.is_finite())
}

/// Everything the ordering reads besides the trades.
pub struct SortContext<'a> {
    pub reviews: &'a HashMap<String, Review>,
    pub playbook_names: &'a HashMap<String, String>,
    pub account_names: &'a HashMap<String, String>,
    pub breakeven_tolerance: Decimal,
}

fn compare_f64(a: Option<f64>, b: Option<f64>) -> Ordering {
    match (a, b) {
        (Some(a), Some(b)) => a.partial_cmp(&b).unwrap_or(Ordering::Equal),
        (Some(_), None) => Ordering::Greater,
        (None, Some(_)) => Ordering::Less,
        (None, None) => Ordering::Equal,
    }
}

fn compare(column: Column, a: &Trade, b: &Trade, context: &SortContext<'_>) -> Ordering {
    let review = |trade: &Trade| context.reviews.get(&trade.key);
    match column {
        Column::Open => a.open_ts_ms.cmp(&b.open_ts_ms),
        Column::Close => a.close_ts_ms.cmp(&b.close_ts_ms),
        Column::Symbol => a
            .instrument
            .symbol
            .to_lowercase()
            .cmp(&b.instrument.symbol.to_lowercase()),
        Column::Account => {
            let name = |t: &Trade| {
                context
                    .account_names
                    .get(&t.account_uid)
                    .map(|n| n.to_lowercase())
                    .unwrap_or_default()
            };
            name(a).cmp(&name(b))
        }
        Column::Side => (a.direction == Direction::Short).cmp(&(b.direction == Direction::Short)),
        Column::Status => {
            let rank = |t: &Trade| match outcome_of(t, context.breakeven_tolerance) {
                OutcomeFilter::Open => 0,
                OutcomeFilter::Win => 1,
                OutcomeFilter::Breakeven => 2,
                OutcomeFilter::Loss => 3,
            };
            rank(a).cmp(&rank(b))
        }
        Column::Quantity => a.entry_quantity.cmp(&b.entry_quantity),
        Column::Entry => a.avg_entry.cmp(&b.avg_entry),
        Column::Exit => a.avg_exit.cmp(&b.avg_exit),
        Column::Gross => a.gross_pnl.cmp(&b.gross_pnl),
        Column::Fees => a.fees.cmp(&b.fees),
        Column::Net => a.net_pnl().cmp(&b.net_pnl()),
        Column::ReturnPct => compare_f64(return_fraction(a), return_fraction(b)),
        Column::R => {
            let r = |t: &Trade| {
                risk::r_multiple(t, review(t).and_then(|r| r.stop))
                    .into_value()
                    .and_then(|r| r.to_f64())
            };
            compare_f64(r(a), r(b))
        }
        Column::Duration => a.duration_ms().cmp(&b.duration_ms()),
        Column::Strategy => {
            let name = |t: &Trade| {
                review(t)
                    .and_then(|r| r.playbook.as_ref())
                    .and_then(|id| context.playbook_names.get(id))
                    .map(|n| n.to_lowercase())
                    .unwrap_or_default()
            };
            name(a).cmp(&name(b))
        }
        Column::Tags => review(a)
            .map(|r| r.tags.join(",").to_lowercase())
            .unwrap_or_default()
            .cmp(
                &review(b)
                    .map(|r| r.tags.join(",").to_lowercase())
                    .unwrap_or_default(),
            ),
        Column::Review => (status_of(review(a)) as u8).cmp(&(status_of(review(b)) as u8)),
    }
}

/// `indices` (positions in `trades`) ordered by `sort`; ties by open time then key, so the order
/// is total and stable between frames.
pub fn sort_indices(
    indices: &mut [usize],
    trades: &[Trade],
    sort: Sort,
    context: &SortContext<'_>,
) {
    indices.sort_by(|&a, &b| {
        let (ta, tb) = (&trades[a], &trades[b]);
        let primary = compare(sort.column, ta, tb, context);
        let primary = if sort.descending {
            primary.reverse()
        } else {
            primary
        };
        primary
            .then_with(|| tb.open_ts_ms.cmp(&ta.open_ts_ms))
            .then_with(|| ta.key.cmp(&tb.key))
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::journal::model::{CostBasis, Side};
    use crate::model::journal::reconstruct::reconstruct;
    use crate::model::journal::testing::{d, exec};

    fn trades() -> Vec<Trade> {
        reconstruct(
            &[
                exec("a1", Side::Buy, "1", "100", 1_000),
                exec("a2", Side::Sell, "1", "110", 3_000),
                exec("b1", Side::Sell, "2", "50", 2_000)
                    .of("ETH")
                    .stop("55"),
                exec("b2", Side::Buy, "2", "55", 2_500).of("ETH"),
                exec("c1", Side::Buy, "1", "20", 4_000).of("SOL"),
            ],
            |_| CostBasis::AverageCost,
        )
    }

    fn order(sort: Sort) -> Vec<String> {
        let trades = trades();
        let reviews = HashMap::new();
        let names = HashMap::new();
        let context = SortContext {
            reviews: &reviews,
            playbook_names: &HashMap::new(),
            account_names: &names,
            breakeven_tolerance: Decimal::ZERO,
        };
        let mut indices: Vec<usize> = (0..trades.len()).collect();
        sort_indices(&mut indices, &trades, sort, &context);
        indices
            .into_iter()
            .map(|i| trades[i].instrument.symbol.clone())
            .collect()
    }

    #[test]
    fn the_default_order_is_newest_first() {
        assert_eq!(order(Sort::default()), ["SOL", "ETH", "BTC"]);
    }

    #[test]
    fn figures_sort_numerically_and_directions_flip() {
        // net: BTC +10, ETH -10 (short 2 sold 50 bought 55), SOL open 0.
        let asc = Sort {
            column: Column::Net,
            descending: false,
        };
        assert_eq!(order(asc), ["ETH", "SOL", "BTC"]);
        assert_eq!(
            order(Sort {
                descending: true,
                ..asc
            }),
            ["BTC", "SOL", "ETH"]
        );
    }

    #[test]
    fn text_and_status_columns_sort() {
        assert_eq!(
            order(Sort {
                column: Column::Symbol,
                descending: false
            }),
            ["BTC", "ETH", "SOL"]
        );
        // Open first, then wins, then losses.
        assert_eq!(
            order(Sort {
                column: Column::Status,
                descending: false
            }),
            ["SOL", "BTC", "ETH"]
        );
    }

    #[test]
    fn missing_values_sort_below_present_ones() {
        // Exit: SOL (open) has none.
        assert_eq!(
            order(Sort {
                column: Column::Exit,
                descending: true
            }),
            ["BTC", "ETH", "SOL"]
        );
        // R: only ETH has a stop, so it is the only one with an R.
        assert_eq!(
            order(Sort {
                column: Column::R,
                descending: true
            })[0],
            "ETH"
        );
    }

    #[test]
    fn a_header_click_flips_or_starts_sensibly() {
        let sort = Sort::default();
        assert!(!sort.toggled(Column::Open).descending);
        assert!(sort.toggled(Column::Net).descending);
        assert!(!sort.toggled(Column::Symbol).descending);
        assert_eq!(sort.toggled(Column::Net).column, Column::Net);
    }

    #[test]
    fn return_is_a_fraction_of_entry_notional() {
        let trades = trades();
        let btc = trades
            .iter()
            .find(|t| t.instrument.symbol == "BTC")
            .unwrap();
        assert_eq!(return_fraction(btc), Some(0.1));
        let open = trades
            .iter()
            .find(|t| t.instrument.symbol == "SOL")
            .unwrap();
        assert_eq!(return_fraction(open), Some(0.0));
        let _ = d("1");
    }

    #[test]
    fn the_order_is_total_for_equal_keys() {
        let trades = reconstruct(
            &[
                exec("1", Side::Buy, "1", "10", 1_000),
                exec("2", Side::Buy, "1", "10", 1_000).of("ETH"),
            ],
            |_| CostBasis::AverageCost,
        );
        let reviews = HashMap::new();
        let names = HashMap::new();
        let context = SortContext {
            reviews: &reviews,
            playbook_names: &HashMap::new(),
            account_names: &names,
            breakeven_tolerance: Decimal::ZERO,
        };
        let sort = Sort {
            column: Column::Quantity,
            descending: false,
        };
        let mut a: Vec<usize> = vec![0, 1];
        let mut b: Vec<usize> = vec![1, 0];
        sort_indices(&mut a, &trades, sort, &context);
        sort_indices(&mut b, &trades, sort, &context);
        assert_eq!(a, b);
    }
}
