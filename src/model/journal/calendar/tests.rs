use super::*;
use crate::model::journal::model::Side;
use crate::model::journal::reconstruct::reconstruct;
use crate::model::journal::testing::{d, exec};

fn ms(day: i64, hour: i64) -> i64 {
    (day * 86_400 + hour * 3_600) * 1_000
}

fn win_loss(trade: &Trade) -> Outcome {
    match trade.net_pnl().cmp(&Decimal::ZERO) {
        std::cmp::Ordering::Greater => Outcome::Win,
        std::cmp::Ordering::Less => Outcome::Loss,
        std::cmp::Ordering::Equal => Outcome::Breakeven,
    }
}

fn round_trip(
    id: u32,
    open: i64,
    close: i64,
    entry: &str,
    exit: &str,
) -> Vec<crate::model::journal::model::Execution> {
    vec![
        exec(&format!("{id}a"), Side::Buy, "1", entry, open),
        exec(&format!("{id}b"), Side::Sell, "1", exit, close),
    ]
}

#[test]
fn civil_dates_round_trip_across_centuries() {
    for days in [-800_000, -1, 0, 1, 59, 60, 19_000, 19_782, 800_000] {
        assert_eq!(days_from_civil(civil_from_days(days)), days);
    }
    assert_eq!(
        civil_from_days(0),
        Civil {
            year: 1970,
            month: 1,
            day: 1
        }
    );
    assert_eq!(
        civil_from_days(days_from_civil(Civil {
            year: 2024,
            month: 2,
            day: 29
        })),
        Civil {
            year: 2024,
            month: 2,
            day: 29
        }
    );
}

#[test]
fn weekdays_and_month_lengths() {
    assert_eq!(weekday(0), 4); // Thursday
    assert_eq!(
        weekday(days_from_civil(Civil {
            year: 2026,
            month: 8,
            day: 1
        })),
        6
    ); // Saturday
    assert_eq!(days_in_month(2024, 2), 29);
    assert_eq!(days_in_month(2100, 2), 28);
    assert_eq!(days_in_month(2000, 2), 29);
    assert_eq!(days_in_month(2026, 11), 30);
}

#[test]
fn a_day_follows_the_offset_at_that_instant() {
    // 23:30 UTC is already the next day at +1h and still the same day at -1h.
    let late = ms(10, 23) + 1_800_000;
    assert_eq!(day_number(late, &utc), 10);
    assert_eq!(day_number(late, &|_| 3_600), 11);
    assert_eq!(day_number(late, &|_| -3_600), 10);
    assert_eq!(day_number(-1, &utc), -1);
}

#[test]
fn a_daylight_saving_day_is_not_24_hours() {
    // Spring forward on day 100: offset +0 before 02:00 UTC of that day, +1h after.
    let change = ms(100, 2);
    let zone = |ts: i64| if ts >= change { 3_600 } else { 0 };
    let start = local_midnight_ms(100, &zone);
    let next = local_midnight_ms(101, &zone);
    assert_eq!(start, ms(100, 0));
    assert_eq!(next, ms(101, 0) - 3_600_000);
    assert_eq!(next - start, 23 * 3_600_000);
}

#[test]
fn trades_count_on_the_day_they_close() {
    // Opens on day 10, closes on day 11 (midnight spanned).
    let executions = round_trip(1, ms(10, 23), ms(11, 1), "100", "110");
    let trades = reconstruct(&executions, |_| Default::default());
    let refs: Vec<&Trade> = trades.iter().collect();
    let days = daily(&refs, &utc, &win_loss);
    assert_eq!(days.len(), 1);
    assert_eq!(days[&11].net, d("10"));
    assert_eq!(days[&11].wins, 1);
}

#[test]
fn open_trades_belong_to_no_day() {
    let trades = reconstruct(&[exec("1", Side::Buy, "1", "100", ms(10, 1))], |_| {
        Default::default()
    });
    let refs: Vec<&Trade> = trades.iter().collect();
    assert!(daily(&refs, &utc, &win_loss).is_empty());
}

fn august() -> MonthView {
    let first = days_from_civil(Civil {
        year: 2026,
        month: 8,
        day: 1,
    });
    let mut executions = Vec::new();
    // Aug 3: -132.29 over 3 trades-ish; keep it simple: one loss, one win.
    executions.extend(round_trip(
        1,
        ms(first + 2, 1),
        ms(first + 2, 2),
        "100",
        "90",
    ));
    executions.extend(round_trip(
        2,
        ms(first + 2, 3),
        ms(first + 2, 4),
        "100",
        "103",
    ));
    executions.extend(round_trip(
        3,
        ms(first + 4, 1),
        ms(first + 4, 2),
        "100",
        "150",
    ));
    executions.extend(round_trip(
        4,
        ms(first + 9, 1),
        ms(first + 9, 2),
        "100",
        "95",
    ));
    // Outside the month.
    executions.extend(round_trip(
        5,
        ms(first + 40, 1),
        ms(first + 40, 2),
        "100",
        "101",
    ));
    let trades = reconstruct(&executions, |_| Default::default());
    let refs: Vec<&Trade> = trades.iter().collect();
    month_view(&refs, 2026, 8, &utc, &win_loss)
}

#[test]
fn a_month_totals_its_days() {
    let view = august();
    assert_eq!(view.trading_days, 3);
    assert_eq!(view.trades, 4);
    assert_eq!(view.net, d("-7") + d("50") + d("-5") + d("0"));
    assert_eq!(view.net, d("38"));
    assert_eq!(view.green_days, 1);
    assert_eq!(view.red_days, 2);
    assert_eq!(view.best_day.as_ref().unwrap().date.day, 5);
    assert_eq!(view.best_day.as_ref().unwrap().net, d("50"));
    assert_eq!(view.worst_day.as_ref().unwrap().date.day, 3);
    assert_eq!(view.worst_day.as_ref().unwrap().net, d("-7"));
    let third = &view.days[&3];
    assert_eq!((third.wins, third.losses, third.trades), (1, 1, 2));
    assert_eq!(third.win_rate(), Some(0.5));
}

#[test]
fn the_grid_starts_on_the_requested_weekday() {
    let view = august(); // 1 Aug 2026 is a Saturday
    let sunday = view.grid(0);
    assert_eq!(sunday[0], [None, None, None, None, None, None, Some(1)]);
    assert_eq!(sunday[1][0], Some(2));
    assert_eq!(sunday.last().unwrap()[1], Some(31)); // 31 Aug is a Monday
    let monday = view.grid(1);
    assert_eq!(monday[0], [None, None, None, None, None, Some(1), Some(2)]);
    assert!(sunday.iter().flatten().flatten().count() == 31);
}

#[test]
fn week_totals_add_up_to_the_month() {
    let view = august();
    let weeks = view.week_totals(0);
    assert_eq!(weeks.iter().map(|week| week.net).sum::<Decimal>(), view.net);
    assert_eq!(
        weeks.iter().map(|week| week.trades).sum::<usize>(),
        view.trades
    );
    // Aug 3 (Mon) and Aug 5 (Wed) share the second row.
    assert_eq!(weeks[1].net, d("43"));
    assert_eq!(weeks[1].trading_days, 2);
}

#[test]
fn a_month_with_no_trades_is_empty_not_zero_days() {
    let view = month_view(&[], 2026, 2, &utc, &win_loss);
    assert_eq!(view.trading_days, 0);
    assert!(view.best_day.is_none() && view.worst_day.is_none());
    assert_eq!(view.grid(0).iter().flatten().flatten().count(), 28);
}

#[test]
fn months_shift_across_year_ends() {
    assert_eq!(shift_month(2026, 1, -1), (2025, 12));
    assert_eq!(shift_month(2026, 12, 1), (2027, 1));
    assert_eq!(shift_month(2026, 6, 0), (2026, 6));
    assert_eq!(shift_month(2026, 3, -15), (2024, 12));
}
