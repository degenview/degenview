//! How long the journal's derived data takes at the sizes the plan names (10,000 trades from
//! 50,000 fills). Ignored by default; run in release for numbers that mean something:
//! `cargo test --release --lib journal::perf_tests -- --ignored --nocapture`.

use std::collections::HashMap;
use std::time::Instant;

use rust_decimal::Decimal;

use super::analytics::{Options, summarize};
use super::marks::Marks;
use super::model::{CostBasis, Execution, Side, Trade};
use super::overview::{Input, compute};
use super::reconstruct::reconstruct;
use super::table::{Column, Sort, SortContext, sort_indices};
use super::testing::exec;

/// `trades` round trips over a few instruments, scaled in and out so that about five fills make a
/// trade, spread over a year.
fn fills(trades: usize) -> Vec<Execution> {
    let mut out = Vec::with_capacity(trades * 5);
    let mut seed: u64 = 0x2545_F491_4F6C_DD1D;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    let day = 86_400_000i64;
    for trade in 0..trades {
        let start = 1_700_000_000_000 + (trade as i64) * (365 * day / trades as i64);
        let symbol = ["BTC", "ETH", "SOL", "AAPL"][(next() % 4) as usize];
        let base = 100 + (next() % 900) as i64;
        let id = |n: usize| format!("{trade}-{n}");
        let make = |n: usize, side: Side, qty: &str, price: i64, at: i64| {
            let mut e = exec(&id(n), side, qty, &price.to_string(), at);
            e.instrument = super::testing::instrument(symbol);
            e.uid = format!("acct|{}", id(n));
            e
        };
        out.push(make(0, Side::Buy, "2", base, start));
        out.push(make(1, Side::Buy, "1", base + 1, start + 60_000));
        out.push(make(2, Side::Sell, "1", base + 3, start + 600_000));
        out.push(make(
            3,
            Side::Sell,
            "1",
            base + (next() % 9) as i64 - 3,
            start + 1_200_000,
        ));
        out.push(make(
            4,
            Side::Sell,
            "1",
            base + (next() % 9) as i64 - 3,
            start + 1_800_000,
        ));
    }
    out
}

#[test]
#[ignore = "timing"]
fn the_derived_data_at_ten_thousand_trades() {
    let executions = fills(10_000);
    assert_eq!(executions.len(), 50_000);

    let t = Instant::now();
    let trades: Vec<Trade> = reconstruct(&executions, |_: &str| CostBasis::AverageCost);
    let reconstruct_ms = t.elapsed().as_secs_f64() * 1e3;
    assert_eq!(trades.len(), 10_000);

    let refs: Vec<&Trade> = trades.iter().collect();
    let t = Instant::now();
    let summary = summarize(&refs, &Options::default()).expect("one currency");
    let summarize_ms = t.elapsed().as_secs_f64() * 1e3;
    assert_eq!(summary.closed, 10_000);

    let reviews = HashMap::new();
    let t = Instant::now();
    let overview = compute(&Input {
        trades: &refs,
        previous: None,
        before: &[],
        accounts: &[],
        from_ms: None,
        to_ms: None,
        currency: None,
        reviews: &reviews,
        breakeven_tolerance: Decimal::ZERO,
        offset: &|_| 0,
    });
    let overview_ms = t.elapsed().as_secs_f64() * 1e3;
    assert!(!overview.equity.is_empty());

    let t = Instant::now();
    let mut rows: Vec<usize> = (0..trades.len()).collect();
    let names = HashMap::new();
    let accounts = HashMap::new();
    sort_indices(
        &mut rows,
        &trades,
        Sort {
            column: Column::Net,
            descending: true,
        },
        &SortContext {
            reviews: &reviews,
            playbook_names: &names,
            account_names: &accounts,
            breakeven_tolerance: Decimal::ZERO,
        },
    );
    let sort_ms = t.elapsed().as_secs_f64() * 1e3;

    let t = Instant::now();
    let marks = Marks::build(&trades, None, Decimal::ZERO);
    let marks_ms = t.elapsed().as_secs_f64() * 1e3;
    assert_eq!(marks.len(), 10_000);

    eprintln!(
        "10,000 trades from 50,000 fills: reconstruct {reconstruct_ms:.1} ms, summary {summarize_ms:.1} ms, \
         overview {overview_ms:.1} ms, sort {sort_ms:.1} ms, chart marks {marks_ms:.1} ms"
    );
}
