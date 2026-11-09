use super::*;
use crate::model::journal::analytics::{Options, summarize};
use crate::model::journal::model::{CostBasis, Execution, Side, Trade};
use crate::model::journal::reconstruct::reconstruct;
use crate::model::journal::testing::{d, exec};

const DAY: i64 = 86_400_000;

fn trades_for(pnls: &[&str]) -> Vec<Trade> {
    let mut executions: Vec<Execution> = Vec::new();
    for (index, pnl) in pnls.iter().enumerate() {
        let close = (index as i64 + 1) * DAY + DAY / 2;
        let exit = (d("1000") + d(pnl)).to_string();
        executions.push(exec(
            &format!("{index}a"),
            Side::Buy,
            "1",
            "1000",
            close - 1_000,
        ));
        executions.push(exec(&format!("{index}b"), Side::Sell, "1", &exit, close));
    }
    reconstruct(&executions, |_| CostBasis::AverageCost)
}

fn score(pnls: &[&str], balance: Option<&str>) -> EdgeScore {
    let trades = trades_for(pnls);
    let refs: Vec<&Trade> = trades.iter().collect();
    let balance = balance.map(d);
    let summary = summarize(
        &refs,
        &Options {
            initial_balance: balance,
            ..Options::default()
        },
    )
    .unwrap();
    edge_score(&summary, balance)
}

fn component(score: &EdgeScore, key: ComponentKey) -> &Component {
    score.components.iter().find(|c| c.key == key).unwrap()
}

#[test]
fn the_weights_sum_to_one_hundred() {
    assert_eq!(WEIGHTS.iter().map(|(_, w)| w).sum::<u32>(), 100);
}

#[test]
fn the_score_is_withheld_below_five_closed_trades_but_components_show() {
    let s = score(&["10", "-5", "10", "10"], Some("1000"));
    assert_eq!(
        s.score,
        Metric::Unavailable(Reason::Insufficient { have: 4, need: 5 })
    );
    assert_eq!(s.closed_trades, 4);
    assert_eq!(s.components.len(), 6);
    assert_eq!(s.version, 2);
}

#[test]
fn a_record_is_scored_component_by_component() {
    // 10 trades on 10 days: +20 x6, -10 x4. cum: 20,40,60,80,100,120,110,100,90,80? order below.
    let pnls = [
        "20", "20", "-10", "20", "-10", "20", "-10", "20", "-10", "20",
    ];
    let s = score(&pnls, Some("1000"));
    // wins 6/10 = 60% -> full marks.
    assert_eq!(component(&s, ComponentKey::WinRate).value, 100.0);
    // gross profit 120 / gross loss 40 = 3.0 -> full marks.
    assert_eq!(component(&s, ComponentKey::ProfitFactor).input, Some(3.0));
    assert_eq!(component(&s, ComponentKey::ProfitFactor).value, 100.0);
    // avg win 20 / avg loss 10 = 2.0 -> 80.
    assert!((component(&s, ComponentKey::AvgWinLoss).value - 80.0).abs() < 1e-9);
    // cum: 20 40 30 50 40 60 50 70 60 80; deepest drawdown 10 from peaks of 40, 60, 70... = 10;
    // at the deepest first reached (depth 10 at cum 30, peak 40): 10 / 1040.
    let dd = component(&s, ComponentKey::Drawdown);
    let pct = 10.0 / 1040.0;
    assert!((dd.input.unwrap() - pct).abs() < 1e-12);
    assert!((dd.value - (1.0 - pct / 0.25) * 100.0).abs() < 1e-9);
    // recovery: net 80 / max dd 10 = 8 -> capped.
    assert_eq!(component(&s, ComponentKey::Recovery).value, 100.0);
    // consistency: winning days all +20 (6 of them... 6 days of 120): 20/120 = 0.1667 > 0.15.
    let c = component(&s, ComponentKey::Consistency);
    let expected = (1.0 - (20.0 / 120.0 - 0.15) / 0.85) * 100.0;
    assert!(
        (c.value - expected).abs() < 1e-9,
        "{} vs {expected}",
        c.value
    );

    let weighted = (100.0 * 15.0
        + 100.0 * 25.0
        + 80.0 * 20.0
        + dd.value * 15.0
        + 100.0 * 10.0
        + c.value * 15.0)
        / 100.0;
    let got = s.score.copied().unwrap();
    assert!((got - (weighted * 100.0).round() / 100.0).abs() < 1e-9);
    assert!(
        (got * 100.0 - (got * 100.0).round()).abs() < 1e-6,
        "two decimals"
    );
}

#[test]
fn full_marks_cap_at_one_hundred_and_floors_at_zero() {
    let great = score(
        &["50", "50", "50", "50", "50", "50", "50", "50", "50", "-1"],
        Some("1000"),
    );
    for c in &great.components {
        assert!((0.0..=100.0).contains(&c.value), "{:?} {}", c.key, c.value);
    }
    let awful = score(&["-5"; 8], Some("1000"));
    assert_eq!(component(&awful, ComponentKey::WinRate).value, 0.0);
    assert_eq!(component(&awful, ComponentKey::ProfitFactor).value, 0.0);
    assert_eq!(component(&awful, ComponentKey::Consistency).value, 0.0);
    assert!(awful.score.copied().unwrap() >= 0.0);
}

#[test]
fn no_losses_count_as_a_profit_factor_of_three() {
    let s = score(&["5"; 6], Some("1000"));
    let pf = component(&s, ComponentKey::ProfitFactor);
    assert_eq!(pf.input, Some(3.0));
    assert_eq!(pf.value, 100.0);
    assert!(pf.note.is_some());
    // And no losses means no average-loss ratio: that component is zero, as published.
    assert_eq!(component(&s, ComponentKey::AvgWinLoss).value, 0.0);
}

#[test]
fn an_all_breakeven_record_has_no_profit_factor_and_scores_low() {
    let s = score(&["0"; 6], Some("1000"));
    assert_eq!(component(&s, ComponentKey::ProfitFactor).value, 0.0);
    assert_eq!(component(&s, ComponentKey::Recovery).value, 0.0);
}

#[test]
fn drawdown_is_neutral_without_an_initial_balance() {
    let s = score(&["10", "-5", "10", "-5", "10", "10"], None);
    let dd = component(&s, ComponentKey::Drawdown);
    assert_eq!(dd.value, 50.0);
    assert!(dd.note.unwrap().contains("initial balance"));
    let zero = score(&["10", "-5", "10", "-5", "10", "10"], Some("0"));
    assert_eq!(component(&zero, ComponentKey::Drawdown).value, 50.0);
}

#[test]
fn no_drawdown_with_a_known_balance_is_full_marks_by_the_documentation() {
    let s = score(&["10"; 6], Some("1000"));
    assert_eq!(component(&s, ComponentKey::Drawdown).value, 100.0);
    // Recovery with no drawdown: 100 only because net P&L is positive.
    assert_eq!(component(&s, ComponentKey::Recovery).value, 100.0);
    let flat = score(&["0"; 6], Some("1000"));
    assert_eq!(component(&flat, ComponentKey::Recovery).value, 0.0);
}

#[test]
fn a_drawdown_of_a_quarter_or_more_scores_zero() {
    // 1000 balance, lose 300 (30%) after a 0 peak.
    let s = score(&["-100", "-100", "-100", "1", "1", "1"], Some("1000"));
    assert_eq!(component(&s, ComponentKey::Drawdown).value, 0.0);
}

#[test]
fn recovery_is_net_over_max_drawdown_against_three() {
    // cum 10, -10, 0, 5, 10, 5: deepest drawdown 20 (from the 10 peak); net 5.
    let s = score(&["10", "-20", "10", "5", "5", "-5"], Some("1000"));
    let r = component(&s, ComponentKey::Recovery);
    let net = 10.0 - 20.0 + 10.0 + 5.0 + 5.0 - 5.0;
    assert!(
        (r.input.unwrap() - net / 20.0).abs() < 1e-12,
        "{:?}",
        r.input
    );
    assert!((r.value - (net / 20.0 / 3.0).clamp(0.0, 1.0) * 100.0).abs() < 1e-9);
}

#[test]
fn consistency_is_full_when_no_day_dominates_and_zero_with_one_winning_day() {
    let spread = score(&["10"; 10], Some("1000"));
    assert_eq!(component(&spread, ComponentKey::Consistency).value, 100.0);
    let one_day = score(&["-1", "-1", "-1", "-1", "-1", "9"], Some("1000"));
    assert_eq!(component(&one_day, ComponentKey::Consistency).value, 0.0);
}

#[test]
fn an_empty_record_has_a_withheld_score_and_finite_components() {
    let trades: Vec<Trade> = Vec::new();
    let summary = summarize(&[], &Options::default()).unwrap();
    let s = edge_score(&summary, None);
    assert_eq!(s.closed_trades, 0);
    assert!(!s.score.is_available());
    assert!(s.components.iter().all(|c| c.value.is_finite()));
    drop(trades);
}
