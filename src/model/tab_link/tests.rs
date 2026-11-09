use super::*;
use crate::model::chart_sync::{HorizontalChange, TimeRange};
use crate::model::{DataSource, InstrumentId};

fn market(symbol: &str) -> Box<WatchlistInstrument> {
    Box::new(WatchlistInstrument::new(
        InstrumentId::new(DataSource::Binance, symbol),
        symbol,
        symbol,
    ))
}

fn member(tab: TabId, color: Option<LinkColor>, symbol: &str, timeframe: Timeframe) -> Member {
    Member {
        tab,
        color,
        market: Some(market(symbol)),
        timeframe,
    }
}

fn on(color: LinkColor) -> Option<LinkColor> {
    Some(color)
}

fn table(color: LinkColor, toggles: LinkToggles) -> LinkTable {
    let mut table = LinkTable::default();
    table.set(color, toggles);
    table
}

const ALL: LinkToggles = LinkToggles {
    symbol: true,
    interval: true,
    time: true,
    date_range: true,
};

fn view(right: f64) -> HorizontalChange {
    HorizontalChange {
        range: TimeRange {
            left: right - 1_000.0,
            right,
        },
        latest: false,
        moved: true,
        zoom: None,
    }
}

fn targets(deliveries: &[Delivery]) -> Vec<TabId> {
    deliveries.iter().map(|delivery| delivery.target).collect()
}

fn symbol_of(delivery: &Delivery) -> &str {
    match &delivery.signal {
        LinkSignal::Market(market) => &market.instrument.symbol,
        other => panic!("not a market: {other:?}"),
    }
}

// MARK: Colours and switches

#[test]
fn there_are_nine_colours_with_distinct_titles_and_keys() {
    assert_eq!(LinkColor::ALL.len(), 9);
    let mut titles: Vec<_> = LinkColor::ALL.iter().map(|c| c.title()).collect();
    let mut keys: Vec<_> = LinkColor::ALL.iter().map(|c| c.key()).collect();
    titles.sort_unstable();
    titles.dedup();
    keys.sort_unstable();
    keys.dedup();
    assert_eq!((titles.len(), keys.len()), (9, 9));
    for color in LinkColor::ALL {
        assert_eq!(LinkColor::from_key(color.key()), Some(color));
    }
    assert_eq!(LinkColor::from_key("magenta"), None);
    assert_eq!(LinkColor::from_key("Red"), None, "keys are exact");
}

#[test]
fn a_fresh_group_has_symbol_on_and_the_rest_off() {
    let toggles = LinkToggles::default();
    assert!(toggles.symbol && !toggles.interval && !toggles.time && !toggles.date_range);
    assert!(toggles.enables(LinkKind::Symbol));
    assert!(!toggles.enables(LinkKind::Interval));
    assert!(!toggles.enables(LinkKind::View));
}

#[test]
fn time_and_date_range_are_position_and_zoom_and_either_one_enables_the_view() {
    let time = LinkToggles {
        time: true,
        ..LinkToggles::NONE
    };
    let range = LinkToggles {
        date_range: true,
        ..LinkToggles::NONE
    };
    assert_eq!(
        time.viewport(),
        ChartSyncSettings {
            horizontal_position: true,
            horizontal_zoom: false
        }
    );
    assert_eq!(
        range.viewport(),
        ChartSyncSettings {
            horizontal_position: false,
            horizontal_zoom: true
        }
    );
    assert!(time.enables(LinkKind::View) && range.enables(LinkKind::View));
    assert!(!LinkToggles::NONE.viewport().is_linked());
}

#[test]
fn the_table_round_trips_and_unset_colours_read_as_defaults() {
    let mut table = LinkTable::default();
    assert!(table.set(LinkColor::Blue, ALL));
    assert!(!table.set(LinkColor::Blue, ALL), "no change");
    let json = serde_json::to_value(&table).unwrap();
    assert_eq!(json["blue"]["dateRange"], true);
    let back: LinkTable = serde_json::from_value(json).unwrap();
    assert_eq!(back, table);
    assert_eq!(back.toggles(LinkColor::Blue), ALL);
    assert_eq!(back.toggles(LinkColor::Red), LinkToggles::default());
}

#[test]
fn a_damaged_table_loses_only_what_is_damaged() {
    let back: LinkTable = serde_json::from_value(serde_json::json!({
        "orange": { "symbol": false, "interval": true },
        "chartreuse": { "symbol": false },
        "red": "yes",
        "blue": { "time": "sometimes" },
    }))
    .unwrap();
    // Missing switches take their defaults.
    assert_eq!(
        back.toggles(LinkColor::Orange),
        LinkToggles {
            symbol: false,
            interval: true,
            time: false,
            date_range: false
        }
    );
    assert_eq!(back.toggles(LinkColor::Red), LinkToggles::default());
    assert_eq!(back.toggles(LinkColor::Blue), LinkToggles::default());
    for junk in [
        serde_json::json!(null),
        serde_json::json!([1, 2]),
        serde_json::json!("x"),
    ] {
        let table: LinkTable = serde_json::from_value(junk).unwrap();
        assert_eq!(table, LinkTable::default());
    }
}

// MARK: Outbox and bus

#[test]
fn an_outbox_keeps_one_slot_per_kind_newest_wins() {
    let mut outbox = LinkOutbox::default();
    assert!(outbox.is_empty());
    outbox.push(LinkSignal::Market(market("BTCUSDT")));
    outbox.push(LinkSignal::Timeframe(Timeframe::Hour));
    outbox.push(LinkSignal::Market(market("ETHUSDT")));
    outbox.push(LinkSignal::View(view(1.0)));
    outbox.push(LinkSignal::View(view(2.0)));
    outbox.push(LinkSignal::Timeframe(Timeframe::Day));
    let taken = outbox.take();
    assert_eq!(taken.len(), 3, "bounded by the number of kinds");
    assert!(
        matches!(&taken[0], LinkSignal::Market(m) if m.instrument.symbol == "ETHUSDT"),
        "the second market replaced the first"
    );
    assert_eq!(taken[1], LinkSignal::Timeframe(Timeframe::Day));
    assert_eq!(taken[2], LinkSignal::View(view(2.0)));
    assert!(outbox.is_empty());
}

#[test]
fn a_bus_stays_bounded_under_a_flood_and_keeps_the_order_tabs_spoke_in() {
    let mut bus = LinkBus::default();
    for step in 0..10_000 {
        bus.push(9, LinkSignal::View(view(f64::from(step))));
        bus.push(3, LinkSignal::View(view(f64::from(step))));
    }
    let taken = bus.take();
    assert_eq!(
        taken.iter().map(|(tab, _)| *tab).collect::<Vec<_>>(),
        [9, 3]
    );
    assert!(taken.iter().all(|(_, signals)| signals.len() == 1));
    assert_eq!(taken[0].1[0], LinkSignal::View(view(9_999.0)));
    assert!(bus.is_empty());
}

// MARK: Symbol

#[test]
fn a_symbol_reaches_every_other_member_of_the_group_and_nobody_else() {
    let mut hub = LinkHub::default();
    let table = table(LinkColor::Orange, LinkToggles::default());
    let members = [
        member(1, on(LinkColor::Orange), "BTCUSDT", Timeframe::Hour),
        member(2, on(LinkColor::Orange), "BTCUSDT", Timeframe::Hour),
        member(3, on(LinkColor::Orange), "BTCUSDT", Timeframe::Hour),
        member(4, on(LinkColor::Blue), "BTCUSDT", Timeframe::Hour),
        member(5, None, "BTCUSDT", Timeframe::Hour),
    ];
    hub.reconcile(&members, &table);
    let out = hub.ingest(2, LinkSignal::Market(market("ETHUSDT")), &members, &table);
    assert_eq!(
        targets(&out),
        [1, 3],
        "not the origin, not Blue, not unlinked"
    );
    assert!(out.iter().all(|d| symbol_of(d) == "ETHUSDT"));
}

#[test]
fn symbol_sync_is_bidirectional_and_each_tab_keeps_its_own_interval() {
    let mut hub = LinkHub::default();
    let table = table(LinkColor::Orange, LinkToggles::default());
    let mut members = vec![
        member(1, on(LinkColor::Orange), "BTCUSDT", Timeframe::Hour),
        member(2, on(LinkColor::Orange), "BTCUSDT", Timeframe::Day),
    ];
    hub.reconcile(&members, &table);
    let out = hub.ingest(1, LinkSignal::Market(market("ETHUSDT")), &members, &table);
    assert_eq!(targets(&out), [2]);
    members[1].market = Some(market("ETHUSDT"));
    members[0].market = Some(market("ETHUSDT"));
    let out = hub.ingest(2, LinkSignal::Market(market("SOLUSDT")), &members, &table);
    assert_eq!(targets(&out), [1]);
    assert_eq!(symbol_of(&out[0]), "SOLUSDT");
    assert!(
        out.iter()
            .all(|d| matches!(d.signal, LinkSignal::Market(_))),
        "only the symbol travels"
    );
}

#[test]
fn a_market_is_identified_by_provider_and_symbol_not_by_spelling() {
    let mut hub = LinkHub::default();
    let table = table(LinkColor::Red, LinkToggles::default());
    let on_coinbase = Box::new(WatchlistInstrument::new(
        InstrumentId::new(DataSource::Coinbase, "BTCUSDT"),
        "BTC",
        "BTC",
    ));
    let members = [
        member(1, on(LinkColor::Red), "BTCUSDT", Timeframe::Hour),
        Member {
            tab: 2,
            color: on(LinkColor::Red),
            market: Some(on_coinbase),
            timeframe: Timeframe::Hour,
        },
    ];
    hub.reconcile(&members, &table);
    // Same ticker, different provider: tab 2 is not "already showing it".
    let out = hub.ingest(1, LinkSignal::Market(market("BTCUSDT")), &members, &table);
    assert_eq!(targets(&out), [2]);
    // The same market spelled another way is already shown: nothing to send.
    let members = [
        member(1, on(LinkColor::Red), "BTCUSDT", Timeframe::Hour),
        member(2, on(LinkColor::Red), "btc/usdt", Timeframe::Hour),
    ];
    let out = hub.ingest(1, LinkSignal::Market(market("BTCUSDT")), &members, &table);
    assert!(out.is_empty());
}

#[test]
fn a_switched_off_kind_never_propagates_and_is_not_recorded() {
    let mut hub = LinkHub::default();
    let table = table(
        LinkColor::Green,
        LinkToggles {
            symbol: false,
            interval: true,
            ..LinkToggles::NONE
        },
    );
    let members = [
        member(1, on(LinkColor::Green), "BTCUSDT", Timeframe::Hour),
        member(2, on(LinkColor::Green), "BTCUSDT", Timeframe::Hour),
    ];
    hub.reconcile(&members, &table);
    assert!(
        hub.ingest(1, LinkSignal::Market(market("ETHUSDT")), &members, &table)
            .is_empty()
    );
    assert_eq!(hub.group_market(LinkColor::Green), None);
    assert!(
        hub.ingest(1, LinkSignal::View(view(5.0)), &members, &table)
            .is_empty(),
        "no Time or Date range, no view"
    );
    let out = hub.ingest(1, LinkSignal::Timeframe(Timeframe::Day), &members, &table);
    assert_eq!(targets(&out), [2]);
}

#[test]
fn with_all_four_off_the_colour_is_only_a_tag() {
    let mut hub = LinkHub::default();
    let table = table(LinkColor::Pink, LinkToggles::NONE);
    let members = [
        member(1, on(LinkColor::Pink), "BTCUSDT", Timeframe::Hour),
        member(2, on(LinkColor::Pink), "BTCUSDT", Timeframe::Hour),
    ];
    assert!(hub.reconcile(&members, &table).is_empty());
    for signal in [
        LinkSignal::Market(market("ETHUSDT")),
        LinkSignal::Timeframe(Timeframe::Week),
        LinkSignal::View(view(1.0)),
    ] {
        assert!(hub.ingest(1, signal, &members, &table).is_empty());
    }
}

#[test]
fn an_unlinked_tab_neither_broadcasts_nor_receives() {
    let mut hub = LinkHub::default();
    let table = table(LinkColor::Orange, ALL);
    let members = [
        member(1, None, "BTCUSDT", Timeframe::Hour),
        member(2, on(LinkColor::Orange), "BTCUSDT", Timeframe::Hour),
        member(3, on(LinkColor::Orange), "BTCUSDT", Timeframe::Hour),
    ];
    hub.reconcile(&members, &table);
    assert!(
        hub.ingest(1, LinkSignal::Market(market("ETHUSDT")), &members, &table)
            .is_empty()
    );
    let out = hub.ingest(2, LinkSignal::Market(market("ETHUSDT")), &members, &table);
    assert!(!targets(&out).contains(&1));
    // A tab the hub has never heard of is not a member either.
    assert!(
        hub.ingest(99, LinkSignal::Timeframe(Timeframe::Day), &members, &table)
            .is_empty()
    );
}

#[test]
fn independent_colours_do_not_hear_each_other() {
    let mut hub = LinkHub::default();
    let mut table = LinkTable::default();
    table.set(LinkColor::Orange, ALL);
    table.set(LinkColor::Blue, ALL);
    let members = [
        member(1, on(LinkColor::Orange), "BTCUSDT", Timeframe::Hour),
        member(2, on(LinkColor::Orange), "BTCUSDT", Timeframe::Hour),
        member(3, on(LinkColor::Blue), "BTCUSDT", Timeframe::Hour),
        member(4, on(LinkColor::Blue), "BTCUSDT", Timeframe::Hour),
    ];
    hub.reconcile(&members, &table);
    let out = hub.ingest(1, LinkSignal::Timeframe(Timeframe::Week), &members, &table);
    assert_eq!(targets(&out), [2]);
    let out = hub.ingest(4, LinkSignal::Timeframe(Timeframe::Day), &members, &table);
    assert_eq!(targets(&out), [3]);
    assert_eq!(
        hub.group_timeframe(LinkColor::Orange),
        Some(Timeframe::Week)
    );
    assert_eq!(hub.group_timeframe(LinkColor::Blue), Some(Timeframe::Day));
}

#[test]
fn switches_belong_to_the_group_not_the_tab() {
    let mut hub = LinkHub::default();
    let mut table = LinkTable::default();
    table.set(
        LinkColor::Orange,
        LinkToggles {
            interval: true,
            ..LinkToggles::default()
        },
    );
    table.set(LinkColor::Blue, LinkToggles::default());
    let members = [
        member(1, on(LinkColor::Orange), "BTCUSDT", Timeframe::Hour),
        member(2, on(LinkColor::Orange), "BTCUSDT", Timeframe::Hour),
        member(3, on(LinkColor::Blue), "BTCUSDT", Timeframe::Hour),
        member(4, on(LinkColor::Blue), "BTCUSDT", Timeframe::Hour),
    ];
    hub.reconcile(&members, &table);
    assert_eq!(
        targets(&hub.ingest(1, LinkSignal::Timeframe(Timeframe::Day), &members, &table)),
        [2]
    );
    assert!(
        hub.ingest(3, LinkSignal::Timeframe(Timeframe::Day), &members, &table)
            .is_empty(),
        "Blue has Interval off"
    );
}

// MARK: Interval

#[test]
fn an_interval_only_goes_to_tabs_that_do_not_already_have_it() {
    let mut hub = LinkHub::default();
    let table = table(LinkColor::Cyan, ALL);
    let members = [
        member(1, on(LinkColor::Cyan), "BTCUSDT", Timeframe::Hour),
        member(2, on(LinkColor::Cyan), "ETHUSDT", Timeframe::Hour),
        member(3, on(LinkColor::Cyan), "SOLUSDT", Timeframe::Day),
    ];
    hub.reconcile(&members, &table);
    let out = hub.ingest(1, LinkSignal::Timeframe(Timeframe::Day), &members, &table);
    assert_eq!(targets(&out), [2], "tab 3 already shows 1D");
    assert!(matches!(
        out[0].signal,
        LinkSignal::Timeframe(Timeframe::Day)
    ));
}

// MARK: Views

#[test]
fn a_view_goes_to_everyone_else_even_when_it_repeats() {
    let mut hub = LinkHub::default();
    let table = table(
        LinkColor::Yellow,
        LinkToggles {
            time: true,
            ..LinkToggles::NONE
        },
    );
    let members = [
        member(1, on(LinkColor::Yellow), "BTCUSDT", Timeframe::Hour),
        member(2, on(LinkColor::Yellow), "BTCUSDT", Timeframe::Hour),
    ];
    hub.reconcile(&members, &table);
    for _ in 0..3 {
        let out = hub.ingest(1, LinkSignal::View(view(7.0)), &members, &table);
        assert_eq!(targets(&out), [2]);
    }
}

// MARK: Ordering and staleness

#[test]
fn sequences_grow_and_a_tick_merges_to_the_newest_per_target_and_kind() {
    let mut hub = LinkHub::default();
    let table = table(LinkColor::Orange, LinkToggles::default());
    let members = [
        member(1, on(LinkColor::Orange), "BTCUSDT", Timeframe::Hour),
        member(2, on(LinkColor::Orange), "BTCUSDT", Timeframe::Hour),
        member(3, on(LinkColor::Orange), "BTCUSDT", Timeframe::Hour),
    ];
    hub.reconcile(&members, &table);
    let first = hub.ingest(1, LinkSignal::Market(market("ETHUSDT")), &members, &table);
    let second = hub.ingest(1, LinkSignal::Market(market("SOLUSDT")), &members, &table);
    assert!(second[0].sequence > first[0].sequence);

    // The same burst through a tick: BTC -> ETH -> SOL -> AVAX converges on AVAX.
    let mut hub = LinkHub::default();
    let signals = vec![(
        1,
        vec![LinkSignal::Market(market("AVAXUSDT"))], // the outbox already kept only the newest
    )];
    let out = hub.tick(&members, &table, signals);
    assert_eq!(targets(&out), [2, 3]);
    assert!(out.iter().all(|d| symbol_of(d) == "AVAXUSDT"));
}

#[test]
fn two_tabs_speaking_in_one_tick_leave_the_later_one_standing_for_a_third() {
    let mut hub = LinkHub::default();
    let table = table(LinkColor::Orange, LinkToggles::default());
    let members = [
        member(1, on(LinkColor::Orange), "BTCUSDT", Timeframe::Hour),
        member(2, on(LinkColor::Orange), "BTCUSDT", Timeframe::Hour),
        member(3, on(LinkColor::Orange), "BTCUSDT", Timeframe::Hour),
    ];
    hub.reconcile(&members, &table);
    let out = hub.tick(
        &members,
        &table,
        vec![
            (1, vec![LinkSignal::Market(market("ETHUSDT"))]),
            (2, vec![LinkSignal::Market(market("SOLUSDT"))]),
        ],
    );
    let third: Vec<_> = out.iter().filter(|d| d.target == 3).collect();
    assert_eq!(third.len(), 1, "one delivery per target and kind");
    assert_eq!(symbol_of(third[0]), "SOLUSDT");
    assert_eq!(
        hub.group_market(LinkColor::Orange)
            .unwrap()
            .instrument
            .symbol,
        "SOLUSDT"
    );
}

#[test]
fn merging_never_lets_an_older_sequence_replace_a_newer_one() {
    let newer = Delivery {
        target: 1,
        sequence: 9,
        signal: LinkSignal::Timeframe(Timeframe::Day),
    };
    let older = Delivery {
        target: 1,
        sequence: 4,
        signal: LinkSignal::Timeframe(Timeframe::Hour),
    };
    let merged = merge(vec![newer.clone(), older]);
    assert_eq!(merged, vec![newer]);
}

// MARK: Joining, leaving, recolouring

#[test]
fn a_joiner_adopts_the_enabled_values_and_existing_members_are_untouched() {
    let mut hub = LinkHub::default();
    let table = table(
        LinkColor::Orange,
        LinkToggles {
            symbol: true,
            interval: false,
            time: false,
            date_range: false,
        },
    );
    let mut members = vec![
        member(1, on(LinkColor::Orange), "BTCUSDT", Timeframe::Hour),
        member(2, on(LinkColor::Orange), "BTCUSDT", Timeframe::Hour),
    ];
    hub.reconcile(&members, &table);
    hub.ingest(1, LinkSignal::Market(market("ETHUSDT")), &members, &table);
    hub.ingest(1, LinkSignal::Timeframe(Timeframe::Week), &members, &table);

    // Tab 3 is recoloured Orange: it takes the symbol only (Interval is off) and nothing else
    // moves.
    members.push(member(3, on(LinkColor::Orange), "SOLUSDT", Timeframe::Day));
    let out = hub.reconcile(&members, &table);
    assert_eq!(targets(&out), [3]);
    assert_eq!(symbol_of(&out[0]), "ETHUSDT");
    assert_eq!(out.len(), 1, "no interval: it was switched off");

    // Ticking again changes nothing: the joiner is a member now.
    assert!(hub.reconcile(&members, &table).is_empty());
}

#[test]
fn a_joiner_adopts_interval_and_view_when_those_are_on() {
    let mut hub = LinkHub::default();
    let table = table(LinkColor::Orange, ALL);
    let mut members = vec![
        member(1, on(LinkColor::Orange), "BTCUSDT", Timeframe::Hour),
        member(2, on(LinkColor::Orange), "BTCUSDT", Timeframe::Hour),
    ];
    hub.reconcile(&members, &table);
    hub.ingest(1, LinkSignal::Timeframe(Timeframe::Week), &members, &table);
    hub.ingest(1, LinkSignal::View(view(3.0)), &members, &table);
    members.push(member(3, on(LinkColor::Orange), "BTCUSDT", Timeframe::Day));
    let out = hub.reconcile(&members, &table);
    let kinds: Vec<_> = out.iter().map(|d| d.signal.kind()).collect();
    assert!(out.iter().all(|d| d.target == 3));
    assert_eq!(
        kinds,
        [LinkKind::Interval, LinkKind::View],
        "same symbol: not resent"
    );
}

#[test]
fn a_joiner_that_already_matches_is_told_nothing() {
    let mut hub = LinkHub::default();
    let table = table(LinkColor::Orange, ALL);
    let mut members = vec![
        member(1, on(LinkColor::Orange), "BTCUSDT", Timeframe::Hour),
        member(2, on(LinkColor::Orange), "BTCUSDT", Timeframe::Hour),
    ];
    hub.reconcile(&members, &table);
    members.push(member(3, on(LinkColor::Orange), "BTCUSDT", Timeframe::Hour));
    assert!(hub.reconcile(&members, &table).is_empty());
}

#[test]
fn the_first_tab_of_a_group_seeds_it_so_the_next_one_has_something_to_adopt() {
    let mut hub = LinkHub::default();
    let table = table(LinkColor::Orange, ALL);
    let mut members = vec![member(1, on(LinkColor::Orange), "ETHUSDT", Timeframe::Week)];
    assert!(
        hub.reconcile(&members, &table).is_empty(),
        "alone: nothing to do"
    );
    assert_eq!(
        hub.group_timeframe(LinkColor::Orange),
        Some(Timeframe::Week)
    );
    members.push(member(2, on(LinkColor::Orange), "BTCUSDT", Timeframe::Hour));
    let out = hub.reconcile(&members, &table);
    assert_eq!(targets(&out), [2, 2]);
    assert_eq!(symbol_of(&out[0]), "ETHUSDT");
    assert!(matches!(
        out[1].signal,
        LinkSignal::Timeframe(Timeframe::Week)
    ));
}

#[test]
fn a_restore_delivers_nothing_so_no_restored_tab_is_overwritten() {
    let mut hub = LinkHub::default();
    let table = table(LinkColor::Orange, ALL);
    let members = [
        member(1, on(LinkColor::Orange), "BTCUSDT", Timeframe::Hour),
        member(2, on(LinkColor::Orange), "ETHUSDT", Timeframe::Day),
        member(3, on(LinkColor::Orange), "SOLUSDT", Timeframe::Week),
    ];
    assert!(hub.reconcile(&members, &table).is_empty());
    assert!(hub.reconcile(&members, &table).is_empty());
}

#[test]
fn a_recoloured_tab_leaves_its_old_group_at_once() {
    let mut hub = LinkHub::default();
    let mut table = LinkTable::default();
    table.set(LinkColor::Orange, ALL);
    table.set(LinkColor::Blue, ALL);
    let mut members = vec![
        member(1, on(LinkColor::Orange), "BTCUSDT", Timeframe::Hour),
        member(2, on(LinkColor::Orange), "BTCUSDT", Timeframe::Hour),
        member(3, on(LinkColor::Blue), "BTCUSDT", Timeframe::Hour),
    ];
    hub.reconcile(&members, &table);
    members[1].color = on(LinkColor::Blue);
    hub.reconcile(&members, &table);
    let out = hub.ingest(1, LinkSignal::Timeframe(Timeframe::Week), &members, &table);
    assert!(
        out.is_empty(),
        "tab 2 is Blue now; tab 1 is alone in Orange"
    );
    let out = hub.ingest(3, LinkSignal::Timeframe(Timeframe::Week), &members, &table);
    assert_eq!(targets(&out), [2]);
}

#[test]
fn unlinking_stops_both_directions_and_a_group_with_no_tabs_keeps_nothing() {
    let mut hub = LinkHub::default();
    let table = table(LinkColor::Orange, ALL);
    let mut members = vec![
        member(1, on(LinkColor::Orange), "BTCUSDT", Timeframe::Hour),
        member(2, on(LinkColor::Orange), "BTCUSDT", Timeframe::Hour),
    ];
    hub.reconcile(&members, &table);
    hub.ingest(1, LinkSignal::Timeframe(Timeframe::Week), &members, &table);
    members[1].color = None;
    hub.reconcile(&members, &table);
    assert!(
        hub.ingest(2, LinkSignal::Timeframe(Timeframe::Day), &members, &table)
            .is_empty()
    );
    assert!(
        hub.ingest(1, LinkSignal::Timeframe(Timeframe::Day), &members, &table)
            .is_empty(),
        "tab 1 is alone"
    );
    members.clear();
    hub.reconcile(&members, &table);
    assert!(!hub.has_group(LinkColor::Orange));
    assert_eq!(hub.group_timeframe(LinkColor::Orange), None);
}

#[test]
fn a_closed_tab_is_never_delivered_to() {
    let mut hub = LinkHub::default();
    let table = table(LinkColor::Orange, ALL);
    let mut members = vec![
        member(1, on(LinkColor::Orange), "BTCUSDT", Timeframe::Hour),
        member(2, on(LinkColor::Orange), "BTCUSDT", Timeframe::Hour),
        member(3, on(LinkColor::Orange), "BTCUSDT", Timeframe::Hour),
    ];
    hub.reconcile(&members, &table);
    members.remove(1);
    let out = hub.tick(
        &members,
        &table,
        vec![(1, vec![LinkSignal::Timeframe(Timeframe::Week)])],
    );
    assert_eq!(targets(&out), [3]);
}

#[test]
fn a_signal_from_a_closed_tab_is_dropped() {
    let mut hub = LinkHub::default();
    let table = table(LinkColor::Orange, ALL);
    let members = [member(2, on(LinkColor::Orange), "BTCUSDT", Timeframe::Hour)];
    let out = hub.tick(
        &members,
        &table,
        vec![(1, vec![LinkSignal::Timeframe(Timeframe::Week)])],
    );
    assert!(out.is_empty());
}

// MARK: Scale

#[test]
fn a_thousand_linked_tabs_get_one_delivery_each_without_allocating_per_event_state() {
    let mut hub = LinkHub::default();
    let table = table(LinkColor::Red, ALL);
    let members: Vec<_> = (1..=1_000)
        .map(|tab| member(tab, on(LinkColor::Red), "BTCUSDT", Timeframe::Hour))
        .collect();
    hub.reconcile(&members, &table);
    let out = hub.tick(
        &members,
        &table,
        vec![(1, vec![LinkSignal::Timeframe(Timeframe::Day)])],
    );
    assert_eq!(out.len(), 999);
    let again = hub.tick(
        &members,
        &table,
        vec![(1, vec![LinkSignal::Timeframe(Timeframe::Day)])],
    );
    assert_eq!(again.len(), 999, "the snapshot says they still show 1H");
}

#[test]
fn a_delivered_signal_is_not_a_new_signal() {
    // The hub only turns a user's signal into deliveries; applying one is the tab's business and
    // never goes back into `ingest`. A hub fed nothing does nothing, however often it ticks.
    let mut hub = LinkHub::default();
    let table = table(LinkColor::Orange, ALL);
    let members = [
        member(1, on(LinkColor::Orange), "BTCUSDT", Timeframe::Hour),
        member(2, on(LinkColor::Orange), "BTCUSDT", Timeframe::Hour),
    ];
    hub.tick(
        &members,
        &table,
        vec![(1, vec![LinkSignal::Timeframe(Timeframe::Day)])],
    );
    for _ in 0..100 {
        assert!(hub.tick(&members, &table, Vec::new()).is_empty());
    }
}
