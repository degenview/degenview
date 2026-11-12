use super::alert::MAX_ALERTS;
use crate::mcp::bridge::McpError;
use crate::mcp::pine::{InputArgs, ScriptToApply};
use crate::mcp::types::{
    AppliedPineDetail, AppliedPineInfo, AppliedPineList, PineAlertInfo, PineAlertsResult,
    PineInstanceChangeResult, PineOpenTradeInfo, PineReportContext, PineReportMetrics,
    PineReportResult, PineStrategySettingsInfo, PineStyleOverridesInfo, PineStyleRowInfo,
    PineStylesResult, PineTradeInfo,
};
use crate::model::alert::pine::PineAlertSubscription;
use crate::workspace::{ChartWorkspace, PineInstanceView};

/// Report trades returned per call at most.
pub const MAX_TRADES_PAGE: usize = 200;

/// Equity points returned at most; longer curves are thinned evenly.
pub(super) const MAX_EQUITY_POINTS: usize = 200;

pub(super) fn applied_info(view: &PineInstanceView) -> AppliedPineInfo {
    let instance = &view.instance;
    let (run_status, error, is_strategy) = match &view.run {
        None => ("not_run", None, None),
        Some(run) => (
            if run.error.is_some() { "error" } else { "ok" },
            run.error.as_ref().map(crate::mcp::pine::diagnostic_info),
            Some(run.is_strategy),
        ),
    };
    AppliedPineInfo {
        instance_id: instance.id.to_string(),
        script_id: instance.script_id.clone(),
        script_name: instance.name.clone(),
        visible: instance.visible,
        source_hash: crate::pine_studio::library::store::hash_source(&instance.source),
        run_status: run_status.to_owned(),
        error,
        is_strategy,
    }
}

fn stored_inputs(view: &PineInstanceView) -> std::collections::BTreeMap<String, serde_json::Value> {
    view.instance
        .inputs
        .iter()
        .filter_map(|(id, value)| Some((id.clone(), serde_json::to_value(value).ok()?)))
        .collect()
}

fn pine_views(
    workspace: &ChartWorkspace,
    chart_id: u64,
    instance: Option<u64>,
    with_report: bool,
) -> Result<Vec<PineInstanceView>, McpError> {
    let views = workspace
        .pine_instance_views(chart_id, instance, with_report)
        .ok_or_else(|| {
            McpError::NotFound(format!(
                "No chart {chart_id} in workspace {}",
                workspace.id()
            ))
        })?;
    if let Some(id) = instance
        && views.is_empty()
    {
        return Err(McpError::NotFound(format!(
            "No script instance {id} on chart {chart_id}"
        )));
    }
    Ok(views)
}

pub fn list_chart_pine(
    workspace: &ChartWorkspace,
    chart_id: u64,
) -> Result<AppliedPineList, McpError> {
    let views = pine_views(workspace, chart_id, None, false)?;
    let (instrument, timeframe) = views.first().map_or_else(
        || {
            workspace
                .chart_summaries()
                .into_iter()
                .find(|chart| chart.id == chart_id)
                .map(|chart| {
                    (
                        chart.instrument.key(),
                        workspace.timeframe().label().to_owned(),
                    )
                })
                .unwrap_or_default()
        },
        |view| (view.instrument.key(), view.timeframe.label().to_owned()),
    );
    Ok(AppliedPineList {
        workspace_id: workspace.id().to_string(),
        chart_id: chart_id.to_string(),
        instrument,
        timeframe,
        instances: views.iter().map(applied_info).collect(),
    })
}

/// `script_status` is filled in by the caller, who can read the Pine Studio off this thread.
pub fn chart_pine_detail(
    workspace: &ChartWorkspace,
    chart_id: u64,
    instance_id: u64,
) -> Result<AppliedPineDetail, McpError> {
    let view = pine_views(workspace, chart_id, Some(instance_id), false)?.remove(0);
    Ok(detail_of(workspace.id(), &view))
}

fn instance_change(
    workspace: &ChartWorkspace,
    chart_id: u64,
    instance_id: u64,
    status: &str,
) -> Result<PineInstanceChangeResult, McpError> {
    Ok(PineInstanceChangeResult {
        status: status.to_owned(),
        instance_id: instance_id.to_string(),
        chart: list_chart_pine(workspace, chart_id)?,
    })
}

/// Applies a checked script to a chart, after the scripts already there.
pub fn add_pine_instance(
    workspace: &mut ChartWorkspace,
    chart_id: u64,
    script: &ScriptToApply,
    visible: bool,
) -> Result<PineInstanceChangeResult, McpError> {
    let id = workspace
        .add_pine_instance_to(
            chart_id,
            script.script_id.clone(),
            &script.name,
            &script.source,
            &script.inputs,
            visible,
        )
        .ok_or_else(|| {
            McpError::NotFound(format!(
                "No chart {chart_id} in workspace {}",
                workspace.id()
            ))
        })?;
    instance_change(workspace, chart_id, id, "added")
}

/// Refuses a change that would retarget or silence a live script alert. The alert host copies an
/// instance's inputs into the alert it arms, and silences the alert when the instance goes, so
/// changing either from a client that holds only the chart permission would alter alerts that
/// have their own (deferred) permissions. The person can still do it in the app.
pub(super) fn guard_armed_alerts(
    subscriptions: &[PineAlertSubscription],
    chart_id: u64,
    instance_id: u64,
    what: &str,
) -> Result<(), McpError> {
    let armed: Vec<String> = subscriptions
        .iter()
        .filter(|alert| {
            alert.is_active()
                && alert.chart_id == chart_id
                && alert.instance_id == Some(instance_id)
        })
        .map(|alert| alert.id.to_string())
        .collect();
    if armed.is_empty() {
        return Ok(());
    }
    Err(McpError::Conflict(format!(
        "Script alert {} is armed on this instance, and {what} would change or silence it. Change it in DegenView, or pause the alert there first",
        armed.join(", ")
    )))
}

/// Changes an applied script's inputs and/or visibility. Inputs are checked against the schema
/// of the instance's own source, which it keeps from when it was applied. A change to the inputs
/// is refused while a script alert is armed on the instance; visibility never affects alerts.
pub fn update_pine_instance(
    workspace: &mut ChartWorkspace,
    chart_id: u64,
    instance_id: u64,
    args: Option<&InputArgs>,
    visible: Option<bool>,
    subscriptions: &[PineAlertSubscription],
) -> Result<PineInstanceChangeResult, McpError> {
    let view = pine_views(workspace, chart_id, Some(instance_id), false)?.remove(0);
    let inputs = args
        .map(|args| {
            crate::mcp::pine::instance_inputs(
                &view.instance.source,
                view.instance.input_values(),
                args,
            )
        })
        .transpose()?;
    if inputs
        .as_ref()
        .is_some_and(|inputs| *inputs != view.instance.input_values())
    {
        guard_armed_alerts(subscriptions, chart_id, instance_id, "changing its inputs")?;
    }
    let changed = workspace
        .change_pine_instance(chart_id, instance_id, inputs.as_ref(), visible)
        .ok_or_else(|| {
            McpError::NotFound(format!(
                "No script instance {instance_id} on chart {chart_id}"
            ))
        })?;
    let status = if changed { "updated" } else { "unchanged" };
    instance_change(workspace, chart_id, instance_id, status)
}

/// Takes a script off a chart, unless a script alert is armed on it.
pub fn remove_pine_instance(
    workspace: &mut ChartWorkspace,
    chart_id: u64,
    instance_id: u64,
    subscriptions: &[PineAlertSubscription],
) -> Result<PineInstanceChangeResult, McpError> {
    pine_views(workspace, chart_id, Some(instance_id), false)?;
    guard_armed_alerts(subscriptions, chart_id, instance_id, "removing it")?;
    workspace.remove_pine_instance_from(chart_id, instance_id);
    instance_change(workspace, chart_id, instance_id, "removed")
}

pub(super) fn detail_of(workspace_id: u64, view: &PineInstanceView) -> AppliedPineDetail {
    AppliedPineDetail {
        workspace_id: workspace_id.to_string(),
        chart_id: view.chart_id.to_string(),
        instrument: view.instrument.key(),
        timeframe: view.timeframe.label().to_owned(),
        instance: applied_info(view),
        inputs: stored_inputs(view),
        style_overrides: view
            .instance
            .style_overrides
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect(),
        bar_count: view.run.as_ref().map(|run| run.bar_count),
        script_status: String::new(),
    }
}

fn color_hex(rgba: u32) -> String {
    format!("#{rgba:08X}")
}

pub fn pine_styles(
    workspace: &ChartWorkspace,
    chart_id: u64,
    instance_id: u64,
) -> Result<PineStylesResult, McpError> {
    let view = pine_views(workspace, chart_id, Some(instance_id), false)?.remove(0);
    Ok(styles_of(workspace.id(), &view))
}

pub(super) fn styles_of(workspace_id: u64, view: &PineInstanceView) -> PineStylesResult {
    let base = PineStylesResult {
        workspace_id: workspace_id.to_string(),
        chart_id: view.chart_id.to_string(),
        instance_id: view.instance.id.to_string(),
        available: false,
        reason: None,
        rows: Vec::new(),
    };
    let Some(run) = &view.run else {
        return PineStylesResult {
            reason: Some(
                "The script has not run yet: its style options exist once its chart has drawn it"
                    .into(),
            ),
            ..base
        };
    };
    let overrides = &view.instance.style_overrides;
    let rows = run
        .style_rows
        .iter()
        .map(|row| {
            let key = |attribute: &str| format!("{}.{attribute}", row.id());
            PineStyleRowInfo {
                id: row.id(),
                kind: row.kind.raw_value().to_owned(),
                title: row.title.clone(),
                default_color: row.default_color.map(color_hex),
                default_width: row.default_width,
                overrides: PineStyleOverridesInfo {
                    visible: overrides.get(&key("visible")).map(|value| value != "false"),
                    color: overrides.get(&key("color")).cloned(),
                    width: overrides
                        .get(&key("width"))
                        .and_then(|value| value.parse().ok()),
                },
            }
        })
        .collect();
    PineStylesResult {
        available: true,
        rows,
        ..base
    }
}

fn finite(value: f64) -> Option<f64> {
    value.is_finite().then_some(value)
}

fn direction(is_long: bool) -> String {
    if is_long { "long" } else { "short" }.to_owned()
}

/// A strategy report exactly as the chart computed it (the same numbers the Report tab shows),
/// one page of trades at a time. `script_status` is filled in by the caller.
pub fn pine_report(
    workspace: &ChartWorkspace,
    chart_id: u64,
    instance_id: u64,
    offset: usize,
    limit: usize,
    include_equity: bool,
) -> Result<PineReportResult, McpError> {
    let view = pine_views(workspace, chart_id, Some(instance_id), true)?.remove(0);
    Ok(report_of(
        workspace.id(),
        &view,
        offset,
        limit,
        include_equity,
    ))
}

pub(super) fn report_of(
    workspace_id: u64,
    view: &PineInstanceView,
    offset: usize,
    limit: usize,
    include_equity: bool,
) -> PineReportResult {
    let (chart_id, instance_id) = (view.chart_id, view.instance.id);
    let unavailable = |reason: &str| PineReportResult {
        workspace_id: workspace_id.to_string(),
        chart_id: chart_id.to_string(),
        instance_id: instance_id.to_string(),
        available: false,
        reason: Some(reason.to_owned()),
        context: None,
        metrics: None,
        settings: None,
        trades: Vec::new(),
        trades_total: 0,
        trades_offset: offset,
        open_trades: Vec::new(),
        equity_curve: Vec::new(),
        equity_points_total: 0,
    };
    let Some(run) = &view.run else {
        return unavailable(
            "No report yet: the chart has not run this script (its window has not drawn it)",
        );
    };
    let Some(report) = &run.strategy else {
        return unavailable(if run.error.is_some() {
            "The script failed to run, so there is no report"
        } else {
            "This script is not a strategy, so it has no report"
        });
    };
    let limit = limit.clamp(1, MAX_TRADES_PAGE);
    let (drawdown, drawdown_fraction) = report.max_drawdown();
    let wins = report.win_count();
    let settings = &report.settings;
    let equity: Vec<f64> = if include_equity {
        let total = report.equity.len();
        if total <= MAX_EQUITY_POINTS {
            report.equity.clone()
        } else {
            (0..MAX_EQUITY_POINTS)
                .map(|index| report.equity[index * (total - 1) / (MAX_EQUITY_POINTS - 1)])
                .collect()
        }
    } else {
        Vec::new()
    };
    PineReportResult {
        available: true,
        reason: None,
        context: Some(PineReportContext {
            kind: "chart_instance".into(),
            script_id: view.instance.script_id.clone(),
            script_name: view.instance.name.clone(),
            source_hash: crate::pine_studio::library::store::hash_source(&view.instance.source),
            script_status: String::new(),
            instrument: view.instrument.key(),
            timeframe: view.timeframe.label().to_owned(),
            bar_count: run.bar_count,
            inputs: stored_inputs(view),
        }),
        metrics: Some(PineReportMetrics {
            net_profit: finite(report.net_profit()),
            gross_profit: finite(report.gross_profit()),
            gross_loss: finite(report.gross_loss()),
            closed_trades: report.trades.len(),
            winning_trades: wins,
            losing_trades: report
                .trades
                .iter()
                .filter(|trade| trade.profit < 0.0)
                .count(),
            win_rate: report.win_rate().and_then(finite),
            profit_factor: report.profit_factor().and_then(finite),
            average_trade: report.average_trade().and_then(finite),
            final_equity: finite(report.final_equity()),
            return_fraction: finite(report.return_fraction()),
            max_drawdown: finite(drawdown),
            max_drawdown_fraction: finite(drawdown_fraction),
            open_profit: finite(report.open_profit),
            open_trades: report.open_trades.len(),
        }),
        settings: Some(PineStrategySettingsInfo {
            initial_capital: settings.initial_capital,
            quantity_type: match settings.quantity_type {
                crate::pine::strategy::QuantityType::Fixed => "fixed",
                crate::pine::strategy::QuantityType::Cash => "cash",
                crate::pine::strategy::QuantityType::PercentOfEquity => "percent_of_equity",
            }
            .to_owned(),
            quantity_value: settings.quantity_value,
            commission_type: match settings.commission_type {
                crate::pine::strategy::CommissionType::Percent => "percent",
                crate::pine::strategy::CommissionType::CashPerOrder => "cash_per_order",
                crate::pine::strategy::CommissionType::CashPerContract => "cash_per_contract",
            }
            .to_owned(),
            commission_value: settings.commission_value,
            slippage_ticks: settings.slippage,
            pyramiding: settings.pyramiding,
            currency: settings.currency.clone(),
            process_orders_on_close: settings.process_orders_on_close,
            calc_on_every_tick: settings.calc_on_every_tick,
        }),
        trades: report
            .trades
            .iter()
            .skip(offset)
            .take(limit)
            .map(|trade| PineTradeInfo {
                id: trade.id,
                entry_id: trade.entry_id.clone(),
                exit_id: trade.exit_id.clone(),
                direction: direction(trade.is_long),
                quantity: trade.quantity,
                entry_bar: trade.entry_bar,
                entry_time: crate::format::iso8601(trade.entry_time),
                entry_price: trade.entry_price,
                exit_bar: trade.exit_bar,
                exit_time: crate::format::iso8601(trade.exit_time),
                exit_price: trade.exit_price,
                commission: trade.commission,
                profit: trade.profit,
                entry_comment: trade.entry_comment.clone(),
                exit_comment: trade.exit_comment.clone(),
                max_runup: trade.max_runup,
                max_drawdown: trade.max_drawdown,
            })
            .collect(),
        trades_total: report.trades.len(),
        trades_offset: offset,
        open_trades: report
            .open_trades
            .iter()
            .take(MAX_TRADES_PAGE)
            .map(|trade| PineOpenTradeInfo {
                entry_id: trade.entry_id.clone(),
                entry_comment: trade.entry_comment.clone(),
                direction: direction(trade.is_long),
                quantity: trade.quantity,
                entry_bar: trade.entry_bar,
                entry_time: crate::format::iso8601(trade.entry_time),
                entry_price: trade.entry_price,
            })
            .collect(),
        equity_points_total: report.equity.len(),
        equity_curve: equity,
        ..unavailable("")
    }
}

/// The scripts' alert subscriptions as stored. DegenView keeps no log of what they fired, so no
/// history is invented: reading never runs a script or sends anything.
pub fn pine_alerts(
    subscriptions: &[crate::model::alert::pine::PineAlertSubscription],
) -> PineAlertsResult {
    use crate::model::alert::pine::PineAlertState;
    PineAlertsResult {
        total: subscriptions.len(),
        subscriptions: subscriptions
            .iter()
            .take(MAX_ALERTS)
            .map(|subscription| PineAlertInfo {
                subscription_id: subscription.id.to_string(),
                script_id: subscription.script_id.clone(),
                script_name: subscription.script_name.clone(),
                instrument: subscription.symbol_key.clone(),
                timeframe: subscription.timeframe.clone(),
                state: match subscription.state {
                    PineAlertState::Active => "active",
                    PineAlertState::Paused => "paused",
                    PineAlertState::ScriptChanged => "script_changed",
                    PineAlertState::ScriptDeleted => "script_deleted",
                }
                .to_owned(),
                note: subscription.note.clone(),
                chart_id: subscription.chart_id.to_string(),
                instance_id: subscription.instance_id.map(|id| id.to_string()),
                source_hash: subscription.source_hash.clone(),
                sends_to_webhooks: !subscription.webhook_endpoint_ids.is_empty(),
                created_at: crate::format::iso8601(subscription.created_at_ms),
            })
            .collect(),
        event_history_available: false,
        recent_events: Vec::new(),
    }
}
