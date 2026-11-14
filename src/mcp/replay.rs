//! Replay tools: read the workspace's replay clock and drive it. The clock is the one the replay
//! bar uses; while it runs every chart in the workspace shows only the past, and the candle and
//! indicator tools read the same cut-off.

use crate::mcp::bridge::{McpCommand, McpReply, REQUEST_TIMEOUT};
use crate::mcp::chart::data::iso;
use crate::mcp::chart::drawings::{TimeArg, parse_time};
use crate::mcp::tools::{
    DegenViewMcp, client_of, failure, invalid, parse_chart_id, parse_workspace_id, unexpected,
};
use crate::model::mcp::Permission;
use crate::model::{ReplayCommand, ReplayStart};
use crate::model::{ReplaySpeed, ReplayStatus, ReplayView};
use rmcp::handler::server::tool::Extension;
use rmcp::handler::server::wrapper::{Json, Parameters};
use rmcp::model::CallToolResult;
use rmcp::{tool, tool_router};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const GET_REPLAY: &str = "degenview_get_replay";
pub const CONTROL_REPLAY: &str = "degenview_control_replay";

/// Bars one step command may move.
const MAX_STEPS: u32 = 500;

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct ReplayResult {
    pub workspace_id: String,
    /// `inactive` (live), `ready`, `selecting_start`, `paused`, `playing` or `completed` (on the
    /// last bar).
    pub status: String,
    /// Whether charts are cut at a replay position.
    pub active: bool,
    pub timeframe: String,
    /// The market the replay runs on.
    pub symbol: Option<String>,
    /// Where it was started, ISO-8601 UTC.
    pub started_at: Option<String>,
    /// The replay position: charts show bars up to and including this one.
    pub current_time: Option<String>,
    pub bar_index: Option<usize>,
    pub bars_total: usize,
    pub bars_remaining: Option<usize>,
    /// `0.5x`, `1x`, `2x`, `5x`, `10x`, `20x`, `50x` or `max`. Only matters while `playing`.
    pub speed: Option<String>,
    /// The timeframe changed under the replay and its bars are still reloading.
    pub preparing: bool,
}

fn status(status: ReplayStatus) -> &'static str {
    match status {
        ReplayStatus::Inactive => "inactive",
        ReplayStatus::Ready => "ready",
        ReplayStatus::SelectingStart => "selecting_start",
        ReplayStatus::Paused => "paused",
        ReplayStatus::Playing => "playing",
        ReplayStatus::Completed => "completed",
    }
}

fn speed_name(speed: ReplaySpeed) -> &'static str {
    match speed {
        ReplaySpeed::Half => "0.5x",
        ReplaySpeed::Normal => "1x",
        ReplaySpeed::Double => "2x",
        ReplaySpeed::Five => "5x",
        ReplaySpeed::Ten => "10x",
        ReplaySpeed::Twenty => "20x",
        ReplaySpeed::Fifty => "50x",
        ReplaySpeed::Max => "max",
    }
}

pub fn replay_result(workspace_id: u64, timeframe: &str, view: &ReplayView) -> ReplayResult {
    ReplayResult {
        workspace_id: workspace_id.to_string(),
        status: status(view.status).to_owned(),
        active: view.current_ms.is_some()
            && !matches!(view.status, ReplayStatus::Inactive | ReplayStatus::Ready),
        timeframe: timeframe.to_owned(),
        symbol: view.symbol.clone(),
        started_at: view.start_ms.map(iso),
        current_time: view.current_ms.map(iso),
        bar_index: view.bar_index,
        bars_total: view.bars_total,
        bars_remaining: view
            .bar_index
            .map(|index| view.bars_total.saturating_sub(index + 1)),
        speed: view.speed.map(|speed| speed_name(speed).to_owned()),
        preparing: view.preparing,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReplayAction {
    /// Start (or restart) the replay, paused, at `time` (default: the chart's first bar).
    Start,
    /// Play at `speed` in real time. Prefer `step_forward`, which is deterministic.
    Play,
    Pause,
    /// Move `steps` bars (default 1) forward, stopping on the last bar.
    StepForward,
    /// Move `steps` bars (default 1) back, stopping on the first bar.
    StepBack,
    /// Back to the bar the replay started on.
    Restart,
    /// To the bar at or before `time`.
    Seek,
    SetSpeed,
    /// Back to live.
    Stop,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
pub enum ReplaySpeedArg {
    #[serde(rename = "0.5x")]
    Half,
    #[serde(rename = "1x")]
    Normal,
    #[serde(rename = "2x")]
    Double,
    #[serde(rename = "5x")]
    Five,
    #[serde(rename = "10x")]
    Ten,
    #[serde(rename = "20x")]
    Twenty,
    #[serde(rename = "50x")]
    Fifty,
    #[serde(rename = "max")]
    Max,
}

impl ReplaySpeedArg {
    fn speed(self) -> ReplaySpeed {
        match self {
            Self::Half => ReplaySpeed::Half,
            Self::Normal => ReplaySpeed::Normal,
            Self::Double => ReplaySpeed::Double,
            Self::Five => ReplaySpeed::Five,
            Self::Ten => ReplaySpeed::Ten,
            Self::Twenty => ReplaySpeed::Twenty,
            Self::Fifty => ReplaySpeed::Fifty,
            Self::Max => ReplaySpeed::Max,
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceReplayParams {
    /// A `workspace_id` from `degenview_list_workspaces`.
    #[schemars(length(max = 20))]
    pub workspace_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ControlReplayParams {
    /// A `workspace_id` from `degenview_list_workspaces`.
    #[schemars(length(max = 20))]
    pub workspace_id: String,
    pub action: ReplayAction,
    /// `start` only: the chart to replay, from `degenview_list_charts`. Default: the focused one.
    /// It becomes the focused chart.
    #[schemars(length(max = 20))]
    pub chart_id: Option<String>,
    /// `start` (where to begin) and `seek` (where to go): ISO-8601 UTC or epoch milliseconds,
    /// snapped to the bar at or before it.
    pub time: Option<TimeArg>,
    /// `step_forward` and `step_back`: bars to move, 1 to 500. Default 1.
    #[schemars(range(min = 1, max = 500))]
    pub steps: Option<u32>,
    /// `set_speed`.
    pub speed: Option<ReplaySpeedArg>,
}

/// The command a call asks for, or the first argument that does not belong to its action.
fn command_of(params: &ControlReplayParams) -> Result<ReplayCommand, CallToolResult> {
    use ReplayAction as A;
    let action = params.action;
    let given = |name: &str, present: bool, allowed: bool| {
        if present && !allowed {
            Err(invalid(format!("{name} does not apply to this action")))
        } else {
            Ok(())
        }
    };
    given("chart_id", params.chart_id.is_some(), action == A::Start)?;
    given(
        "time",
        params.time.is_some(),
        matches!(action, A::Start | A::Seek),
    )?;
    given(
        "steps",
        params.steps.is_some(),
        matches!(action, A::StepForward | A::StepBack),
    )?;
    given("speed", params.speed.is_some(), action == A::SetSpeed)?;
    let time = params
        .time
        .as_ref()
        .map(parse_time)
        .transpose()
        .map_err(|error| failure(&error))?;
    let steps = params.steps.unwrap_or(1);
    if !(1..=MAX_STEPS).contains(&steps) {
        return Err(invalid(format!("steps must be between 1 and {MAX_STEPS}")));
    }
    Ok(match action {
        A::Start => ReplayCommand::Start {
            chart_id: params.chart_id.as_deref().map(parse_chart_id).transpose()?,
            at: time.map_or(ReplayStart::First, ReplayStart::At),
        },
        A::Play => ReplayCommand::Play,
        A::Pause => ReplayCommand::Pause,
        A::StepForward => ReplayCommand::StepForward(steps as usize),
        A::StepBack => ReplayCommand::StepBack(steps as usize),
        A::Restart => ReplayCommand::Restart,
        A::Seek => ReplayCommand::Seek(time.ok_or_else(|| invalid("seek needs a time"))?),
        A::SetSpeed => ReplayCommand::SetSpeed(
            params
                .speed
                .ok_or_else(|| invalid("set_speed needs a speed"))?
                .speed(),
        ),
        A::Stop => ReplayCommand::Stop,
    })
}

#[tool_router(router = replay_router, vis = "pub(super)")]
impl DegenViewMcp {
    #[tool(
        name = "degenview_get_replay",
        description = "Read a workspace's replay: whether charts are cut at a past position, where, on which market, how many bars there are and how many remain, and the playback speed. Replay is workspace-wide and runs on one chart's bars; while it runs, degenview_get_candles and degenview_get_indicator_values return only bars up to the position, as the charts show them.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn get_replay(
        &self,
        Extension(parts): Extension<http::request::Parts>,
        Parameters(params): Parameters<WorkspaceReplayParams>,
    ) -> Result<Json<ReplayResult>, CallToolResult> {
        let client = client_of(&parts)?;
        let workspace_id = parse_workspace_id(&params.workspace_id)?;
        match self
            .bridge
            .call(
                &client,
                Permission::ChartsRead,
                McpCommand::GetReplay { workspace_id },
                REQUEST_TIMEOUT,
            )
            .await
        {
            Ok(McpReply::Replay(result)) => Ok(Json(result)),
            Ok(_) => Err(unexpected()),
            Err(error) => Err(failure(&error)),
        }
    }

    #[tool(
        name = "degenview_control_replay",
        description = "Drive a workspace's replay, the one behind the chart's replay bar: `start` (paused, at `time` or the first bar, on `chart_id` or the focused chart), `step_forward` / `step_back` by `steps` bars, `seek` to a `time`, `restart` to the start bar, `set_speed`, `play` / `pause` (real-time playback; stepping is the deterministic way to walk bars), and `stop` to go back to live. Returns the replay state. Stepping through a chart bar by bar and reading degenview_get_candles each time is how to test a read of the market without seeing the future. Replay changes what every chart in the workspace shows. Paper-trading orders still fill at live prices during replay.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    async fn control_replay(
        &self,
        Extension(parts): Extension<http::request::Parts>,
        Parameters(params): Parameters<ControlReplayParams>,
    ) -> Result<Json<ReplayResult>, CallToolResult> {
        let client = client_of(&parts)?;
        let workspace_id = parse_workspace_id(&params.workspace_id)?;
        let command = command_of(&params)?;
        match self
            .bridge
            .call(
                &client,
                Permission::ChartsReplayWrite,
                McpCommand::ControlReplay {
                    workspace_id,
                    command,
                },
                REQUEST_TIMEOUT,
            )
            .await
        {
            Ok(McpReply::Replay(result)) => Ok(Json(result)),
            Ok(_) => Err(unexpected()),
            Err(error) => Err(failure(&error)),
        }
    }
}
