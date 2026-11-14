use super::super::test_support::{chart_with_drawings, drawing_ids, empty_workspace};
use crate::model::tab::link::{LinkColor, LinkSignal};
use crate::model::watchlist::WatchlistInstrument;
use crate::model::{DataSource, InstrumentId, Timeframe};
use crate::service::ProviderError;
use crate::workspace::ReplayStatus;
use eframe::egui;

fn row(symbol: &str) -> WatchlistInstrument {
    WatchlistInstrument::from_market(DataSource::Binance, symbol, symbol)
}

fn market(symbol: &str) -> LinkSignal {
    LinkSignal::Market(Box::new(row(symbol)))
}

fn shown(workspace: &super::ChartWorkspace, chart_id: u64) -> String {
    workspace
        .charts
        .iter()
        .find(|chart| chart.id == chart_id)
        .map(|chart| chart.symbol.clone())
        .expect("chart")
}

#[test]
fn an_unlinked_tab_announces_nothing_and_ignores_deliveries() {
    let ctx = egui::Context::default();
    let mut workspace = empty_workspace(1);
    let chart = chart_with_drawings(&mut workspace, "BTCUSDT", &[1]);
    workspace.announce_market(&row("ETHUSDT"));
    workspace.announce_timeframe();
    assert!(workspace.take_link_signals().is_empty());
    assert!(!workspace.apply_link(1, &market("ETHUSDT"), &ctx));
    assert_eq!(shown(&workspace, chart), "BTCUSDT");
}

#[test]
fn a_linked_tab_queues_what_the_user_did_newest_per_kind() {
    let mut workspace = empty_workspace(1);
    workspace.set_link(Some(LinkColor::Orange));
    workspace.announce_market(&row("ETHUSDT"));
    workspace.announce_market(&row("SOLUSDT"));
    workspace.announce_timeframe();
    let signals = workspace.take_link_signals();
    assert_eq!(signals.len(), 2);
    assert!(matches!(&signals[0], LinkSignal::Market(m) if m.instrument.symbol == "SOLUSDT"));
    assert_eq!(signals[1], LinkSignal::Timeframe(Timeframe::Day));
    assert!(workspace.take_link_signals().is_empty());
}

#[test]
fn applying_a_symbol_changes_the_focused_chart_and_keeps_everything_else() {
    let ctx = egui::Context::default();
    let mut workspace = empty_workspace(1);
    let first = chart_with_drawings(&mut workspace, "BTCUSDT", &[1, 2]);
    let second = chart_with_drawings(&mut workspace, "SOLUSDT", &[3]);
    workspace.focused = Some(first);
    workspace.set_link(Some(LinkColor::Orange));
    let timeframe = workspace.timeframe;

    assert!(workspace.apply_link(1, &market("ETHUSDT"), &ctx));
    assert_eq!(shown(&workspace, first), "ETHUSDT");
    assert_eq!(
        shown(&workspace, second),
        "SOLUSDT",
        "only the focused chart"
    );
    assert_eq!(drawing_ids(&workspace, first), vec![1, 2], "drawings stay");
    assert_eq!(workspace.timeframe, timeframe);
    assert!(
        workspace.take_link_signals().is_empty(),
        "applying never announces"
    );
}

#[test]
fn a_symbol_the_tab_already_shows_changes_nothing() {
    let ctx = egui::Context::default();
    let mut workspace = empty_workspace(1);
    chart_with_drawings(&mut workspace, "BTCUSDT", &[]);
    workspace.set_link(Some(LinkColor::Red));
    assert!(!workspace.apply_link(1, &market("btcusdt"), &ctx));
    assert!(workspace.link_state.probation.is_none());
}

#[test]
fn applying_an_interval_moves_the_tab_and_keeps_its_symbols() {
    let ctx = egui::Context::default();
    let mut workspace = empty_workspace(1);
    let chart = chart_with_drawings(&mut workspace, "BTCUSDT", &[]);
    workspace.set_link(Some(LinkColor::Cyan));
    assert!(workspace.apply_link(1, &LinkSignal::Timeframe(Timeframe::Week), &ctx));
    assert_eq!(workspace.timeframe, Timeframe::Week);
    assert_eq!(shown(&workspace, chart), "BTCUSDT");
    assert!(workspace.take_link_signals().is_empty());
    assert!(!workspace.apply_link(2, &LinkSignal::Timeframe(Timeframe::Week), &ctx));
}

#[test]
fn a_delivery_older_than_one_already_applied_is_dropped() {
    let ctx = egui::Context::default();
    let mut workspace = empty_workspace(1);
    let chart = chart_with_drawings(&mut workspace, "BTCUSDT", &[]);
    workspace.set_link(Some(LinkColor::Pink));
    assert!(workspace.apply_link(7, &market("SOLUSDT"), &ctx));
    assert!(!workspace.apply_link(5, &market("ETHUSDT"), &ctx));
    assert!(!workspace.apply_link(7, &market("ETHUSDT"), &ctx));
    assert_eq!(shown(&workspace, chart), "SOLUSDT");
    // Another kind has its own counter.
    assert!(workspace.apply_link(6, &LinkSignal::Timeframe(Timeframe::Hour), &ctx));
}

#[test]
fn a_blank_tab_has_nothing_to_change() {
    let ctx = egui::Context::default();
    let mut workspace = empty_workspace(1);
    workspace.set_link(Some(LinkColor::Green));
    assert!(!workspace.apply_link(1, &market("ETHUSDT"), &ctx));
    assert!(workspace.charts.is_empty(), "no chart is invented");
}

#[test]
fn a_tab_in_replay_takes_neither_symbol_nor_interval() {
    let ctx = egui::Context::default();
    let mut workspace = empty_workspace(1);
    let chart = chart_with_drawings(&mut workspace, "BTCUSDT", &[]);
    workspace.set_link(Some(LinkColor::Blue));
    workspace.replay.status = ReplayStatus::Paused;
    assert!(!workspace.apply_link(1, &market("ETHUSDT"), &ctx));
    assert!(!workspace.apply_link(2, &LinkSignal::Timeframe(Timeframe::Hour), &ctx));
    assert_eq!(shown(&workspace, chart), "BTCUSDT");
    assert_eq!(
        workspace.replay.status,
        ReplayStatus::Paused,
        "replay untouched"
    );
    assert_eq!(workspace.timeframe, Timeframe::Day);
}

#[test]
fn the_hub_sees_the_focused_market_only_for_a_linked_tab() {
    let mut workspace = empty_workspace(4);
    let chart = chart_with_drawings(&mut workspace, "BTCUSDT", &[]);
    workspace.focused = Some(chart);
    assert_eq!(
        workspace.link_member().market,
        None,
        "unlinked: no row built"
    );
    workspace.set_link(Some(LinkColor::Violet));
    let member = workspace.link_member();
    assert_eq!(member.tab, 4);
    assert_eq!(member.color, Some(LinkColor::Violet));
    assert_eq!(
        member.market.map(|row| row.instrument),
        Some(InstrumentId::new(DataSource::Binance, "BTCUSDT"))
    );
}

#[test]
fn a_symbol_that_fails_to_load_is_put_back_and_the_tab_says_so() {
    let ctx = egui::Context::default();
    let mut workspace = empty_workspace(1);
    let chart = chart_with_drawings(&mut workspace, "BTCUSDT", &[1]);
    workspace.set_link(Some(LinkColor::Orange));
    assert!(workspace.apply_link(1, &market("NOPEUSDT"), &ctx));
    assert_eq!(shown(&workspace, chart), "NOPEUSDT");

    // Its first fetch fails with nothing on the chart.
    let index = workspace.chart_index(chart).unwrap();
    workspace.charts[index].inbox = None;
    workspace.charts[index].loading = false;
    workspace.charts[index].error = Some(ProviderError::other("Unknown symbol"));
    workspace.tick_link_probation(&ctx);

    assert_eq!(shown(&workspace, chart), "BTCUSDT");
    assert_eq!(drawing_ids(&workspace, chart), vec![1]);
    assert!(workspace.link_state.probation.is_none());
    assert!(workspace.take_link_signals().is_empty(), "not announced");
    let notice = workspace.context.notice.as_ref().expect("a notice");
    assert!(notice.error && notice.text.contains("NOPEUSDT"));
}

#[test]
fn a_symbol_that_loads_is_accepted_and_one_the_user_left_is_forgotten() {
    let ctx = egui::Context::default();
    let mut workspace = empty_workspace(1);
    let chart = chart_with_drawings(&mut workspace, "BTCUSDT", &[]);
    workspace.set_link(Some(LinkColor::Orange));
    workspace.apply_link(1, &market("ETHUSDT"), &ctx);
    let index = workspace.chart_index(chart).unwrap();
    workspace.charts[index]
        .candles
        .push(super::super::test_support::candle(1, 1.0));
    workspace.tick_link_probation(&ctx);
    assert!(workspace.link_state.probation.is_none());
    assert_eq!(shown(&workspace, chart), "ETHUSDT");

    workspace.apply_link(2, &market("SOLUSDT"), &ctx);
    workspace.charts[index].symbol = "ADAUSDT".into(); // the user chose another meanwhile
    workspace.charts[index].error = Some(ProviderError::other("x"));
    workspace.charts[index].loading = false;
    workspace.tick_link_probation(&ctx);
    assert_eq!(
        shown(&workspace, chart),
        "ADAUSDT",
        "not reverted over the user's choice"
    );
}

/// One app tick over a few workspaces: what `DegenViewApp::tick_tab_links` does.
fn tick(
    hub: &mut crate::model::tab::link::LinkHub,
    table: &crate::model::tab::link::LinkTable,
    tabs: &mut [&mut super::ChartWorkspace],
    ctx: &egui::Context,
) -> usize {
    let mut members = Vec::new();
    let mut signals = Vec::new();
    for tab in tabs.iter_mut() {
        let member = tab.link_member();
        let said = tab.take_link_signals();
        if !said.is_empty() {
            signals.push((member.tab, said));
        }
        members.push(member);
    }
    let mut applied = 0;
    for delivery in hub.tick(&members, table, signals) {
        let tab = tabs
            .iter_mut()
            .find(|tab| tab.id() == delivery.target)
            .expect("a member");
        applied += usize::from(tab.apply_link(delivery.sequence, &delivery.signal, ctx));
    }
    applied
}

#[test]
fn two_linked_tabs_follow_each_other_both_ways_and_settle() {
    let ctx = egui::Context::default();
    let mut hub = crate::model::tab::link::LinkHub::default();
    let table = crate::model::tab::link::LinkTable::default();
    let mut a = empty_workspace(1);
    let mut b = empty_workspace(2);
    let (chart_a, chart_b) = (
        chart_with_drawings(&mut a, "BTCUSDT", &[1]),
        chart_with_drawings(&mut b, "BTCUSDT", &[2]),
    );
    a.set_link(Some(LinkColor::Orange));
    b.set_link(Some(LinkColor::Orange));
    assert_eq!(tick(&mut hub, &table, &mut [&mut a, &mut b], &ctx), 0);

    // The user picks ETH in tab A, the way the symbol picker does.
    a.switch_chart(chart_a, &row("ETHUSDT"), &ctx);
    a.announce_market(&row("ETHUSDT"));
    assert_eq!(tick(&mut hub, &table, &mut [&mut a, &mut b], &ctx), 1);
    assert_eq!(shown(&b, chart_b), "ETHUSDT");
    assert_eq!(
        drawing_ids(&b, chart_b),
        vec![2],
        "B keeps its own drawings"
    );

    // Nothing echoes, however long the app ticks.
    for _ in 0..50 {
        assert_eq!(tick(&mut hub, &table, &mut [&mut a, &mut b], &ctx), 0);
    }
    assert!(!a.has_link_signals() && !b.has_link_signals());

    // And the other way round.
    b.switch_chart(chart_b, &row("SOLUSDT"), &ctx);
    b.announce_market(&row("SOLUSDT"));
    assert_eq!(tick(&mut hub, &table, &mut [&mut a, &mut b], &ctx), 1);
    assert_eq!(shown(&a, chart_a), "SOLUSDT");
    assert_eq!(tick(&mut hub, &table, &mut [&mut a, &mut b], &ctx), 0);
}

#[test]
fn symbol_and_interval_are_independent_switches_across_tabs() {
    let ctx = egui::Context::default();
    let mut hub = crate::model::tab::link::LinkHub::default();
    let mut table = crate::model::tab::link::LinkTable::default();
    let mut a = empty_workspace(1);
    let mut b = empty_workspace(2);
    let mut c = empty_workspace(3);
    let chart_a = chart_with_drawings(&mut a, "BTCUSDT", &[]);
    let chart_b = chart_with_drawings(&mut b, "BTCUSDT", &[]);
    chart_with_drawings(&mut c, "BTCUSDT", &[]);
    for tab in [&mut a, &mut b] {
        tab.set_link(Some(LinkColor::Blue));
    }
    c.set_link(Some(LinkColor::Red)); // another group
    tick(&mut hub, &table, &mut [&mut a, &mut b, &mut c], &ctx);

    // Symbol on, Interval off (the default): the symbol moves, the interval stays.
    a.reload_for_timeframe(Timeframe::Hour, &ctx);
    a.announce_timeframe();
    a.switch_chart(chart_a, &row("ETHUSDT"), &ctx);
    a.announce_market(&row("ETHUSDT"));
    tick(&mut hub, &table, &mut [&mut a, &mut b, &mut c], &ctx);
    assert_eq!(shown(&b, chart_b), "ETHUSDT");
    assert_eq!(b.timeframe, Timeframe::Day, "Interval is off");
    assert_eq!(c.timeframe, Timeframe::Day);
    assert_eq!(c.charts[0].symbol, "BTCUSDT", "Red never heard");

    // Symbol off, Interval on.
    table.set(
        LinkColor::Blue,
        crate::model::tab::link::LinkToggles {
            symbol: false,
            interval: true,
            time: false,
            date_range: false,
        },
    );
    a.switch_chart(chart_a, &row("SOLUSDT"), &ctx);
    a.announce_market(&row("SOLUSDT"));
    a.reload_for_timeframe(Timeframe::Week, &ctx);
    a.announce_timeframe();
    tick(&mut hub, &table, &mut [&mut a, &mut b, &mut c], &ctx);
    assert_eq!(b.timeframe, Timeframe::Week);
    assert_eq!(shown(&b, chart_b), "ETHUSDT", "Symbol is off");
}

#[test]
fn unlinking_stops_a_tab_at_once_and_leaves_its_chart_alone() {
    let ctx = egui::Context::default();
    let mut hub = crate::model::tab::link::LinkHub::default();
    let table = crate::model::tab::link::LinkTable::default();
    let mut a = empty_workspace(1);
    let mut b = empty_workspace(2);
    let chart_a = chart_with_drawings(&mut a, "BTCUSDT", &[1]);
    let chart_b = chart_with_drawings(&mut b, "BTCUSDT", &[]);
    a.set_link(Some(LinkColor::Pink));
    b.set_link(Some(LinkColor::Pink));
    tick(&mut hub, &table, &mut [&mut a, &mut b], &ctx);

    // A speaks, then is unlinked before the app collects it: nothing leaves.
    a.switch_chart(chart_a, &row("ETHUSDT"), &ctx);
    a.announce_market(&row("ETHUSDT"));
    assert!(a.set_link(None));
    assert_eq!(tick(&mut hub, &table, &mut [&mut a, &mut b], &ctx), 0);
    assert_eq!(shown(&b, chart_b), "BTCUSDT");
    assert_eq!(
        shown(&a, chart_a),
        "ETHUSDT",
        "unlinking never resets the chart"
    );
    assert_eq!(drawing_ids(&a, chart_a), vec![1]);

    // And it no longer hears either.
    b.switch_chart(chart_b, &row("SOLUSDT"), &ctx);
    b.announce_market(&row("SOLUSDT"));
    assert_eq!(tick(&mut hub, &table, &mut [&mut a, &mut b], &ctx), 0);
    assert_eq!(shown(&a, chart_a), "ETHUSDT");
}

#[test]
fn a_tab_joining_a_group_adopts_its_symbol_without_touching_the_others() {
    let ctx = egui::Context::default();
    let mut hub = crate::model::tab::link::LinkHub::default();
    let table = crate::model::tab::link::LinkTable::default();
    let mut a = empty_workspace(1);
    let mut b = empty_workspace(2);
    let mut c = empty_workspace(3);
    let chart_a = chart_with_drawings(&mut a, "ETHUSDT", &[]);
    let chart_b = chart_with_drawings(&mut b, "ETHUSDT", &[]);
    let chart_c = chart_with_drawings(&mut c, "SOLUSDT", &[]);
    a.set_link(Some(LinkColor::Cyan));
    b.set_link(Some(LinkColor::Cyan));
    tick(&mut hub, &table, &mut [&mut a, &mut b, &mut c], &ctx);

    // The user tags C Cyan in the popover: only the tag changes at that moment.
    c.set_link(Some(LinkColor::Cyan));
    assert_eq!(shown(&c, chart_c), "SOLUSDT");
    tick(&mut hub, &table, &mut [&mut a, &mut b, &mut c], &ctx);
    assert_eq!(shown(&c, chart_c), "ETHUSDT", "adopted the group's symbol");
    assert_eq!(shown(&a, chart_a), "ETHUSDT");
    assert_eq!(shown(&b, chart_b), "ETHUSDT");
}

#[test]
fn a_view_delivered_while_the_tab_is_hidden_waits_for_its_next_draw_even_if_the_switches_changed() {
    use crate::model::chart::sync::{HorizontalChange, TimeRange};
    use crate::model::tab::link::LinkToggles;
    let ctx = egui::Context::default();
    let mut workspace = empty_workspace(1);
    chart_with_drawings(&mut workspace, "BTCUSDT", &[]);
    let host = crate::host::tab::link::TabLinkHost::default();
    workspace.set_link_host(&host);
    workspace.set_link(Some(LinkColor::Orange));
    let change = HorizontalChange {
        range: TimeRange {
            left: 0.0,
            right: 1_000.0,
        },
        latest: false,
        moved: true,
        zoom: None,
    };
    // Symbol only: the tab's last draw happened under "nothing to follow".
    assert!(!workspace.link_view_settings().is_linked());
    // The user turns Time on while this tab is hidden, then pans another tab.
    host.set_toggles(
        None,
        LinkColor::Orange,
        LinkToggles {
            time: true,
            ..LinkToggles::default()
        },
    )
    .unwrap();
    assert!(workspace.apply_link(1, &LinkSignal::View(change), &ctx));
    assert!(
        workspace.link_state.hub.latest_event().is_some(),
        "the event is kept for the tab's next draw"
    );
}

#[test]
fn choosing_an_already_open_market_in_the_watchlist_is_announced() {
    let ctx = egui::Context::default();
    let mut workspace = empty_workspace(1);
    let first = chart_with_drawings(&mut workspace, "BTCUSDT", &[]);
    let second = chart_with_drawings(&mut workspace, "ETHUSDT", &[]);
    workspace.focused = Some(first);
    workspace.set_link(Some(LinkColor::Orange));
    assert!(workspace.take_link_signals().is_empty());

    // The user clicks ETH in the watchlist: it is already on the tab, so its chart takes the focus.
    workspace.open_watchlist_instrument(&row("ETHUSDT"), &ctx);
    assert_eq!(workspace.focused, Some(second), "focus moved");
    let signals = workspace.take_link_signals();
    assert_eq!(signals.len(), 1, "{signals:?}");
    assert!(matches!(&signals[0], LinkSignal::Market(m) if m.instrument.symbol == "ETHUSDT"));
}

#[test]
fn a_symbol_the_user_chose_over_a_pending_link_is_not_reverted_when_it_fails() {
    let ctx = egui::Context::default();
    let mut workspace = empty_workspace(1);
    let chart = chart_with_drawings(&mut workspace, "ETHUSDT", &[]);
    workspace.set_link(Some(LinkColor::Orange));

    // The group asks for Coinbase BTCUSDT; before it settles the user picks Binance BTCUSDT
    // (same ticker, another provider).
    let coinbase = LinkSignal::Market(Box::new(WatchlistInstrument::from_market(
        DataSource::Coinbase,
        "BTCUSDT",
        "BTCUSDT",
    )));
    assert!(workspace.apply_link(1, &coinbase, &ctx));
    workspace.switch_chart(chart, &row("BTCUSDT"), &ctx);
    workspace.announce_market(&row("BTCUSDT"));
    let index = workspace.chart_index(chart).unwrap();
    assert_eq!(workspace.charts[index].source, DataSource::Binance);

    // The user's choice fails to load.
    workspace.charts[index].inbox = None;
    workspace.charts[index].loading = false;
    workspace.charts[index].error = Some(ProviderError::other("Unknown symbol"));
    workspace.tick_link_probation(&ctx);

    assert_eq!(workspace.charts[index].source, DataSource::Binance);
    assert_eq!(
        workspace.charts[index].symbol, "BTCUSDT",
        "not put back to ETH"
    );
    assert!(workspace.link_state.probation.is_none());
}

fn pan_to(right: f64) -> crate::model::chart::sync::HorizontalChange {
    crate::model::chart::sync::HorizontalChange {
        range: crate::model::chart::sync::TimeRange {
            left: right - 1_000.0,
            right,
        },
        latest: false,
        moved: true,
        zoom: None,
    }
}

#[test]
fn a_symbol_or_interval_change_drops_a_view_the_old_series_was_waiting_for() {
    let ctx = egui::Context::default();
    for (name, change) in [
        ("symbol", market("ETHUSDT")),
        ("interval", LinkSignal::Timeframe(Timeframe::Week)),
    ] {
        let mut workspace = empty_workspace(1);
        chart_with_drawings(&mut workspace, "BTCUSDT", &[]);
        workspace.set_link(Some(LinkColor::Orange));
        // A pan arrives while the tab is hidden or loading, then the group changes the series.
        assert!(workspace.apply_link(1, &LinkSignal::View(pan_to(5_000.0)), &ctx));
        assert!(workspace.link_state.hub.latest_event().is_some());
        assert!(workspace.apply_link(2, &change, &ctx), "{name}");
        assert!(
            workspace.link_state.hub.latest_event().is_none(),
            "{name}: the old series' view must not be replayed on the new one"
        );
    }
}

#[test]
fn a_view_that_arrives_after_the_series_changed_is_kept() {
    let ctx = egui::Context::default();
    let mut workspace = empty_workspace(1);
    chart_with_drawings(&mut workspace, "BTCUSDT", &[]);
    workspace.set_link(Some(LinkColor::Orange));
    assert!(workspace.apply_link(1, &market("ETHUSDT"), &ctx));
    assert!(workspace.apply_link(2, &LinkSignal::View(pan_to(5_000.0)), &ctx));
    assert!(
        workspace.link_state.hub.latest_event().is_some(),
        "the leader panned after changing symbol: that pan is for the new series"
    );
}

#[test]
fn the_users_own_symbol_or_interval_change_also_drops_a_pending_view() {
    let ctx = egui::Context::default();
    let mut workspace = empty_workspace(1);
    let chart = chart_with_drawings(&mut workspace, "BTCUSDT", &[]);
    workspace.set_link(Some(LinkColor::Orange));
    workspace.apply_link(1, &LinkSignal::View(pan_to(5_000.0)), &ctx);
    workspace.switch_chart(chart, &row("ETHUSDT"), &ctx);
    assert!(workspace.link_state.hub.latest_event().is_none());
    workspace.apply_link(2, &LinkSignal::View(pan_to(6_000.0)), &ctx);
    workspace.reload_for_timeframe(Timeframe::Hour, &ctx);
    assert!(workspace.link_state.hub.latest_event().is_none());
}

#[test]
fn a_switch_to_the_market_already_shown_keeps_a_pending_view() {
    let ctx = egui::Context::default();
    let mut workspace = empty_workspace(1);
    let chart = chart_with_drawings(&mut workspace, "BTCUSDT", &[]);
    workspace.set_link(Some(LinkColor::Orange));
    workspace.apply_link(1, &LinkSignal::View(pan_to(5_000.0)), &ctx);
    workspace.switch_chart(chart, &row("BTCUSDT"), &ctx); // nothing changes on the chart
    assert!(workspace.link_state.hub.latest_event().is_some());
}

#[test]
fn a_zoom_waits_for_a_chart_that_has_no_bars_yet() {
    use crate::model::chart::sync::{ChartSyncSettings, HorizontalChange, TimeAxis, TimeRange};
    use crate::model::chart::viewport::HorizontalViewport;
    let zoom_only = ChartSyncSettings {
        horizontal_position: false,
        horizontal_zoom: true,
    };
    let bars: Vec<_> = (0..400)
        .map(|index| super::super::test_support::candle(index * 60_000, 1.0))
        .collect();
    let zoomed = HorizontalChange {
        range: TimeRange {
            left: 0.0,
            right: 1.0,
        },
        latest: true,
        moved: false,
        zoom: Some(crate::model::chart::sync::Zoom { from: 120, to: 95 }),
    };
    let run = |waiting_frames: usize, waiting: bool| {
        let mut state = super::LinkState::default();
        state.hub.publish(u64::MAX, zoomed);
        // Frames drawn while the chart has no candles: it is not asked to follow.
        for _ in 0..waiting_frames {
            state.end_frame(waiting);
        }
        let mut view = HorizontalViewport {
            visible: 120,
            right_offset: 0.0,
        };
        state
            .hub
            .deliver(1, zoom_only, &TimeAxis::new(&bars), &mut view);
        view.visible
    };
    assert_eq!(run(5, true), 95, "the zoom is still there when bars arrive");
    assert_eq!(run(0, false), 95);
    // A burst that was followed does end with the first frame that does not zoom.
    let mut state = super::LinkState::default();
    state.hub.publish(u64::MAX, zoomed);
    state.end_frame(false);
    state.end_frame(false);
    let mut view = HorizontalViewport {
        visible: 120,
        right_offset: 0.0,
    };
    state
        .hub
        .deliver(1, zoom_only, &TimeAxis::new(&bars), &mut view);
    assert_eq!(view.visible, 120, "a delivered-or-expired burst is over");
}
