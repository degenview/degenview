# Modules

The important boundary is ownership, not a complete file list. Use `rg --files src` for the current
inventory.

## Coordinators

| Module | Owns |
| --- | --- |
| `main.rs`, `lib.rs` | Binary entry, and the library root: store opening, native options, and app startup |
| `app.rs` | Hidden coordinator, tabs, session, hosts, and native-window actions |
| `workspace/mod.rs` | One chart tab: grid, charts, drawings, fetches, and replay |
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

`workspace/mod.rs` keeps `ChartWorkspace`'s state, constructors and the per-frame `ui` entry point.
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
| `pine_studio/` | File-backed scripts, editor operations/diff/completion/symbols/multi-cursor, preview, inputs, reports, preferences, and demos |
| `host/` | The stateful seams: one host per feature, bridging engines, stores, workers, and views |
| `service/` | Market providers, HTTP/WebSocket work, updater, caches, images/icons, FX, notifications, files, Keychain, quotes, webhooks, and login-item registration |
| `store/` | Schema/helpers plus watchlist, layout/template, portfolio, paper, alert, Pine, and webhook persistence |
| `ui/` | egui screens, one directory per domain (see `ui/` layout below), plus the shared `kit/`, `theme/` and `icons` |
| `mcp/` | Loopback HTTP transport, auth, shared permission state, app bridge, tools, and wire types |
| `sync/` | iCloud Sync: policy (what syncs), records, merge, outbox, id tagging and the CloudKit transport (bridge in `native/swift`); no UI |
| `trading/` | Order entry for every venue (paper account and connected brokers) behind `TradingBackend`: drafts, previews, confirmation, valuation; no drawing code |
| `bin/` | `degenview-alert-agent`, the login-item alert agent |

## `model/` layout

`model/` holds one directory per domain, and nothing but `mod.rs` at its root. A module sits in the
directory of the domain it belongs to, one concern per file, with the domain left out of the file's
name: `model::paper::engine`, not `model::paper_engine`. The domain's own types live in its
`mod.rs` (`model::paper::PaperOrderSide`); a file with tests keeps them in a `tests.rs` beside it
(`foo/mod.rs` + `foo/tests.rs`, never `foo.rs` + `foo/`).

| Directory | Holds |
| --- | --- |
| `alert/`, `webhook/` | Price and Pine alerts (engine, feed, line, messages, notifications) and webhook endpoints and policy |
| `paper/`, `portfolio/`, `watchlist/`, `broker/` | Paper trading, the portfolio ledger and its statistics, watchlists, and the shared live-trading values |
| `chart/` | One chart: appearance, canvas, viewport, price scale, kind, layout, templates, context menu, linked-chart sync, snapshots, replay, constructed bars (`synthetic/`) |
| `market/` | Instruments, sources, timeframes, candles, history paging, prediction-market series |
| `drawings/`, `annotation/`, `pattern/`, `fib/`, `forecast/` | Drawings and their tools: the shared model and styles, text/image/post/table content, patterns and Elliott waves, Fibonacci and Gann, positions and measurers |
| `indicators/`, `overlay/`, `pine/` | Built-in indicators and the catalog, markets compared on a chart, Pine instances and logs on a chart |
| `mcp/`, `tab/`, `session_stats/`, `editor_theme/` | MCP activity, tabs, windows and Tab Linking, session statistics, editor themes |

Flat re-exports for the most used types are in `model/mod.rs` (`model::Kline`, `model::ChartType`).
A new module goes in the matching directory; a new domain gets its own directory and a row here.

## `pine/` and `pine_studio/` layout

Both follow the rule `model/` does: one directory per concern, children named without the directory's
name, `foo/mod.rs` + `foo/tests.rs` rather than `foo.rs` + `foo/`, whole-window or end-to-end tests
beside the code they exercise.

`pine/` (the engine, pure and egui-free):

| Directory | Holds |
| --- | --- |
| `syntax/` | `token`, `lexer`, `parser`, `ast`, `traversal` |
| `check/` | The type checker and the rules that need no types: names, structure, declarations, identifiers, ignored features, `callables` |
| `compiler/` | `compiler` (source → `CompiledProgram`), `program`, the builtin `registry`, `symbol_catalog` |
| `runtime/` | The interpreter: `session`, `state`, `value`, `support`, `containers`, `execution`, call routing and binding, user methods, `request.security` |
| `builtins/` | Constants and colour math (`mod.rs`), then one child per namespace: `alert`, `array`, `map`, `matrix`, `math`, `strings`, `number_format`, `regex_lite`, `ta`, `misc`, `scalar`, `market_values` |
| `outputs/`, `inputs/` | Chart output types with `visuals`, `drawings`, `style_overrides`; how an `input.*` call is read |
| `strategy/`, `footprint/`, `logs/`, `time/` | The strategy model, `broker` and `strategy.*`; `request.footprint()`; Pine Logs; calendar, time zones, sessions |
| `debug/`, `profile/`, `tests/`, `corpus/` | The debugger, the profiler, end-to-end and benchmark tests, the corpus data |

`pine_studio/`:

| Directory | Holds |
| --- | --- |
| `ai/`, `debug/`, `profiler/` | The assistant, the debugger and the profiler: their state, host, panel, gallery pictures and `studio_tests` |
| `editor/` | The text editor and everything that edits or reads text: `edit_ops`, `multi_cursor`, `completion`, `signature`, `symbols`, `find`, `syntax/`, `diff`, `call_sites`, `color_sites` |
| `library/` | Where scripts live: `store`, `registry`, `sidebar`, `prefs`, `name`, `demo` and `demos/`, `unsaved`, `external/` |
| `drawer/` | The drawer under the preview: `problems/`, `logs/`, the intro, `bottom_bar`, `tab_chrome` |
| root | `mod.rs` (the manager), `model`, `preview`, `inputs`, `workspace`, `sheets`, `indicators/`, `theme_preview/` |

## `ui/` layout

`ui/` follows the same rule as `model/`: one directory per domain, children named without the domain
(`ui::broker::panel`, not `ui::broker_panel`), `foo/mod.rs` + `foo/tests.rs`. A screen's gallery or
picture test sits beside it (`broker/panel_gallery.rs`). A helper used by several domains belongs in
`kit/`, not in the domain that happened to write it first.

| Directory | Holds |
| --- | --- |
| `chart/` | The chart card (`card/`), canvas and series rendering, chart menus, scale menu, indicator strip, table view, object tree, session stats, the Add Chart sheet (`add/`), chart settings with templates and overlays (`settings/`), and the symbol and indicator pickers |
| `drawing/` | Drawings: pure geometry (`geometry/`, with `fib`, `line`, `annotation`, `pattern`), the interaction layer (`layer/`), the properties panel (`properties/`), `render`, and the text/note/table content tools (`content/`) |
| `alert/`, `webhook/`, `mcp/` | Alert center, editors, banner and chart lines; webhook editor, picker and settings; MCP activity and confirmation |
| `paper/`, `broker/`, `trade/` | Paper trading screens, live trading screens, and what both share (the order sheet and its fields) |
| `portfolio/`, `watchlist/`, `replay/` | The Portfolio tab and its cards, the watchlist sidebar, the replay bar, date sheet and overlay |
| `pine/` | Pine output on a chart, report, style view, settings popover, Pine Logs (`logs_view/`) and the script catalog |
| `settings/` | The Settings window (`window`) and one file per page; the Pine Studio page owns the font picker and theme editor |
| `workspace/`, `window/` | Workspace chrome (toolbar, tool rail, layouts, empty page, chart linking, snapshot menu) and window chrome (tab strip, tab-link menu, update indicator) |
| `kit/`, `theme/` | Shared widgets (`kit`, with `dialog`, `sheet`, `popover`, `color_picker`, `date_picker`, `split`, `source_logo`, `art`, ...) and colours and fonts |
| root | `mod.rs` and the three single-file foundations: `icons`, `shortcut`, `accessibility` (plus `editor_appearance/`) |

A new screen goes in its domain's directory; a new domain gets a directory and a row here.

## Boundaries

- `model/` and `pine/` stay pure and egui-free.
- UI renders supplied state; it does not own network or persistence.
- `service/` performs external I/O; `store/` alone owns SQLite details.
- Hosts in `host/` are the stateful seams. Do not bypass them from workers or views.
- `tools/bundle.sh` builds the macOS app; `tools/pine-corpus/` holds the optional corpus runner.
