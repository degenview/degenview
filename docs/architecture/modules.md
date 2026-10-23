# Modules

The important boundary is ownership, not a complete file list. Use `rg --files src` for the current
inventory.

## Coordinators

| Module | Owns |
| --- | --- |
| `main.rs`, `lib.rs` | Binary entry, and the library root: store opening, native options, and app startup |
| `app.rs` | Hidden coordinator, tabs, session, hosts, and native-window actions |
| `workspace.rs` | One chart tab: grid, charts, drawings, fetches, and replay |
| `host/tab_window_host.rs` | Window grouping, active tabs, drag/reorder/detach |
| `native_bridge.rs` | AppKit menus, window behavior, cursor, Keychain/accessibility boundaries |
| `host/update_host.rs` | Update checks, install progress, and restart state |
| `host/portfolio_import_flow.rs` | CSV/CoinMarketCap import orchestration |
| `format.rs`, `platform.rs` | Leaf helpers every layer imports: number/price text and per-platform wording |

Feature hosts in `host/` (`alert_host`, `paper_host`, `pine_host`, `pine_subscription_host`,
`portfolio_host`, `portfolio_card_host`, `webhook_host`, `snapshot_host`, `mcp_host`, `sync_host`)
bridge pure engines, persistence, workers, and UI. `host/alert_runtime.rs` is the alert loop shared
by the GUI fallback and the login-item agent (`bin/degenview-alert-agent.rs`).

## Inside a workspace

`workspace.rs` keeps `ChartWorkspace`'s state, constructors and the per-frame `ui` entry point.
Every other concern is a child module in `workspace/` that adds an `impl ChartWorkspace` block; a
child sees the root's private fields, and anything another child calls is `pub(super)`.

| Module | Owns |
| --- | --- |
| `chart` | One chart's market data: fetching, refresh cadence, older-history paging |
| `charts` | The chart collection: add, replace, focus, remove, summaries for MCP |
| `grid` | The column grid: drawing cards, drag between columns, applying what a pass collected |
| `drawings` | Drawing properties, templates, locking, merging another Mac's drawings |
| `undo` | Undo/redo of drawing and indicator edits; restyle bursts as one step |
| `tool_rail` | The drawing tool rail and its grouped tiles |
| `pine` | Scripts and indicators on a chart, and the views the app and MCP take |
| `layouts` | Saved layouts: persisted form, dirty state, open/save/rename/delete |
| `autosave` | The autosave debounce |
| `replay` | The replay bar, timeline and stepping |
| `live` | The live Binance feed |
| `paper` | Paper quotes, portfolio cards and chart orders |
| `alerts` | The price-alert request and editor sheet |
| `watchlist` | The watchlist sidebar and what it reports |
| `empty_page` | The page a tab shows before it has a chart |
| `toolbar`, `settings`, `input` | The toolbar, the chart settings window, keyboard shortcuts |
| `content`, `context`, `links`, `snapshot` | Content tools, chart menus, Post cards, chart snapshots |
| `test_support` | Fixtures the module tests share (test builds only) |

## Feature directories

| Directory | Contents |
| --- | --- |
| `model/` | Alerts, charts/canvas/types, drawings, indicators, instruments, layouts, paper/portfolio engines, replay, tabs, watchlists, and webhooks |
| `pine/` | Lexer → parser → compiler → session, builtins, strategies, alerts, drawings, libraries, `request.*`, and corpus tests |
| `script_manager/` | File-backed scripts, editor operations/diff/completion, preview, inputs, reports, preferences, and demos |
| `host/` | The stateful seams: one host per feature, bridging engines, stores, workers, and views |
| `service/` | Market providers, HTTP/WebSocket work, updater, caches, images/icons, FX, notifications, files, Keychain, quotes, webhooks, and login-item registration |
| `store/` | Schema/helpers plus watchlist, layout/template, portfolio, paper, alert, Pine, and webhook persistence |
| `ui/` | Shared chrome plus chart, drawing, watchlist, alert, replay, paper, portfolio, Pine, settings, and webhook renderers |
| `mcp/` | Loopback HTTP transport, auth, shared permission state, app bridge, tools, and wire types |

## Boundaries

- `model/` and `pine/` stay pure and egui-free.
- UI renders supplied state; it does not own network or persistence.
- `service/` performs external I/O; `store/` alone owns SQLite details.
- Hosts in `host/` are the stateful seams. Do not bypass them from workers or views.
- `tools/bundle.sh` builds the macOS app; `tools/pine-corpus/` holds the optional corpus runner.
