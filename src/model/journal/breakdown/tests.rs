use super::*;
use crate::model::journal::calendar::{Civil, days_from_civil, utc};
use crate::model::journal::model::{CostBasis, Execution, Side};
use crate::model::journal::reconstruct::reconstruct;
use crate::model::journal::testing::{d, exec};

const HOUR: i64 = 3_600_000;

fn at(year: i32, month: u8, day: u8, hour: i64) -> i64 {
    days_from_civil(Civil { year, month, day }) * 86_400_000 + hour * HOUR
}

/// One long trade: open at `open`, held `hold_ms`, net `pnl`, on `symbol`.
fn trade(id: &str, symbol: &str, open: i64, hold_ms: i64, pnl: &str, qty: &str) -> Vec<Execution> {
    let exit = (d("100") + d(pnl) / d(qty)).to_string();
    vec![
        exec(&format!("{id}a"), Side::Buy, qty, "100", open).of(symbol),
        exec(&format!("{id}b"), Side::Sell, qty, &exit, open + hold_ms).of(symbol),
    ]
}

fn build(executions: Vec<Execution>) -> Vec<Trade> {
    reconstruct(&executions, |_| CostBasis::AverageCost)
}

fn run(trades: &[Trade], dimension: Dimension, reviews: &HashMap<String, Review>) -> Vec<Slice> {
    let refs: Vec<&Trade> = trades.iter().collect();
    breakdown(
        &refs,
        dimension,
        &Context {
            reviews,
            offset: &utc,
            breakeven_tolerance: Decimal::ZERO,
        },
    )
}

fn slice<'a>(slices: &'a [Slice], label: &str) -> &'a Slice {
    slices
        .iter()
        .find(|s| s.label == label)
        .unwrap_or_else(|| panic!("{label} in {slices:#?}"))
}

#[test]
fn sample_quality_thresholds() {
    assert_eq!(sample_of(0), Sample::Thin);
    assert_eq!(sample_of(4), Sample::Thin);
    assert_eq!(sample_of(5), Sample::Low);
    assert_eq!(sample_of(29), Sample::Low);
    assert_eq!(sample_of(30), Sample::Enough);
    assert!(Sample::Thin.note().is_some() && Sample::Enough.note().is_none());
}

#[test]
fn weekday_is_the_close_day_and_hour_is_the_open_hour() {
    // Mon 2026-11-09 23:30 open, held 1h: closes Tuesday 00:30.
    let open = at(2026, 11, 9, 23) + HOUR / 2;
    let trades = build(trade("a", "BTC", open, HOUR, "10", "1"));
    let weekdays = run(&trades, Dimension::Weekday, &HashMap::new());
    assert_eq!(weekdays.len(), 1);
    assert_eq!(weekdays[0].label, "Tue");
    let hours = run(&trades, Dimension::Hour, &HashMap::new());
    assert_eq!(hours[0].label, "23:00");
    // In a zone five hours ahead it opened at 04:30 the next day.
    let refs: Vec<&Trade> = trades.iter().collect();
    let shifted = breakdown(
        &refs,
        Dimension::Hour,
        &Context {
            reviews: &HashMap::new(),
            offset: &|_| 5 * 3_600,
            breakeven_tolerance: Decimal::ZERO,
        },
    );
    assert_eq!(shifted[0].label, "04:00");
}

#[test]
fn slices_total_the_selection_and_carry_their_statistics() {
    let mut e = Vec::new();
    e.extend(trade("a", "BTC", at(2026, 11, 9, 10), HOUR, "30", "1"));
    e.extend(trade("b", "BTC", at(2026, 11, 9, 12), HOUR, "-10", "1"));
    e.extend(trade("c", "ETH", at(2026, 11, 9, 14), HOUR, "-20", "1"));
    let trades = build(e);
    let slices = run(&trades, Dimension::Instrument, &HashMap::new());
    assert_eq!(slices.len(), 2);
    let btc = slice(&slices, "BTC");
    assert_eq!((btc.trades, btc.wins, btc.losses), (2, 1, 1));
    assert_eq!(btc.net, d("20"));
    assert_eq!(btc.win_rate, Metric::Value(0.5));
    assert_eq!(btc.profit_factor, Metric::Value(3.0));
    assert_eq!(btc.expectancy, Metric::Value(d("10")));
    assert_eq!(btc.sample, Sample::Thin);
    let eth = slice(&slices, "ETH");
    assert_eq!(eth.profit_factor, Metric::Value(0.0));
    assert_eq!(slices.iter().map(|s| s.trades).sum::<usize>(), 3);
    assert_eq!(slices.iter().map(|s| s.net).sum::<Decimal>(), d("0"));
    // Ranked by result.
    assert_eq!(slices[0].label, "BTC");
}

#[test]
fn a_slice_with_no_losses_has_no_profit_factor() {
    let trades = build(trade("a", "BTC", at(2026, 11, 9, 10), HOUR, "5", "1"));
    let slices = run(&trades, Dimension::Side, &HashMap::new());
    assert_eq!(
        slices[0].profit_factor,
        Metric::Unavailable(Reason::NoLosses)
    );
    assert_eq!(slices[0].label, "Long");
}

#[test]
fn hold_times_fall_in_their_buckets() {
    let mut e = Vec::new();
    for (index, (hold, expected)) in [
        (30_000, "Under 1 min"),
        (120_000, "1 to 5 min"),
        (600_000, "5 to 15 min"),
        (1_800_000, "15 to 60 min"),
        (2 * HOUR, "1 to 4 hours"),
        (10 * HOUR, "4 to 24 hours"),
        (3 * 24 * HOUR, "1 to 7 days"),
        (9 * 24 * HOUR, "Over 7 days"),
    ]
    .iter()
    .enumerate()
    {
        e.extend(trade(
            &format!("t{index}"),
            "BTC",
            at(2026, 1, 1, 0) + index as i64 * 30 * 24 * HOUR,
            *hold,
            "1",
            "1",
        ));
        let _ = expected;
    }
    let trades = build(e);
    let slices = run(&trades, Dimension::HoldTime, &HashMap::new());
    let labels: Vec<_> = slices.iter().map(|s| s.label.as_str()).collect();
    assert_eq!(
        labels,
        [
            "Under 1 min",
            "1 to 5 min",
            "5 to 15 min",
            "15 to 60 min",
            "1 to 4 hours",
            "4 to 24 hours",
            "1 to 7 days",
            "Over 7 days"
        ]
    );
    assert!(slices.iter().all(|s| s.trades == 1));
}

#[test]
fn size_quartiles_are_relative_to_the_selection() {
    let mut e = Vec::new();
    for (index, qty) in ["1", "2", "3", "4", "5", "6", "7", "8"].iter().enumerate() {
        e.extend(trade(
            &format!("t{index}"),
            "BTC",
            at(2026, 1, 1, 0) + index as i64 * 24 * HOUR,
            HOUR,
            "1",
            qty,
        ));
    }
    let trades = build(e);
    let refs: Vec<&Trade> = trades.iter().collect();
    let q = notional_quartiles(&refs).unwrap();
    assert_eq!(q, [d("300"), d("500"), d("700")]);
    let slices = run(&trades, Dimension::Size, &HashMap::new());
    assert_eq!(slices.len(), 4);
    assert!(slices.iter().all(|s| s.trades == 2));
    assert!(slices[0].label.starts_with("Q1") && slices[3].label.starts_with("Q4"));
    // Too few trades for quartiles: one bucket.
    let few = build(trade("x", "BTC", at(2026, 1, 1, 0), HOUR, "1", "1"));
    let one = run(&few, Dimension::Size, &HashMap::new());
    assert_eq!(one[0].label, "All sizes");
}

#[test]
fn review_dimensions_use_the_review_and_none_comes_last() {
    let mut e = Vec::new();
    e.extend(trade("a", "BTC", at(2026, 11, 9, 10), HOUR, "10", "1"));
    e.extend(trade("b", "BTC", at(2026, 11, 9, 12), HOUR, "-30", "1"));
    e.extend(trade("c", "BTC", at(2026, 11, 9, 14), HOUR, "5", "1"));
    let trades = build(e);
    let mut reviews = HashMap::new();
    let mut a = Review::new(&trades[0].key, "acct");
    a.strategy = "Breakout".into();
    a.tags = vec!["fomo".into(), "early".into()];
    a.rating = Some(4);
    let mut b = Review::new(&trades[1].key, "acct");
    b.strategy = "Breakout".into();
    b.tags = vec!["fomo".into()];
    b.mistakes = vec!["no stop".into()];
    reviews.insert(trades[0].key.clone(), a);
    reviews.insert(trades[1].key.clone(), b);

    let strategies = run(&trades, Dimension::Strategy, &reviews);
    assert_eq!(strategies.len(), 2);
    assert_eq!(slice(&strategies, "Breakout").net, d("-20"));
    assert_eq!(
        strategies.last().unwrap().label,
        "(none)",
        "none is last even when it is the best"
    );

    // Tags overlap: the first trade is in two slices.
    let tags = run(&trades, Dimension::Tag, &reviews);
    assert_eq!(slice(&tags, "fomo").trades, 2);
    assert_eq!(slice(&tags, "early").trades, 1);
    assert_eq!(slice(&tags, "(none)").trades, 1);
    assert_eq!(
        tags.iter().map(|s| s.trades).sum::<usize>(),
        4,
        "more than the 3 trades"
    );
    assert!(Dimension::Tag.overlapping() && !Dimension::Strategy.overlapping());

    let mistakes = run(&trades, Dimension::Mistake, &reviews);
    assert_eq!(slice(&mistakes, "no stop").net, d("-30"));
    let ratings = run(&trades, Dimension::Rating, &reviews);
    assert_eq!(ratings[0].label, "4 stars");
    assert_eq!(ratings.last().unwrap().label, "(none)");
}

#[test]
fn average_r_uses_only_trades_with_a_stop_including_a_reviewed_one() {
    let mut e = trade("a", "BTC", at(2026, 11, 9, 10), HOUR, "20", "1");
    e[0] = e[0].clone().stop("90");
    e.extend(trade("b", "BTC", at(2026, 11, 9, 12), HOUR, "-5", "1"));
    e.extend(trade("c", "BTC", at(2026, 11, 9, 14), HOUR, "10", "1"));
    let trades = build(e);
    let mut reviews = HashMap::new();
    let mut manual = Review::new(&trades[1].key, "acct");
    manual.stop = Some(d("95"));
    reviews.insert(trades[1].key.clone(), manual);
    let slices = run(&trades, Dimension::Instrument, &reviews);
    // a: +20 on risk 10 = 2R; b: -5 on risk 5 = -1R; c has no stop.
    assert_eq!(slices[0].r_count, 2);
    assert_eq!(slices[0].average_r, Metric::Value(0.5));
}

#[test]
fn open_trades_and_empty_selections_have_no_slices() {
    let open = build(vec![exec("1", Side::Buy, "1", "100", 1_000)]);
    assert!(run(&open, Dimension::Instrument, &HashMap::new()).is_empty());
    assert!(run(&[], Dimension::Weekday, &HashMap::new()).is_empty());
    for dimension in Dimension::ALL {
        assert!(!dimension.label().is_empty() && !dimension.description().is_empty());
    }
}
