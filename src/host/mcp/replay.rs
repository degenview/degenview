use crate::host::mcp::McpTabs;
use crate::host::mcp::confirmation;
use crate::mcp::bridge::McpError;
use crate::mcp::bridge::McpReply;
use crate::mcp::confirm::Summary;
use crate::mcp::replay::ReplayRequest;
use crate::mcp::replay::{self, ReplayResult};
use crate::model::{ReplayCommand, ReplayError};
use crate::workspace::ChartWorkspace;

/// Runs a replay command on the workspace it names.
pub fn run(command: ReplayRequest, tabs: &impl McpTabs) -> Result<McpReply, McpError> {
    match command {
        ReplayRequest::Get { workspace_id } => tabs.with_workspace(workspace_id, |workspace| {
            Ok(McpReply::Replay(replay_state(workspace)))
        }),
        ReplayRequest::Control {
            workspace_id,
            command,
        } => tabs.with_workspace(workspace_id, |workspace| {
            control_replay(workspace, command).map(McpReply::Replay)
        }),
    }
}

/// Replay control is asked about only of a remote credential, from the command alone.
pub fn describe(command: &ReplayRequest) -> Result<Summary, McpError> {
    confirmation::plainly(command)
}

pub fn replay_state(workspace: &ChartWorkspace) -> ReplayResult {
    replay::replay_result(
        workspace.id(),
        workspace.timeframe().label(),
        &workspace.replay_view(),
    )
}

pub fn control_replay(
    workspace: &mut ChartWorkspace,
    command: ReplayCommand,
) -> Result<ReplayResult, McpError> {
    let view =
        workspace.replay_command(command).map_err(|error| {
            match error {
        ReplayError::NoSuchChart => McpError::NotFound(format!(
            "No market chart with that id in workspace {}",
            workspace.id()
        )),
        ReplayError::NoCandles => McpError::Conflict(
            "The chart has no candles yet (it may still be loading), so there is nothing to replay"
                .into(),
        ),
        ReplayError::NotRunning => McpError::Conflict(
            "No replay is running in this workspace: start one with action `start`".into(),
        ),
    }
        })?;
    Ok(replay::replay_result(
        workspace.id(),
        workspace.timeframe().label(),
        &view,
    ))
}
