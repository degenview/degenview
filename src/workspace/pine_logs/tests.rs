use super::*;
use crate::model::pine_logs_view::{Levels, SearchOptions};
use crate::pine::logs::LogLevel;
use crate::store::watchlist::WatchlistStore;
use crate::ui::ChartCard;
use eframe::egui::Context;

fn workspace_on(store: &SharedStore) -> ChartWorkspace {
    let book = WatchlistQuoteBook::default();
    ChartWorkspace::empty(
        1,
        Timeframe::Day,
        Arc::new(Mutex::new(WatchlistStore::new(store.clone()))),
        book.clone(),
        WatchlistQuoteCoordinator::new(book, Arc::new(|| {})),
        store.clone(),
    )
}

fn store() -> SharedStore {
    Arc::new(Mutex::new(
        crate::store::Store::open_in_memory().expect("in-memory store"),
    ))
}

/// A workspace with one chart that has two scripts of the same name and one of another.
fn rig() -> (ChartWorkspace, u64, [u64; 3]) {
    let mut workspace = workspace_on(&store());
    let ctx = Context::default();
    let chart = workspace
        .add_chart_for(&InstrumentId::new(DataSource::Binance, "BTCUSDT"), &ctx)
        .chart_id;
    let card = &mut workspace
        .charts
        .iter_mut()
        .find(|candidate| candidate.id == chart)
        .expect("chart")
        .chart;
    let ids = [
        card.add_pine_instance(None, "Probe", "//@version=6\nindicator(\"A\")\n"),
        card.add_pine_instance(None, "Probe", "//@version=6\nindicator(\"B\")\n"),
        card.add_pine_instance(None, "Other", "//@version=6\nindicator(\"C\")\n"),
    ];
    (workspace, chart, ids)
}

fn card(workspace: &mut ChartWorkspace, chart: u64) -> &mut ChartCard {
    &mut workspace
        .charts
        .iter_mut()
        .find(|candidate| candidate.id == chart)
        .expect("chart")
        .chart
}

#[test]
fn two_copies_of_a_script_are_told_apart() {
    let (mut workspace, chart, _) = rig();
    let labels: Vec<String> = script_entries(card(&mut workspace, chart).pine_instances())
        .into_iter()
        .map(|entry| entry.label)
        .collect();
    assert_eq!(labels, ["Probe", "Probe #2", "Other"]);
}

#[test]
fn opening_the_pane_points_it_at_the_chart_and_script() {
    let (mut workspace, chart, ids) = rig();
    assert_eq!(workspace.context.logs.chart, None);
    workspace.open_pine_logs(chart, Some(ids[1]));
    assert_eq!(workspace.context.logs.chart, Some(chart));
    assert_eq!(workspace.context.logs.view.selected, Some(ids[1]));
    // Opening again from another script's menu points the one pane there.
    workspace.open_pine_logs(chart, Some(ids[2]));
    assert_eq!(workspace.context.logs.chart, Some(chart));
    assert_eq!(workspace.context.logs.view.selected, Some(ids[2]));
}

#[test]
fn opening_without_a_script_keeps_the_one_already_chosen() {
    let (mut workspace, chart, ids) = rig();
    workspace.open_pine_logs(chart, Some(ids[2]));
    workspace.open_pine_logs(chart, None);
    assert_eq!(workspace.context.logs.view.selected, Some(ids[2]));
}

#[test]
fn the_pane_closes_with_its_chart() {
    let (mut workspace, chart, _) = rig();
    workspace.open_pine_logs(chart, None);
    workspace.charts.retain(|candidate| candidate.id != chart);
    workspace.sync_context_panels();
    assert_eq!(workspace.context.logs.chart, None);
}

#[test]
fn the_close_button_closes_the_pane() {
    let (mut workspace, chart, _) = rig();
    workspace.open_pine_logs(chart, None);
    workspace.apply_logs_action(chart, LogsAction::Close);
    assert_eq!(workspace.context.logs.chart, None);
}

#[test]
fn choosing_a_script_in_the_selector_switches_the_history() {
    let (mut workspace, chart, ids) = rig();
    workspace.open_pine_logs(chart, Some(ids[0]));
    workspace.apply_logs_action(chart, LogsAction::Select(ids[2]));
    assert_eq!(workspace.context.logs.view.selected, Some(ids[2]));
}

#[test]
fn disabling_logging_applies_to_that_script_only() {
    let (mut workspace, chart, ids) = rig();
    workspace.apply_logs_action(chart, LogsAction::SetLogging(ids[0], false));
    assert!(!card(&mut workspace, chart).pine_logging(ids[0]));
    assert!(card(&mut workspace, chart).pine_logging(ids[1]));
    workspace.apply_logs_action(chart, LogsAction::SetLogging(ids[0], true));
    assert!(card(&mut workspace, chart).pine_logging(ids[0]));
}

#[test]
fn a_removed_script_leaves_the_selector_on_one_that_exists() {
    let (mut workspace, chart, ids) = rig();
    workspace.open_pine_logs(chart, Some(ids[1]));
    card(&mut workspace, chart).remove_pine_instance(ids[1]);
    let scripts = script_entries(card(&mut workspace, chart).pine_instances());
    workspace.context.logs.view.select_valid(&scripts);
    assert_eq!(workspace.context.logs.view.selected, Some(ids[0]));
}

#[test]
fn the_panes_options_are_remembered_on_this_machine_but_not_its_messages() {
    let shared = store();
    {
        let mut workspace = workspace_on(&shared);
        let ctx = Context::default();
        let chart = workspace
            .add_chart_for(&InstrumentId::new(DataSource::Binance, "BTCUSDT"), &ctx)
            .chart_id;
        workspace.open_pine_logs(chart, None);
        let view = &mut workspace.context.logs.view;
        view.filter.levels.toggle(LogLevel::Info);
        view.filter.options.regex = true;
        view.filter.query = "not remembered".to_owned();
        workspace.save_logs_prefs();
    }
    let mut workspace = workspace_on(&shared);
    workspace.open_pine_logs(1, None);
    let view = &workspace.context.logs.view;
    assert_eq!(
        view.filter.levels,
        Levels {
            info: false,
            ..Levels::default()
        }
    );
    assert_eq!(
        view.filter.options,
        SearchOptions {
            regex: true,
            ..SearchOptions::default()
        }
    );
    assert!(view.filter.query.is_empty());
}

#[test]
fn the_options_are_never_synced() {
    assert_eq!(
        crate::sync::policy::setting_policy(PREFS_KEY),
        crate::sync::policy::SettingPolicy::Local
    );
}
