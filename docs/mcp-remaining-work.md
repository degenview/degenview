# MCP server — remaining work

Status of the local MCP server (`src/mcp/`, `src/host/mcp_host.rs`; design in
[architecture/mcp.md](architecture/mcp.md)) and what is still planned. Ordered by what I would do
next. Items below are not started unless they say otherwise.

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
| 8 | Markets and watchlists: search symbols, read quotes, list/read watchlists (with quotes), create, rename, add/remove markets, delete | `markets.read`, `watchlists.read`, `watchlists.write`, `watchlists.delete` |
| 9 | Indicator values and removal: RSI, EMA, Bollinger and trend-flip series for a chart, and take a built-in indicator off | `charts.candles.read`, `charts.indicators.write` |
| 10 | Chart control: set a workspace's timeframe, a chart's type, and remove a chart | `charts.view.write`, `charts.delete` |
| 11 | Paper trading, read-only: accounts and metrics, positions, working orders, fills, closed trades | `paper.read` |
| 12 | Portfolio, read-only: portfolios, holdings valued at live prices (per currency), transactions | `portfolio.read` |
| 13 | Candles of any market by symbol and timeframe, no chart needed | `markets.read` |
| 14 | Watchlist sections: add, rename, delete, and put or move markets into them | `watchlists.write` |
| 15 | Replay: read the workspace's replay and start, step, seek, play, pause, set speed and stop it | `charts.read`, `charts.replay.write` |
| 16 | Paper trading writes: place (dry run, request id), modify, cancel, close positions; never on an account another Mac runs | `paper.orders.place`, `paper.orders.cancel`, `paper.positions.close` |
| 17 | More drawing tools: rays, extended lines, arrows, Fibonacci retracement, horizontal / vertical lines, crosslines, text | `drawings.write` |
| 18 | Chart snapshot: a PNG of a chart as the camera draws it, returned as an image | `charts.snapshot.read` |
| 20 | Approval step: deletes, saving over a script and paper orders wait for the user in the app; `CONFIRMATION_REQUIRED` / `CONFIRMATION_DENIED`, repeat the same call | none (Settings only) |
| 19 | Activity log: a row per tool call (who, what, how it ended, how long), listed and cleared in Settings, local to this Mac | none (Settings only) |

## 0. What a trader still cannot do over MCP

Ranked by how soon someone would ask. Not started unless listed under "Done so far".

- Chart control still missing: change a chart's symbol (remove and add instead), price scale mode,
  scroll to a time, focus a chart, layouts. Timeframe, chart type and removal are done.
- Indicators: hide and show one without removing it (`degenview_remove_indicator` and
  `degenview_get_indicator_values` are done); the chart's EMA period is changed through
  `degenview_add_indicator`.
- Paper trading: reverse a position, close at a limit, edit an account's settings, create or reset
  accounts. Orders and closes are done, behind three off-by-default permissions and the approval
  step (item 3). No real-broker order tool: Alpaca is data only.
- Portfolio: cross-currency totals (needs the FX service the Portfolio tab uses), value history and statistics. Snapshots: hosted sharing (copy link, tweet) is not exposed and will not be.
- Drawing tools beyond the eleven (item 4b); swing, support/resistance and
  volatility helpers; older history for `get_market_candles` (needs a time-range fetch).
- Watchlist sorting, columns, collapsed sections and flags (not exposed), and trending symbols.
- On-demand backtests and a watchlist screener (item 6); percentage alerts (item 7); layouts.

## 1. Real MCP clients: what is left

Done (2026-11-05, release build; see [architecture/mcp.md](architecture/mcp.md)): MCP Inspector
CLI, Claude Code and `mcp-remote` connect in stateless mode, list only the permitted tools and call
read tools; with a read/write credential, drawings and charts were added and removed (curl and
Claude Code) and denied, not-found and bad-argument calls came back as `isError`. An unsupported
`MCP-Protocol-Version` is refused (tested). `list_changed` is not advertised and cannot be pushed in
stateless mode.

- Ids are string-typed; a client that sends `46` as an integer is refused. Decide whether to accept
  integers for `*_id` arguments (Inspector's CLI does this).
- Run Claude Desktop itself rather than only the bridge.
- Decide on `2024-11-05` (rmcp accepts it) and try `2026-07-28`; then say in the docs which are
  supported.
- Review tool descriptions with a weaker model driving a longer task; the one short run showed no
  confusion.

## 2. Audit log: what is left

Done (phase 19): every tool call leaves a row in a bounded SQLite table, shown under Settings ▸ MCP
Server ▸ Recent activity (the newest five rows) and its "View All Activity…" sheet, which searches,
filters by client, outcome and date, pages the log and clears it. See
[architecture/mcp.md](architecture/mcp.md).

- Only tool calls are recorded: `initialize` and `tools/list` are not, and a bad token is never a
  row (a flood would push real history out). A client over its rate limit is one row a minute with
  a count.
- Agents cannot read or clear the log over MCP. If an agent should be able to read its own history,
  that needs its own permission.
- No export. Add one if people want the log outside the app (it keeps 1,000 rows).
- The row is written when the call ends, so a call still running when the app is killed (not
  quit) leaves nothing.

## 3. Confirmation: what is left

Done (phase 20): see [architecture/mcp.md](architecture/mcp.md). Deletes, saving over a script and
paper orders wait for the user's approval in the app; the setting is on by default.

- Applying a script to a chart, changing its inputs, and the drawing and chart edits are not gated
  (they are easy to see and undo in the app). Add a command to `confirm::gate` if that changes.
- Approval is "Allow once" for the exact call. There is no "allow this client for ten minutes" or
  per-client trust; an agent doing twenty deletes asks twenty times.
- The question shows in the tab windows and Settings, not in the Alerts window or Pine Studio
  windows; the notification covers a user who is elsewhere, and a question waits two minutes.
- Enabling, pausing or deleting script-alert subscriptions has no MCP tool yet; when it does it
  should be gated (item 5).

## 4. Applying scripts to charts: what is left

Applying, changing inputs/visibility and removing are done (phase 6). They sit behind the
permission; removing an instance asks the user (item 3), applying and changing inputs do not.

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

Reading candles and drawing are done (phase 7, widened by the drawing-tools PR).

- Created and edited over MCP: trend lines, rectangles, rays, extended lines, arrows, Fibonacci
  retracements, horizontal lines and rays, vertical lines, crosslines and text labels (one point, or
  two). The rest are listed read-only (`editable: false`) and deletable: the other Fibonacci and Gann
  tools, brushes, circles and the other shapes, channels, regression trend, pitchforks, and the
  note, price note, pin, table, callout, comment, price label, signpost, flag mark, image and post
  tools. Those need an argument shape with three or four anchors and per-tool settings (levels,
  angles, content). The retracement uses the app's default levels, which cannot be set. A picture's
  bytes and a card's preview are not exposed; a table's cells and a pinned table's place are not in
  `anchors` (it has none).
- `degenview_get_candles` returns what the chart holds (about 500 bars, 1500 after one older page).
  DegenView fetches older history only as the chart is scrolled back, so an agent cannot ask for
  more; `has_older_loaded` and `older_history_available` say what is possible.
- Indicator values come from `degenview_get_indicator_values` (phase 9); there is no swing/pivot
  helper, so the agent works those out from the bars.
- The caller's drawing defaults (templates) are not used: new drawings take the tool's built-in
  default, so a result does not depend on the person's settings.
- Drawings reach saved layouts the way any drawing edit does; a manual layout is not marked
  unsaved by an agent's drawing.

## 5. Script alert subscriptions and a log of what they fired

- DegenView stores no history of fired script alerts (`pine_alert_event` keeps only a dedupe key), so
  `degenview_list_pine_alerts` returns subscriptions only. Persisting fired events (script, time,
  message, delivery state) is an app feature first; then expose it read-only.
- Creating, pausing or deleting subscriptions over MCP has no tool yet; add them behind the
  confirmation step (item 3).

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

## Remote access

Step 1 (transport) is done: `BindMode`, TLS 1.3 with a self-signed pinned certificate, the `Host` list
from the bind, no non-loopback bind without TLS. See "Remote access: what exists" in
[mcp-implementation.md](mcp-implementation.md). Not started, in this order:

2. Per-client scope: `remote_allowed` (default off, so every existing client stays loopback-only), an
   optional CIDR allowlist checked from the peer address before the bearer check, and a remote
   permission ceiling (default read-only) intersected with the client's grants.
3. Audit log fields for the peer address and a `remote` flag; the approval step always on for a remote
   client's write tools; failed-auth limits keyed by peer address (today keyed globally).
4. Settings: interface picker, certificate fingerprint with a copy button, the per-client switch, the
   ceiling; user-entered hostnames for the `Host` list (MagicDNS names); docs.

Until step 4 a non-loopback bind can only be set by editing the store, which the seal then rejects.
Check first that a real client accepts a pinned self-signed certificate; if none does, a certificate
from Tailscale or Let's Encrypt (import) is the fallback, and the generated one is not worth polishing.

## 10. Hardening

Done: failed-authentication and per-client rate limits, header and connection limits, and
cancellation on disconnect (see [architecture/mcp.md](architecture/mcp.md)). Left:

- IPv6 loopback (`::1`) with the same Host/Origin checks, if wanted.
- The limits are constants. Make them settings only if someone needs different numbers.
- A script compile that has already started is not interrupted by a cancel; it finishes and frees
  its permit. Stopping it part-way needs a cancel hook inside the Pine compiler. Other
  `spawn_blocking` jobs (provider search, candle fetch) are not aborted when still queued either;
  their permit and deadline bound them.
- `notifications/cancelled` is honoured for a single message, not inside a JSON-RPC batch (a
  batch is passed on to rmcp, which cannot match it). No real client has been seen sending one.
- Real clients have been used to check that normal calls still work, but not that a client closing
  its stream mid-call drops the call; the loopback test does.
- Behaviour when the app is quitting or a window closes mid-request (covered for stop and revoke, not
  for every window event).

## 11. Known limitations to keep visible

- A freshly drawn chart is needed for styles and reports of applied scripts (`not_run` until then).
- Draft reads can lag the open editor by about a second (the editor writes drafts after a 1 s pause);
  edits are checked against the live text, so this is a staleness, not a data-loss, risk.
- `ScriptStore` scans have housekeeping side effects (metadata for unseen files), as when the Pine
  Studio opens.
- Creating a price alert goes through the sheet's Save path, which may prompt for notification
  permission.
- Alerts, script alerts and strategies run only while DegenView does.
- A debug build is far too slow to use (about 5x the CPU of release); use `--release` for manual checks.

## 12. Cleanup

- `src/host/mcp_host.rs` has grown to ~2,500 lines (host, chart/alert/Pine projections, tests). Split the
  projections into `mcp/projection/{charts,alerts,pine}.rs` and keep the host to lifecycle and
  credentials.
- `mcp/tools.rs` (~1,500 lines) could split per domain the same way.
- Add the 2026-07-28-style `server/discover` check to the integration tests if a client uses it.

## 13. Spec items still to verify before calling it complete

From the design brief's test and invariant lists: SQLite lock contention, UI/window closure during a
request, app shutdown during a backtest (once backtests exist), a real-client end-to-end run of the
acceptance scenario, and the final implementation report (architecture, protocol revision and SDK,
permission matrix, tool catalog, limitations, tests run, example client configuration).

## 14. Open items from the production-readiness review

[The review](reviews/mcp-server-code-review.md) has the status of each finding. Still open:
compile work for Pine saves and input changes
still runs on the app thread (a burst of permitted calls can stall a frame: prepare off-thread, then
recheck the revision and commit); a draft with no open editor can still overwrite a later outside edit
of the file (`based_on_revision_id` is never recorded); passing the permission to bridge calls from the
command instead of each handler; resources or an explicit tools-only statement (item 8); and a
real-client acceptance run.

## Sealed credentials: left over

- Re-verify the seal while the server runs (today only at launch), for example when the file's
  modification time moves, and drop to "off" with the same banner.
- A monotonic counter in the seal, kept in the Keychain, so replaying an older database with its
  own seal is detected.
- Windows has no secret store behind `KeyBackend` (every call answers "none"), so the key cannot be
  created and the server runs unsealed there, with a notice on the MCP Settings page. Sealing
  needs a Windows credential store first.
