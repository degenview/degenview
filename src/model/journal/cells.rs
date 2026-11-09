//! The text of one table cell. Pure so the table, the exports and the tests agree on what a trade
//! looks like in each column.

use std::collections::HashMap;

use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;

use super::calendar::Offset;
use super::filters::{OutcomeFilter, outcome_of};
use super::format;
use super::metric::Metric;
use super::model::{Direction, Trade};
use super::reviews::{Review, status_of};
use super::risk;
use super::table::{Column, return_fraction};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Tone {
    Normal,
    Positive,
    Negative,
    Muted,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Cell {
    pub text: String,
    pub tone: Tone,
    /// Drawn as a small capsule (side, status, review).
    pub pill: bool,
}

impl Cell {
    fn plain(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            tone: Tone::Normal,
            pill: false,
        }
    }

    fn muted(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            tone: Tone::Muted,
            pill: false,
        }
    }

    fn toned(text: impl Into<String>, tone: Tone) -> Self {
        Self {
            text: text.into(),
            tone,
            pill: false,
        }
    }

    fn pill(text: impl Into<String>, tone: Tone) -> Self {
        Self {
            text: text.into(),
            tone,
            pill: true,
        }
    }
}

pub struct CellContext<'a> {
    pub reviews: &'a HashMap<String, Review>,
    pub account_names: &'a HashMap<String, String>,
    pub breakeven_tolerance: Decimal,
    pub offset: Offset<'a>,
}

/// The dash shown where a figure does not exist.
pub const NONE: &str = "\u{2014}";

pub fn tone_of(value: Decimal) -> Tone {
    match value.cmp(&Decimal::ZERO) {
        std::cmp::Ordering::Greater => Tone::Positive,
        std::cmp::Ordering::Less => Tone::Negative,
        std::cmp::Ordering::Equal => Tone::Normal,
    }
}

pub fn status_label(outcome: OutcomeFilter) -> &'static str {
    match outcome {
        OutcomeFilter::Open => "OPEN",
        OutcomeFilter::Win => "WIN",
        OutcomeFilter::Loss => "LOSS",
        OutcomeFilter::Breakeven => "BREAKEVEN",
    }
}

pub fn status_tone(outcome: OutcomeFilter) -> Tone {
    match outcome {
        OutcomeFilter::Win => Tone::Positive,
        OutcomeFilter::Loss => Tone::Negative,
        OutcomeFilter::Open | OutcomeFilter::Breakeven => Tone::Muted,
    }
}

pub fn cell(column: Column, trade: &Trade, context: &CellContext<'_>) -> Cell {
    let review = context.reviews.get(&trade.key);
    let open = trade.is_open();
    match column {
        Column::Open => Cell::plain(format::datetime(trade.open_ts_ms, context.offset)),
        Column::Close => match trade.close_ts_ms {
            Some(close) => Cell::plain(format::datetime(close, context.offset)),
            None => Cell::muted(NONE),
        },
        Column::Symbol => Cell::plain(trade.instrument.display_name.clone()),
        Column::Account => Cell::plain(
            context
                .account_names
                .get(&trade.account_uid)
                .cloned()
                .unwrap_or_else(|| trade.account_uid.clone()),
        ),
        Column::Side => match trade.direction {
            Direction::Long => Cell::pill("LONG", Tone::Positive),
            Direction::Short => Cell::pill("SHORT", Tone::Negative),
        },
        Column::Status => {
            let outcome = outcome_of(trade, context.breakeven_tolerance);
            Cell::pill(status_label(outcome), status_tone(outcome))
        }
        Column::Quantity => Cell::plain(format::quantity(trade.entry_quantity)),
        Column::Entry => Cell::plain(format::price(trade.avg_entry)),
        Column::Exit => match trade.avg_exit {
            Some(exit) => Cell::plain(format::price(exit)),
            None => Cell::muted(NONE),
        },
        Column::Gross => {
            if open && trade.exit_quantity.is_zero() {
                Cell::muted(NONE)
            } else {
                Cell::toned(
                    format::signed_money(trade.gross_pnl),
                    tone_of(trade.gross_pnl),
                )
            }
        }
        Column::Fees => {
            let text = format::money(trade.fees);
            Cell::muted(if trade.fees_known {
                text
            } else {
                format!("\u{2265} {text}")
            })
        }
        Column::Net => {
            if open && trade.exit_quantity.is_zero() {
                Cell::muted(NONE)
            } else {
                Cell::toned(
                    format::signed_money(trade.net_pnl()),
                    tone_of(trade.net_pnl()),
                )
            }
        }
        Column::ReturnPct => match return_fraction(trade) {
            Some(fraction) if !(open && trade.exit_quantity.is_zero()) => Cell::toned(
                format::signed_percent(fraction),
                if fraction > 0.0 {
                    Tone::Positive
                } else if fraction < 0.0 {
                    Tone::Negative
                } else {
                    Tone::Normal
                },
            ),
            _ => Cell::muted(NONE),
        },
        Column::R => match risk::r_multiple(trade, review.and_then(|r| r.stop)) {
            Metric::Value(r) => {
                let r = r.to_f64().unwrap_or(f64::NAN);
                Cell::toned(
                    format::r_multiple(r),
                    if r > 0.0 {
                        Tone::Positive
                    } else if r < 0.0 {
                        Tone::Negative
                    } else {
                        Tone::Normal
                    },
                )
            }
            Metric::Unavailable(_) => Cell::muted(NONE),
        },
        Column::Duration => match trade.duration_ms() {
            Some(ms) => Cell::plain(format::duration(ms)),
            None => Cell::muted(NONE),
        },
        Column::Strategy => match review
            .map(|r| r.strategy.as_str())
            .filter(|s| !s.is_empty())
        {
            Some(strategy) => Cell::plain(strategy),
            None => Cell::muted(NONE),
        },
        Column::Tags => match review.map(|r| r.tags.join(", ")).filter(|t| !t.is_empty()) {
            Some(tags) => Cell::plain(tags),
            None => Cell::muted(NONE),
        },
        Column::Review => {
            let status = status_of(review);
            Cell::pill(
                status.label(),
                match status {
                    super::reviews::ReviewStatus::Reviewed => Tone::Positive,
                    _ => Tone::Muted,
                },
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::journal::calendar::utc;
    use crate::model::journal::model::{CostBasis, Side};
    use crate::model::journal::reconstruct::reconstruct;
    use crate::model::journal::testing::{d, exec};

    fn trade(executions: &[crate::model::journal::model::Execution]) -> Trade {
        reconstruct(executions, |_| CostBasis::AverageCost).remove(0)
    }

    fn context<'a>(
        reviews: &'a HashMap<String, Review>,
        names: &'a HashMap<String, String>,
    ) -> CellContext<'a> {
        CellContext {
            reviews,
            account_names: names,
            breakeven_tolerance: Decimal::ZERO,
            offset: &utc,
        }
    }

    #[test]
    fn a_closed_winning_long_reads_as_one_row() {
        let t = trade(&[
            exec("1", Side::Buy, "2", "1000.5", 1_700_000_000_000)
                .stop("990.5")
                .fee("1"),
            exec("2", Side::Sell, "2", "1010.5", 1_700_000_090_000).fee("1"),
        ]);
        let reviews = HashMap::new();
        let names = HashMap::from([("acct".to_owned(), "Main".to_owned())]);
        let c = context(&reviews, &names);
        let text = |column| cell(column, &t, &c).text;
        assert_eq!(text(Column::Open), "Nov 14, 2023 22:13");
        assert_eq!(text(Column::Close), "Nov 14, 2023 22:14");
        assert_eq!(text(Column::Symbol), "BTC");
        assert_eq!(text(Column::Account), "Main");
        assert_eq!(text(Column::Side), "LONG");
        assert_eq!(text(Column::Status), "WIN");
        assert_eq!(text(Column::Quantity), "2");
        assert_eq!(text(Column::Entry), "1,000.5");
        assert_eq!(text(Column::Exit), "1,010.5");
        assert_eq!(text(Column::Gross), "+20.00");
        assert_eq!(text(Column::Fees), "2.00");
        assert_eq!(text(Column::Net), "+18.00");
        assert_eq!(text(Column::Duration), "1m 30s");
        assert_eq!(text(Column::R), "+0.90R");
        assert_eq!(text(Column::Review), "Pending");
        assert_eq!(cell(Column::Net, &t, &c).tone, Tone::Positive);
        assert!(cell(Column::Side, &t, &c).pill);
        assert!(text(Column::ReturnPct).starts_with('+'));
    }

    #[test]
    fn an_open_trade_shows_no_result_or_exit() {
        let t = trade(&[exec("1", Side::Buy, "1", "100", 1_000)]);
        let (reviews, names) = (HashMap::new(), HashMap::new());
        let c = context(&reviews, &names);
        for column in [
            Column::Close,
            Column::Exit,
            Column::Gross,
            Column::Net,
            Column::ReturnPct,
            Column::Duration,
            Column::R,
        ] {
            assert_eq!(cell(column, &t, &c).text, NONE, "{column:?}");
        }
        assert_eq!(cell(Column::Status, &t, &c).text, "OPEN");
        // Unknown account falls back to its uid rather than a blank.
        assert_eq!(cell(Column::Account, &t, &c).text, "acct");
    }

    #[test]
    fn a_partly_closed_trade_shows_what_is_realised_so_far() {
        let t = trade(&[
            exec("1", Side::Buy, "2", "100", 1_000),
            exec("2", Side::Sell, "1", "110", 2_000),
        ]);
        let (reviews, names) = (HashMap::new(), HashMap::new());
        let c = context(&reviews, &names);
        assert_eq!(cell(Column::Net, &t, &c).text, "+10.00");
        assert_eq!(cell(Column::Status, &t, &c).text, "OPEN");
    }

    #[test]
    fn fees_with_an_unknown_part_are_a_floor() {
        let t = trade(&[
            exec("1", Side::Buy, "1", "100", 1_000).no_fee_figure(),
            exec("2", Side::Sell, "1", "110", 2_000).fee("1"),
        ]);
        let (reviews, names) = (HashMap::new(), HashMap::new());
        assert_eq!(
            cell(Column::Fees, &t, &context(&reviews, &names)).text,
            "\u{2265} 1.00"
        );
    }

    #[test]
    fn review_fields_show_in_their_columns_and_a_manual_stop_gives_r() {
        let t = trade(&[
            exec("1", Side::Buy, "1", "100", 1_000),
            exec("2", Side::Sell, "1", "110", 2_000),
        ]);
        let mut review = Review::new(&t.key, "acct");
        review.strategy = "Breakout".into();
        review.tags = vec!["a".into(), "b".into()];
        review.stop = Some(d("95"));
        review.reviewed = true;
        let reviews = HashMap::from([(t.key.clone(), review)]);
        let names = HashMap::new();
        let c = context(&reviews, &names);
        assert_eq!(cell(Column::Strategy, &t, &c).text, "Breakout");
        assert_eq!(cell(Column::Tags, &t, &c).text, "a, b");
        assert_eq!(cell(Column::R, &t, &c).text, "+2.00R");
        assert_eq!(cell(Column::Review, &t, &c).text, "Reviewed");
        assert_eq!(cell(Column::Review, &t, &c).tone, Tone::Positive);
    }

    #[test]
    fn a_loss_is_negative_and_marked_so_by_its_sign() {
        let t = trade(&[
            exec("1", Side::Buy, "1", "100", 1_000),
            exec("2", Side::Sell, "1", "90", 2_000),
        ]);
        let (reviews, names) = (HashMap::new(), HashMap::new());
        let c = context(&reviews, &names);
        assert_eq!(cell(Column::Net, &t, &c).text, "-10.00");
        assert_eq!(cell(Column::Net, &t, &c).tone, Tone::Negative);
        assert_eq!(cell(Column::Status, &t, &c).text, "LOSS");
    }
}
