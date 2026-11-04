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

Permissions separate reads and mutations: charts read/create/indicator-write/Pine-instance-write/candles-read/view-write/delete; drawings read/write/delete; alerts
read/create/edit/pause/delete; Pine script list/read/write/save plus diagnostics, inputs, styles,
reports, and alert reads; markets read (symbol search and quotes); watchlists read/write/delete.
Delete and saved-source writes require their own grants.

## Tool behavior

| Area | Tools (`degenview_…`) |
| --- | --- |
| Charts | `list_workspaces`, `list_charts`, `add_chart`, `list_indicators`, `add_indicator`, `remove_indicator`, `set_timeframe`, `set_chart_type`, `remove_chart` |
| Price alerts | `list_price_alerts`, `create_price_alert`, `update_price_alert`, `pause_price_alert`, `resume_price_alert`, `delete_price_alert` |
| Pine scripts | `list_pine_scripts`, `get_pine_script`, `update_pine_script`, `save_pine_script`, `compile_pine_script`, `get_pine_inputs` |
| Candles and drawings | `get_candles`, `get_indicator_values`, `list_drawings`, `add_drawings`, `update_drawing`, `delete_drawings` |
| Markets | `search_symbols`, `get_quotes` |
| Watchlists | `list_watchlists`, `get_watchlist`, `create_watchlist`, `update_watchlist`, `delete_watchlist` |
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
  (add) or 50 (delete). Only trend lines and rectangles are writable.
- Chart control (`mcp::chart_control`) calls the workspace's own methods: `set_timeframe` is the
  toolbar's timeframe change (`ChartWorkspace::reload_for_timeframe`, shared with the toolbar) and
  moves every chart in the workspace, since the timeframe belongs to the workspace; it reloads, so
  charts read `loading` for a moment. `set_chart_type` uses `ChartCard::set_chart_type` after
  `ChartType::unsupported_reason` (a line-drawn market, volume candles without volume → `UNSUPPORTED`)
  and names types by `ChartType::slug`. `remove_chart` is the Remove button's `remove_chart`, so the
  chart's drawings go with it; it has its own permission (`charts.delete`) and is refused for a
  portfolio card. The first two need `charts.view.write`. All three answer `unchanged` where they can,
  so a retry is harmless (not `remove_chart`: the second call is `NOT_FOUND`).
- `get_indicator_values` (`mcp::indicator_tools`, pure part in `mcp::indicator_values`) computes RSI 14,
  an EMA of any length 2 to 400, Bollinger (20, 2) and Supertrend flips with `model::indicators`,
  the functions the chart draws with, over every candle the chart holds (the market's own, cut at
  the replay position), then returns the newest `limit` bars as columns aligned with `times` and
  `closes`. Values are warm at the first returned bar; `null` is warm-up (or, for `trend_flip`, no
  flip, and the newest bar's flip is never reported). It needs `charts.candles.read` and the
  indicator need not be applied. `remove_indicator` is `ChartWorkspace::remove_indicator`
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
- Watchlist reads and writes run on the app thread against the shared `WatchlistStore`, the same
  path the sidebar uses (so iCloud Sync queues the write in the same transaction).
  `update_watchlist` is `WatchlistStore::edit_list`: rename, removals then additions, one copy, one
  save; a refusal leaves the list as it was, and a market already present or absent is skipped and
  reported, so a retry is harmless. Symbols are checked with the import rules
  (`watchlist_text::market_item`), so `BTC/USDT` is refused where the Add Chart sheet would also
  want `BTCUSDT`. Favorites cannot be deleted. `get_watchlist` pages entries (max 500) and, with
  `include_quotes`, needs `markets.read` as well and at most 50 markets per page. Sections can be
  read but not yet created or edited, and neither can display settings (sort, columns) or flags.
- Draft update and save have separate permissions. Both compare `expected_revision` to live editor
  text, else draft, else saved source. The app thread atomically checks, writes, and calls
  `PineStudio::adopt_external_text`. Save rejects compile errors but keeps the draft. Text is
  capped at 200,000 characters and excludes controls other than tab/newline.

The Settings page shows status/port, a global maximum, credential rows, and a shared permissions
sheet with presets. See [remaining work](../mcp-remaining-work.md).
