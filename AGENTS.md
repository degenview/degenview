# AGENTS.md — DegenView

Guidance for AI coding agents working in this repo.

## Project

macOS crypto candlestick chart app. SwiftUI views, AppKit Canvas rendering, REST + WebSocket data from Binance/Coinbase/CoinGecko/DEXScreener. One external dependency: [GRDB.swift](https://github.com/groue/GRDB.swift) (SQLite persistence), via SPM.

See [Architecture](docs/architecture.md) for the project structure and data flow.

## Build

```bash
open DegenView.xcodeproj
# ⌘R in Xcode, or: xcodebuild -project DegenView.xcodeproj -scheme DegenView build
```

Requires Xcode 16+, macOS 14+.

Every compile of the app target bumps `CFBundleVersion` (the "Bump Build Number" run-script phase,
`tools/bump-build-number.sh`). The counter is the gitignored `.build-number`; it is written into
the built Info.plist only, never the project. That phase is why the app target has script
sandboxing off.

## Code conventions

- **MVVM**: `Model/` — data + enums, `ViewModel/` — `@ObservableObject` state, `View/` — SwiftUI views
- **@MainActor** on all ViewModels that publish UI state
- **No SwiftUI Charts** — candles are hand-drawn via AppKit `Canvas`
- **Protocol abstraction** for data sources: `TickerDataSource` protocol, `DataSourceFactory` singleton
- **Persistence**: user data (tabs, saved views, watchlists, drawings, portfolios, paper
  trading, alerts) lives in SQLite (`degenview.sqlite`, WAL) through `AppDatabase`, shared
  with the alert agent. Small caches stay in `JSONStore<T>`; closed daily candles for
  portfolio history are the exception — large, append-mostly, read by range — and live in the
  `candle`/`candle_coverage` tables via `PortfolioCandleStore`. The schema is one
  `AppDatabase.createSchema` in `AppDatabase+Schema.swift` (`IF NOT EXISTS`); there are no
  versioned migrations and no readers for older data — this is the first version. The one
  exception is the old `favorite` table, read once by `WatchlistStore+FavoritesMigration`
- **Pine is a feature folder**: everything for the Pine Script engine lives under
  `DegenView/Pine/` (`Language`, `Runtime`, `Broker`, `Model`, `Editor`, `View`). One type per
  file still applies; a long type is split into `Type+Concern.swift` extensions (members
  shared across those files are internal, not `private`). `Pine/Model` and `Model/Script`
  also compile into the `DegenViewAlertAgent` target — keep them free of compiler and
  runtime types, and add any new file there to the agent's Sources phase as well
- **One `ContentViewModel` per tab** — never treat it as app-global state
- **Caching**: `ChartViewModel.fetchData` caches results keyed by (symbol, interval, limit) in a dictionary
- **WebSocket**: Only Binance, Coinbase and Alpaca tickers get live streams; connect/disconnect on ticker add/remove

## Key patterns

### Adding a new data source
1. Add case to `DataSourceType` enum
2. Create service class conforming to `TickerDataSource`
3. Register in `DataSourceFactory.service(for:)` and `allSources`
4. Add kline parser init in `KlineData` if API format differs
5. Update `AddTickerSheet` search to include new source
6. Add a `SourceLogo-<case>` imageset to `Assets.xcassets` and a `DataSourceType.logoAsset`
   case; `SourceLogoView` draws it (falls back to the SF Symbol `icon`). Record the logo's
   origin in `docs/source-logos.md`

Prediction markets (Polymarket, Kalshi) are the exception to steps 3 and 5: conform to
`PredictionMarketDataSource`, add the case to `DataSourceType.predictionMarkets`, and use
`isPredictionMarket` rather than comparing against `.polymarket`. They get a
`PredictionMarketSearchViewModel` and appear under the "Prediction Markets" tab
(`PredictionMarketPicker`) in both `AddTickerSheet` and `ChartSettingsSheet`. Kalshi has no
keyword search, so `KalshiSeriesIndex` ranks the cached series list locally; its
WebSocket needs a signed API key, so both providers refresh over REST. Kalshi ids are
`"SERIES/MARKET"` (`KalshiMarketID`) and ride in `pmSeries.tokenID`. `KalshiService`,
`KalshiSeriesIndex`, `KalshiModels` and `PredictionMarketDataSource` also compile into
`DegenViewAlertAgent`.

### Adding a new timeframe
1. Add case to `TimeRange` enum
2. Set `binanceInterval`, `dataPointLimit`, `chartTitle`, `dateFormat`
3. If a provider can't serve the interval natively, fold a finer one into calendar buckets
   (`KlineData.folded(into:)`, boundaries from `KlineData.bucketStart`) rather than picking a
   nearby size. `3M` and `1Y` are quarterly/yearly candles on every source: Binance and Alpaca
   fold monthly bars (`KlineData.monthlyFold`), Coinbase folds daily, CoinGecko and
   GeckoTerminal build them from their own series. Binance has no live stream for them — the
   5 s REST refresh covers it. Prediction markets are line charts, so they keep their own
   `effectiveSpanDays`/`lineChartPointCount` windows

### Chart rendering
- `CandleChartView` owns the `Canvas` draw loop
- `CandleChartStyle` is a plain struct — no `@ObservedObject`, passed by value
- Y-axis: `yForPrice(_:)` handles both linear and log scale
- Grid lines drawn first, then candles, then price overlay, then X-axis labels
- Price formatting: auto-adjusts decimal places based on price magnitude

### Coin icons
- `IconResolver` (actor) resolves one icon per `"<source>:<ticker>"` key and caches the
  result in `icon_cache.json` — misses too, on a shorter TTL, so a dead lookup doesn't
  re-walk the chain on every card appearance
- Chain: market snapshot (symbol *and* coin id) → source-specific (CoinGecko `ids=`,
  batched; DEXScreener pair address) → CoinGecko `/search` → static icon set → `nil`
- A DEXScreener pair lookup also yields the base token symbol, which the symbol-keyed
  steps then reuse — the ticker itself is a contract address
- All CoinGecko traffic (OHLC *and* icons) queues behind `CGRateLimiter.shared`
- The same `/coins/markets` snapshot and `ids=` batch also carry the coin **name**
  (`IconResolver.coinName(forSymbol:)` / `coinName(forCoinID:)`); `PortfolioAssetInfoViewModel`
  uses it for portfolio subtitles ("Bitcoin"), falling back to the stored label. DEX tickers are
  deliberately not name-resolved by symbol — they collide with unrelated coins
- `nil` is not a failure state for the UI: `TickerIconView` draws a monogram, so the
  20×20 slot is occupied either way and card headers stay aligned
- Icon lookups key off `ChartViewModel.iconKey`, never `uniqueID` — `uniqueID` survives
  `updateTicker` by design and would pin the old coin's artwork to a renamed card

### Alert events
- Every alert trigger is an `AlertDomainEvent` published on `AlertEventBus` (`AlertStore.reload()` for
  price alerts, `PineAlertStore.record` for script alerts). A new trigger path must publish there.
  `UnseenAlertsStore` turns them into the sidebar bell's unseen count; "seen" is the
  `alerts.lastSeenAt` setting, never a flag on the events (`replaceSnapshot` rewrites them)

### Webhooks
- `docs/webhooks.md` has the behaviour; `docs/architecture.md` has the flow. Endpoints are global
  (`WebhookEndpointStore`, `webhook_endpoint` table). The URL and header values are templates stored in
  plain text in that row (the agent reads them); show the URL only through `WebhookRedactor`. A secret
  is the one value that lives in the Keychain (`WebhookSecretStore`, shared access group, per endpoint),
  placed with `{{secret}}` by `WebhookRequestResolver`: percent-encoded in the URL, raw in headers, never
  in the host or the body. Never log or store a resolved URL/header or touch `webhook_delivery` with one;
  Debug builds have no access group, so the agent can't read secrets there
- `WebhookDeliveryService` is the only webhook HTTP. Price alerts send from `AlertRuntimeHost` through
  `PriceAlertWebhookDispatcher` (lock owner only; `AlertStore` must never send); Pine alerts send through
  `WebhookPineAlertChannel`. Both claim a `webhook_delivery` row before sending. No retries, no redirects
- The webhook models, policy, renderer, transport, service, `AppDatabase+Webhooks` and
  `PriceAlertWebhookDispatcher` also compile into `DegenViewAlertAgent`: keep them free of SwiftUI,
  `AlertStore` and Pine runtime types, and add any new file to the agent's Sources phase
- New fields on `PriceAlert`, `AlertTriggerEvent`, `PineAlertSubscription` and `PineAlertNotification` must
  decode old rows (`decodeIfPresent`): a row that fails to decode is dropped, and `replaceSnapshot` then
  rewrites the table without it
- Tests use `RecordingWebhookTransport` and `InMemoryWebhookSecretStore` (`WebhookTestSupport.swift`);
  nothing reaches the network or the Keychain

### Pine editor assistance
- Pure logic, no AppKit: `PineEditorPairing` (auto-pair, overtype, empty-pair Backspace, wrap),
  `PineIndentationEngine` (Return, Tab/Shift-Tab, closing-delimiter alignment, paste),
  `PineEditorCommands` (⌘/, ⌥↑↓ move, ⇧⌥↑↓ duplicate), `PineDelimiterMatcher`. Each takes a
  `PineEditorContext` (text + selection + `PineLexicalSnapshot`, all UTF-16) and returns one
  `PineEditorEdit`; `PineTextView.perform(_:)` applies it as a single undoable change. New
  assistance goes there, not into `PineTextView` or the `Coordinator`
- `PineLexicalSnapshot` is the one lex per text version (cached; the classifier uses it too).
  Strings are lexer tokens, comments are `//` runs between them, brackets are lexer tokens —
  so nothing inside a string or comment can pair. Never use `.indent`/`.dedent` tokens for
  indentation decisions: an unclosed `(` mid-typing erases them
- Indent unit is four spaces (the lexer reads other widths as wrapped lines). Pine has no
  braces, so `{}` is deliberately not paired
- Decorations that follow the caret are never stored text attributes (`PineSyntaxHighlighter`
  rewrites storage attributes every edit): bracket match and occurrences are layout-manager
  temporary attributes (`PineEditorDecorations`, one owner of `.backgroundColor`); current-line
  band and indent guides draw in `PineLayoutManager.drawBackground(forGlyphRange:at:)`
- Everything is skipped while `hasMarkedText()` (IME) — keep that guard in `editingContext`

### Pine completion and signature help
- One analysis per text version: `PineEditorAnalysisCache.analysis(for:)` returns the lexical
  snapshot and a `PineSourceSymbolIndex` (declarations with kinds, parameters and visibility,
  scopes with ranges, imports, user types and enums, exports). The classifier resolves user
  shadowing through the same index, so highlighting and completion cannot disagree. It reads
  content tokens, never the lexer's newline/indent/dedent (`PineStatementSplitter`): an unclosed
  `(` mid-typing erases those, and completion must keep its scopes then
- Pure engine, Foundation only, unit-testable without a text view: `PineCompletionContext`
  (word, member base, position, suppression, scope, `PineCallSite`), `PineCompletionEngine`,
  `PineCompletionRanking` (deterministic, no fuzzy matching), `PineCompletionTrigger` (the one
  place the 2-letter threshold lives), `PineCompletionInsertion` (one `PineEditorEdit` per
  acceptance; a call's `(` pairs by `PineEditorPairing.canOpenPair`), `PineSignatureResolver`.
  Completion never compiles, runs, lexes again, or touches the network or database;
  `PineCompletionPerformanceTests` greps the files for that
- Builtins come from `PineSymbolCatalog` (`members(of:)`) and signatures/docs from
  `PineSymbolMetadata`, one DSL line per overload in the `PineSymbolMetadata+<Family>.swift`
  files. A new builtin needs a line there; `PineSymbolMetadataTests` fails until it has one, and
  fails a line the runtime does not dispatch. Never add a second list of names to the engine
- Libraries: `PineLibraryExportDirectory` reads a library's `export`s from `PineLibraryRegistry`
  (memory only) through the same symbol index, with its own lex so the edited script's cached
  lex is not evicted
- UI is `PineCompletionController` (per `PineTextView`, `textView.completion`) over two child
  `NSPanel`s that never become key (`PineCompletionPanel`, `PineSignaturePanel`). Keys: Down/Up,
  Return/Tab accept, Escape closes the list then the help — the Escape check is first in the
  container's key monitor, ahead of the find bar. With nothing open every key keeps its editing
  behaviour. `perform(_:)` calls `didChangeText`, so the controller ignores changes it makes
  itself (`isApplying`). Never call `NSTextView.mouseDown` from a test without a queued mouse-up:
  it tracks until the button comes up

### Script Manager preview
- The Script Manager window is a `SplitContainer` (sidebar | `ScriptWorkspaceView`); the
  workspace splits the editor and `ScriptPreviewPane` left/top/bottom. `SplitLayout` is a real
  `Layout`, not an `HStack`/`VStack` switch, so the editor `NSTextView` keeps its cursor and undo
  stack when the chart moves — keep new panes inside it rather than branching on the axis
- `ScriptPreviewViewModel` (owned by `ScriptManagerView`, outside the per-script `.id`) drives one
  standalone `ChartViewModel`: market, timeframe, 5 s refresh, `ChartLiveFeed` streams, occlusion
  gating, scroll-zoom (`ScrollZoomMonitor`) and price-axis drag (`PriceAxisDragMonitor`, shared with
  `ContentViewModel`). It never attaches `PineAlertCoordinator`, so a preview cannot raise alerts. Markets are
  crypto and stock only (`PreviewMarket.isSupported`)
- `PineInputsView` is the one `input.*` → controls renderer, shared with `ChartSettingsSheet`;
  it takes a precompiled `PineInputSchema` — never compile inside a view body

### Example scripts
- `DegenView/Resources/DemoScripts/*.pine` ship in the app's Resources phase (each one is a
  `project.pbxproj` file reference + build file, like any new file). `DemoScriptLibrary` copies them
  into `ScriptStore` — once at launch (`scripts.demosSeeded`, skipped under XCTest) and on demand from
  the Script Manager's empty state — so a demo the user deletes stays deleted. The file name is the script name
  (no "Demo" prefix — the word never appears in user-visible script names or titles)
- Demos are original work only (the repo is GPL-3.0; never copy community scripts) and must compile with **no
  diagnostics**, warnings included: `PineExampleScriptsTests` compiles and runs every bundled file

### WebSocket updates
- `ChartLiveFeed` owns the three socket services and routes ticks to charts; `ContentViewModel`
  and the Script Manager preview each hold one
- `BinanceWebSocketService.connect(symbols:interval:)` opens one combined stream
- Callback dispatches to matching `ChartViewModel.applyKlineUpdate(_:)`
- `applyKlineUpdate` updates last candle in-place (no full refetch)
- **Coinbase has no candle channel** (`candles is not a valid channel`). `CoinbaseWebSocketService`
  subscribes to `ticker` + `heartbeat` and emits one `CoinbaseTick` per trade;
  `ChartViewModel.applyTick` folds it into the last candle (REST supplies the open), flags the
  old candle `isClosed` on rollover, and the 5 s REST refresh reconciles volume. The socket does
  not depend on the chart interval
- Coinbase REST only serves 1m/5m/15m/1h/6h/1d, so `1w` and `1M` are folded from daily candles
  (`CoinbaseGranularity`, bucket boundaries from `KlineData.bucketStart`). One request spans at
  most 300 candles — `CoinbaseAPIService` pages backwards, keeps the merged source candles so a
  refresh is one request, and remembers when a listing's history is exhausted. Coinbase reports
  no turnover, so `quoteVolume` is volume priced at the OHLC average (live: size × price)
- **Search order is priority, not alphabetical**: `DataSourceFactory.allSources` is
  Binance → Coinbase → the rest, and `TickerSearchViewModel.orderedSources` keeps that order
  (sources with results first). A new crypto source goes in that list at its intended rank

### Watchlists
- `docs/watchlists.md` has the behaviour. `WatchlistStore` is the one source of truth (it replaced
  `FavoritesStore`): `watchlist` document table, one row per list with its flat ordered `entries`
  (instrument or section). Mutations validate, persist in one transaction, then publish. A list that
  fails to read sets `loadFailed` and every mutation throws — never write an empty list over data that
  did not load. Favorites live on as the list with `isFavorites` (the chart-card star); old `favorite`
  rows migrate once and are left untouched
- Identity is `InstrumentID` (source + the provider's own id; `chain` is metadata, not identity). Never
  key a watchlist by ticker text. Drawings, alerts and paper trading keep their own key formats; do not
  add another. CoinMarketCap cards are not instruments
- Flags are global per market (`setting` key `watchlist.flags`), not per list. Sorting, filtering and
  the selected list never change the stored order; `WatchlistLayoutEngine` is pure and sorts inside
  each section. Per-window state (selected list, filter, highlighted row) is `WatchlistSidebarViewModel`
- Quotes are transient: `WatchlistQuoteBook` (one observable cell per market, so a tick redraws one row)
  fed by `WatchlistQuoteCoordinator`, which merges every sidebar's needs by `InstrumentID`, makes one
  batched request per source, shares one Coinbase socket, and backs off per source. It never writes to
  the database and does not use `MarketQuoteCoordinator` (alerts own that). Provider quote calls return
  `SourceQuote`, which the agent target also compiles: keep those edits Foundation-only. A new quote
  field goes through `WatchlistQuote(source:quote:receivedAt:)`; a missing reference is nil, never zero
- Sections are flat headings (`WatchlistSectionRow`): never indent the symbols under them. Every part of the
  panel uses `WatchlistMetrics.leadingInset` (8) and `trailingInset` (12), so logos, section titles and column heads share one left edge and
  values and chevrons one right edge. Headings and the empty-section placeholder are not selectable
  (`.selectionDisabled()`); only instrument rows carry a `.tag`, and it is the entry's `UUID`
- A flag is a small bookmark on its side (`WatchlistFlagMark`), drawn as the row's `listRowBackground`
  (`WatchlistFlagGutter`) so it spans the full row and touches the panel's left border; never inline in the row
  content, which starts inside the list's own side padding (8pt left, 9pt right, `WatchlistMetrics.listCell*`,
  checked by `WatchlistListMetricsTests` against a hosted sidebar). Rows get `rowLeadingInset`/`rowTrailingInset`
  so content lands on `leadingInset` (8) / `trailingInset` (12): logos, section titles and column heads share the
  left edge, values and column titles the right. The bookmark is 6pt wide, so it ends 2pt short of the logo. Menu
  swatches are non-template `NSImage`s (`WatchlistFlag.swatch`) so AppKit keeps the colour
- Watchlist reordering is the List's native `ForEach.onMove` over the flat rows (off while sorted or filtered).
  `WatchlistLayoutEngine.resolveMove` maps the drop onto stored entries and `WatchlistStore.moveEntries` writes
  it once. Do not add SwiftUI `onDrag`/`onDrop`/`onTapGesture` to rows: layered over the List's own table
  handling they made dragging fail most of the time. Selecting a row opens it in the focused chart and the
  highlighted row mirrors the focused chart's market (`syncSelection`); only instrument rows are tagged/selectable
- The watchlist is the secondary pane of a `SplitContainer` (drag its edge, double-click to reset); the
  width is the global `watchlistSidebarLength` default, clamped between `WatchlistSidebarViewModel.minimumWidth`
  (every column plus room for a name) and 640. The pane stays built while hidden, so the sidebar is told
  it is inactive then and must not request prices
- Chart cards measure their plot (`GeometryReader` under the header) instead of assuming a chrome height,
  and the grid's end-of-column drop zone is a column background, never a spacer: anything in a column
  with a minimum height the `ChartLayout` sizing doesn't count overflows the grid, and SwiftUI re-centres
  the overflow, pushing the first row's top edge out of view
- Each tab has a transient `focusedChartID` (not in `LayoutSnapshot`). A row click calls
  `ContentViewModel.openWatchlistInstrument`: a market already on screen takes focus, otherwise the
  focused chart switches via `ChartViewModel.updateTicker`, the one safe in-place switch (cancels the
  fetch, bumps the generation, clears selections, drafts and Pine results, reloads that market's
  drawings). `uniqueID` follows the market; `chartID` is the identity. Do not mutate `ticker` any other way
- Tests never touch the real database: `ContentViewModel(tabID:savedViews:tabs:)` takes in-memory
  stores and `ChartViewModel.serviceResolver` stubs the provider a switch refetches from

### Tabs and windows
- Each tab is a real `NSWindow` in a tab group, rendering one `ContentView` +
  `ContentViewModel` keyed by a `ChartTab.id`. The scene is `WindowGroup(for: UUID.self)`,
  so a window carries its tab id as its scene value
- **AppKit owns all tab dragging.** Reorder, drag-out-to-detach, drag-window-onto-tab-bar
  to merge, the `+` button, and the Window-menu tab items are free from
  `tabbingIdentifier` + `tabbingMode = .preferred`. Do not reimplement any of it
- `WindowCoordinator` only (a) joins a newly opened window to the right tab group via
  `addTabbedWindow` and (b) reads the arrangement back out with `captureGrouping()` on
  quit. `NSWindow.tabGroup` is the authority on layout — nothing is tracked incrementally
- **`AppDelegate.newWindowForTab(_:)` is load-bearing beyond the `+` button.** AppKit
  refuses to *draw* the tab bar for a single-tab window unless something in the responder
  chain answers that selector — `toggleTabBar` will report `isTabBarVisible == true` and
  the bar still won't appear. Deleting that method silently hides the bar (and the `+`)
  whenever a window is down to one tab
- A window SwiftUI opens with **no scene value** (the `+` button, File ▸ New Window) goes
  through `tabForUnvaluedWindow()`. Only the launch window adopts the persisted session;
  every later one mints a blank tab. Resolving nil to `firstTabID()` unconditionally opens
  a second window onto a tab that is already on screen
- That resolution lives in a `StateObject` (`ResolvedTab`), not `State(initialValue:)` —
  it can create a tab, and a plain `State` autoclosure re-runs that side effect on every
  re-init of the enclosing view even though only the first value is kept
- The tab label **is** `window.title`. `ContentViewModel.tabName` drives it through
  `navigationTitle` plus an explicit `WindowCoordinator.syncTitle`
- The tab's icon (charts / portfolio / script manager) is **not** drawn by us: `WindowTabIcon` puts an
  SF Symbol attachment in `window.tab.attributedTitle`, and `WindowTabDecorator` re-applies it after
  a title change, when the window joins or leaves a tab group (one run-loop turn later — the bar
  ignores a label set before its items exist) and from `refreshTabBars()`. Anything that rebuilds
  tabs or titles must leave that path intact. The system draws the tab shape; its corner radius
  isn't customisable.
- `WindowAccessor` is how a view gets its `NSWindow`. Three things need it: tab-group
  registration, scoping the scroll monitor, and occlusion gating
- Restore is ours, not AppKit's: windows are `isRestorable = false` and rebuilt from
  the persisted `TabsStore` session by `restoreWindows(adopted:using:)`
- `ContentView` keeps `.toolbar` and `.navigationTitle` outside the empty/non-empty
  branch — an empty new tab still needs both

### Anything app-wide must be scoped per tab
Three things broke when a second instance appeared — check for this shape when adding state:
- `NSEvent` local monitors see the **whole app**. The scroll-zoom monitor gates on
  `event.window === ownWindow` or every open tab zooms at once
- `CoinGeckoAPIService.ActiveSymbols` is keyed by tab id and returns a **union**. A flat
  list made the last tab to sync evict the others from the batched prime
- The 5 s refresh timer and the WebSocket follow `NSWindow.occlusionState`. Hidden tabs
  suspend, so API load doesn't scale with tab count

### State management
- Saved layouts: `SavedViewStore.shared` owns the library (every tab writes through it, writes throw,
  `lastOpenedAt` drives "Recently used"); `ContentViewModel.layout` (`SavedLayoutController`) is the
  tab's active layout. **Dirty is a fingerprint, not a flag**: `syncTab()` calls `layout.refresh()`,
  which compares a `LayoutSnapshot` (timeframe, chart configs, column membership — not zoom,
  drawings, replay or market data) with the snapshot taken at open/save. Layouts are applied only via
  the controller's `restore`, which suspends tracking and re-baselines. Autosave is per layout
  (`SavedView.autosave`), debounced 2 s, never applies to Unnamed, and is flushed before a switch
- `tabName` is always the active layout's name or `UI.unnamedView`; there is no tab-only rename
- `isHydrating` guards `syncTab()` during `init` — a `didSet` that reached it would otherwise
  write the tab back before `chartViewModels` is populated and erase it
- `syncTab()` writes the whole `ChartTab` back; `TabsStore` debounces the database write
- `@AppStorage("appTheme")` for theme preference

## File naming

- One type per file, filename = type name
- Services: `*Service.swift` or `*APIService.swift`
- ViewModels: `*ViewModel.swift`
- Views: `*View.swift` or `*Sheet.swift` for sheets
- Stores: `*Store.swift`

## Testing

The `DegenViewTests` unit test target covers market-data parsing and caching, indicators,
chart plotting and fetching, WebSocket updates, JSON persistence, replay, local price
alerts, portfolio accounting, and paper trading. Run it with:

```bash
xcodebuild test \
  -project DegenView.xcodeproj \
  -scheme DegenView \
  -destination 'platform=macOS'
```

`PineCorpusTests` runs cached community scripts (`tools/pine-corpus/fetch.py` fills the
gitignored `.pine-corpus/`) and skips without them. Third-party sources are never committed or
copied into tests; reproduce a failure with an original reduction in `PineRegressionTests`, then
update `DegenViewTests/PineCorpus/expectations.json`. See `tools/pine-corpus/README.md`.

Tests never touch Keychain: `KeychainPolicy.isDisabled` is true under XCTest, so the Alpaca and
CoinMarketCap stores report "nothing saved". When an agent launches the app itself, set
`DEGENVIEW_NO_KEYCHAIN=1` in the environment — otherwise a rebuilt, re-signed binary blocks on a
Keychain access prompt.

### Linting

- Every Swift file created or modified in a task must be linted before handoff, including
  new files that are still untracked by Git.
- Use Xcode's bundled formatter in non-mutating strict lint mode:
  `xcrun swift-format lint --strict <changed Swift files>`.
- Fix every diagnostic in the changed files. Do not run a repository-wide in-place
  formatter, which can rewrite unrelated user code.

Use the following manual flow for native window/tab behavior and end-to-end UI checks:

1. Launch app, add BTC from Binance, then BTC/USD from Coinbase (live ticks should move its last candle)
2. Add same symbol from CoinGecko (different source, no duplicate rejection)
3. Switch timeframes, toggle log scale
4. Scroll-zoom on chart, verify candle count changes
5. Layout button reads "Unnamed"; ⌘S names it. Add a ticker — an "(unsaved)" tag appears after the
   name; scroll-zoom and live ticks never raise it; ⌘S clears it
6. Layout ▸ Open layout… / Recently used loads a saved layout; with changes pending it asks
   Save / Don't Save / Cancel. Autosave on: change a setting, "Save" never shows, change persists
   after relaunch. Make a copy keeps unsaved changes; Rename keeps the tab title in sync in every
   tab; Create new layout opens a blank tab. ⌘S in the Script Manager still saves the script
7. Add a DEX pair (e.g. search "BONK" on DEXScreener)
8. With one tab open, confirm the tab bar and its `+` are still visible
9. Both ⌘T and the tab bar's `+` open an empty tab named "Unnamed" — never a second view
   onto an existing tab — with the toolbar present and every saved view listed for
   one-click loading
10. Give the two tabs different timeframes; confirm neither follows the other,
   and that scrolling one doesn't zoom the other
11. Drag a tab out to detach it, then put it back with File ▸ Merge All Windows or by
    dragging the window onto a tab bar. Confirm the tab bar survives both, at one tab
12. Quit and relaunch — same tabs, same order, same window grouping
13. Arm the ruler, press-drag-release a rectangle up (green) and down (red), and right to left;
    check the card's percent against the price axis tags and its bar count against the
    candles inside. A click-move-click rectangle works too. Hover a corner or edge: handles
    and the cursor change; drag a corner to resize and an edge to move, and start a second
    rectangle inside the first. Delete removes the selected one, Esc cancels a draft, then
    clears all, then disarms. Switching tool or timeframe drops them, and they never come
    back after a relaunch
13d. Brush: arm it and drag a circle, a V and a zigzag — corners stay sharp, curves smooth, and
    the stroke follows the pointer with no snapping to candles. A click leaves a dot. Esc
    mid-stroke leaves nothing and ⌘Z has nothing to undo. ⌘Z removes a whole stroke, ⇧⌘Z
    brings it back. Switch timeframe, scroll-zoom, drag the price axis and resize the window:
    the stroke stays on the same price action. Hover shows a halo and the move cursor; click
    opens the editor (colour, width, opacity, lock), drag moves the stroke (one undo step),
    Delete removes it. Relaunch keeps strokes; the same symbol on another source has none
13a. Portfolio and Script Manager buttons sit in the title bar of a chart tab, the portfolio tab
    and the Script Manager tab. Each focuses the existing tab (never a duplicate); from the
    Script Manager tab, Portfolio opens inside the same tab group. "Add Chart" is its own
    labelled bubble, apart from the watchlist toggle
13b. Alerts window (bell in the tool strip): tabs Active / Triggered / Paused / All / History / Scripts
    with counts, each row showing the coin artwork with its source logo. Search matches symbol and name;
    a search with no hits says so, an empty tab explains itself. Hover a rule for edit and pause/resume;
    Delete and Clear History ask first. History groups by day; prices read `$67,432.19`, `$1.2346`,
    `$0.00000278` — never a long decimal tail — in rows, the banner and the macOS notification
    The tab bar spans the full width. A fired alert (price or script) puts a red count on the sidebar
    bell, also for ones the agent fired while the app was closed; opening the window clears it and
    relaunch keeps it clear; alerts firing while the window is key never show a bubble
13c. Replay: toolbar Replay ▸ Select Bar on Chart shows the "Click a candle" strip; the marker
    follows the pointer with a date tag and Esc / Cancel exit. Click a candle: strip shows
    Paused, the card gets an orange border and "Replay" badge. Play / pause / step ⇧→ ⇧← ⇧↓,
    drag and click the timeline (pauses, tick marks the start), speed and Bars menus (Auto
    shows the resolved interval, a spinner while loading). Play to the end: "Ended", the play
    button restarts. A CoinGecko chart shows a dismissible fallback notice. **Live** returns to
    latest. Choose Date & Time… presets and "Snaps to" preview; relaunch mid-replay restores
    paused at the same spot; narrow window wraps the timeline to a second row; light and dark
13e. Webhooks: Settings ▸ Webhooks, add a local receiver (`python3 -m http.server`, or `nc -l 8080` for the
    raw request; add a secret and `?token={{secret}}` via Use secret, and a Bearer header: the receiver sees the
    percent-encoded token in the query and the raw one in `Authorization`; Headers starts collapsed). Test sends `{"event":"DegenView webhook test"}` as application/json; change the test message to
    plain text and it goes as text/plain. A disabled webhook still tests. A price alert with two
    webhooks (one dead) shows `Webhooks 1/2 delivered`; deleting a used webhook asks first with the
    count. A script alert posts once per admitted `alert()`, and loading a script over history posts nothing
14a. Script editor: type `ta.sma(` → `()`; `)` steps over it; Backspace in `()` removes both; select
    text and type `(` / `"`; one ⌘Z undoes each. Return after `if x` indents; ⌘/, Tab/⇧Tab on a
    multi-line selection, ⌥↑↓ and ⇧⌥↓ work and each undoes in one step. Caret beside a bracket
    tints its partner; guides and the current-line band follow scrolling and don't eat clicks
14b. Completion: type `plot(ta.rs` — the list opens under the caret with `rsi` and its signature;
    Down/Up move, Tab or Return accept (`ta.rsi(|)`, one ⌘Z undoes it), Esc closes the list. `ta.`
    lists only `ta` members; `plot(clo` offers `close`; a variable and a function you declare appear
    (parameters first inside their function); `ta.sma(close, ` shows the signature with `length`
    underlined; `// ta.rs` and `"ta.rs"` stay quiet; Control-Space / Option-Esc complete with nothing
    typed; clicking a row selects it, double-click accepts, the caret never moves; scrolling, a
    window resize, clicking into the text, switching scripts and focus loss close the popup; with
    the list closed Return and Tab still indent. In an `import user/` line the library names appear,
    and `lib.` lists a library's exports
14. Script Manager: collapse the sidebar (⌃⌘S) and relaunch — it stays collapsed. Open an
    indicator: the preview chart appears left of the code. Type — the plot updates after a pause;
    break the syntax — the banner appears and the last plot stays. Move the chart left/top/bottom
    without losing the editor's cursor. Change an input, relaunch, reopen the script — the value
    is back. The market picker offers only Crypto and Stock. A favorited script highlights one sidebar row
    (whichever you click), right-click ▸ Duplicate makes "<name> copy" and selects it, and the
    "Community scripts" link sits under the list

15. Watchlists: relaunch keeps your old favorites in "Favorites", in order. Create "Crypto" from the
    selector, add BINANCE:BTCUSDT, BINANCE:ETHUSDT and COINBASE:BTC-USD from the + sheet (it stays open,
    Recents don't grow). Add sections Majors and Watching; drag symbols between them and a section by its
    header; the insertion line shows where it lands and nothing is written until you drop. Click Chg%
    to sort, again to flip, again for Manual — the manual order returns. Filter "btc" and the flag filter;
    drag is off while either is active. Columns menu adds Vol and the panel widens. Prices and Chg%
    appear without any chart for them; hide the sidebar or minimise the window and requests stop
16. Watchlist and charts: with two chart cards, click one (a faint accent ring shows the focused card),
    click ETHUSDT — only that card switches, drawings follow their own market (draw on BTC, switch to ETH
    and back), price alerts and paper positions are unchanged, and the layout reads "(unsaved)". Right-click
    BTCUSDT ▸ Add as New Chart adds a card; a market already shown just takes focus. Open in New Tab keeps the
    old behaviour. Right-click the chart-card star and a search result in Add Chart for Add to Watchlist
17. Watchlists across windows: open a second window; rename or reorder a list in one and the other
    follows, while each keeps its own selected list. Quit and relaunch: lists, sections, order, flags
    and columns are back. ⋯ ▸ Export to File and Import Symbols round-trip, with a summary of skipped rows

Adding a new `.swift` file means four hand-edits to `project.pbxproj` (`PBXBuildFile`,
`PBXFileReference`, the group's `children`, the `Sources` phase). The project does not use
synchronized folder groups.
