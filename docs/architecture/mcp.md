# MCP server architecture

The local MCP server is off by default. `rmcp` Streamable HTTP runs statelessly on `axum` with one
current-thread Tokio runtime confined to the `mcp-server` thread.

```text
client → 127.0.0.1:<port>/mcp → bearer/Host/Origin/size checks
       → per-call permission check → Request queue → DegenViewApp::tick_mcp
       → permission/deadline/cancel recheck → existing workspace/host mutation path
```

## Security and execution

- Bind loopback only. Accept `Host` for `127.0.0.1`/`localhost`; reject every `Origin`. Without a
  token, answer nothing—not even 404.
- Tokens are 256 random bits (`dvmcp_<hex>`), shown once. Store only constant-time-compared SHA-256
  verifiers. Revocation affects the next call.
- Effective permissions are client grants intersected with the global cap, both read per call.
  New clients get `charts.read`; the cap defaults read-only. `tools/list` filters and `call_tool`
  rechecks.
- Workers never access workspaces or SQLite. `Request::run` skips abandoned, expired, cancelled,
  pre-stop, or newly unauthorized work; started work reports its real result.
- Settings holds the store lock, so it only queues `McpAction`; `tick_mcp` applies it later.
  Undecodable MCP settings/clients keep the server off without rewriting data.
- Protocol support is rmcp 3.5 / MCP `2025-03-26` through `2026-07-28`, stateless JSON.

Permissions separate reads and mutations: charts read/create/indicator-write/Pine-instance-write; alerts
read/create/edit/pause/delete; Pine script list/read/write/save plus diagnostics, inputs, styles,
reports, and alert reads. Delete and saved-source writes require their own grants.

## Tool behavior

| Area | Tools (`degenview_…`) |
| --- | --- |
| Charts | `list_workspaces`, `list_charts`, `add_chart`, `list_indicators`, `add_indicator` |
| Price alerts | `list_price_alerts`, `create_price_alert`, `update_price_alert`, `pause_price_alert`, `resume_price_alert`, `delete_price_alert` |
| Pine scripts | `list_pine_scripts`, `get_pine_script`, `update_pine_script`, `save_pine_script`, `compile_pine_script`, `get_pine_inputs` |
| Pine instances | `list_chart_pine_instances`, `get_pine_instance`, `get_pine_styles`, `get_pine_report`, `list_pine_alerts`, `add_pine_script_to_chart`, `update_pine_instance`, `remove_pine_instance` |

- Workspace ids are persisted tab ids. Reject Portfolio/Script Manager tabs. Timeframe is
  workspace-wide; adding an existing market returns its chart.
- Indicators use `model::BuiltIn` and `ChartWorkspace::apply_indicator`: one instance per chart,
  EMA alone takes one of `EMA_PERIODS`, and Volume is invalid on line charts.
- Alert tools plan on the app thread with `PriceAlertEditorState`, then queue the runtime's
  `Save`/`SetState`/`Delete`. Responses briefly settle to report applied vs queued state. Alert ids
  are strings. Market is immutable; duplicates conflict; paused remains paused; changed conditions
  can re-arm a fired once-alert. Only crossing alerts can currently be created/re-levelled. Webhook
  endpoints are neither exposed nor editable.
- Script-level reads/compile/inputs run at most two workers against `ScriptStore` and the real
  compiler. Saved and draft text stay distinct; compile is pure; paths never leave the module.
- Instance/style/report tools queue to the app because chart run state is private. They return only
  already-computed data; missing runs are explicitly unavailable. Reports include execution context,
  saved-script status, paged trades (max 200), and thinned equity. Script-alert history is not stored.
- Applying a script resolves the saved script on a worker (`mcp::pine::script_to_apply`): the saved
  text, never the draft; libraries and scripts with compile errors are refused; input overrides are
  checked against the compiled `InputSchema` (known id, right kind, min/max, options, source names,
  `#RRGGBB[AA]` colours), and a value equal to the default is no override. The app thread then calls
  `ChartWorkspace::add_pine_instance_to`. Updates check against the instance's own stored source
  (compiled on the app thread, as the settings popover does); `null` resets an input. All three
  push the same undo step as the chart's Scripts tab, only when something changed. The alert host
  copies an instance's inputs into the script alert armed on it and silences the alert when the
  instance goes, so an input change or removal is refused (`CONFLICT`) while an active subscription
  references the instance; visibility is always allowed. They need
  `charts.pine_instances.write`, which no other grant implies.
- Draft update and save have separate permissions. Both compare `expected_revision` to live editor
  text, else draft, else saved source. The app thread atomically checks, writes, and calls
  `ScriptManager::adopt_external_text`. Save rejects compile errors but keeps the draft. Text is
  capped at 200,000 characters and excludes controls other than tab/newline.

The Settings page shows status/port, a global maximum, credential rows, and a shared permissions
sheet with presets. See [remaining work](../mcp-remaining-work.md).
