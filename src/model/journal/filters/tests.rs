use super::*;
use crate::model::journal::calendar::utc;
use crate::model::journal::model::{CostBasis, Side};
use crate::model::journal::reconstruct::reconstruct;
use crate::model::journal::testing::{d, exec};

const DAY: i64 = 86_400_000;

fn trades() -> Vec<Trade> {
    let executions = vec![
        // BTC long, wins 10, closes day 10.
        exec("a1", Side::Buy, "1", "100", 10 * DAY),
        exec("a2", Side::Sell, "1", "110", 10 * DAY + 1_000),
        // ETH short, loses 5, closes day 12, other account.
        exec("b1", Side::Sell, "1", "50", 12 * DAY)
            .of("ETH")
            .in_account("other"),
        exec("b2", Side::Buy, "1", "55", 12 * DAY + 1_000)
            .of("ETH")
            .in_account("other"),
        // SOL long, still open since day 14.
        exec("c1", Side::Buy, "1", "20", 14 * DAY).of("SOL"),
        // BTC long, breakeven, closes day 15.
        exec("d1", Side::Buy, "1", "100", 15 * DAY),
        exec("d2", Side::Sell, "1", "100", 15 * DAY + 1_000),
    ];
    reconstruct(&executions, |_| CostBasis::AverageCost)
}

fn keys(trades: &[&Trade]) -> Vec<String> {
    trades
        .iter()
        .map(|trade| trade.instrument.symbol.clone())
        .collect()
}

#[test]
fn an_empty_filter_selects_everything_and_is_inactive() {
    let all = trades();
    let filter = TradeFilter::default();
    assert!(!filter.is_active());
    assert!(filter.active_labels().is_empty());
    assert_eq!(filter.apply(&all, Decimal::ZERO).len(), 4);
}

#[test]
fn each_dimension_narrows() {
    let all = trades();
    let tol = Decimal::ZERO;

    let mut filter = TradeFilter::default();
    filter.accounts.insert("other".into());
    assert_eq!(keys(&filter.apply(&all, tol)), ["ETH"]);

    let mut filter = TradeFilter::default();
    filter.markets.insert("TEST:BTC".into());
    assert_eq!(keys(&filter.apply(&all, tol)), ["BTC", "BTC"]);

    let filter = TradeFilter {
        direction: Some(Direction::Short),
        ..Default::default()
    };
    assert_eq!(keys(&filter.apply(&all, tol)), ["ETH"]);

    let filter = TradeFilter {
        outcome: Some(OutcomeFilter::Open),
        ..Default::default()
    };
    assert_eq!(keys(&filter.apply(&all, tol)), ["SOL"]);
    let filter = TradeFilter {
        outcome: Some(OutcomeFilter::Win),
        ..Default::default()
    };
    assert_eq!(filter.apply(&all, tol).len(), 1);
    let filter = TradeFilter {
        outcome: Some(OutcomeFilter::Loss),
        ..Default::default()
    };
    assert_eq!(keys(&filter.apply(&all, tol)), ["ETH"]);
    let filter = TradeFilter {
        outcome: Some(OutcomeFilter::Breakeven),
        ..Default::default()
    };
    assert_eq!(filter.apply(&all, tol).len(), 1);

    let filter = TradeFilter {
        search: " et ".into(),
        ..Default::default()
    };
    assert_eq!(keys(&filter.apply(&all, tol)), ["ETH"]);
}

#[test]
fn a_breakeven_tolerance_reclassifies_outcomes() {
    let all = trades();
    let filter = TradeFilter {
        outcome: Some(OutcomeFilter::Breakeven),
        ..Default::default()
    };
    // With a tolerance of 6 the 5-unit loss is breakeven too.
    assert_eq!(filter.apply(&all, d("6")).len(), 2);
}

#[test]
fn filters_combine() {
    let all = trades();
    let mut filter = TradeFilter {
        direction: Some(Direction::Long),
        ..Default::default()
    };
    filter.markets.insert("TEST:BTC".into());
    filter.accounts.insert("acct".into());
    assert_eq!(filter.apply(&all, Decimal::ZERO).len(), 2);
    assert_eq!(filter.active_labels().len(), 4 - 1);
    filter.reset();
    assert!(!filter.is_active());
}

#[test]
fn a_closed_trade_is_in_a_range_by_its_close_open_ones_by_their_open() {
    let all = trades();
    // Day 12 up to (not including) day 15: the ETH close (12) and the SOL open (14).
    let filter = TradeFilter {
        from_ms: Some(12 * DAY),
        to_ms: Some(15 * DAY),
        ..Default::default()
    };
    assert_eq!(keys(&filter.apply(&all, Decimal::ZERO)), ["ETH", "SOL"]);
    // A trade that opens before the range and closes inside it is inside.
    let spanning = reconstruct(
        &[
            exec("s1", Side::Buy, "1", "1", DAY),
            exec("s2", Side::Sell, "1", "2", 20 * DAY),
        ],
        |_| CostBasis::AverageCost,
    );
    let filter = TradeFilter {
        from_ms: Some(19 * DAY),
        to_ms: Some(21 * DAY),
        ..Default::default()
    };
    assert_eq!(filter.apply(&spanning, Decimal::ZERO).len(), 1);
    // The end is exclusive.
    let filter = TradeFilter {
        to_ms: Some(20 * DAY),
        ..Default::default()
    };
    assert!(filter.apply(&spanning, Decimal::ZERO).is_empty());
}

// 2026-11-09 is a Monday: day number 20_766.
fn now() -> i64 {
    days_from_civil(Civil {
        year: 2026,
        month: 11,
        day: 9,
    }) * DAY
        + 15 * 3_600_000
}

fn range(preset: RangePreset, week_start: u8) -> (i64, i64) {
    let (from, to) = range_for(preset, now(), &utc, week_start);
    (from.unwrap(), to.unwrap())
}

fn day(civil: Civil) -> i64 {
    days_from_civil(civil) * DAY
}

#[test]
fn presets_cover_the_local_calendar() {
    let (from, to) = range(RangePreset::Today, 0);
    assert_eq!(
        (from, to),
        (
            day(Civil {
                year: 2026,
                month: 11,
                day: 9
            }),
            day(Civil {
                year: 2026,
                month: 11,
                day: 10
            })
        )
    );

    let (from, to) = range(RangePreset::Month, 0);
    assert_eq!(
        (from, to),
        (
            day(Civil {
                year: 2026,
                month: 11,
                day: 1
            }),
            day(Civil {
                year: 2026,
                month: 12,
                day: 1
            })
        )
    );

    let (from, to) = range(RangePreset::YearToDate, 0);
    assert_eq!(
        (from, to),
        (
            day(Civil {
                year: 2026,
                month: 1,
                day: 1
            }),
            day(Civil {
                year: 2026,
                month: 11,
                day: 10
            })
        )
    );

    let (from, to) = range(RangePreset::Year, 0);
    assert_eq!(to - from, 365 * DAY);
    assert_eq!(
        to,
        day(Civil {
            year: 2026,
            month: 11,
            day: 10
        })
    );
    assert_eq!(range_for(RangePreset::All, now(), &utc, 0), (None, None));
}

#[test]
fn a_week_starts_on_the_chosen_weekday() {
    // Monday 9 Nov: a Sunday-start week began Sunday 8th, a Monday-start one today.
    let (from, to) = range(RangePreset::Week, 0);
    assert_eq!(
        from,
        day(Civil {
            year: 2026,
            month: 11,
            day: 8
        })
    );
    assert_eq!(to - from, 7 * DAY);
    let (from, _) = range(RangePreset::Week, 1);
    assert_eq!(
        from,
        day(Civil {
            year: 2026,
            month: 11,
            day: 9
        })
    );
}

#[test]
fn months_end_correctly_in_december() {
    let december = days_from_civil(Civil {
        year: 2026,
        month: 12,
        day: 20,
    }) * DAY;
    let (from, to) = range_for(RangePreset::Month, december, &utc, 0);
    assert_eq!(
        to,
        Some(day(Civil {
            year: 2027,
            month: 1,
            day: 1
        }))
    );
    assert_eq!(
        from,
        Some(day(Civil {
            year: 2026,
            month: 12,
            day: 1
        }))
    );
}

#[test]
fn a_range_follows_the_zone_not_utc() {
    // 23:00 UTC Sunday is already Monday at +2h.
    let sunday_late = days_from_civil(Civil {
        year: 2026,
        month: 11,
        day: 8,
    }) * DAY
        + 23 * 3_600_000;
    let plus_two = |_: i64| 7_200;
    let (from, to) = range_for(RangePreset::Today, sunday_late, &plus_two, 0);
    assert_eq!(
        from,
        Some(
            day(Civil {
                year: 2026,
                month: 11,
                day: 9
            }) - 7_200_000
        )
    );
    assert_eq!(
        to,
        Some(
            day(Civil {
                year: 2026,
                month: 11,
                day: 10
            }) - 7_200_000
        )
    );
}

#[test]
fn the_previous_window_is_the_same_length_just_before() {
    assert_eq!(previous_window(Some(100), Some(160)), Some((40, 100)));
    assert_eq!(previous_window(None, Some(160)), None);
    assert_eq!(previous_window(Some(100), None), None);
    assert_eq!(previous_window(Some(100), Some(100)), None);
}
