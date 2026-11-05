//! A picture of a chart as an image the client can look at.
//!
//! The chart is frozen on the app thread, in one frame and from the very data the chart on screen
//! is drawn from; the drawing and the PNG run on the snapshot worker. The tool starts the job
//! through the bridge, then asks after it every 100 ms until it is ready, so no request ever
//! waits on the app thread. Nothing is saved, copied or uploaded. A picture over 8 MiB is refused (ask for `1x`).

use crate::mcp::bridge::{McpCommand, McpError, McpReply, REQUEST_TIMEOUT};
use crate::mcp::tools::{DegenViewMcp, client_of, failure, parse_chart_id, parse_id, unexpected};
use crate::model::mcp::Permission;
use crate::model::snapshot::SnapshotScale;
use rmcp::handler::server::tool::Extension;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock};
use rmcp::{tool, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;
use std::sync::Arc;
use std::time::{Duration, Instant};

pub const GET_CHART_SNAPSHOT: &str = "degenview_get_chart_snapshot";

/// How long the picture is waited for. A chart takes a second or two; a loaded machine more.
const DEADLINE: Duration = Duration::from_secs(45);
const POLL: Duration = Duration::from_millis(100);

#[derive(Debug, Clone, PartialEq)]
pub struct SnapshotStarted {
    pub job_id: u64,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SnapshotProgress {
    Pending,
    Ready {
        png: Arc<Vec<u8>>,
        width: u32,
        height: u32,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
pub enum ScaleArg {
    #[serde(rename = "1x")]
    Standard,
    #[serde(rename = "2x")]
    High,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SnapshotParams {
    /// A `workspace_id` from `degenview_list_workspaces`.
    #[schemars(length(max = 20))]
    pub workspace_id: String,
    /// A `chart_id` from `degenview_list_charts`. A market chart, not a portfolio card.
    #[schemars(length(max = 20))]
    pub chart_id: String,
    /// Pixels per point: `1x` (default, smaller to send) or `2x` (sharper). A very large chart is
    /// drawn at a smaller scale than asked.
    pub scale: Option<ScaleArg>,
}

/// Standard base64 (RFC 4648, padded), which is what an MCP image block carries.
pub fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let group = u32::from(chunk[0]) << 16
            | u32::from(*chunk.get(1).unwrap_or(&0)) << 8
            | u32::from(*chunk.get(2).unwrap_or(&0));
        for (index, shift) in [18, 12, 6, 0].into_iter().enumerate() {
            if index <= chunk.len() {
                out.push(ALPHABET[(group >> shift & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[tool_router(router = snapshot_router, vis = "pub(super)")]
impl DegenViewMcp {
    #[tool(
        name = "degenview_get_chart_snapshot",
        description = "A picture (PNG) of a chart exactly as DegenView draws it: candles or the chart's type, built-in indicators, every applied script's plots and trades, drawings, the last-price line, both axes and the chart's alert lines, cut at the replay position when a replay runs. Left out: the crosshair, selection, menus and everything around the chart. Orders and positions from paper trading appear only if the user included them in Settings > Snapshots. The image is returned as an image block, followed by a text block with its size and what it shows. Nothing is saved, copied or uploaded. Use it to look at the chart you are drawing on, or to check a script's output. The chart must have been drawn on screen at least once (it has a size) and have candles.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn get_chart_snapshot(
        &self,
        Extension(parts): Extension<http::request::Parts>,
        Parameters(params): Parameters<SnapshotParams>,
    ) -> Result<CallToolResult, CallToolResult> {
        let client = client_of(&parts)?;
        let workspace_id = parse_id(&params.workspace_id, "workspace_id")?;
        let chart_id = parse_chart_id(&params.chart_id)?;
        let scale = match params.scale {
            None | Some(ScaleArg::Standard) => SnapshotScale::X1,
            Some(ScaleArg::High) => SnapshotScale::X2,
        };
        let started = match self
            .bridge
            .call(
                &client.id,
                Permission::ChartsSnapshotRead,
                McpCommand::BeginSnapshot {
                    workspace_id,
                    chart_id,
                    scale,
                },
                REQUEST_TIMEOUT,
            )
            .await
        {
            Ok(McpReply::SnapshotStarted(started)) => started,
            Ok(_) => return Err(unexpected()),
            Err(error) => return Err(failure(&error)),
        };
        let deadline = Instant::now() + DEADLINE;
        loop {
            tokio::time::sleep(POLL).await;
            match self
                .bridge
                .call(
                    &client.id,
                    Permission::ChartsSnapshotRead,
                    McpCommand::PollSnapshot {
                        job_id: started.job_id,
                    },
                    REQUEST_TIMEOUT,
                )
                .await
            {
                Ok(McpReply::SnapshotProgress(SnapshotProgress::Ready { png, width, height })) => {
                    let meta = serde_json::json!({
                        "workspace_id": workspace_id.to_string(),
                        "chart_id": chart_id.to_string(),
                        "width": width,
                        "height": height,
                        "scale": if scale == SnapshotScale::X1 { "1x" } else { "2x" },
                        "bytes": png.len(),
                    });
                    return Ok(CallToolResult::success(vec![
                        ContentBlock::image(base64(&png), "image/png"),
                        ContentBlock::text(meta.to_string()),
                    ]));
                }
                Ok(McpReply::SnapshotProgress(SnapshotProgress::Pending)) => {}
                Ok(_) => return Err(unexpected()),
                Err(error) => return Err(failure(&error)),
            }
            if Instant::now() >= deadline {
                return Err(failure(&McpError::Timeout));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_the_rfc_vectors() {
        for (input, expected) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(base64(input.as_bytes()), expected, "{input:?}");
        }
        assert_eq!(base64(&[0xFF, 0xEE, 0xDD]), "/+7d");
    }
}
