# Modules

The important boundary is ownership, not a complete file list. Use `rg --files src` for the current
inventory.

## Coordinators

| Module | Owns |
| --- | --- |
| `main.rs`, `lib.rs` | Binary entry, and the library root: store opening, native options, and app startup |
| `app/` | Hidden coordinator: `mod.rs` holds `DegenViewApp`, its constructor and per-frame update; children hold the rest (see `app/` layout below) |
| `workspace/mod.rs` | One chart tab: grid, charts, drawings, fetches, and replay |
| `host/tab/window.rs` | Window grouping, active tabs, drag/reorder/detach |
| `native_bridge.rs` | AppKit menus, window behavior, cursor, Keychain/accessibility boundaries |
| `host/update/mod.rs` | Update checks, install progress, and restart state |
| `host/portfolio/import_flow.rs` | CSV/CoinMarketCap import orchestration |
| `format.rs`, `platform.rs`, `lock.rs` | Leaf helpers every layer imports: number/price text, per-platform wording, and locks that survive a poisoning panic |

Feature hosts in `host/` (one directory per domain, see `host/` layout below) bridge pure engines,
persistence, workers, and UI. `host/alert/runtime.rs` is the alert loop shared by the GUI fallback and
the login-item agent (`bin/degenview-alert-agent.rs`).

## Inside a workspace

`workspace/mod.rs` keeps `ChartWorkspace`'s state, constructors and the per-frame `ui` entry point.
Every other concern is a child module in `workspace/` that adds an `impl ChartWorkspace` block; a
child sees the root's private fields, and anything another child calls is `pub(in crate::workspace)`.

The root holds the charts and their grid (charts, columns, focus, timeframe, sync), shared handles
(store, watchlist store, quote book and coordinator, card environment, snapshot settings), and the
state of views that already have a type of their own (the pickers and popovers, the chart menu's
`ContextState`, `SnapshotState`, `EmptyPageCache`, Tab Linking's `LinkState`). Each other concern is
an owned part, a struct whose fields only its own module (or directory) sees; the rest of the
workspace calls its methods:

| Part | Field | Module | Owns |
| --- | --- | --- | --- |
| `DrawingTools` | `tools` | `drawing/tools` | The armed tool and Zoom In, the strip's options (`DrawingPrefs`) and the templates store, hiding, the selection, the copied style, the properties dialog and Remove all's question |
| `LayoutSession` | `layout` | `layout/session` | The open layout's id and name, the saved fingerprint and drawings (dirty state, merging another Mac's edit), the name prompt, the pending open, the error alert, the autosave debounce |
| `UndoHistory` | `history` | `undo` | The undo and redo stacks, and the runs (a drawing's restyles, an overlay's colour drag) that collapse into one step; `record` is the one way to take a step |
| `ReplayState` | `replay` | `replay` | The replay clock, its step timer, the rebase after a timeframe change, the date sheet |
| `WatchlistPane` | `watchlist_pane` | `watchlist` | Whether the sidebar shows, its view state, notices, import report and export, the list the symbol picker adds to, a row's open-in-new-tab request |
| `TradeSide` | `trade` | `trade/` | The trading hub, the order sheet and orders panel and whether they show, the venue, the broker and paper panels |
| `AlertUi` | `alerts` | `alerts` | The bell's unseen count, the requested and open price-alert sheet, its FX rates |

`DrawingTools` and `LayoutSession` keep their fields `pub(super)`, visible to their directory
(`drawing/`, `layout/`) and nowhere else. An operation that spans parts (opening a layout clears the
undo history; an edit bumps the layout's drawings version) stays on `ChartWorkspace` and calls each
part. The children are grouped by domain, like `model/` and `ui/`:

| Directory | Owns |
| --- | --- |
| `chart/` | `mod.rs`: one chart's market data, fetching, refresh cadence, older-history paging; `collection` (add, replace, focus, remove, summaries for MCP), `live` (the live Binance feed), `settings` (the chart settings window), `symbol_picker`, `snapshot`, `context/` (the right-click menu) |
| `overlay/` | Compared markets: their candles fetched once per market (`mod.rs`) and the undoable add, edit and remove (`actions`) |
| `drawing/` | `mod.rs`: drawing properties, templates, locking, merging another Mac's drawings; `tools` (`DrawingTools`), `content` (where a placed picture comes from), `links` (Post cards), `tool_rail` (the tool rail and its grouped tiles) |
| `layout/` | Saved layouts: persisted form, dirty state, open/save/rename/delete (`mod.rs`), the tab's `LayoutSession` (`session`) and the autosave debounce (`autosave`) |
| `trade/` | The tab's trading side: `TradeSide` and its panels (`mod.rs`), `broker`, `paper` (quotes, portfolio cards, chart orders) and `sheet` (the order sheet's venue and actions) |
| `pine/` | Scripts and indicators on a chart, and the views the app and MCP take (`mod.rs`); the Pine Logs pane (`logs/`) |
| `tab_link/` | A tab's side of Tab Linking |
| `empty_page/` | The new-tab page's slow reads (recents, layouts, Alpaca key) in `mod.rs`, and the week of closes behind its sparklines (`trends`) |
| root | `grid` (the column grid), `undo` (drawing and indicator edits as steps), `replay`, `alerts`, `watchlist`, `toolbar`, `input` (shortcuts), `test_support` (test fixtures) |

## Feature directories

| Directory | Contents |
| --- | --- |
| `model/` | Alerts, charts/canvas/types, drawings, indicators, instruments, layouts, paper/portfolio engines, replay, tabs, watchlists, and webhooks |
| `pine/` | Lexer → parser → compiler → session, builtins, strategies, alerts, drawings, libraries, `request.*`, and corpus tests |
| `pine_studio/` | The script library (registry, names, sidebar), editor operations/diff/completion/symbols/multi-cursor, preview, inputs, reports, preferences, and demos |
| `host/` | The stateful seams: one host per feature, bridging engines, stores, workers, and views (one directory per domain, see below) |
| `service/` | Market providers, HTTP/WebSocket work, updater, caches, images/icons, FX, notifications, files, Keychain, quotes, webhooks, and login-item registration (see the layout below) |
| `store/` | Schema/helpers plus watchlist, layout/template, portfolio, paper, alert, Pine, webhook persistence, and the file-backed script library (`scripts/`) |
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
| `alert/`, `webhook/` | Price and Pine alerts (engine, feed, line, messages, notifications, the `draft` the sheet and MCP build) and webhook endpoints and policy |
| `paper/`, `portfolio/`, `watchlist/`, `broker/` | Paper trading, the portfolio ledger and its statistics, watchlists, and the shared live-trading values |
| `chart/` | One chart: appearance, canvas, viewport, price scale, kind, layout, templates, context menu, linked-chart sync, snapshots, replay (the clock and what an agent asks of it), constructed bars (`synthetic/`) |
| `market/` | Instruments, sources, timeframes, candles, history paging, prediction-market series, CoinMarketCap chart keys and titles (`cmc`) |
| `drawings/`, `annotation/`, `pattern/`, `fib/`, `forecast/` | Drawings and their tools: the shared model and styles, text/image/post/table content, patterns and Elliott waves, Fibonacci and Gann, positions and measurers |
| `indicators/`, `overlay/`, `pine/` | Built-in indicators and the catalog, markets compared on a chart, Pine instances and logs on a chart, script value types (`script`) and the Pine indicators bundled with the app (`bundled/`) |
| `mcp/`, `tab/`, `session_stats/`, `editor_theme/` | MCP activity, tabs, windows and Tab Linking, session statistics, editor themes |
| `ids/` | The id space engines mint into (`LEGACY_LIMIT`, `GLOBAL_BASE`); sync tags it in `sync::record::ids` |

Flat re-exports for the most used types are in `model/mod.rs` (`model::Kline`, `model::ChartType`).
A new module goes in the matching directory; a new domain gets its own directory and a row here.

## `service/`, `host/`, `store/`, `sync/`, `mcp/`, `workspace/` and `trading/` layout

They follow the rule `model/` does: a directory per domain, children named without the directory's
name, `foo/mod.rs` and never `foo.rs` beside `foo/`, no three files with one prefix. The domains reuse
the names `model/` and `ui/` already use (`alert`, `webhook`, `portfolio`, `paper`, `broker`, `chart`,
`market`, `annotation`, `editor_theme`, `mcp`, `pine`, `tab`, `sync`); only plumbing gets a new word
(`net`, `cache`, `keychain`, `system`). `layout_tests.rs` fails the build on the two mechanical
mistakes, on an import that points up a layer, and on drawing code outside the view layers.

`service/` (external I/O; root: `mod.rs` with `MarketSource`, `SearchResult` and `fetch_*`, and `work_queue`):

| Directory | Holds |
| --- | --- |
| `market/` | Every provider (`alpaca binance cmc coinbase coingecko dexscreener geckoterminal kalshi polymarket`), their `error` (`ProviderError`), `live` feed, `candles` shaping, symbol `catalog`, `search_rank`, `fx_rates` |
| `net/` | Shared transport: `http`, `rate_limit`, `ws`, `multipart` |
| `cache/` | `mod.rs` the candle cache; `disk`, `lookup` and `control` (what Settings ▸ Storage does) |
| `annotation/` | What drawings' pictures and Post cards need: `asset_host`, `ingest`, `link_fetch` |
| `image/` | Artwork: `icon` resolver and the remote-image `loader` |
| `keychain/` | Secrets by kind (`ai_keys api_keys broker_secrets mcp_seal snapshot_secrets webhook_secrets`) and the platform stores (`secret_service`, `windows_credentials`) |
| `update/` | `check`, `download`, `archive`, `tag_signature`, `code_signature`, and `install/` (the per-platform self-update) |
| `snapshot/` | `export`, `job`, `raster`, `upload` and `providers/` |
| `portfolio/`, `webhook/`, `alert/`, `editor_theme/` | Portfolio history, performers, quotes and ticker mapping; webhook delivery; macOS notifications and the login item; theme files and the font catalog (`fonts/`) |
| `system/` | `entitlements`, `file_dialog`, `sandbox`, `timezone` |
| `llm/`, `watchlist_quotes/` | The AI providers; the shared quote stream |

`host/` has only `mod.rs` at its root. One directory per domain, the host in `mod.rs` and its helpers
beside it: `alert/` (with `runtime`), `broker/`, `editor_theme/`, `mcp/` (with `confirmation`), `paper/`,
`pine/` (with `chart/`, the runs of a chart's scripts and their workers; `batch`, scripts run once
each; `subscription`), `portfolio/` (with `card` and `import_flow`), `settings/` (the Settings
window's controller: it builds the page's input and applies its actions), `snapshot/`, `sync/` (with
`scripts`, the script sink over the file store), `tab/` (`window`, `title`, `link`), `trading/` (the
hub), `update/`. Hosts draw nothing: the views that sit on them live in `ui/` (`ui::portfolio::tab`,
`ui::paper::panel`), and views run nothing: the chart card hands its bars to `host::pine::chart` and
draws what comes back.

`store/` has one file per single-table domain store (`alert`, `asset_store`, `drawing_template`,
`identity`, `paper`, `pine`, `portfolio`, `watchlist`, `webhook`) and a directory once a store has a
second file: `chart/` (`template/`, `saved_view`), `editor_theme/`, `mcp/` (with `audit`), `sync/`,
`scripts/` (the user's Pine scripts as `.pine` files with their metadata beside them, not SQLite).

`sync/`:

| Directory | Holds |
| --- | --- |
| `engine/` | The state machine and what it does: `core`, `initial` merge, `merge`, `outbox`, `apply/`, `account` |
| `record/` | What goes over the wire: `mod.rs` the envelope; `ids`, `policy`, `project`, `validate`, `paper`, `scripts` |
| `transport/` | The CloudKit boundary: `mod.rs` the trait, its factory, the event sink and events, `native/` (the Swift bridge), `mock` (tests) |
| `tests/` | End-to-end sync tests, one file per concern |

`mcp/` keeps `mod.rs`, `bridge` (the queue and `McpCommand`, one variant per domain), `state`,
`types` (results and their word enums), `confirm`, `audit`, `tools` (names, permissions, the `Caller`
extractor, the routers put together and the handler that guards them), `replay`, `snapshot` and
`tests/` at its root. Tools are grouped by domain: `alert/`, `chart/` (`control`, `data`, `drawings`),
`indicator/`, `market/`, `paper/`, `pine/`, `portfolio/`, `session_stats/`, where `mod.rs` holds the
domain's command enum (what it hands the app thread and whether each command asks first) and its
pure result types, and `tools.rs` the tool router. `transport/` holds the HTTP surface
and what guards it: `auth`, `cancel`, `limits`, `seal`, `tls`, `tls_protect`.

`app/` splits `impl DegenViewApp` by concern, each child adding an `impl` block (like `workspace/`):
`tabs` (open, close, show, tab quotes, Tab Linking), `window` (the tab windows and Settings, and their viewports), `commands`
(native menu commands, quit), `mcp` (the MCP command loop, which routes each command to its domain, and confirmation), `pine` (Pine Studio and
scripts), `alert/` (the alert coordinator and the Alerts window), `background` (broker notices,
editor themes, sync), `session` (the saved session) and `tests`. A helper used by one child stays
private to it; what two children share is `pub(super)`.

The root keeps the shared hosts every window is handed (store, watchlist store, quote book and
coordinator, portfolio, paper, broker and trading hosts, Settings, snapshots, editor themes, updates,
sync, Pine subscriptions), the notifier and the window callbacks' action queue. Each other concern is
an owned part whose fields only its module sees. A deferred viewport callback must be `'static`, so
what a window shares with the app is a part's cheap `Clone` handle, not the app's fields:

| Part | Field | Module | Owns | Handle the windows hold |
| --- | --- | --- | --- | --- |
| `TabState` | `tabs` | `tabs` | The open `TabSlot`s, the next tab id, Tab Linking's hub, what the tab windows were last repainted for (quotes, title style, broker, links); opening, closing, showing and the per-frame quote and link ticks | `SharedTabs`: the window host, the id→content registry, the focused tab, the title and link hosts |
| `AlertCoordinator` | `alerts` | `alert/` | The alert host, the runtime this window runs, the FX feed (`alert/fx`), the banner, the Alerts window (`alert/window`: its open flag, and the centre, editor, Webhooks… sheet and chart datasets behind one lock) | `AlertRegistration` (`alert/registration`: notifier, background service, runtime control; commit an alert, retry), `SharedFxRates` |
| `McpState` | `mcp` | `mcp` | The MCP host, the clients' questions, the orders placed and pictures being made between requests | `McpQuestions` |
| `SessionState` | `session` | `session` | When the session was last written, what was written, why the write failed | `SessionSaveError` |

Cross-part work (a tick that reads the tabs and runs the alerts, an MCP command on a chart) stays on
`DegenViewApp` and calls each part. Where locks nest, the split kept the order they were taken in:
the Settings window takes its host, then the store, then the alert host, the MCP host, the Pine host
or the registration, one at a time; the Alerts window takes the store, then its own state, then the
alert, Pine or Settings host; the registration takes its runtime control before the background
service. Never take one of these locks again while holding it: the frame and every window callback
run on the main thread, so that deadlocks at once.

`host/mcp/` keeps `McpHost` (listener lifecycle, settings, credentials, activity log) and the
`McpTabs` view of the app's tabs in `mod.rs`. What the app thread runs for a tool is grouped by domain
like `mcp/`: `chart`, `pine`, `alert`, `paper`, `portfolio`, `watchlist`, `replay`, `snapshot`, each with
a `run` for its commands and a `describe` for the confirmation step, plus `confirmation` (the
questions and the summaries); its tests are in `tests/`.

`trading/` keeps the trait, drafts and previews at its root and one directory per backend family:
`broker/` (`backend`, `commands`) and `paper/` (`backend`, `stats`).

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
| `builtins/` | Constants and colour math (`mod.rs`), then one child per namespace: `alert`, `array`, `map`, `matrix`, `math`, `strings`, `number_format`, `regex_lite`, `ta`, `misc`, `market_values` |
| `outputs/`, `inputs/` | Chart output types with `visuals`, `drawings`, `style_overrides`; how an `input.*` call is read |
| `strategy/`, `footprint/`, `logs/`, `time/` | The strategy model, `broker` and `strategy.*`; `request.footprint()`; Pine Logs; calendar, time zones, sessions |
| `debug/`, `profile/`, `tests/`, `corpus/` | The debugger, the profiler, end-to-end and benchmark tests, the corpus data |

`pine_studio/`:

| Directory | Holds |
| --- | --- |
| `ai/`, `debug/`, `profiler/` | The assistant, the debugger and the profiler: their state, host, panel, gallery pictures and `studio_tests` |
| `editor/` | The text editor and everything that edits or reads text: `edit_ops`, `multi_cursor`, `completion`, `signature`, `symbols`, `find`, `syntax/`, `diff`, `call_sites`, `color_sites` |
| `library/` | The script library around its file store (`store::scripts`): `registry`, `sidebar`, `prefs`, `name`, `demo` and `demos/`, `unsaved`, `external/` |
| `drawer/` | The drawer under the preview: `problems/`, `logs/`, the intro, `bottom_bar`, `tab_chrome` |
| root | `mod.rs` (the manager), `preview`, `inputs`, `workspace`, `sheets`, `theme_preview/` |

## `ui/` layout

`ui/` follows the same rule as `model/`: one directory per domain, children named without the domain
(`ui::broker::panel`, not `ui::broker_panel`), `foo/mod.rs` + `foo/tests.rs`. A screen's gallery or
picture test sits beside it (`broker/panel_gallery.rs`). A helper used by several domains belongs in
`kit/`, not in the domain that happened to write it first.

| Directory | Holds |
| --- | --- |
| `chart/` | The chart card (`card/`), canvas and series rendering, chart menus, scale menu, indicator strip, table view, object tree, session stats, chart settings with templates and overlays (`settings/`), the symbol and indicator pickers, and the pieces the symbol picker shares with the portfolio pickers (`picker_kit/`: recents, suggestions, artwork, result rows, search box, header art) |
| `drawing/` | Drawings: pure geometry (`geometry/`, with `fib`, `line`, `annotation`, `pattern`), the interaction layer (`layer/`), the properties panel (`properties/`), `render`, and the text/note/table content tools (`content/`) |
| `alert/`, `webhook/`, `mcp/` | Alert center, editors, banner and chart lines; webhook editor, picker and settings; MCP activity and confirmation |
| `paper/`, `broker/`, `trade/` | Paper trading screens (the panel under a chart in `panel/`), live trading screens, and what both share (the order sheet and its fields) |
| `portfolio/`, `watchlist/`, `replay/` | The Portfolio tab (`tab/`: its bar, pages and import sheets) and its cards, the watchlist sidebar, the replay bar, date sheet and overlay |
| `pine/` | Pine output on a chart, report, style view, settings popover, Pine Logs (`logs_view/`) and the script catalog |
| `settings/` | The Settings window (`window`, run by `host::settings`) and one file per page; the Pine Studio page owns the font picker and theme editor |
| `workspace/`, `window/` | Workspace chrome (toolbar, tool rail, layouts, the new-tab page in `empty_state/`, chart linking, snapshot menu) and window chrome (tab strip, tab-link menu, update indicator) |
| `kit/`, `theme/` | Shared widgets, one file per family and all re-exported from `kit` (`kit::push_button`, not `kit::button::push_button`): `text` (type scale, galleys), `palette`, `pill`, `ticker_icon` (market logos, monograms), `tab_bar`, `menu/` (rows, `fit`, `picker`), `button`, `field`, `form` (switch, segmented, stepper, colour well), `card` (frame, divider), `page` (empty state, footer bar, notice card), `confirm`, `autosave`; plus the modules called by path: `bar` (the flat top bar every tab kind shares), `dialog`, `sheet`, `popover`, `icon_tabs`, `color_picker`, `date_picker`, `split`, `source_logo`, `time_text`, `art`, ... And colours and fonts (`theme`) |
| root | `mod.rs` and the three single-file foundations: `icons`, `shortcut`, `accessibility` (plus `editor_appearance/`) |

A new screen goes in its domain's directory; a new domain gets a directory and a row here.

## Boundaries

- `model/` and `pine/` stay pure and egui-free.
- UI renders supplied state; it does not own network or persistence.
- `service/` performs external I/O; `store/` alone owns SQLite details.
- Hosts in `host/` are the stateful seams. Do not bypass them from workers or views.
- `tools/bundle.sh` builds the macOS app; `tools/pine-corpus/` holds the optional corpus runner.
