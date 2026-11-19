use super::*;

const NOW: i64 = 10_000_000_000;

fn liquidation(side: Side, notional: f64, age_ms: i64) -> Liquidation {
    Liquidation {
        exchange: Exchange::Binance,
        symbol: "BTCUSDT".to_owned(),
        side,
        price: 100.0,
        quantity: notional / 100.0,
        time_ms: NOW - age_ms,
    }
}

fn tape(events: &[(Side, f64, i64)]) -> Tape {
    let mut tape = Tape::default();
    // Oldest first, as they arrive.
    let mut events = events.to_vec();
    events.sort_by_key(|&(_, _, age)| std::cmp::Reverse(age));
    for (side, notional, age) in events {
        tape.push(liquidation(side, notional, age), NOW);
    }
    tape
}

#[test]
fn a_liquidation_is_worth_its_price_times_its_size() {
    assert_eq!(liquidation(Side::Long, 5_000.0, 0).notional(), 5_000.0);
}

#[test]
fn totals_reach_back_a_minute_five_minutes_and_an_hour() {
    let tape = tape(&[
        (Side::Long, 1_000.0, 10_000),
        (Side::Short, 2_000.0, 120_000),
        (Side::Long, 4_000.0, 1_800_000),
        (Side::Short, 8_000.0, 3_599_000),
    ]);
    let snapshot = tape.snapshot(NOW, MinSize::Any);
    assert_eq!(
        snapshot.minute,
        Totals {
            long: 1_000.0,
            short: 0.0
        }
    );
    assert_eq!(
        snapshot.five_minutes,
        Totals {
            long: 1_000.0,
            short: 2_000.0
        }
    );
    assert_eq!(
        snapshot.hour,
        Totals {
            long: 5_000.0,
            short: 10_000.0
        }
    );
    assert_eq!(snapshot.count, 4);
    assert_eq!(snapshot.hour.total(), 15_000.0);
}

#[test]
fn the_totals_roll_off_as_time_passes_without_a_new_liquidation() {
    let tape = tape(&[(Side::Long, 1_000.0, 30_000)]);
    assert_eq!(tape.snapshot(NOW, MinSize::Any).minute.long, 1_000.0);
    assert_eq!(tape.snapshot(NOW + 40_000, MinSize::Any).minute.long, 0.0);
    assert_eq!(tape.snapshot(NOW + HOUR_MS, MinSize::Any).count, 0);
}

#[test]
fn the_tape_lists_the_newest_first_and_only_the_size_asked_for() {
    let tape = tape(&[
        (Side::Long, 500.0, 30_000),
        (Side::Short, 50_000.0, 20_000),
        (Side::Long, 2_000_000.0, 10_000),
    ]);
    let sizes = |minimum| {
        tape.snapshot(NOW, minimum)
            .latest
            .iter()
            .map(Liquidation::notional)
            .collect::<Vec<_>>()
    };
    assert_eq!(sizes(MinSize::Any), [2_000_000.0, 50_000.0, 500.0]);
    assert_eq!(sizes(MinSize::TenThousand), [2_000_000.0, 50_000.0]);
    assert_eq!(sizes(MinSize::Million), [2_000_000.0]);
    // The size only filters the rows: the totals count what was left out.
    assert_eq!(
        tape.snapshot(NOW, MinSize::Million).hour.total(),
        2_050_500.0
    );
}

#[test]
fn a_busy_hour_is_capped_at_the_most_rows_and_events() {
    let mut tape = Tape::default();
    for index in 0..(MAX_EVENTS + 50) {
        tape.push(
            liquidation(Side::Long, 100.0, 1_000 + index as i64 % 500),
            NOW,
        );
    }
    let snapshot = tape.snapshot(NOW, MinSize::Any);
    assert_eq!(snapshot.count, MAX_EVENTS);
    assert_eq!(snapshot.latest.len(), TAPE_ROWS);
}

#[test]
fn what_is_older_than_an_hour_is_dropped_when_the_next_one_arrives() {
    let mut tape = Tape::default();
    tape.push(
        liquidation(Side::Long, 100.0, HOUR_MS + 5_000),
        NOW - 10_000,
    );
    assert_eq!(tape.snapshot(NOW - 10_000, MinSize::Any).count, 1);
    tape.push(liquidation(Side::Short, 100.0, 0), NOW);
    assert_eq!(tape.snapshot(NOW, MinSize::Any).count, 1);
}

#[test]
fn the_revision_follows_every_change() {
    let mut tape = Tape::default();
    let start = tape.revision();
    tape.push(liquidation(Side::Long, 100.0, 0), NOW);
    assert_eq!(tape.revision(), start + 1);
    tape.clear();
    assert_eq!(tape.revision(), start + 2);
    assert_eq!(tape.snapshot(NOW, MinSize::Any), Snapshot::default());
}

#[test]
fn every_size_has_a_label_and_they_grow() {
    let sizes: Vec<f64> = MinSize::ALL.iter().map(|size| size.notional()).collect();
    assert!(sizes.windows(2).all(|pair| pair[0] < pair[1]));
    assert_eq!(MinSize::default(), MinSize::Any);
    assert!(MinSize::ALL.iter().all(|size| !size.label().is_empty()));
}
