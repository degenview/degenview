# MCP server — remaining work

Status of the local MCP server (`src/mcp/`, `src/mcp_host.rs`; design in
[architecture/mcp.md](architecture/mcp.md)) and what is still planned. Ordered by what I would do
next. Nothing here is started.

## Done so far

| Phase | What | Permissions |
|---|---|---|
| 1 | Server, loopback transport, bearer credentials, Settings page, workspaces and charts (`list_workspaces`, `list_charts`, `add_chart`) | `charts.read`, `charts.create` |
| 2 | Built-in indicators (`list_indicators`, `add_indicator`) | `charts.indicators.write` |
| 3 | Price alerts (`list`, `create`, `update`, `pause`, `resume`, `delete`) | `alerts.read/create/edit/pause/delete` |
| 4 | Read-only Pine: scripts, source, diagnostics, inputs, applied instances, styles, strategy reports, script-alert subscriptions | `pine.scripts.list/read`, `pine.diagnostics.read`, `pine.inputs.read`, `pine.styles.read`, `pine.reports.read`, `pine.alerts.read` |
| 5 | Pine source editing: draft update and explicit save, revision-checked against the open editor | `pine.scripts.write`, `pine.scripts.save` |
| 6 | Pine on charts: apply a saved script (with input overrides), change an applied script's inputs and visibility, remove it | `charts.pine_instances.write` |
| 7 | Candles and drawings: read a chart's candles, list drawings, add/move/restyle/delete trend lines and rectangles | `charts.candles.read`, `drawings.read`, `drawings.write`, `drawings.delete` |

## 1. Try it with real MCP clients

Only curl and the loopback test suite have been used so far.

- Connect MCP Inspector, Claude Desktop (via a bridge such as `mcp-remote`) and Claude Code, using a
  credential created in Settings and the "Copy Connection Details" JSON.
- Confirm the server's stateless mode (`legacy_session_mode = false`, JSON responses) works for each
  client; older clients may expect an `Mcp-Session-Id`. If one does, decide between enabling sessions
  for legacy protocol versions or documenting the client as unsupported.
- Pin and document the supported protocol revisions (rmcp 3.5 advertises `2025-03-26` through
  `2026-07-28`) and add a test that an unsupported `MCP-Protocol-Version` is refused.
- Check that `tools/list` filtering and `isError` tool results render sensibly in each client, and
  fix tool descriptions that confuse models.
- Send `notifications/tools/list_changed` when a client's effective permissions change while it is
  connected (stateless mode has no push channel; if that is the case, say so in the docs).

## 2. Audit log and recent-activity view

Required before wider use; Pine source edits and alert deletion are now possible remotely.

- Record per call: time, client id and name, tool, allowed or denied, success or error code, duration,
  request correlation id, and target ids (workspace, chart, alert, script). Never the token, Pine
  source, webhook data or full arguments.
- Store in SQLite (a bounded document table, e.g. 1,000 rows, oldest pruned) through `store::mcp`;
  write from the app thread or a short-lock helper, never from the HTTP path while holding a lock.
- Settings: a "Recent activity" list on the MCP Server page and a Clear button.
- Tests: denied and failed calls are recorded, the log is bounded, no secret or source text reaches it.

## 3. Confirmation policy for high-impact actions

Today these are gated only by separate, off-by-default permissions.

- Candidates: delete alert, save Pine source, apply a script to a chart (once it exists), enable a
  script alert.
- An in-app confirmation tied to the client and the exact operation, with a short expiry. The tool
  returns promptly with a typed "confirmation required / pending" result (or waits a bounded time) so
  an MCP worker is never parked on user input; a timeout or dismissal means nothing ran.
- Setting: "Require confirmation for destructive actions" (default on).

## 4. Applying scripts to charts: what is left

Applying, changing inputs/visibility and removing are done (phase 6). They sit behind the
permission alone: no confirmation step (item 3 would add one for all high-impact tools at once).

- Not exposed: style overrides, visual order, pane collapse and reordering. Add them if an agent
  needs them; `ChartCard::set_pine_instance_style` and `set_pine_visual_order` exist.
- An instance keeps the source it was applied with, so a later `save_pine_script` does not change it.
  `get_pine_instance` reports `script_status: changed`; the way to run the new version is remove and
  apply again. A "refresh from saved script" call is possible if that proves clumsy.
- Changing inputs or removing an instance is refused while a script alert is armed on it (the alert
  host would retarget or silence the alert, and alert changes wait for item 3). Lift this once script
  alerts can be managed over MCP.
- Applying does not run a strategy live or touch alert subscriptions; the script runs when its chart
  draws it, so a new instance reads `not_run` until then.
- Layout autosave's dirty check ignores Pine changes (`LayoutSnapshot` has no script fields), so a
  script applied over MCP reaches a saved layout at the next explicit or autosave write, as for a
  script added in the GUI.

## 4b. Candles and drawings: what is left

Reading candles and drawing trend lines and rectangles are done (phase 7).

- Only trend lines and rectangles can be created or edited. Fib, arrows, text, brushes, circles and
  the rest are listed read-only (`editable: false`), including the app's horizontal and vertical
  lines, rays, channels, regression trend and pitchforks (they can be deleted over MCP). Creating
  them needs an argument shape with one to four anchors and per-tool settings; until then a
  horizontal level made over MCP is a trend line with equal prices.
- `degenview_get_candles` returns what the chart holds (about 500 bars, 1500 after one older page).
  DegenView fetches older history only as the chart is scrolled back, so an agent cannot ask for
  more; `has_older_loaded` and `older_history_available` say what is possible.
- No indicator values (EMA, RSI, Bollinger) alongside the candles, and no swing/pivot helper: the
  agent works those out from the bars.
- The caller's drawing defaults (templates) are not used: new drawings take the tool's built-in
  default, so a result does not depend on the person's settings.
- Drawings reach saved layouts the way any drawing edit does; a manual layout is not marked
  unsaved by an agent's drawing.

## 5. Script alert subscriptions and a log of what they fired

- DegenView stores no history of fired script alerts (`pine_alert_event` keeps only a dedupe key), so
  `degenview_list_pine_alerts` returns subscriptions only. Persisting fired events (script, time,
  message, delivery state) is an app feature first; then expose it read-only.
- Creating, pausing or deleting subscriptions over MCP waits for the confirmation work (item 3).

## 6. On-demand backtests

`pine_host::run_script_checked` can run a script headlessly; today reports come only from a chart's
own run, and a chart that has not drawn the script has none.

- Run on a worker with the existing limits plus: max bars, wall-clock deadline (already 10 s per run),
  at most one or two concurrent runs, cancellation on server stop, bounded result size and trade
  paging (already in the report tool).
- Must not touch paper trading, webhooks, alerts or saved scripts. Report the context (script hash,
  instrument, timeframe, inputs, range, bar count, timestamp) and mark it `on_demand`.
- Needs candle fetches (`service::fetch_klines`), which are network calls: bound and time-limit them.

## 7. Price alerts: percentage conditions

`rises_by` / `falls_by` need a live reference price. Offer them once a quote can be awaited with a
bounded wait (demand the quote, wait, create, release the demand), or require the caller to pass an
explicit reference price. Existing percentage alerts are already listed and accept note and
frequency edits.

## 8. MCP resources

Read-only resources mirroring the read tools (`degenview://workspaces`, `.../charts`,
`degenview://pine/scripts/{id}` with `/diagnostics`, `/inputs`), with stable URIs and MIME types and
the same permission checks as the equivalent tools. Resources must not become an authorization
bypass.

## 9. Credentials and Settings

- Rename and rotate a credential (new token, same permissions, old token revoked at once).
- Show each client's effective permissions (grants ∩ global cap) in the list.
- Optional expiry for a credential.
- Consider whether to keep the verifier in SQLite or move it to Keychain; the current design stores
  only a SHA-256 verifier, which is not a reusable secret.

## 10. Hardening

- Per-client and failed-authentication rate limiting (today: a concurrency cap and 256-bit tokens).
- JSON nesting depth limit; confirm rmcp's body limit and header limits are enough.
- IPv6 loopback (`::1`) with the same Host/Origin checks, if wanted.
- Timeouts and cancellation: map MCP cancellation notifications onto the bridge's cancel switch.
- Behaviour when the app is quitting or a window closes mid-request (covered for stop and revoke, not
  for every window event).

## 11. Known limitations to keep visible

- A freshly drawn chart is needed for styles and reports of applied scripts (`not_run` until then).
- Draft reads can lag the open editor by about a second (the editor writes drafts after a 1 s pause);
  edits are checked against the live text, so this is a staleness, not a data-loss, risk.
- `ScriptStore` scans have housekeeping side effects (metadata for unseen files), as when the Script
  Manager opens.
- Creating a price alert goes through the sheet's Save path, which may prompt for notification
  permission.
- Alerts, script alerts and strategies run only while DegenView does.
- A debug build is far too slow to use (about 5x the CPU of release); use `--release` for manual checks.

## 12. Cleanup

- `src/mcp_host.rs` has grown to ~2,500 lines (host, chart/alert/Pine projections, tests). Split the
  projections into `mcp/projection/{charts,alerts,pine}.rs` and keep the host to lifecycle and
  credentials.
- `mcp/tools.rs` (~1,500 lines) could split per domain the same way.
- Add the 2026-07-28-style `server/discover` check to the integration tests if a client uses it.

## 13. Spec items still to verify before calling it complete

From the design brief's test and invariant lists: SQLite lock contention, UI/window closure during a
request, app shutdown during a backtest (once backtests exist), a real-client end-to-end run of the
acceptance scenario, and the final implementation report (architecture, protocol revision and SDK,
permission matrix, tool catalog, limitations, tests run, example client configuration).
