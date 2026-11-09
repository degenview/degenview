use super::*;
use crate::model::journal::calendar::utc;
use crate::model::journal::model::{CostBasis, Execution, Side, SourceId, TradingSourceKind};
use crate::model::journal::reconstruct::reconstruct;
use crate::model::journal::testing::{d, exec};
use std::collections::HashMap;

const DAY: i64 = 86_400_000;

fn account(uid: &str, currency: &str, balance: Option<&str>) -> JournalAccount {
    JournalAccount {
        uid: uid.into(),
        source: SourceId("test".into()),
        kind: TradingSourceKind::Paper,
        external_id: "1".into(),
        name: uid.into(),
        currency: currency.into(),
        initial_balance: balance.map(d),
        cost_basis: CostBasis::AverageCost,
        archived: false,
        created_at_ms: 1,
    }
}

/// One trade per `(pnl, day)`: entry 1000, size 1, closed at noon of `day`.
fn trades_on(spec: &[(&str, i64)]) -> Vec<Trade> {
    let mut executions: Vec<Execution> = Vec::new();
    for (index, (pnl, day)) in spec.iter().enumerate() {
        let close = day * DAY + DAY / 2;
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

fn run(
    trades: &[Trade],
    previous: Option<&[Trade]>,
    before: &[Trade],
    accounts: &[JournalAccount],
    window: (Option<i64>, Option<i64>),
) -> Overview {
    let refs: Vec<&Trade> = trades.iter().collect();
    let prev: Option<Vec<&Trade>> = previous.map(|p| p.iter().collect());
    let before: Vec<&Trade> = before.iter().collect();
    let accounts: Vec<&JournalAccount> = accounts.iter().collect();
    compute(&Input {
        trades: &refs,
        previous: prev.as_deref(),
        before: &before,
        accounts: &accounts,
        from_ms: window.0,
        to_ms: window.1,
        currency: None,
        reviews: &HashMap::new(),
        breakeven_tolerance: Decimal::ZERO,
        offset: &utc,
    })
}

#[test]
fn nice_steps_round_up_to_one_two_two_and_a_half_or_five() {
    assert_eq!(nice_step(0.8), 1.0);
    assert_eq!(nice_step(1.1), 2.0);
    assert_eq!(nice_step(2.2), 2.5);
    assert_eq!(nice_step(3.0), 5.0);
    assert_eq!(nice_step(7.0), 10.0);
    assert_eq!(nice_step(0.034), 0.05);
    assert_eq!(nice_step(0.0), 1.0);
    assert_eq!(nice_step(f64::NAN), 1.0);
}

#[test]
fn a_histogram_has_zero_as_an_edge_and_counts_every_value() {
    let values = [-12.0, -3.0, -3.0, 0.0, 4.0, 9.0, 25.0];
    let histogram = histogram(&values);
    assert_eq!(
        histogram.bins.iter().map(|b| b.count).sum::<usize>(),
        values.len()
    );
    assert!(
        histogram.bins.iter().any(|bin| bin.low == 0.0),
        "zero is an edge: {:?}",
        histogram.bins
    );
    for bin in &histogram.bins {
        assert!(
            bin.low >= 0.0 || bin.high <= 0.0,
            "a bin straddles zero: {bin:?}"
        );
    }
    // Bins are contiguous.
    for pair in histogram.bins.windows(2) {
        assert!((pair[0].high - pair[1].low).abs() < 1e-9);
    }
    // Zero (a breakeven) is counted with the wins' side.
    let zero_bin = histogram.bins.iter().find(|b| b.low == 0.0).unwrap();
    assert!(zero_bin.count >= 1);
}

#[test]
fn degenerate_histograms_do_not_panic() {
    assert!(histogram(&[]).bins.is_empty());
    assert!(histogram(&[f64::NAN]).bins.is_empty());
    let single = histogram(&[5.0]);
    assert_eq!(single.bins.iter().map(|b| b.count).sum::<usize>(), 1);
    let same = histogram(&[0.0, 0.0, 0.0]);
    assert_eq!(same.bins.iter().map(|b| b.count).sum::<usize>(), 3);
    let tiny = histogram(&[1e-9, 2e-9]);
    assert_eq!(tiny.bins.iter().map(|b| b.count).sum::<usize>(), 2);
}

#[test]
fn the_overview_describes_the_selection() {
    let trades = trades_on(&[("10", 1), ("-4", 2), ("6", 3), ("-2", 5)]);
    let accounts = [account("acct", "USD", Some("1000"))];
    let o = run(&trades, None, &[], &accounts, (None, None));
    assert_eq!(o.currency.as_deref(), Some("USD"));
    assert_eq!(o.currencies.len(), 1);
    assert_eq!(o.summary.closed, 4);
    assert_eq!(o.summary.net_pnl, d("10"));
    assert_eq!(o.initial_balance, Some(d("1000")));
    assert_eq!(o.equity.len(), 4);
    assert_eq!(o.equity.last().unwrap().equity, Some(d("1010")));
    assert_eq!(o.drawdown.len(), 4);
    assert_eq!(o.days.len(), 4);
    assert_eq!(o.days.iter().map(|day| day.net).sum::<Decimal>(), d("10"));
    assert_eq!((o.long.trades, o.short.trades), (4, 0));
    assert_eq!(o.long.net, d("10"));
    assert_eq!(o.long.win_rate, Metric::Value(0.5));
    assert!(matches!(o.short.win_rate, Metric::Unavailable(_)));
    assert_eq!(o.histogram.bins.iter().map(|b| b.count).sum::<usize>(), 4);
    assert_eq!(o.recent.len(), 4);
    assert_eq!(
        o.recent[0],
        trades.iter().max_by_key(|t| t.close_ts_ms).unwrap().key
    );
    assert!(!o.edge.score.is_available(), "only four trades");
    assert!(o.previous.is_none() && o.previous_equity.is_empty());
}

#[test]
fn a_range_measures_from_what_was_earned_before_it() {
    let before = trades_on(&[("100", 1)]);
    let inside = trades_on(&[("-50", 10), ("20", 11)]);
    let accounts = [account("acct", "USD", Some("1000"))];
    let o = run(&inside, None, &before, &accounts, (Some(10 * DAY), None));
    // 1000 + 100 earned before the range.
    assert_eq!(o.initial_balance, Some(d("1100")));
    assert_eq!(o.equity[0].equity, Some(d("1050")));
    // Drawdown of 50 from a 1100 balance, not from 1000.
    assert!((o.drawdown[0].pct.unwrap() - 50.0 / 1100.0).abs() < 1e-12);
}

#[test]
fn no_balance_means_no_equity_or_percent() {
    let trades = trades_on(&[("10", 1), ("-4", 2)]);
    let o = run(
        &trades,
        None,
        &[],
        &[account("acct", "USD", None)],
        (None, None),
    );
    assert_eq!(o.initial_balance, None);
    assert!(o.equity.iter().all(|point| point.equity.is_none()));
    assert!(o.drawdown.iter().all(|point| point.pct.is_none()));
    let none = run(&trades, None, &[], &[], (None, None));
    assert_eq!(none.initial_balance, None);
}

#[test]
fn an_account_without_a_balance_makes_the_total_unknown() {
    let accounts = [account("a", "USD", Some("1000")), account("b", "USD", None)];
    let refs: Vec<&JournalAccount> = accounts.iter().collect();
    assert_eq!(starting_balance(&refs, &Some("USD".into()), &[]), None);
    let known = [
        account("a", "USD", Some("1000")),
        account("b", "USD", Some("500")),
    ];
    let refs: Vec<&JournalAccount> = known.iter().collect();
    assert_eq!(
        starting_balance(&refs, &Some("USD".into()), &[]),
        Some(d("1500"))
    );
    // Other currencies' accounts are not added in.
    let mixed = [
        account("a", "USD", Some("1000")),
        account("c", "EUR", Some("9")),
    ];
    let refs: Vec<&JournalAccount> = mixed.iter().collect();
    assert_eq!(
        starting_balance(&refs, &Some("USD".into()), &[]),
        Some(d("1000"))
    );
}

#[test]
fn a_mixed_selection_shows_one_currency_and_offers_the_rest() {
    let mut executions = Vec::new();
    for (id, currency, symbol, pnl) in [("a", "USD", "BTC", "10"), ("b", "USDT", "ETH", "30")] {
        let exit = (d("1000") + d(pnl)).to_string();
        executions.push(
            exec(&format!("{id}1"), Side::Buy, "1", "1000", 1_000)
                .of(symbol)
                .currency(currency),
        );
        executions.push(
            exec(&format!("{id}2"), Side::Sell, "1", &exit, 2_000)
                .of(symbol)
                .currency(currency),
        );
    }
    let trades = reconstruct(&executions, |_| CostBasis::AverageCost);
    let refs: Vec<&Trade> = trades.iter().collect();
    assert_eq!(
        currencies_of(&refs),
        [Some("USD".to_owned()), Some("USDT".to_owned())]
    );

    let make = |currency: Option<Option<String>>| {
        compute(&Input {
            trades: &refs,
            previous: None,
            before: &[],
            accounts: &[],
            from_ms: None,
            to_ms: None,
            currency,
            reviews: &HashMap::new(),
            breakeven_tolerance: Decimal::ZERO,
            offset: &utc,
        })
    };
    let first = make(None);
    assert_eq!(first.currency.as_deref(), Some("USD"));
    assert_eq!(first.summary.net_pnl, d("10"));
    let second = make(Some(Some("USDT".into())));
    assert_eq!(second.summary.net_pnl, d("30"));
    assert_eq!(second.currencies.len(), 2);
    // A currency that is not there falls back to the first.
    assert_eq!(
        make(Some(Some("JPY".into()))).currency.as_deref(),
        Some("USD")
    );
}

#[test]
fn the_previous_period_is_shifted_onto_the_range() {
    let current = trades_on(&[("10", 20), ("5", 22)]);
    let previous = trades_on(&[("-3", 12), ("8", 14)]);
    let accounts = [account("acct", "USD", Some("1000"))];
    let (from, to) = (20 * DAY, 30 * DAY);
    let o = run(
        &current,
        Some(&previous),
        &[],
        &accounts,
        (Some(from), Some(to)),
    );
    let prev = o.previous.as_ref().unwrap();
    assert_eq!(prev.net_pnl, d("5"));
    assert_eq!(o.previous_equity.len(), 2);
    // Day 12 + the 10-day window lands on day 22.
    assert_eq!(o.previous_equity[0].ts_ms, 12 * DAY + DAY / 2 + (to - from));
    assert_eq!(o.previous_drawdown.len(), 2);
}

#[test]
fn long_and_short_are_split() {
    let mut executions = vec![
        exec("l1", Side::Buy, "1", "100", 1_000),
        exec("l2", Side::Sell, "1", "110", 2_000),
        exec("s1", Side::Sell, "1", "100", 3_000).of("ETH"),
        exec("s2", Side::Buy, "1", "105", 4_000).of("ETH"),
    ];
    executions.push(exec("s3", Side::Sell, "1", "50", 5_000).of("SOL"));
    executions.push(exec("s4", Side::Buy, "1", "40", 6_000).of("SOL"));
    let trades = reconstruct(&executions, |_| CostBasis::AverageCost);
    let o = run(&trades, None, &[], &[], (None, None));
    assert_eq!((o.long.trades, o.short.trades), (1, 2));
    assert_eq!(o.long.net, d("10"));
    assert_eq!(o.short.net, d("5"));
    assert_eq!((o.short.wins, o.short.losses), (1, 1));
}

#[test]
fn an_empty_selection_is_a_clean_empty_overview() {
    let o = run(&[], None, &[], &[], (None, None));
    assert_eq!(o.currency, None);
    assert!(o.currencies.is_empty());
    assert_eq!(o.summary.closed, 0);
    assert!(o.equity.is_empty() && o.days.is_empty() && o.recent.is_empty());
    assert!(o.histogram.bins.is_empty());
    assert!(!o.edge.score.is_available());
}

#[test]
fn accounts_in_scope_follow_the_choice_or_the_trades() {
    let accounts = vec![
        account("a", "USD", Some("1")),
        account("b", "USD", Some("2")),
    ];
    let trades = trades_on(&[("1", 1)]);
    let refs: Vec<&Trade> = trades.iter().collect();
    // The fixture trades are in "acct": neither named account has trades.
    assert!(accounts_in_scope(&accounts, None, &refs).is_empty());
    assert_eq!(accounts_in_scope(&accounts, Some("b"), &refs)[0].uid, "b");
}

#[test]
fn a_stop_in_a_review_gives_the_overview_its_r() {
    let trades = trades_on(&[("20", 1), ("-10", 2)]);
    let refs: Vec<&Trade> = trades.iter().collect();
    let make = |reviews: &HashMap<String, crate::model::journal::reviews::Review>| {
        compute(&Input {
            trades: &refs,
            previous: None,
            before: &[],
            accounts: &[],
            from_ms: None,
            to_ms: None,
            currency: None,
            reviews,
            breakeven_tolerance: Decimal::ZERO,
            offset: &utc,
        })
    };
    let none = make(&HashMap::new());
    assert_eq!(none.summary.r.count, 0);
    assert!(none.r_histogram.bins.is_empty());
    let mut reviews = HashMap::new();
    for trade in &trades {
        let mut review = crate::model::journal::reviews::Review::new(&trade.key, "acct");
        review.stop = Some(d("990"));
        reviews.insert(trade.key.clone(), review);
    }
    let with = make(&reviews);
    // Entry 1000, stop 990: +20 is 2R, -10 is -1R.
    assert_eq!(with.summary.r.count, 2);
    assert_eq!(with.summary.r.total, Metric::Value(1.0));
    assert_eq!(
        with.r_histogram.bins.iter().map(|b| b.count).sum::<usize>(),
        2
    );
}
