use rust_decimal::Decimal;

use super::*;
use crate::model::journal::testing::{d, exec};

fn avg(_: &str) -> CostBasis {
    CostBasis::AverageCost
}

fn one(executions: &[Execution]) -> Trade {
    let trades = reconstruct(executions, avg);
    assert_eq!(trades.len(), 1, "{trades:#?}");
    trades.into_iter().next().unwrap()
}

#[test]
fn one_entry_and_one_exit() {
    let trade = one(&[
        exec("1", Side::Buy, "1", "100", 1_000).fee("1"),
        exec("2", Side::Sell, "1", "110", 2_000).fee("1"),
    ]);
    assert_eq!(trade.status, TradeStatus::Closed);
    assert_eq!(trade.direction, Direction::Long);
    assert_eq!(trade.key, "acct|1");
    assert_eq!(trade.open_ts_ms, 1_000);
    assert_eq!(trade.close_ts_ms, Some(2_000));
    assert_eq!(trade.duration_ms(), Some(1_000));
    assert_eq!(trade.gross_pnl, d("10"));
    assert_eq!(trade.fees, d("2"));
    assert_eq!(trade.net_pnl(), d("8"));
    assert_eq!(trade.avg_entry, d("100"));
    assert_eq!(trade.avg_exit, Some(d("110")));
    assert_eq!(trade.open_quantity, Decimal::ZERO);
    assert_eq!(trade.open_fees, Decimal::ZERO);
    assert!(trade.fees_known);
}

#[test]
fn a_short_profits_when_price_falls() {
    let trade = one(&[
        exec("1", Side::Sell, "2", "50", 1_000),
        exec("2", Side::Buy, "2", "45", 2_000),
    ]);
    assert_eq!(trade.direction, Direction::Short);
    assert_eq!(trade.gross_pnl, d("10"));
}

/// Buy 1 @100, buy 0.5 @103, sell 0.7 @110, sell 0.8 @105: one flat-to-flat trade.
fn scaled() -> Vec<Execution> {
    vec![
        exec("1", Side::Buy, "1", "100", 1_000),
        exec("2", Side::Buy, "0.5", "103", 2_000),
        exec("3", Side::Sell, "0.7", "110", 3_000),
        exec("4", Side::Sell, "0.8", "105", 4_000),
    ]
}

#[test]
fn scaling_in_and_out_is_one_trade() {
    let trade = one(&scaled());
    assert_eq!(trade.legs.len(), 4);
    assert_eq!(trade.entry_quantity, d("1.5"));
    assert_eq!(trade.exit_quantity, d("1.5"));
    assert_eq!(trade.max_quantity, d("1.5"));
    // (1*100 + 0.5*103) / 1.5 = 101
    assert_eq!(trade.avg_entry.round_dp(8), d("101"));
    // (0.7*110 + 0.8*105) / 1.5 = 107.3333...
    assert_eq!(trade.avg_exit.unwrap().round_dp(4), d("107.3333"));
    // average cost: (110-101)*0.7 + (105-101)*0.8 = 6.3 + 3.2
    assert_eq!(trade.gross_pnl.round_dp(8), d("9.5"));
    assert_eq!(trade.legs[2].realized_gross.round_dp(8), d("6.3"));
}

#[test]
fn every_cost_basis_agrees_on_a_closed_trades_total() {
    let totals: Vec<_> = [CostBasis::AverageCost, CostBasis::Fifo, CostBasis::Lifo]
        .into_iter()
        .map(|basis| {
            let trades = reconstruct(&scaled(), |_| basis);
            assert_eq!(trades.len(), 1);
            (
                trades[0].gross_pnl.round_dp(8),
                trades[0].legs[2].realized_gross.round_dp(8),
            )
        })
        .collect();
    // Same total, different leg: average 6.3, FIFO 0.7*(110-100), LIFO 0.5*7 + 0.2*10.
    assert_eq!(totals[0], (d("9.5"), d("6.3")));
    assert_eq!(totals[1], (d("9.5"), d("7")));
    assert_eq!(totals[2], (d("9.5"), d("5.5")));
}

#[test]
fn a_reversal_splits_the_closing_and_opening_quantity() {
    // Long 1 @100 (fee 2), then sell 3 @110 (fee 3): closes 1, opens a short of 2.
    let trades = reconstruct(
        &[
            exec("1", Side::Buy, "1", "100", 1_000).fee("2"),
            exec("2", Side::Sell, "3", "110", 2_000).fee("3"),
            exec("3", Side::Buy, "2", "105", 3_000).fee("0"),
        ],
        avg,
    );
    assert_eq!(trades.len(), 2);
    let (long, short) = (&trades[0], &trades[1]);

    assert_eq!(long.direction, Direction::Long);
    assert_eq!(long.status, TradeStatus::Closed);
    assert_eq!(long.close_ts_ms, Some(2_000));
    assert_eq!(long.gross_pnl, d("10"));
    // entry fee 2 + the closing third of the sell's fee, 1
    assert_eq!(long.fees, d("3"));
    assert_eq!(long.net_pnl(), d("7"));

    assert_eq!(short.direction, Direction::Short);
    assert_eq!(short.key, "acct|2");
    assert_eq!(short.open_ts_ms, 2_000);
    assert_eq!(short.entry_quantity, d("2"));
    assert_eq!(short.avg_entry, d("110"));
    assert_eq!(short.gross_pnl, d("10"));
    // the opening two thirds of the sell's fee, 2
    assert_eq!(short.fees, d("2"));
    assert_eq!(short.net_pnl(), d("8"));
    // The split fees add back to the execution's fee exactly.
    let split: Decimal = long.legs[1].fee.unwrap() + short.legs[0].fee.unwrap();
    assert_eq!(split, d("3"));
}

#[test]
fn a_reversal_fee_that_does_not_divide_evenly_still_adds_up() {
    let trades = reconstruct(
        &[
            exec("1", Side::Buy, "1", "100", 1_000),
            exec("2", Side::Sell, "3", "110", 2_000).fee("1"),
        ],
        avg,
    );
    let closing = trades[0].legs[1].fee.unwrap();
    let opening = trades[1].legs[0].fee.unwrap();
    assert_eq!(closing + opening, d("1"));
}

#[test]
fn a_partly_closed_position_is_open_with_its_entry_fee_pro_rata() {
    let trade = one(&[
        exec("1", Side::Buy, "2", "100", 1_000).fee("2"),
        exec("2", Side::Sell, "0.5", "110", 2_000).fee("0.1"),
    ]);
    assert_eq!(trade.status, TradeStatus::Open);
    assert_eq!(trade.close_ts_ms, None);
    assert_eq!(trade.duration_ms(), None);
    assert_eq!(trade.open_quantity, d("1.5"));
    assert_eq!(trade.gross_pnl, d("5"));
    // a quarter of the entry fee (0.5) plus the exit fee
    assert_eq!(trade.fees, d("0.6"));
    assert_eq!(trade.open_fees, d("1.5"));
    assert_eq!(trade.avg_exit, Some(d("110")));
}

#[test]
fn an_entry_alone_is_an_open_trade_with_no_exit() {
    let trade = one(&[exec("1", Side::Buy, "1", "100", 1_000).fee("1")]);
    assert_eq!(trade.status, TradeStatus::Open);
    assert_eq!(trade.avg_exit, None);
    assert_eq!(trade.gross_pnl, Decimal::ZERO);
    assert_eq!(trade.fees, Decimal::ZERO);
    assert_eq!(trade.open_fees, d("1"));
}

#[test]
fn a_missing_fee_is_not_a_zero_fee() {
    let trade = one(&[
        exec("1", Side::Buy, "1", "100", 1_000).no_fee_figure(),
        exec("2", Side::Sell, "1", "110", 2_000).fee("1"),
    ]);
    assert!(!trade.fees_known);
    assert_eq!(trade.fees, d("1"));
}

#[test]
fn a_contract_multiplier_scales_the_result() {
    let trade = one(&[
        exec("1", Side::Buy, "2", "100", 1_000).times("50"),
        exec("2", Side::Sell, "2", "101", 2_000).times("50"),
    ]);
    assert_eq!(trade.multiplier, d("50"));
    assert_eq!(trade.gross_pnl, d("100"));
}

#[test]
fn executions_are_replayed_by_time_not_by_arrival() {
    let trade = one(&[
        exec("2", Side::Sell, "1", "110", 2_000),
        exec("1", Side::Buy, "1", "100", 1_000),
    ]);
    assert_eq!(trade.direction, Direction::Long);
    assert_eq!(trade.gross_pnl, d("10"));
}

#[test]
fn fills_with_the_same_time_price_and_size_stay_separate() {
    let trade = one(&[
        exec("1", Side::Buy, "1", "100", 1_000),
        exec("2", Side::Buy, "1", "100", 1_000),
        exec("3", Side::Sell, "2", "101", 2_000),
    ]);
    assert_eq!(trade.entry_quantity, d("2"));
    assert_eq!(trade.legs.len(), 3);
    assert_eq!(trade.gross_pnl, d("2"));
}

#[test]
fn a_flat_position_starts_a_new_trade() {
    let trades = reconstruct(
        &[
            exec("1", Side::Buy, "1", "100", 1_000),
            exec("2", Side::Sell, "1", "101", 2_000),
            exec("3", Side::Buy, "1", "102", 3_000),
            exec("4", Side::Sell, "1", "100", 4_000),
        ],
        avg,
    );
    assert_eq!(trades.len(), 2);
    assert_eq!(trades[0].gross_pnl, d("1"));
    assert_eq!(trades[1].gross_pnl, d("-2"));
    assert!(trades[0].open_ts_ms < trades[1].open_ts_ms);
}

#[test]
fn accounts_and_instruments_do_not_mix() {
    let trades = reconstruct(
        &[
            exec("1", Side::Buy, "1", "100", 1_000),
            exec("2", Side::Buy, "1", "100", 1_500).in_account("other"),
            exec("3", Side::Buy, "1", "10", 1_600).of("ETH"),
            exec("4", Side::Sell, "1", "110", 2_000),
        ],
        avg,
    );
    assert_eq!(trades.len(), 3);
    let closed: Vec<_> = trades
        .iter()
        .filter(|trade| trade.status == TradeStatus::Closed)
        .collect();
    assert_eq!(closed.len(), 1);
    assert_eq!(closed[0].account_uid, "acct");
    assert_eq!(closed[0].gross_pnl, d("10"));
}

#[test]
fn voided_and_empty_executions_are_ignored() {
    let mut voided = exec("2", Side::Sell, "1", "200", 1_500);
    voided.status = ExecutionStatus::Voided;
    let zero = exec("3", Side::Sell, "0", "300", 1_600);
    let trade = one(&[
        exec("1", Side::Buy, "1", "100", 1_000),
        voided,
        zero,
        exec("4", Side::Sell, "1", "110", 2_000),
    ]);
    assert_eq!(trade.gross_pnl, d("10"));
    assert_eq!(trade.legs.len(), 2);
}

#[test]
fn the_first_planned_stop_is_the_trades_stop() {
    let trade = one(&[
        exec("1", Side::Buy, "1", "100", 1_000).stop("95"),
        exec("2", Side::Buy, "1", "101", 1_500).stop("90"),
        exec("3", Side::Sell, "2", "110", 2_000),
    ]);
    assert_eq!(trade.initial_stop, Some(d("95")));
}

#[test]
fn reconstruction_is_deterministic() {
    let input = scaled();
    assert_eq!(reconstruct(&input, avg), reconstruct(&input, avg));
}

#[test]
fn nothing_in_nothing_out() {
    assert!(reconstruct(&[], avg).is_empty());
}
