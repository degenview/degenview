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
- Limits (constants in `mcp/limits.rs` and `mcp/transport.rs`): 64 requests in flight before
  authentication and 16 after; bad tokens share a bucket of 20 refilled at 2/s, past which they get
  429 with `Retry-After` (a valid token is never affected); each client has a bucket of 60 refilled
  at 20/s; a request head may have 64 headers and 16 KiB (431 beyond), a client has 10 s to send it
  (which also closes idle keep-alive connections), and 128 connections may be open (more are closed
  on arrival); the body is capped at 256 KiB. A client that hangs up drops its tool call: queued work
  does not run, work the app already started keeps its outcome. A `notifications/cancelled` cancels the
  same way. Stateless mode serves it as its own POST, so `mcp/cancel.rs` keeps the tool calls in
  flight by client and request id, and a layer reads the notification before rmcp, cancels that
  client's matching calls (all of them, if a client reuses an id) and answers 202 itself. The layer
  sits outside the 16-request cap, so a cancel gets through while the calls it cancels hold every
  permit, and inside the bearer check, so a client can only cancel its own calls. Batches are not
  read.
- Activity log: every `tools/call` leaves one row (`McpAuditEntry`) whichever way it ends. An
  `AuditGuard` in `call_tool` writes it when the call finishes, or as cancelled if the future is
  dropped first, into a bounded in-memory queue on `McpShared`; `McpHost::tick` writes the queue to
  the `mcp_audit` table about once a second in one transaction (and from the app's `on_exit` hook,
  through `McpHost::shutdown`, so a quit loses nothing; a kill still loses the last second) and keeps
  the whole log (at most 1,000 rows, behind an `Arc`) for the Settings page, which never reads
  SQLite itself. The page previews the newest five rows; "View All Activity…" opens a sheet
  (`ui::mcp_activity`) that searches the log by tool, id or error code, filters it by client,
  outcome (`McpActivityStatus`) and date (`McpActivityRange`: presets or a custom range with
  either end open), and pages it 25 rows at a time (`model::mcp_activity`, pure: the filter runs
  over the in-memory rows every frame, so there is no query and no cache to invalidate). The
  table keeps 1,000 rows,
  is local to this Mac (`sync::policy`), and Clear writes one "cleared" row. A row holds the tool,
  client, outcome, an error *code*, duration, the client's request id, and up to four ids from a
  fixed list of argument names, kept only if they look like ids (digits or a hex hash). Never a
  token, a script, a note, a message or any other argument value. Rate-limited calls are counted into
  one row per client per minute; bad tokens and `tools/list` are not rows.
- Confirmation (`mcp::confirm`, `host::mcp_confirmation`, `ui::mcp_confirmation`): with "Ask before
  high-impact requests" on (`McpSettings.require_confirmation`, default on, also for settings saved
  before it existed; published to `McpShared` so the app thread reads it without the host lock),
  these commands wait for the user: delete alert / watchlist / drawings, remove chart or script
  instance, save a script over its saved source (a draft write is free), and paper order place (a dry
  run is free), modify, cancel and close. `confirm::gate` decides, as an exhaustive match, so a new
  `McpCommand` cannot compile until someone says whether it asks. The gate runs on the app thread in
  `App::run_gated`, before dispatch, and never parks a worker: the first call answers
  `CONFIRMATION_REQUIRED` at once (with `confirmation_id`, `expires_in_seconds`, `operation`,
  `next_step`; nothing changed) and opens a question; the user answers in a modal drawn by the
  focused window (tab windows and Settings), with an OS notification if notifications are on; the
  agent repeats the exact same call, which then runs once. An approval is tied to the client and to
  the exact `McpCommand` (`PartialEq`, so any changed argument asks again, and another client's call
  never matches) and is used up by the call that runs it, and only if what it would do is still what the user saw:
  the summary carries a hidden `scope` (the working orders an "all" cancels, the positions it
  closes), the repeat is described again, and a changed effect spends the yes and asks again about
  what it is now. The client is told only the action (`Summary::client_line`: "Cancel paper
  orders"), never the target the app looked up, because a credential that may cancel orders may not
  be allowed to read them (a write grant is not a read grant, as in `hide_unread_account`); the
  user reads the whole summary. A question lives 120 s, an approval 120 s
  after it is given, a "no" 300 s (the agent gets `CONFIRMATION_DENIED`, not another question); a
  client may have 8 open and the app 32 (`RESOURCE_LIMIT` beyond). The summary the user reads is
  built by the app from what it knows of the target (`host::mcp_confirmation::delete_alert` and
  the others; a target that does not exist is refused with its own error, unasked), never from
  agent text, and is cut to one clean line. Revoking a client drops what it had open; turning the setting off, or stopping and starting
  the server (off and on, a new port: `McpShared::generation` moved), drops everything, and the
  questions nobody answered are logged as expired. Answers and expiries are rows in the activity log (`Confirmation` kind: Approved,
  Declined, Expired); the held call itself is a `Held` row. Return alone never approves: a button
  must be chosen, and Escape or a click outside is a No. Turning the setting off is asked about.
- Workers never access workspaces or SQLite. `Request::run` skips abandoned, expired, cancelled,
  pre-stop, or newly unauthorized work; started work reports its real result.
- Settings holds the store lock, so it only queues `McpAction`; `tick_mcp` applies it later.
  Undecodable MCP settings/clients keep the server off without rewriting data.
- Protocol support is rmcp 3.5 / MCP `2025-03-26` through `2026-07-28`, stateless JSON: no
  `Mcp-Session-Id` is issued, `GET /mcp` answers 405 (no server-to-client stream), and `initialize`
  advertises `tools` without `listChanged`. A permission change therefore shows at the client's next
  `tools/list` or call; nothing is pushed. An unknown `MCP-Protocol-Version` header is refused with
  400 (test `an_unknown_protocol_version_header_is_refused`); a missing one is read as `2025-03-26`.
  rmcp also accepts `2024-11-05`; that and `2026-07-28` (which needs per-request metadata) have not
  been tried with a real client.
- Tried with real clients on a release build (2026-11-05): MCP Inspector CLI
  (`tools/list`, a read call), Claude Code (`--mcp-config` with an `http` server: sees only the
  permitted tools, calls one) and `mcp-remote` as the Claude Desktop bridge (`initialize`,
  `tools/list`). All worked without sessions. A second run with a read/write credential added and
  deleted a trend line, added and removed a chart (by curl and by Claude Code), and got
  `PERMISSION_DENIED`, `NOT_FOUND` and argument-error results as `isError: true`. Finding: Inspector's
  `--tool-arg chart_id=999` sends a JSON integer, which the string-typed ids refuse
  (`invalid type: integer`); a client that does the same fails every id argument.

Permissions separate reads and mutations: charts read/create/indicator-write/Pine-instance-write/candles-read/snapshot-read/view-write/delete/replay-write; drawings read/write/delete; alerts
read/create/edit/pause/delete; Pine script list/read/write/save plus diagnostics, inputs, styles,
reports, and alert reads; markets read (symbol search and quotes); watchlists read/write/delete; paper-trading read, place/change, cancel and close; portfolio read.
Delete and saved-source writes require their own grants.

## Tool behavior

| Area | Tools (`degenview_…`) |
| --- | --- |
| Charts | `list_workspaces`, `list_charts`, `add_chart`, `list_indicators`, `add_indicator`, `remove_indicator`, `set_timeframe`, `set_chart_type`, `remove_chart`, `get_replay`, `control_replay`, `get_chart_snapshot` |
| Price alerts | `list_price_alerts`, `create_price_alert`, `update_price_alert`, `pause_price_alert`, `resume_price_alert`, `delete_price_alert` |
| Pine scripts | `list_pine_scripts`, `get_pine_script`, `update_pine_script`, `save_pine_script`, `compile_pine_script`, `get_pine_inputs` |
| Candles and drawings | `get_candles`, `get_indicator_values`, `get_session_stats`, `list_drawings`, `add_drawings`, `update_drawing`, `delete_drawings` |
| Markets | `search_symbols`, `get_quotes`, `get_market_candles` |
| Watchlists | `list_watchlists`, `get_watchlist`, `create_watchlist`, `update_watchlist`, `delete_watchlist` |
| Paper trading | `list_paper_accounts`, `get_paper_account`, `list_paper_fills`, `list_paper_trades`, `place_paper_order`, `modify_paper_order`, `cancel_paper_orders`, `close_paper_positions` |
| Portfolio (read-only) | `list_portfolios`, `get_portfolio_holdings`, `list_portfolio_transactions` |
| Pine instances | `list_chart_pine_instances`, `get_pine_instance`, `get_pine_styles`, `get_pine_report`, `list_pine_alerts`, `add_pine_script_to_chart`, `update_pine_instance`, `remove_pine_instance` |

- Workspace ids are persisted tab ids. Reject Portfolio/Pine Studio tabs. Timeframe is
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
- `get_candles` returns the market's own candles, never a constructed Renko/Heikin Ashi series,
  cut at the replay position as the chart is, so the bars are what drawings anchor to. A row is
  `[time, open, high, low, close, volume]` under named `columns`; times are ISO-8601 UTC to the
  second and prices plain JSON numbers (so a tiny DEX price is `2.78e-7`, not a subscript). Pages
  are the newest `limit` bars (max 1000), paged back with `before`; volume is `null` for a
  provider that reports none. `mcp::chart_data` builds it from plain values, with no workspace.
- Drawing tools plan on the app thread (`mcp::chart_data::plan_add/plan_edit/plan_remove`) from
  `ChartWorkspace::chart_data`, then commit with `apply_planned_drawings` → `apply_drawing_commands`,
  so a batch is one undo step. Points are `{time, price}` or `{time, price_from}`; a time inside the
  candles snaps to its bar's open (the pointer magnet's rule), one outside is kept, `price_from`
  reads that bar. Arguments are checked on the tool thread (`mcp::drawings`); everything is all or
  none. Ids come from `next_free_id` (the counter, but never one already stored). A locked drawing
  is never edited or removed, a chart holds at most 500 drawings, and a call carries at most 25
  (add) or 50 (delete). Writable tools are `McpDrawingTool::ALL`: trend line, rectangle, ray, extended
  line, arrow and Fibonacci retracement (two points) and horizontal line, horizontal ray, vertical line,
  crossline and text (one point, no `b`). A tool takes only the points and styles it has
  (`McpDrawingTool::points`/`check_patch`): fill is for rectangles, text for rectangles and labels
  (a label needs it), extension flags for trend lines and rectangles. Every other tool is listed
  read-only and deletable. The Fibonacci retracement is drawn with the app's default levels.
- Portfolio tools (`mcp::portfolio`, `mcp::portfolio_tools`) are read-only. Holdings come from
  `model::portfolio_accounting::holdings_and_closed` on the app thread (the app's own weighted-average
  accounting, so a sale realizes profit against the average cost and a position sold to nothing still
  counts in `closed_positions_realized_pnl`). They are then valued on the tool thread from the quote
  feed with `wait_for_quotes`, the same lease as `get_quotes`. That accounting is unit-naive: cost
  is in the currency the transactions were priced in. The Portfolio tab converts with an FX service;
  this read does not, so a holding bought in a currency other than its market quotes in gets price
  and value but no profit, return or allocation (`currency_mismatch`) and is left out of `totals`,
  and totals are per currency, never summed across them. Privacy mode only hides amounts on screen;
  a client with `portfolio.read` gets the figures (and `list_portfolios` reports the mode).
- Paper-trading tools (`mcp::paper`, `mcp::paper_tools`) read and write the app's `PaperHost` on the
  app thread. Reads: accounts with the engine's own metrics, positions marked the way the panel marks
  them (bid for a long, ask for a short, last as fallback; no quote → null mark and no profit
  figure), orders that can still fill, fills and closed trades newest first. Writes go through the
  host's own `submit`, `modify`, `cancel` and `close_position`, so the engine validates (margin,
  increments, stale or missing prices) and persists exactly as the order ticket does, and the
  integrity check runs on a candidate copy first. **Simulated money only: nothing reaches a broker,
  and Alpaca stays data-only.** Safeguards, because an agent can trade here with no step the user
  sees: three off-by-default permissions (`paper.orders.place`, which also covers changing an
  order, `paper.orders.cancel`, `paper.positions.close`); an account another Mac runs
  (`PaperHost::owned_elsewhere`) is never changed over MCP, since acting on it from the GUI takes it
  over; `place_paper_order` takes `dry_run` (checked on a throwaway engine copy, nothing recorded,
  not even a refused order) and `request_id` (`OrderMemory`, in memory, 512 entries: a retried call
  returns the order already placed instead of a second one); quantities and prices must be finite,
  positive and under 1e15; order type decides which of limit and stop price is required or
  forbidden. Orders fill from the prices of open charts (`sync_paper_quotes`), so a market with no
  chart is refused for lack of a price, and a working order fills only while the app runs; replay
  does not hold orders back. With confirmation on, each write waits for the user's approval (next
  bullet).
- Chart control (`mcp::chart_control`) calls the workspace's own methods: `set_timeframe` is the
  toolbar's timeframe change (`ChartWorkspace::reload_for_timeframe`, shared with the toolbar) and
  moves every chart in the workspace, since the timeframe belongs to the workspace; it reloads, so
  charts read `loading` for a moment. `set_chart_type` uses `ChartCard::set_chart_type` after
  `ChartType::unsupported_reason` (a line-drawn market, volume candles without volume → `UNSUPPORTED`)
  and names types by `ChartType::slug`. `remove_chart` is the Remove button's `remove_chart`, so the
  chart's drawings go with it; it has its own permission (`charts.delete`) and is refused for a
  portfolio card. The first two need `charts.view.write`. All three answer `unchanged` where they can,
  so a retry is harmless (not `remove_chart`: the second call is `NOT_FOUND`).
- `get_chart_snapshot` (`mcp::snapshot_tools`, `charts.snapshot.read`) returns the chart as the app's
  camera would draw it, as an MCP image block (PNG, base64) plus a text block with its size. It is the
  same renderer (`ChartCard::snapshot_scene`, frozen on the UI thread in one frame by
  `ChartWorkspace::capture_scene`, then rasterized and encoded on the snapshot worker), so it holds
  what the chart shows and the camera menu would save: replay cut-off, indicators, scripts, drawings,
  alert lines, and orders and positions **only if** Settings includes them in snapshots (the same
  privacy choice; the tool does not override it). Nothing is saved, copied or uploaded and no notice
  is shown. The bridge never waits on a frame: `BeginSnapshot` starts a job (`SnapshotJobs`, at most
  four, forgotten after 120 s unless collected) and the tool polls `PollSnapshot` every 100 ms for up
  to 45 s. Default 1x (smaller to send), 2x on request; a chart too large for the scale is drawn
  smaller. The chart must have been drawn on screen once, so a window that has never shown it has no
  size and answers `CONFLICT`. Base64 is a small local encoder (`snapshot_tools::base64`, tested against
  the RFC 4648 vectors) so no dependency is added.
- Replay (`mcp::replay_tools`) drives the workspace's one `ReplayEngine`, the clock behind the
  replay bar, through `ChartWorkspace::replay_command` (`start` on a chart's bars, then
  `model::ClockCommand`s: play, pause, step, restart, seek by time, speed, stop). Replay is
  workspace-wide and runs on one chart's candles, so `start` focuses the chart it names. While it
  runs `chart_data` cuts every chart's candles at the position, so `get_candles` and
  `get_indicator_values` show the past exactly as the chart does; this is how an agent walks a
  market bar by bar without seeing the future. `get_replay` needs `charts.read`, `control_replay`
  `charts.replay.write`. Stepping is deterministic (up to 500 bars); `play` runs in real time at
  the chosen speed. Each action accepts only its own arguments. Paper orders still fill at live
  prices during replay (the unresolved question in `docs/agent-guides/market-data.md`), which the
  tool description says.
- `get_indicator_values` (`mcp::indicator_tools`, pure part in `mcp::indicator_values`) computes RSI 14,
  an EMA of any length 2 to 400, Bollinger (20, 2) and Supertrend flips with `model::indicators`,
  the functions the chart draws with, over every candle the chart holds (the market's own, cut at
  the replay position), then returns the newest `limit` bars as columns aligned with `times` and
  `closes`. Values are warm at the first returned bar; `null` is warm-up (or, for `trend_flip`, no
  flip, and the newest bar's flip is never reported). It needs `charts.candles.read` and the
  indicator need not be applied. `get_session_stats` (`mcp::session_stats_tools`, result in
  `mcp::session_stats`, counting in `model::session_stats`; see [Session statistics](../session-stats.md))
  counts an outcome over the days of the same candles with its sample size and a 95% interval; it
  is a read under `charts.candles.read`, refuses bars longer than a day, and never reads a
  constructed type's bars. `remove_indicator` is `ChartWorkspace::remove_indicator`
  (`charts.indicators.write`); removing one that is not there answers `not_applied`.
- Markets and watchlists live in `mcp::market_tools` (a second `tool_router`, merged in
  `DegenViewMcp::new`) and `mcp::markets` (result types and pure projections). `search_symbols`
  runs `service::search` for each named provider side by side on a blocking worker (at most four
  searches at once, 20 s ceiling), keeps only hits of the provider asked, and reports a failing
  provider in `source_errors` while the others answer; the search function is
  `McpShared::market_search` so tests never reach the network. `get_quotes` needs no app thread:
  `McpShared::lease_quotes` puts a demand on the app's own `WatchlistQuoteCoordinator` under a
  fresh owner id (so concurrent calls never replace each other's demand), polls the
  `WatchlistQuoteBook` every 100 ms for up to `wait_seconds` (default 5, max 15), and releases the
  demand when the lease drops, including when the call is cancelled. The app hands the feed over
  once at startup (`McpShared::attach_quotes`); without it quotes answer `APP_UNAVAILABLE`. A quote
  is settled when it is priced or carries a reason (`note`, `unsupported`); `pending` counts the
  rest. Quote status and change basis are the watchlist's (`Freshness`, `ChangeBasis`): a stock
  outside US hours reads `market_closed`, and Alpaca's change is since the previous close.
- `get_market_candles` reads candles for any market with no chart: it calls `service::fetch_klines`
  (the same provider calls and caches a chart uses, via `McpShared::candle_fetch` so tests never
  reach the network) on the provider's own `work_queue` lane, so a rate-limited provider queues as it
  does for a chart. At most three run at once and the call waits 30 s. It returns the newest
  `limit` bars (max 1000) with a summary, through the same `candles_result` as `get_candles`.
  There is no `before`: older history needs a chart (`get_candles`). It needs `markets.read`.
- Watchlist reads and writes run on the app thread against the shared `WatchlistStore`, the same
  path the sidebar uses (so iCloud Sync queues the write in the same transaction).
  `update_watchlist` is `WatchlistStore::edit_list(id, ListChanges)`: rename, new sections, renamed
  sections, removals, moves, additions, deleted sections, in that order, on one copy with one save;
  a refusal leaves the list as it was, and a market already present or absent is skipped and
  reported, so a retry is harmless. A section is named by id or by title (`Watchlist::resolve_section`,
  case ignored, two sections with one title are `INVALID_ARGUMENT` until named by id), and one made
  in the same call can be used by the moves and additions after it. Deleting a section removes the
  header only; its markets stay. Symbols are checked with the import rules
  (`watchlist_text::market_item`), so `BTC/USDT` is refused where the Add Chart sheet would also
  want `BTCUSDT`. Favorites cannot be deleted. `get_watchlist` pages entries (max 500) and, with
  `include_quotes`, needs `markets.read` as well and at most 50 markets per page. Display settings
  (sort, columns, collapsed sections) and flags are not exposed.
- Draft update and save have separate permissions. Both compare `expected_revision` to live editor
  text, else draft, else saved source. The app thread atomically checks, writes, and calls
  `PineStudio::adopt_external_text`. Save rejects compile errors but keeps the draft. Text is
  capped at 200,000 characters and excludes controls other than tab/newline.

The Settings page shows status/port, a global maximum, credential rows, and a shared permissions
sheet with presets. See [remaining work](../mcp-remaining-work.md).
