# MCP guidance

- `src/mcp/` alone may use async: one current-thread Tokio runtime on the `mcp-server` thread for
  rmcp/axum. Keep it egui-free; `mcp_host.rs` bridges stateful workspace access.
- Tools validate and enqueue `mcp::bridge::Request`; `DegenViewApp::tick_mcp` executes through
  workspace methods on the app thread. Tool/HTTP threads never lock `TabSlot`/`ChartWorkspace` or
  write SQLite.
- Authorization is per call from fresh `McpShared` state. Add a `Permission`, map it in
  `tools::required_permission`, and test direct-call denial. Persist before publishing shared state.
  Store/log only SHA-256 token verifiers, never tokens.
- Price-alert tools plan against `AlertHost` in `mcp_host.rs`, then enqueue
  `AlertRuntimeCommand`; they never write alert tables or reimplement `PriceAlertEditorState`.
- Read-only script tools use `mcp/pine.rs` with `ScriptStore` and compiler on a worker. Chart-private
  Pine state goes through `ChartWorkspace::pine_instance_views` on the app thread. Do not invent
  reports or alert history.
- Applying/changing/removing a script on a chart goes through `ChartWorkspace::{add_pine_instance_to,
  change_pine_instance,remove_pine_instance_from}`, which take the undo step; validate scripts and
  inputs with `mcp::pine` before queuing, and apply only the saved text, never the draft.
- Pine edits use revision-checked `mcp::pine::{update_draft,save_script}` on the app thread, followed
  by `ScriptManager::adopt_external_text`. Never bypass the live editor revision or write script
  files from a worker.
- Drawings from MCP are planned from `ChartWorkspace::chart_data` with `mcp::chart_data` and
  committed through `apply_planned_drawings` (the pointer's path, one undo step); tool threads never
  read `Chart.drawings` or `Chart.candles`. Candles returned are the market's own, not constructed
  bars. Validate arguments in `mcp::drawings` before queueing, and never edit a locked drawing.
- Settings holds the store lock, so its MCP UI returns `McpAction`s for `McpHost::queue`; never call
  `McpHost::apply` inside the Settings closure.
- Verify against `127.0.0.1:<port>/mcp` with seeded `mcp.settings`/`mcp_client` rows, a throwaway
  `HOME`, `DEGENVIEW_NO_KEYCHAIN=1`, and `Authorization: Bearer ...`.

See [implementation](../mcp-implementation.md) and [remaining work](../mcp-remaining-work.md).
