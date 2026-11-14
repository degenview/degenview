use crate::host::mcp::McpTabs;
use crate::host::mcp::confirmation;
use crate::mcp::bridge::McpError;
use crate::mcp::bridge::McpReply;
use crate::mcp::chart::ChartCommand;
use crate::mcp::chart::control::{ChartChangeResult, TimeframeResult};
use crate::mcp::chart::data;
use crate::mcp::chart::drawings::{DrawingEdit, NewDrawing};
use crate::mcp::confirm::Summary;
use crate::mcp::indicator::{IndicatorValuesResult, ReadingMeta, ReadingRequest, indicator_values};
use crate::mcp::types::{
    AddChartResult, AddIndicatorResult, CandlesResult, ChartInfo, ChartsResult,
    DrawingsChangeResult, DrawingsResult, IndicatorsInfo, PineInstanceInfo, WorkspaceInfo,
};
use crate::model::watchlist::now_ms;
use crate::model::{
    BuiltIn, ChartType, DataSource, Drawing, DrawingCommand, IndicatorConfig, InstrumentId,
    Timeframe,
};
use crate::workspace::{
    ChartSummary, ChartTypeError, ChartWorkspace, IndicatorError, IndicatorOutcome, RemoveOutcome,
};
use eframe::egui;

/// Runs a chart command on the app thread, on the workspace it names.
pub fn run(
    command: ChartCommand,
    tabs: &impl McpTabs,
    ctx: &egui::Context,
) -> Result<McpReply, McpError> {
    use ChartCommand as C;
    match command {
        C::ListWorkspaces => {
            let mut workspaces = Vec::new();
            tabs.each_workspace(|workspace, focused| {
                workspaces.push(workspace_info(workspace, focused));
            });
            Ok(McpReply::Workspaces(workspaces))
        }
        C::ListCharts { workspace_id } => tabs.with_workspace(workspace_id, |workspace| {
            Ok(McpReply::Charts(charts_result(workspace)))
        }),
        C::AddChart {
            workspace_id,
            instrument,
        } => tabs.with_workspace(workspace_id, |workspace| {
            Ok(McpReply::ChartAdded(add_chart(workspace, &instrument, ctx)))
        }),
        C::ApplyIndicator {
            workspace_id,
            chart_id,
            indicator,
            ema_period,
        } => tabs.with_workspace(workspace_id, |workspace| {
            apply_indicator(workspace, chart_id, indicator, ema_period)
                .map(McpReply::IndicatorApplied)
        }),
        C::RemoveIndicator {
            workspace_id,
            chart_id,
            indicator,
        } => tabs.with_workspace(workspace_id, |workspace| {
            remove_indicator(workspace, chart_id, indicator).map(McpReply::IndicatorApplied)
        }),
        C::GetIndicatorValues {
            workspace_id,
            chart_id,
            request,
        } => tabs.with_workspace(workspace_id, |workspace| {
            indicator_readings(workspace, chart_id, &request)
                .map(|result| McpReply::IndicatorValues(Box::new(result)))
        }),
        C::GetSessionStats {
            workspace_id,
            chart_id,
            query,
        } => tabs.with_workspace(workspace_id, |workspace| {
            session_statistics(workspace, chart_id, &query)
                .map(|result| McpReply::SessionStats(Box::new(result)))
        }),
        C::GetCandles {
            workspace_id,
            chart_id,
            limit,
            before,
        } => tabs.with_workspace(workspace_id, |workspace| {
            chart_candles(workspace, chart_id, limit, before)
                .map(|result| McpReply::Candles(Box::new(result)))
        }),
        C::ListDrawings {
            workspace_id,
            chart_id,
        } => tabs.with_workspace(workspace_id, |workspace| {
            chart_drawings(workspace, chart_id).map(|result| McpReply::Drawings(Box::new(result)))
        }),
        C::AddDrawings {
            workspace_id,
            chart_id,
            drawings,
        } => tabs.with_workspace(workspace_id, |workspace| {
            add_drawings(workspace, chart_id, &drawings)
                .map(|result| McpReply::DrawingsChanged(Box::new(result)))
        }),
        C::EditDrawing {
            workspace_id,
            chart_id,
            edit,
        } => tabs.with_workspace(workspace_id, |workspace| {
            edit_drawing(workspace, chart_id, &edit)
                .map(|result| McpReply::DrawingsChanged(Box::new(result)))
        }),
        C::RemoveDrawings {
            workspace_id,
            chart_id,
            drawing_ids,
        } => tabs.with_workspace(workspace_id, |workspace| {
            remove_drawings(workspace, chart_id, &drawing_ids)
                .map(|result| McpReply::DrawingsChanged(Box::new(result)))
        }),
        C::SetTimeframe {
            workspace_id,
            timeframe,
        } => tabs.with_workspace(workspace_id, |workspace| {
            Ok(McpReply::Timeframe(set_timeframe(
                workspace, timeframe, ctx,
            )))
        }),
        C::SetChartType {
            workspace_id,
            chart_id,
            chart_type,
        } => tabs.with_workspace(workspace_id, |workspace| {
            set_chart_type(workspace, chart_id, chart_type)
                .map(|result| McpReply::ChartChanged(Box::new(result)))
        }),
        C::RemoveChart {
            workspace_id,
            chart_id,
        } => tabs.with_workspace(workspace_id, |workspace| {
            remove_chart(workspace, chart_id).map(|result| McpReply::ChartChanged(Box::new(result)))
        }),
    }
}

/// What a chart command the user is asked about would do (see [`confirmation`]).
pub fn describe(command: &ChartCommand, tabs: &impl McpTabs) -> Result<Summary, McpError> {
    match command {
        ChartCommand::RemoveChart {
            workspace_id,
            chart_id,
        } => tabs.with_workspace(*workspace_id, |workspace| {
            confirmation::remove_chart(workspace, *chart_id)
        }),
        ChartCommand::RemoveDrawings {
            workspace_id,
            chart_id,
            drawing_ids,
        } => tabs.with_workspace(*workspace_id, |workspace| {
            confirmation::remove_drawings(workspace, *chart_id, drawing_ids)
        }),
        _ => confirmation::plainly(command),
    }
}

pub fn source_slug(source: DataSource) -> &'static str {
    match source {
        DataSource::Binance => "binance",
        DataSource::Coinbase => "coinbase",
        DataSource::CoinGecko => "coingecko",
        DataSource::DexScreener => "dexscreener",
        DataSource::Alpaca => "alpaca",
        DataSource::Polymarket => "polymarket",
        DataSource::Kalshi => "kalshi",
        DataSource::CoinMarketCap => "coinmarketcap",
    }
}

pub fn workspace_info(workspace: &ChartWorkspace, active: bool) -> WorkspaceInfo {
    WorkspaceInfo {
        workspace_id: workspace.id().to_string(),
        title: workspace.title(),
        chart_count: workspace.chart_count(),
        active,
        timeframe: workspace.timeframe().label().to_owned(),
    }
}

fn chart_info(workspace: &ChartWorkspace, chart: ChartSummary) -> ChartInfo {
    let indicators = chart.indicators;
    let status = if chart.error.is_some() {
        "error"
    } else if chart.loading {
        "loading"
    } else {
        "ready"
    };
    ChartInfo {
        chart_id: chart.id.to_string(),
        instrument: chart.instrument.key(),
        source: source_slug(chart.instrument.source).to_owned(),
        symbol: chart.instrument.symbol.clone(),
        display_symbol: chart.display_symbol,
        timeframe: workspace.timeframe().label().to_owned(),
        chart_type: if chart.is_line_chart {
            "line"
        } else {
            chart.chart_type.slug()
        }
        .to_owned(),
        indicators: indicators_info(indicators),
        pine_instances: chart
            .pine_instances
            .into_iter()
            .map(|(id, name)| PineInstanceInfo {
                instance_id: id.to_string(),
                script_name: name,
            })
            .collect(),
        status: status.to_owned(),
        error: chart.error,
        focused: chart.focused,
    }
}

/// What the chart draws now: applied and not hidden with its eye.
fn indicators_info(config: IndicatorConfig) -> IndicatorsInfo {
    let drawn = config.drawn();
    IndicatorsInfo {
        volume: drawn.show_volume,
        rsi: drawn.show_rsi,
        ema: drawn.show_ema,
        ema_period: config.ema_period,
        bollinger: drawn.show_bollinger,
        trend_flips: drawn.show_trend_flips,
    }
}

/// Applies a built-in indicator through the workspace's own method and describes the result.
pub fn apply_indicator(
    workspace: &mut ChartWorkspace,
    chart_id: u64,
    indicator: BuiltIn,
    ema_period: Option<usize>,
) -> Result<AddIndicatorResult, McpError> {
    let workspace_id = workspace.id();
    match workspace.apply_indicator(chart_id, indicator, ema_period) {
        Ok((outcome, config)) => Ok(AddIndicatorResult {
            workspace_id: workspace_id.to_string(),
            chart_id: chart_id.to_string(),
            indicator: indicator.id().to_owned(),
            status: match outcome {
                IndicatorOutcome::Added => "added",
                IndicatorOutcome::Updated => "updated",
                IndicatorOutcome::AlreadyApplied => "already_applied",
            }
            .to_owned(),
            indicators: indicators_info(config),
        }),
        Err(IndicatorError::NoSuchChart) => Err(McpError::NotFound(format!(
            "No chart {chart_id} in workspace {workspace_id}"
        ))),
        Err(IndicatorError::UnsupportedOnLineChart) => Err(McpError::Unsupported(format!(
            "{} is not available on line charts",
            indicator.menu_title()
        ))),
        Err(IndicatorError::NoParameters) => Err(McpError::InvalidArgument(format!(
            "{} has no parameters",
            indicator.menu_title()
        ))),
        Err(IndicatorError::InvalidEmaPeriod) => Err(McpError::InvalidArgument(format!(
            "EMA period must be one of {:?}",
            crate::model::EMA_PERIODS
        ))),
    }
}

pub fn charts_result(workspace: &ChartWorkspace) -> ChartsResult {
    ChartsResult {
        workspace_id: workspace.id().to_string(),
        charts: workspace
            .chart_summaries()
            .into_iter()
            .map(|chart| chart_info(workspace, chart))
            .collect(),
    }
}

/// Adds the chart through the workspace's own path and describes the outcome.
pub fn add_chart(
    workspace: &mut ChartWorkspace,
    instrument: &InstrumentId,
    ctx: &eframe::egui::Context,
) -> AddChartResult {
    let outcome = workspace.add_chart_for(instrument, ctx);
    let chart = workspace
        .chart_summaries()
        .into_iter()
        .find(|chart| chart.id == outcome.chart_id);
    AddChartResult {
        workspace_id: workspace.id().to_string(),
        chart_id: outcome.chart_id.to_string(),
        instrument: instrument.key(),
        source: source_slug(instrument.source).to_owned(),
        symbol: chart.as_ref().map_or_else(
            || instrument.symbol.clone(),
            |chart| chart.instrument.symbol.clone(),
        ),
        display_symbol: chart.map_or_else(
            || instrument.source.display_symbol(&instrument.symbol),
            |chart| chart.display_symbol,
        ),
        timeframe: workspace.timeframe().label().to_owned(),
        status: if outcome.created {
            "created"
        } else {
            "already_present"
        }
        .to_owned(),
    }
}

pub(super) fn no_chart(workspace: &ChartWorkspace, chart_id: u64) -> McpError {
    McpError::NotFound(format!(
        "No chart {chart_id} in workspace {}",
        workspace.id()
    ))
}

/// The newest `limit` real candles of a chart, or those before `before`.
pub fn chart_candles(
    workspace: &ChartWorkspace,
    chart_id: u64,
    limit: usize,
    before: Option<i64>,
) -> Result<CandlesResult, McpError> {
    let data = workspace
        .chart_data(chart_id)
        .ok_or_else(|| no_chart(workspace, chart_id))?;
    let meta = data::CandleMeta {
        workspace_id: workspace.id(),
        chart_id,
        older_supported: data
            .instrument
            .source
            .supports_older_history(workspace.timeframe()),
        instrument: data.instrument,
        timeframe: workspace.timeframe().label().to_owned(),
        chart_type: data.chart_type,
        replay_ms: data.replay_ms,
        now_ms: now_ms(),
    };
    Ok(data::candles_result(&meta, data.candles, limit, before))
}

pub fn chart_drawings(
    workspace: &ChartWorkspace,
    chart_id: u64,
) -> Result<DrawingsResult, McpError> {
    let data = workspace
        .chart_data(chart_id)
        .ok_or_else(|| no_chart(workspace, chart_id))?;
    Ok(data::drawings_result(
        workspace.id(),
        chart_id,
        data.drawings,
    ))
}

/// The result of a change: the drawings it touched as they are now.
fn drawings_changed(
    workspace: &ChartWorkspace,
    chart_id: u64,
    status: &str,
    ids: &[u64],
) -> Result<DrawingsChangeResult, McpError> {
    let data = workspace
        .chart_data(chart_id)
        .ok_or_else(|| no_chart(workspace, chart_id))?;
    Ok(DrawingsChangeResult {
        workspace_id: workspace.id().to_string(),
        chart_id: chart_id.to_string(),
        status: status.to_owned(),
        drawing_ids: ids.iter().map(u64::to_string).collect(),
        drawings: data
            .drawings
            .iter()
            .filter(|drawing| ids.contains(&drawing.id()))
            .filter_map(data::drawing_info)
            .collect(),
        total_drawings: data::counted(data.drawings),
    })
}

/// Adds the drawings as one undo step. All or none: a drawing that cannot be placed refuses the
/// call before anything changes.
pub fn add_drawings(
    workspace: &mut ChartWorkspace,
    chart_id: u64,
    specs: &[NewDrawing],
) -> Result<DrawingsChangeResult, McpError> {
    let data = workspace
        .chart_data(chart_id)
        .ok_or_else(|| no_chart(workspace, chart_id))?;
    let first_id = data::next_free_id(data.drawings, data.next_drawing_id);
    let drawings = data::plan_add(data.drawings, data.candles, first_id, specs)?;
    let ids: Vec<u64> = drawings.iter().map(Drawing::id).collect();
    // `plan_add` already refused a range that does not fit.
    let next_free = first_id.saturating_add(ids.len() as u64);
    let commands = drawings.into_iter().map(DrawingCommand::Add).collect();
    workspace.apply_planned_drawings(chart_id, commands, next_free);
    drawings_changed(workspace, chart_id, "added", &ids)
}

pub fn edit_drawing(
    workspace: &mut ChartWorkspace,
    chart_id: u64,
    edit: &DrawingEdit,
) -> Result<DrawingsChangeResult, McpError> {
    let data = workspace
        .chart_data(chart_id)
        .ok_or_else(|| no_chart(workspace, chart_id))?;
    let command = data::plan_edit(data.drawings, data.candles, edit)?;
    let status = if command.is_some() {
        "updated"
    } else {
        "unchanged"
    };
    if let Some(command) = command {
        workspace.apply_planned_drawings(chart_id, vec![command], 0);
    }
    drawings_changed(workspace, chart_id, status, &[edit.drawing_id])
}

pub fn remove_drawings(
    workspace: &mut ChartWorkspace,
    chart_id: u64,
    ids: &[u64],
) -> Result<DrawingsChangeResult, McpError> {
    let data = workspace
        .chart_data(chart_id)
        .ok_or_else(|| no_chart(workspace, chart_id))?;
    let commands = data::plan_remove(data.drawings, ids)?;
    let removed: Vec<u64> = commands
        .iter()
        .filter_map(|command| match command {
            DrawingCommand::Remove(id) => Some(*id),
            _ => None,
        })
        .collect();
    workspace.apply_planned_drawings(chart_id, commands, 0);
    drawings_changed(workspace, chart_id, "removed", &removed)
}

/// Takes a built-in indicator off a chart through the workspace's own method.
pub fn remove_indicator(
    workspace: &mut ChartWorkspace,
    chart_id: u64,
    indicator: BuiltIn,
) -> Result<AddIndicatorResult, McpError> {
    let workspace_id = workspace.id();
    match workspace.remove_indicator(chart_id, indicator) {
        Ok((outcome, config)) => Ok(AddIndicatorResult {
            workspace_id: workspace_id.to_string(),
            chart_id: chart_id.to_string(),
            indicator: indicator.id().to_owned(),
            status: match outcome {
                RemoveOutcome::Removed => "removed",
                RemoveOutcome::NotApplied => "not_applied",
            }
            .to_owned(),
            indicators: indicators_info(config),
        }),
        Err(_) => Err(no_chart(workspace, chart_id)),
    }
}

/// Indicator readings over a market chart's own candles, cut at the replay position.
pub fn indicator_readings(
    workspace: &ChartWorkspace,
    chart_id: u64,
    request: &ReadingRequest,
) -> Result<IndicatorValuesResult, McpError> {
    let data = workspace
        .chart_data(chart_id)
        .ok_or_else(|| no_chart(workspace, chart_id))?;
    // A constructed chart type (Heikin Ashi, Renko, …) draws bars built from the candles, and its
    // indicators are computed over those: read the same ones.
    let constructed = workspace.constructed_bars(chart_id);
    let meta = ReadingMeta {
        workspace_id: workspace.id(),
        chart_id,
        instrument: data.instrument.key(),
        timeframe: workspace.timeframe().label().to_owned(),
        chart_type: data.chart_type.slug().to_owned(),
        constructed: constructed.is_some(),
        replay_ms: data.replay_ms,
        now_ms: now_ms(),
    };
    let bars = constructed.as_deref().unwrap_or(data.candles);
    Ok(indicator_values(&meta, bars, request))
}

/// Session statistics over a market chart's own candles, cut at the replay position. Never the
/// bars of a constructed chart type: a Renko brick or a Heikin Ashi bar is not a trading day.
pub fn session_statistics(
    workspace: &ChartWorkspace,
    chart_id: u64,
    query: &crate::mcp::session_stats::Query,
) -> Result<crate::mcp::session_stats::SessionStatsResult, McpError> {
    let data = workspace
        .chart_data(chart_id)
        .ok_or_else(|| no_chart(workspace, chart_id))?;
    let meta = crate::mcp::session_stats::Meta {
        workspace_id: workspace.id(),
        chart_id,
        instrument: data.instrument.key(),
        timeframe: workspace.timeframe().label().to_owned(),
        bar_ms: crate::model::session_stats::bar_ms(workspace.timeframe()),
        hours: crate::model::session_stats::MarketHours::for_source(data.instrument.source),
    };
    crate::mcp::session_stats::session_stats(&meta, data.candles, query)
}

/// Moves the workspace to another timeframe through the toolbar's own path.
pub fn set_timeframe(
    workspace: &mut ChartWorkspace,
    timeframe: Timeframe,
    ctx: &eframe::egui::Context,
) -> TimeframeResult {
    let previous = workspace.timeframe();
    let changed = workspace.set_timeframe_for_mcp(timeframe, ctx);
    TimeframeResult {
        workspace_id: workspace.id().to_string(),
        timeframe: workspace.timeframe().label().to_owned(),
        previous_timeframe: previous.label().to_owned(),
        status: if changed { "changed" } else { "unchanged" }.to_owned(),
        chart_count: workspace.chart_count(),
    }
}

fn chart_now(workspace: &ChartWorkspace, chart_id: u64) -> Option<ChartInfo> {
    let summary = workspace
        .chart_summaries()
        .into_iter()
        .find(|chart| chart.id == chart_id)?;
    Some(chart_info(workspace, summary))
}

/// Switches a market chart's type through the chart's own method.
pub fn set_chart_type(
    workspace: &mut ChartWorkspace,
    chart_id: u64,
    chart_type: ChartType,
) -> Result<ChartChangeResult, McpError> {
    let changed = match workspace.set_chart_type_for_mcp(chart_id, chart_type) {
        Ok(changed) => changed,
        Err(ChartTypeError::NoSuchChart) => return Err(no_chart(workspace, chart_id)),
        Err(ChartTypeError::Unsupported(reason)) => {
            return Err(McpError::Unsupported(format!(
                "{} is not available on this chart: {reason}",
                chart_type.label()
            )));
        }
    };
    Ok(ChartChangeResult {
        workspace_id: workspace.id().to_string(),
        chart_id: chart_id.to_string(),
        status: if changed { "changed" } else { "unchanged" }.to_owned(),
        chart: chart_now(workspace, chart_id),
        charts_in_workspace: workspace.chart_count(),
    })
}

/// Removes a market chart through the workspace's own path.
pub fn remove_chart(
    workspace: &mut ChartWorkspace,
    chart_id: u64,
) -> Result<ChartChangeResult, McpError> {
    if !workspace.remove_market_chart(chart_id) {
        return Err(no_chart(workspace, chart_id));
    }
    Ok(ChartChangeResult {
        workspace_id: workspace.id().to_string(),
        chart_id: chart_id.to_string(),
        status: "removed".to_owned(),
        chart: None,
        charts_in_workspace: workspace.chart_count(),
    })
}
