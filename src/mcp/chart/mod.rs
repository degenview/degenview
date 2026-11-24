//! The chart tools (`tools`, `control`) and what they hand the app thread ([`ChartCommand`], run in
//! `host::mcp::chart`): workspaces, charts, built-in indicators, candles, drawings and chart
//! control. `data` builds their results from plain values; `drawings` checks drawing arguments.

pub mod control;
pub mod data;
pub mod drawings;
pub mod tools;

use crate::i18n::tr_n;
use crate::mcp::chart::control::{REMOVE_CHART, SET_CHART_TYPE, SET_TIMEFRAME};
use crate::mcp::chart::drawings::{DrawingEdit, NewDrawing};
use crate::mcp::confirm::{Confirm, Gate, Operation};
use crate::mcp::indicator::ReadingRequest;
use crate::mcp::indicator::tools::REMOVE_INDICATOR;
use crate::mcp::session_stats::Query;
use crate::mcp::tools::{
    ADD_CHART, ADD_DRAWINGS, ADD_INDICATOR, DELETE_DRAWINGS, McpIndicator, UPDATE_DRAWING,
};
use crate::model::{ChartType, InstrumentId, Timeframe};

#[derive(Debug, Clone, PartialEq)]
pub enum ChartCommand {
    ListWorkspaces,
    ListCharts {
        workspace_id: u64,
    },
    AddChart {
        workspace_id: u64,
        instrument: InstrumentId,
    },
    ApplyIndicator {
        workspace_id: u64,
        chart_id: u64,
        indicator: McpIndicator,
        ema_period: Option<usize>,
    },
    /// Takes a built-in indicator off a chart.
    RemoveIndicator {
        workspace_id: u64,
        chart_id: u64,
        indicator: McpIndicator,
    },
    /// Built-in indicator readings over a chart's candles.
    GetIndicatorValues {
        workspace_id: u64,
        chart_id: u64,
        request: ReadingRequest,
    },
    /// Session statistics over a chart's candles.
    GetSessionStats {
        workspace_id: u64,
        chart_id: u64,
        query: Query,
    },
    /// The newest `limit` candles of a chart, or those before `before` (epoch ms).
    GetCandles {
        workspace_id: u64,
        chart_id: u64,
        limit: usize,
        before: Option<i64>,
    },
    ListDrawings {
        workspace_id: u64,
        chart_id: u64,
    },
    AddDrawings {
        workspace_id: u64,
        chart_id: u64,
        drawings: Vec<NewDrawing>,
    },
    EditDrawing {
        workspace_id: u64,
        chart_id: u64,
        edit: Box<DrawingEdit>,
    },
    RemoveDrawings {
        workspace_id: u64,
        chart_id: u64,
        drawing_ids: Vec<u64>,
    },
    /// Moves the whole workspace to another timeframe.
    SetTimeframe {
        workspace_id: u64,
        timeframe: Timeframe,
    },
    SetChartType {
        workspace_id: u64,
        chart_id: u64,
        chart_type: ChartType,
    },
    RemoveChart {
        workspace_id: u64,
        chart_id: u64,
    },
}

impl Confirm for ChartCommand {
    fn gate(&self) -> Gate {
        match self {
            // Cannot be undone from here.
            Self::RemoveDrawings { .. } => Gate::Ask(DELETE_DRAWINGS),
            Self::RemoveChart { .. } => Gate::Ask(REMOVE_CHART),
            // Changes, but easy to see and to undo in the app.
            Self::AddChart { .. } => Gate::AskRemote(ADD_CHART),
            Self::ApplyIndicator { .. } => Gate::AskRemote(ADD_INDICATOR),
            Self::RemoveIndicator { .. } => Gate::AskRemote(REMOVE_INDICATOR),
            Self::AddDrawings { .. } => Gate::AskRemote(ADD_DRAWINGS),
            Self::EditDrawing { .. } => Gate::AskRemote(UPDATE_DRAWING),
            Self::SetTimeframe { .. } => Gate::AskRemote(SET_TIMEFRAME),
            Self::SetChartType { .. } => Gate::AskRemote(SET_CHART_TYPE),
            // Reads.
            Self::ListWorkspaces
            | Self::ListCharts { .. }
            | Self::GetIndicatorValues { .. }
            | Self::GetSessionStats { .. }
            | Self::GetCandles { .. }
            | Self::ListDrawings { .. } => Gate::Free,
        }
    }

    fn plain_summary(&self) -> Option<(Operation, String)> {
        Some(match self {
            Self::AddChart { instrument, .. } => (Operation::AddChart, instrument.symbol.clone()),
            Self::ApplyIndicator { indicator, .. } => {
                (Operation::AddIndicator, format!("{indicator:?}"))
            }
            Self::RemoveIndicator { indicator, .. } => {
                (Operation::RemoveIndicator, format!("{indicator:?}"))
            }
            Self::AddDrawings { drawings, .. } => (
                Operation::DrawOnChart,
                tr_n!(drawings.len(), "{count} drawing", "{count} drawings"),
            ),
            Self::EditDrawing { .. } => (Operation::ChangeDrawing, String::new()),
            Self::SetTimeframe { timeframe, .. } => {
                (Operation::ChangeTimeframe, timeframe.to_string())
            }
            Self::SetChartType { chart_type, .. } => {
                (Operation::ChangeChartType, format!("{chart_type:?}"))
            }
            _ => return None,
        })
    }
}
