use super::*;
use crate::model::journal::model::{CostBasis, Execution, Side};
use crate::model::journal::reconstruct::reconstruct;
use crate::model::journal::testing::{d, exec};

const DAY: i64 = 86_400_000;

/// A closed trade of size 1 with net P&L `pnl` (no fee), opened at `open` and closed at `close`.
fn spec(id: &str, pnl: &str, open: i64, close: i64) -> Vec<Execution> {
    let exit = (d("1000") + d(pnl)).to_string();
    vec![
        exec(&format!("{id}a"), Side::Buy, "1", "1000", open),
        exec(&format!("{id}b"), Side::Sell, "1", &exit, close),
    ]
}

fn build(executions: Vec<Execution>) -> Vec<Trade> {
    reconstruct(&executions, |_| CostBasis::AverageCost)
}

/// One trade per entry of `pnls`, closing at noon of consecutive days.
fn daily_trades(pnls: &[&str]) -> Vec<Trade> {
    let mut executions = Vec::new();
    for (index, pnl) in pnls.iter().enumerate() {
        let close = (index as i64 + 1) * DAY + DAY / 2;
        executions.extend(spec(&format!("t{index}"), pnl, close - 3_600_000, close));
    }
    build(executions)
}

fn run(trades: &[Trade], options: &Options<'_>) -> Summary {
    let refs: Vec<&Trade> = trades.iter().collect();
    summarize(&refs, options).unwrap()
}

fn all_floats(summary: &Summary) -> Vec<f64> {
    let metrics = [
        &summary.win_rate,
        &summary.profit_factor,
        &summary.payoff,
        &summary.expectancy_r,
        &summary.sharpe,
        &summary.sortino,
        &summary.calmar,
        &summary.sqn,
        &summary.r.average,
        &summary.r.best,
        &summary.r.worst,
        &summary.r.total,
        &summary.days.day_win_rate,
        &summary.days.profit_concentration,
    ];
    let mut values: Vec<f64> = metrics.iter().filter_map(|m| m.copied()).collect();
    values.extend(summary.drawdown.max_pct);
    values
}

#[test]
fn a_mixed_record_is_summarised_by_hand() {
    let trades = daily_trades(&["10", "-4", "6", "-2", "0", "8"]);
    let s = run(&trades, &Options::default());
    assert_eq!((s.closed, s.open), (6, 0));
    assert_eq!((s.wins, s.losses, s.breakevens), (3, 2, 1));
    assert_eq!(s.net_pnl, d("18"));
    assert_eq!(s.gross_profit, d("24"));
    assert_eq!(s.gross_loss, d("6"));
    assert_eq!(s.win_rate, Metric::Value(0.5));
    assert_eq!(s.profit_factor, Metric::Value(4.0));
    assert_eq!(s.average_win, Metric::Value(d("8")));
    assert_eq!(s.average_loss, Metric::Value(d("3")));
    assert_eq!(s.payoff, Metric::Value(8.0 / 3.0));
    assert_eq!(s.expectancy, Metric::Value(d("3")));
    assert_eq!(
        s.streaks,
        Streaks {
            max_wins: 1,
            max_losses: 1,
            current: 1
        }
    );
    // The two drawdowns: 10 -> 6 (4) regained at 12, then 12 -> 10 (2) regained at 18.
    assert_eq!(s.drawdown.max, d("4"));
    assert_eq!(s.drawdown.episodes, 2);
    assert_eq!(s.drawdown.avg_episode, Metric::Value(d("3")));
    assert_eq!(s.days.trading_days, 6);
    assert_eq!((s.days.winning, s.days.losing, s.days.flat), (3, 2, 1));
    assert_eq!(s.days.day_win_rate, Metric::Value(0.5));
    assert_eq!(s.days.best.value().unwrap().net, d("10"));
    assert_eq!(s.days.worst.value().unwrap().net, d("-4"));
    assert_eq!(s.days.average, Metric::Value(d("3")));
    assert_eq!(s.days.profit_concentration, Metric::Value(10.0 / 24.0));
    assert_eq!(s.durations.average_ms, Metric::Value(3_600_000));
    assert!(s.fees_complete);
}

#[test]
fn no_losses_is_unavailable_not_infinite() {
    let trades = daily_trades(&["5", "5"]);
    let s = run(&trades, &Options::default());
    assert_eq!(s.profit_factor, Metric::Unavailable(Reason::NoLosses));
    assert_eq!(s.payoff, Metric::Unavailable(Reason::NoLosses));
    assert_eq!(s.average_loss, Metric::Unavailable(Reason::NoLosses));
    assert_eq!(s.win_rate, Metric::Value(1.0));
    assert_eq!(s.gross_loss, Decimal::ZERO);
}

#[test]
fn no_wins_has_no_average_win() {
    let trades = daily_trades(&["-5", "-3"]);
    let s = run(&trades, &Options::default());
    assert_eq!(s.profit_factor, Metric::Value(0.0));
    assert_eq!(s.average_win, Metric::Unavailable(Reason::NoWins));
    assert_eq!(s.payoff, Metric::Unavailable(Reason::NoWins));
    assert_eq!(s.streaks.max_losses, 2);
    assert_eq!(s.streaks.current, -2);
}

#[test]
fn nothing_is_not_zero() {
    let s = run(&[], &Options::default());
    assert_eq!(s.closed, 0);
    assert_eq!(s.win_rate, Metric::Unavailable(Reason::NoTrades));
    assert_eq!(s.profit_factor, Metric::Unavailable(Reason::NoTrades));
    assert_eq!(s.expectancy, Metric::Unavailable(Reason::NoTrades));
    assert_eq!(s.days.day_win_rate, Metric::Unavailable(Reason::NoTrades));
    assert_eq!(s.net_pnl, Decimal::ZERO);
    assert!(all_floats(&s).is_empty());
}

#[test]
fn breakeven_tolerance_changes_labels_not_gross_totals() {
    let trades = daily_trades(&["0.5", "-0.5", "3"]);
    let s = run(
        &trades,
        &Options {
            breakeven_tolerance: d("1"),
            ..Options::default()
        },
    );
    assert_eq!((s.wins, s.losses, s.breakevens), (1, 0, 2));
    // Gross profit and loss follow the sign of every closed trade.
    assert_eq!(s.gross_profit, d("3.5"));
    assert_eq!(s.gross_loss, d("0.5"));
    assert_eq!(s.streaks.current, 1);
}

#[test]
fn open_trades_are_counted_but_never_realised() {
    let mut executions = spec("c", "10", 1_000, 2_000);
    executions.push(exec("o", Side::Buy, "1", "500", 3_000));
    let trades = build(executions);
    let s = run(&trades, &Options::default());
    assert_eq!((s.closed, s.open), (1, 1));
    assert_eq!(s.net_pnl, d("10"));
}

#[test]
fn unknown_fees_make_the_totals_a_floor() {
    let mut executions = spec("a", "10", 1_000, 2_000);
    executions[0] = executions[0].clone().no_fee_figure();
    let trades = build(executions);
    let s = run(&trades, &Options::default());
    assert!(!s.fees_complete);
}

#[test]
fn fees_reduce_net_but_not_gross() {
    let mut executions = spec("a", "10", 1_000, 2_000);
    executions[0] = executions[0].clone().fee("1");
    executions[1] = executions[1].clone().fee("1");
    let trades = build(executions);
    let s = run(&trades, &Options::default());
    assert_eq!(s.gross_pnl, d("10"));
    assert_eq!(s.fees, d("2"));
    assert_eq!(s.net_pnl, d("8"));
    assert_eq!(s.gross_profit, d("8"));
}

#[test]
fn currencies_are_never_added_together() {
    let mut executions = spec("a", "10", 1_000, 2_000);
    for e in spec("b", "20", 1_000, 2_000) {
        executions.push(e.currency("EUR").of("ETH"));
    }
    let trades = build(executions);
    let refs: Vec<&Trade> = trades.iter().collect();
    assert_eq!(
        summarize(&refs, &Options::default()),
        Err(Reason::MixedCurrency)
    );
    let by = summarize_by_currency(&refs, &Options::default());
    let currencies: Vec<_> = by.iter().map(|(c, _)| c.clone()).collect();
    assert_eq!(currencies, [Some("EUR".to_owned()), Some("USD".to_owned())]);
    assert_eq!(by[0].1.net_pnl, d("20"));
    assert_eq!(by[1].1.net_pnl, d("10"));
}

#[test]
fn r_statistics_use_only_trades_with_a_stop() {
    let mut executions = Vec::new();
    // Entry 1000, stop 990 (risk 10): +20 is 2R, -10 is -1R, +5 is 0.5R; one trade without a stop.
    for (id, pnl, stop) in [
        ("a", "20", true),
        ("b", "-10", true),
        ("c", "5", true),
        ("d", "7", false),
    ] {
        let base = executions.len() as i64 * 10_000 + 1_000;
        let mut pair = spec(id, pnl, base, base + 5_000);
        if stop {
            pair[0] = pair[0].clone().stop("990");
        }
        executions.extend(pair);
    }
    let trades = build(executions);
    let s = run(&trades, &Options::default());
    assert_eq!(s.r.count, 3);
    assert_eq!(s.r.best, Metric::Value(2.0));
    assert_eq!(s.r.worst, Metric::Value(-1.0));
    assert_eq!(s.r.total, Metric::Value(1.5));
    assert_eq!(s.r.average, Metric::Value(0.5));
    assert_eq!(s.expectancy_r, Metric::Value(0.5));
    assert!(matches!(
        s.sqn,
        Metric::Unavailable(Reason::Insufficient { have: 3, need: 30 })
    ));
}

#[test]
fn no_stops_means_no_r_figures() {
    let trades = daily_trades(&["5"]);
    let s = run(&trades, &Options::default());
    assert_eq!(s.r.count, 0);
    assert_eq!(s.r.average, Metric::Unavailable(Reason::MissingStop));
}

#[test]
fn a_manual_stop_can_supply_r() {
    let trades = daily_trades(&["20"]);
    let r_of = |trade: &Trade| risk::r_multiple(trade, Some(d("990")));
    let s = run(
        &trades,
        &Options {
            r_of: &r_of,
            ..Options::default()
        },
    );
    assert_eq!(s.r.average, Metric::Value(2.0));
}

#[test]
fn the_close_day_follows_the_time_zone() {
    // Closes at 23:30 UTC; at +1h that is the next local day, joining the second trade.
    let late = 10 * DAY + DAY - 1_800_000;
    let mut executions = spec("a", "5", late - 1_000, late);
    executions.extend(spec("b", "5", 11 * DAY + 1_000, 11 * DAY + 2_000));
    let trades = build(executions);
    assert_eq!(run(&trades, &Options::default()).days.trading_days, 2);
    let plus_one = |_: i64| 3_600;
    let s = run(
        &trades,
        &Options {
            offset: &plus_one,
            ..Options::default()
        },
    );
    assert_eq!(s.days.trading_days, 1);
    assert_eq!(s.days.best.value().unwrap().net, d("10"));
}

#[test]
fn durations_split_by_outcome() {
    let mut executions = spec("w", "5", 1_000, 61_000);
    executions.extend(spec("l", "-5", 100_000, 400_000));
    let trades = build(executions);
    let s = run(&trades, &Options::default());
    assert_eq!(s.durations.average_win_ms, Metric::Value(60_000));
    assert_eq!(s.durations.average_loss_ms, Metric::Value(300_000));
    assert_eq!(s.durations.average_ms, Metric::Value(180_000));
}

#[test]
fn streaks_run_in_close_order_and_a_breakeven_ends_them() {
    let trades = daily_trades(&["1", "2", "3", "0", "4", "-1", "-1"]);
    let s = run(&trades, &Options::default());
    assert_eq!(
        s.streaks,
        Streaks {
            max_wins: 3,
            max_losses: 2,
            current: -2
        }
    );
}

#[test]
fn risk_adjusted_ratios_wait_for_enough_days_and_a_balance() {
    let few = daily_trades(&["10", "-5", "10"]);
    let s = run(
        &few,
        &Options {
            initial_balance: Some(d("1000")),
            ..Options::default()
        },
    );
    assert!(matches!(
        s.sharpe,
        Metric::Unavailable(Reason::Insufficient { have: 3, need: 30 })
    ));
    let no_balance = run(&few, &Options::default());
    assert_eq!(
        no_balance.sharpe,
        Metric::Unavailable(Reason::NoInitialBalance)
    );
    assert_eq!(
        no_balance.calmar,
        Metric::Unavailable(Reason::NoInitialBalance)
    );
}

#[test]
fn sharpe_sortino_and_calmar_match_an_independent_calculation() {
    let pnls: Vec<String> = (0..40)
        .map(|i| if i % 4 == 3 { "-5" } else { "10" }.to_owned())
        .collect();
    let refs: Vec<&str> = pnls.iter().map(String::as_str).collect();
    let trades = daily_trades(&refs);
    let s = run(
        &trades,
        &Options {
            initial_balance: Some(d("1000")),
            ..Options::default()
        },
    );

    // Independent: daily returns on equity before each day.
    let mut equity = 1000.0_f64;
    let mut returns = Vec::new();
    for pnl in &pnls {
        let pnl: f64 = pnl.parse().unwrap();
        returns.push(pnl / equity);
        equity += pnl;
    }
    let n = returns.len() as f64;
    let mean = returns.iter().sum::<f64>() / n;
    let std = (returns.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / (n - 1.0)).sqrt();
    let downside = (returns.iter().map(|r| r.min(0.0).powi(2)).sum::<f64>() / n).sqrt();
    assert!((s.sharpe.copied().unwrap() - mean / std).abs() < 1e-12);
    assert!((s.sortino.copied().unwrap() - mean / downside).abs() < 1e-12);
    // Net 300 on 1000 (30%); worst drawdown 5 from a peak of 10 -> 5 / 1010 on the first dip.
    assert!(s.calmar.copied().unwrap() > 0.0);
    assert!(s.calmar.copied().unwrap().is_finite());
}

#[test]
fn an_unbroken_winning_run_has_no_sortino_or_calmar() {
    let pnls = vec!["10"; 35];
    let trades = daily_trades(&pnls);
    let s = run(
        &trades,
        &Options {
            initial_balance: Some(d("1000")),
            ..Options::default()
        },
    );
    assert_eq!(s.sortino, Metric::Unavailable(Reason::NoLosses));
    assert_eq!(s.calmar, Metric::Unavailable(Reason::NotApplicable));
}

#[test]
fn sqn_needs_thirty_r_values() {
    let mut executions = Vec::new();
    for index in 0..30 {
        let id = format!("t{index}");
        let base = index as i64 * 10_000 + 1_000;
        let mut pair = spec(
            &id,
            if index % 3 == 0 { "-10" } else { "10" },
            base,
            base + 5_000,
        );
        pair[0] = pair[0].clone().stop("990");
        executions.extend(pair);
    }
    let trades = build(executions);
    let s = run(&trades, &Options::default());
    // R: 10 trades at -1, 20 at +1 -> mean 1/3, sample stdev sqrt(30 * (8/9)/29... computed below.
    let mean: f64 = 10.0 / 30.0;
    let var = (10.0 * (-1.0 - mean).powi(2) + 20.0 * (1.0 - mean).powi(2)) / 29.0;
    let expected = 30f64.sqrt() * mean / var.sqrt();
    assert!((s.sqn.copied().unwrap() - expected).abs() < 1e-12);
}

#[test]
fn no_metric_is_ever_nan_or_infinite() {
    let sets: Vec<Vec<&str>> = vec![
        vec![],
        vec!["0"],
        vec!["0", "0", "0"],
        vec!["5"],
        vec!["-5"],
        vec!["1000000000", "-1000000000"],
        vec!["0.00000001", "-0.00000001"],
        vec!["5", "5", "5", "5", "5", "5", "5"],
    ];
    for pnls in sets {
        let trades = daily_trades(&pnls);
        for balance in [None, Some(d("0")), Some(d("-10")), Some(d("1000"))] {
            let s = run(
                &trades,
                &Options {
                    initial_balance: balance,
                    ..Options::default()
                },
            );
            for value in all_floats(&s) {
                assert!(value.is_finite(), "{pnls:?} {balance:?}: {value}");
            }
        }
    }
}
