use crate::mcp::bridge::McpError;
use crate::mcp::replay::{self, ReplayResult};
use crate::workspace::{ChartWorkspace, ReplayCommand, ReplayError};

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
