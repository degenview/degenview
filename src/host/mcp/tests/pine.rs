use super::alert::NOW;
use super::*;

use crate::model::alert::pine::{PineAlertState, PineAlertSubscription};
use crate::model::pine::instance::{PineInstance, StoredInput};
use crate::pine::outputs::style_overrides::{StyleKind, StyleRow};
use crate::pine::strategy::{OpenTrade, StrategyReport, StrategySettings, Trade};
use crate::pine::syntax::token::{Diagnostic, DiagnosticCategory, DiagnosticSeverity, SourceRange};
use crate::workspace::PineRunView;

pub(super) fn empty_workspace(id: u64) -> ChartWorkspace {
    use crate::service::watchlist_quotes::WatchlistQuoteCoordinator;
    use crate::store::watchlist::WatchlistStore;
    let store: SharedStore = Arc::new(Mutex::new(Store::open_in_memory().unwrap()));
    let book = WatchlistQuoteBook::default();
    ChartWorkspace::empty(
        id,
        crate::model::Timeframe::Day,
        Arc::new(Mutex::new(WatchlistStore::new(store.clone()))),
        book.clone(),
        WatchlistQuoteCoordinator::new(book, Arc::new(|| {})),
        store,
    )
}

const STRATEGY_SOURCE: &str = "//@version=6\nstrategy(\"S\")\n";

fn trade(id: usize, profit: f64) -> Trade {
    Trade {
        id,
        entry_id: format!("e{id}"),
        exit_id: format!("x{id}"),
        is_long: id.is_multiple_of(2),
        quantity: 1.0,
        entry_bar: id * 2,
        entry_time: NOW + id as i64 * 1000,
        entry_price: 100.0,
        exit_bar: id * 2 + 1,
        exit_time: NOW + id as i64 * 1000 + 500,
        exit_price: 100.0 + profit,
        commission: 0.0,
        profit,
        ..Trade::default()
    }
}

fn report(trades: Vec<Trade>, equity: Vec<f64>) -> StrategyReport {
    StrategyReport {
        settings: StrategySettings::default(),
        trades,
        open_trades: vec![OpenTrade {
            entry_id: "open".into(),
            is_long: true,
            quantity: 2.0,
            entry_bar: 9,
            entry_time: NOW,
            entry_price: 101.0,
            entry_commission: 0.0,
            ..OpenTrade::default()
        }],
        equity,
        open_profit: 3.5,
        chart: crate::model::ChartKind::Standard,
    }
}

fn view(run: Option<PineRunView>) -> PineInstanceView {
    let mut instance = PineInstance::new(4, Some("script-1".into()), "Strat", STRATEGY_SOURCE);
    instance.inputs.insert("len".into(), StoredInput::Int(30));
    instance
        .style_overrides
        .insert("plot.0.color".into(), "FF0000FF".into());
    instance
        .style_overrides
        .insert("plot.0.visible".into(), "false".into());
    instance
        .style_overrides
        .insert("plot.0.width".into(), "3".into());
    PineInstanceView {
        chart_id: 2,
        instrument: InstrumentId::new(DataSource::Binance, "BTCUSDT"),
        timeframe: crate::model::Timeframe::Day,
        instance,
        run,
    }
}

fn run(strategy: Option<StrategyReport>, error: Option<Diagnostic>) -> PineRunView {
    PineRunView {
        error,
        bar_count: 500,
        style_rows: vec![StyleRow {
            kind: StyleKind::Plot,
            output_id: 0,
            title: "SMA".into(),
            default_color: Some(0x2962_FFFF),
            default_width: Some(2),
        }],
        is_strategy: strategy.is_some(),
        strategy,
    }
}

#[test]
fn applying_changing_and_removing_a_script_report_what_happened() {
    const SOURCE: &str =
        "//@version=6\nindicator(\"I\")\nlen = input.int(20, \"L\", minval=1)\nplot(close)\n";
    let ctx = eframe::egui::Context::default();
    let mut workspace = empty_workspace(3);
    let chart_id = workspace
        .add_chart_for(&InstrumentId::new(DataSource::Binance, "BTCUSDT"), &ctx)
        .chart_id;
    let script = ScriptToApply {
        script_id: "s1".into(),
        name: "I".into(),
        source: SOURCE.into(),
        inputs: Default::default(),
    };
    let args = |json: serde_json::Value| -> InputArgs { serde_json::from_value(json).unwrap() };

    assert!(matches!(
        add_pine_instance(&mut workspace, 99, &script, true),
        Err(McpError::NotFound(_))
    ));
    let added = add_pine_instance(&mut workspace, chart_id, &script, true).unwrap();
    assert_eq!(added.status, "added");
    assert_eq!(added.chart.instances.len(), 1);
    assert_eq!(added.chart.instances[0].instance_id, added.instance_id);
    assert_eq!(added.chart.instances[0].run_status, "not_run");
    let id: u64 = added.instance_id.parse().unwrap();

    let set = args(serde_json::json!({"len": 5}));
    let changed =
        update_pine_instance(&mut workspace, chart_id, id, Some(&set), None, &[]).unwrap();
    assert_eq!(changed.status, "updated");
    let again = update_pine_instance(&mut workspace, chart_id, id, Some(&set), None, &[]).unwrap();
    assert_eq!(again.status, "unchanged");
    let hidden =
        update_pine_instance(&mut workspace, chart_id, id, None, Some(false), &[]).unwrap();
    assert!(!hidden.chart.instances[0].visible);
    assert!(matches!(
        update_pine_instance(
            &mut workspace,
            chart_id,
            id,
            Some(&args(serde_json::json!({"nope": 1}))),
            None,
            &[]
        ),
        Err(McpError::InvalidArgument(_))
    ));
    assert!(matches!(
        update_pine_instance(&mut workspace, chart_id, id + 1, None, Some(true), &[]),
        Err(McpError::NotFound(_))
    ));

    let removed = remove_pine_instance(&mut workspace, chart_id, id, &[]).unwrap();
    assert_eq!(removed.status, "removed");
    assert!(removed.chart.instances.is_empty());
    assert!(matches!(
        remove_pine_instance(&mut workspace, chart_id, id, &[]),
        Err(McpError::NotFound(_))
    ));
}

#[test]
fn a_report_carries_the_charts_own_numbers_and_the_context_that_made_them() {
    let trades = vec![
        trade(0, 10.0),
        trade(1, -4.0),
        trade(2, 6.0),
        trade(3, -2.0),
    ];
    let strategy = report(trades, vec![1000.0, 1010.0, 1006.0, 1012.0, 1010.0]);
    let expected_net = strategy.net_profit();
    let (drawdown, fraction) = strategy.max_drawdown();
    let result = report_of(1, &view(Some(run(Some(strategy), None))), 0, 50, true);

    assert!(result.available && result.reason.is_none());
    let metrics = result.metrics.unwrap();
    assert_eq!(metrics.net_profit, Some(expected_net));
    assert_eq!(metrics.net_profit, Some(10.0));
    assert_eq!(
        (
            metrics.closed_trades,
            metrics.winning_trades,
            metrics.losing_trades
        ),
        (4, 2, 2)
    );
    assert_eq!(metrics.win_rate, Some(0.5));
    assert_eq!(metrics.max_drawdown, Some(drawdown));
    assert_eq!(metrics.max_drawdown_fraction, Some(fraction));
    assert_eq!((metrics.open_profit, metrics.open_trades), (Some(3.5), 1));

    let context = result.context.unwrap();
    assert_eq!(context.kind, "chart_instance");
    assert_eq!(context.instrument, "BINANCE:BTCUSDT");
    assert_eq!(context.timeframe, "1D");
    assert_eq!(context.bar_count, 500);
    assert_eq!(context.script_id.as_deref(), Some("script-1"));
    assert_eq!(
        context.source_hash,
        crate::pine_studio::library::store::hash_source(STRATEGY_SOURCE)
    );
    assert_eq!(
        context.inputs["len"],
        serde_json::json!({"kind": "int", "value": 30})
    );
    assert_eq!(
        result.equity_curve,
        vec![1000.0, 1010.0, 1006.0, 1012.0, 1010.0]
    );
    assert_eq!(
        result.settings.unwrap().initial_capital,
        StrategySettings::default().initial_capital
    );
    assert_eq!(result.open_trades[0].direction, "long");
    assert!(result.trades[0].entry_time.ends_with('Z'));
}

#[test]
fn trades_are_paged_and_equity_is_thinned_but_keeps_its_ends() {
    let trades: Vec<Trade> = (0..450).map(|id| trade(id, 1.0)).collect();
    let equity: Vec<f64> = (0..1000).map(f64::from).collect();
    let view = view(Some(run(Some(report(trades, equity)), None)));

    let first = report_of(1, &view, 0, 200, false);
    assert_eq!(
        (first.trades.len(), first.trades_total, first.trades_offset),
        (200, 450, 0)
    );
    assert_eq!(first.trades[0].id, 0);
    assert!(first.equity_curve.is_empty(), "equity is opt-in");
    let last = report_of(1, &view, 400, 200, false);
    assert_eq!((last.trades.len(), last.trades[0].id), (50, 400));
    let beyond = report_of(1, &view, 10_000, 10, false);
    assert!(beyond.trades.is_empty() && beyond.trades_total == 450);
    // A limit over the cap, or zero, is clamped, never honoured.
    assert_eq!(
        report_of(1, &view, 0, 10_000, false).trades.len(),
        MAX_TRADES_PAGE
    );
    assert_eq!(report_of(1, &view, 0, 0, false).trades.len(), 1);

    let curve = report_of(1, &view, 0, 1, true);
    assert_eq!(curve.equity_curve.len(), MAX_EQUITY_POINTS);
    assert_eq!(curve.equity_points_total, 1000);
    assert_eq!(curve.equity_curve.first(), Some(&0.0));
    assert_eq!(curve.equity_curve.last(), Some(&999.0));
    assert!(curve.equity_curve.windows(2).all(|w| w[0] < w[1]));
}

#[test]
fn no_report_is_a_typed_unavailable_result_never_a_stale_one() {
    let never_run = report_of(1, &view(None), 0, 50, true);
    assert!(!never_run.available);
    assert!(never_run.reason.as_deref().unwrap().contains("not run"));
    assert!(
        never_run.context.is_none() && never_run.metrics.is_none() && never_run.trades.is_empty()
    );

    let indicator = report_of(1, &view(Some(run(None, None))), 0, 50, false);
    assert!(!indicator.available);
    assert!(
        indicator
            .reason
            .as_deref()
            .unwrap()
            .contains("not a strategy")
    );

    let failed = Diagnostic::error(
        "PINE8004",
        DiagnosticCategory::Runtime,
        "boom",
        SourceRange::ZERO,
    );
    let broken = report_of(1, &view(Some(run(None, Some(failed)))), 0, 50, false);
    assert!(!broken.available);
    assert!(broken.reason.as_deref().unwrap().contains("failed"));
}

#[test]
fn non_finite_numbers_never_reach_the_json() {
    let mut strategy = report(vec![trade(0, 5.0)], vec![1.0]);
    strategy.open_profit = f64::NAN;
    let result = report_of(1, &view(Some(run(Some(strategy), None))), 0, 50, false);
    let metrics = result.metrics.unwrap();
    assert_eq!(metrics.open_profit, None);
    assert_eq!(
        metrics.profit_factor, None,
        "no losing trade: no finite factor"
    );
    assert!(serde_json::to_string(&metrics).unwrap().contains("null"));
}

#[test]
fn style_rows_show_defaults_and_this_instances_overrides() {
    let styles = styles_of(1, &view(Some(run(None, None))));
    assert!(styles.available && styles.reason.is_none());
    assert_eq!(styles.rows.len(), 1);
    let row = &styles.rows[0];
    assert_eq!(
        (row.id.as_str(), row.kind.as_str(), row.title.as_str()),
        ("plot.0", "plot", "SMA")
    );
    assert_eq!(row.default_color.as_deref(), Some("#2962FFFF"));
    assert_eq!(row.default_width, Some(2));
    assert_eq!(row.overrides.visible, Some(false));
    assert_eq!(row.overrides.color.as_deref(), Some("FF0000FF"));
    assert_eq!(row.overrides.width, Some(3));

    let unrun = styles_of(1, &view(None));
    assert!(!unrun.available && unrun.rows.is_empty());
    assert!(unrun.reason.is_some());
}

#[test]
fn applied_instances_report_run_status_and_the_hash_of_their_own_source() {
    let info = applied_info(&view(None));
    assert_eq!(
        (info.instance_id.as_str(), info.run_status.as_str()),
        ("4", "not_run")
    );
    assert_eq!(info.is_strategy, None);
    assert_eq!(info.script_id.as_deref(), Some("script-1"));
    assert_eq!(
        info.source_hash,
        crate::pine_studio::library::store::hash_source(STRATEGY_SOURCE)
    );

    assert_eq!(applied_info(&view(Some(run(None, None)))).run_status, "ok");
    let failed = Diagnostic::error(
        "PINE8004",
        DiagnosticCategory::Runtime,
        "boom",
        SourceRange::ZERO,
    );
    let broken = applied_info(&view(Some(run(None, Some(failed)))));
    assert_eq!(broken.run_status, "error");
    assert_eq!(broken.error.unwrap().code, "PINE8004");
    assert!(matches!(
        applied_info(&view(Some(run(Some(report(vec![], vec![])), None)))).is_strategy,
        Some(true)
    ));

    let detail = detail_of(1, &view(None));
    assert_eq!(
        detail.inputs["len"],
        serde_json::json!({"kind": "int", "value": 30})
    );
    assert_eq!(detail.style_overrides["plot.0.visible"], "false");
    assert_eq!(detail.bar_count, None);
    let _ = DiagnosticSeverity::Error;
}

#[test]
fn a_workspace_lists_its_applied_scripts_and_says_when_they_have_not_run() {
    let ctx = eframe::egui::Context::default();
    let mut workspace = empty_workspace(11);
    let chart = workspace
        .add_chart_for(&InstrumentId::new(DataSource::Binance, "BTCUSDT"), &ctx)
        .chart_id;
    workspace.add_pine_on_focused(Some("s1".into()), "One", STRATEGY_SOURCE);
    workspace.add_pine_on_focused(Some("s1".into()), "One", STRATEGY_SOURCE);

    let list = list_chart_pine(&workspace, chart).unwrap();
    assert_eq!(
        (
            list.workspace_id.as_str(),
            list.instrument.as_str(),
            list.timeframe.as_str()
        ),
        ("11", "BINANCE:BTCUSDT", "1D")
    );
    assert_eq!(
        list.instances.len(),
        2,
        "the same script can be applied twice"
    );
    assert_ne!(list.instances[0].instance_id, list.instances[1].instance_id);
    assert!(list.instances.iter().all(|i| i.run_status == "not_run"));

    let second: u64 = list.instances[1].instance_id.parse().unwrap();
    assert_eq!(
        chart_pine_detail(&workspace, chart, second)
            .unwrap()
            .instance
            .script_name,
        "One"
    );
    let report = pine_report(&workspace, chart, second, 0, 50, false).unwrap();
    assert!(!report.available);
    assert!(!pine_styles(&workspace, chart, second).unwrap().available);

    assert!(matches!(
        list_chart_pine(&workspace, 999),
        Err(McpError::NotFound(_))
    ));
    assert!(matches!(
        chart_pine_detail(&workspace, chart, 999),
        Err(McpError::NotFound(_))
    ));
    assert!(matches!(
        pine_report(&workspace, chart, 999, 0, 1, false),
        Err(McpError::NotFound(_))
    ));
}

#[test]
fn a_script_with_an_armed_alert_keeps_its_inputs_and_stays_on_the_chart() {
    const SOURCE: &str =
        "//@version=6\nindicator(\"I\")\nlen = input.int(20, \"L\", minval=1)\nplot(close)\n";
    let ctx = eframe::egui::Context::default();
    let mut workspace = empty_workspace(3);
    let chart_id = workspace
        .add_chart_for(&InstrumentId::new(DataSource::Binance, "BTCUSDT"), &ctx)
        .chart_id;
    let script = ScriptToApply {
        script_id: "s1".into(),
        name: "I".into(),
        source: SOURCE.into(),
        inputs: Default::default(),
    };
    let id: u64 = add_pine_instance(&mut workspace, chart_id, &script, true)
        .unwrap()
        .instance_id
        .parse()
        .unwrap();
    let alert = |state, instance_id, chart_id| PineAlertSubscription {
        id: 55,
        chart_id,
        instance_id: Some(instance_id),
        script_id: Some("s1".into()),
        script_name: "I".into(),
        source: SOURCE.into(),
        inputs: Default::default(),
        symbol_key: "binance:BTCUSDT".into(),
        timeframe: "1d".into(),
        source_hash: "abc".into(),
        note: String::new(),
        state,
        created_at_ms: NOW,
        webhook_endpoint_ids: Vec::new(),
    };
    let armed = [alert(PineAlertState::Active, id, chart_id)];
    let set: InputArgs = serde_json::from_value(serde_json::json!({"len": 5})).unwrap();

    let refused = update_pine_instance(&mut workspace, chart_id, id, Some(&set), None, &armed);
    assert!(
        matches!(&refused, Err(McpError::Conflict(message)) if message.contains("55")),
        "{refused:?}"
    );
    assert!(matches!(
        remove_pine_instance(&mut workspace, chart_id, id, &armed),
        Err(McpError::Conflict(_))
    ));
    // Nothing changed.
    let listed = list_chart_pine(&workspace, chart_id).unwrap();
    assert_eq!(listed.instances.len(), 1);
    assert_eq!(
        chart_pine_detail(&workspace, chart_id, id)
            .unwrap()
            .inputs
            .len(),
        0
    );

    // Hiding it, or setting an input to what it already is, leaves the alert alone.
    assert!(update_pine_instance(&mut workspace, chart_id, id, None, Some(false), &armed).is_ok());
    let same: InputArgs = serde_json::from_value(serde_json::json!({"len": 20})).unwrap();
    assert_eq!(
        update_pine_instance(&mut workspace, chart_id, id, Some(&same), None, &armed)
            .unwrap()
            .status,
        "unchanged"
    );
    // Another instance's alert, a paused or silenced one, or another chart's, does not block.
    for other in [
        alert(PineAlertState::Active, id + 1, chart_id),
        alert(PineAlertState::Paused, id, chart_id),
        alert(PineAlertState::ScriptChanged, id, chart_id),
        alert(PineAlertState::Active, id, chart_id + 1),
    ] {
        assert!(
            update_pine_instance(
                &mut workspace,
                chart_id,
                id,
                Some(&set),
                None,
                std::slice::from_ref(&other)
            )
            .is_ok(),
            "{other:?}"
        );
    }
    assert!(remove_pine_instance(&mut workspace, chart_id, id, &[]).is_ok());
}

#[test]
fn script_alert_subscriptions_are_listed_without_webhooks_or_invented_history() {
    let subscription = PineAlertSubscription {
        id: 1_791_000_000_000_000_001,
        chart_id: 2,
        instance_id: Some(4),
        script_id: Some("script-1".into()),
        script_name: "Strat".into(),
        source: STRATEGY_SOURCE.into(),
        inputs: Default::default(),
        symbol_key: "binance:BTCUSDT".into(),
        timeframe: "1d".into(),
        source_hash: "abc".into(),
        note: "watch".into(),
        state: PineAlertState::ScriptChanged,
        created_at_ms: NOW,
        webhook_endpoint_ids: vec![424_242],
    };
    let result = pine_alerts(&[subscription]);
    assert_eq!(result.total, 1);
    let info = &result.subscriptions[0];
    assert_eq!(info.subscription_id, "1791000000000000001");
    assert_eq!(
        (
            info.state.as_str(),
            info.instrument.as_str(),
            info.timeframe.as_str()
        ),
        ("script_changed", "binance:BTCUSDT", "1d")
    );
    assert!(info.sends_to_webhooks);
    assert!(!result.event_history_available && result.recent_events.is_empty());
    let json = serde_json::to_string(&result).unwrap();
    assert!(
        !json.contains("424242") && !json.contains(STRATEGY_SOURCE.trim()),
        "{json}"
    );
}
