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

## Code conventions

- **MVVM**: `Model/` — data + enums, `ViewModel/` — `@ObservableObject` state, `View/` — SwiftUI views
- **@MainActor** on all ViewModels that publish UI state
- **No SwiftUI Charts** — candles are hand-drawn via AppKit `Canvas`
- **Protocol abstraction** for data sources: `TickerDataSource` protocol, `DataSourceFactory` singleton
- **Persistence**: user data (tabs, saved views, favorites, drawings, portfolios, paper
  trading, alerts) lives in SQLite (`degenview.sqlite`, WAL) through `AppDatabase`, shared
  with the alert agent. Small caches stay in `JSONStore<T>`; closed daily candles for
  portfolio history are the exception — large, append-mostly, read by range — and live in the
  `candle`/`candle_coverage` tables via `PortfolioCandleStore`. The schema is one
  `AppDatabase.createSchema` in `AppDatabase+Schema.swift` (`IF NOT EXISTS`); there are no
  versioned migrations and no readers for older data — this is the first version
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
- `ContentViewModel.markChanged()` sets `hasUnsavedChanges = true` (skipped during `loadView`)
- `isApplyingView` flag prevents false unsaved-change detection on view load
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
5. Save view, add a ticker, verify unsaved changes indicator
6. Load saved view, verify state restores
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
13. Arm the ruler, drag a rectangle up (green) and down (red); check the read-out's percent
    against the price axis and its bar count against the candles inside. One more click
    puts it away — on that chart only. Switching tool or timeframe drops it, and it never
    comes back after a relaunch
13a. Portfolio and Script Manager buttons sit in the title bar of a chart tab, the portfolio tab
    and the Script Manager tab. Each focuses the existing tab (never a duplicate); from the
    Script Manager tab, Portfolio opens inside the same tab group. "Add Chart" is its own
    labelled bubble, apart from the Favorites button
14a. Script editor: type `ta.sma(` → `()`; `)` steps over it; Backspace in `()` removes both; select
    text and type `(` / `"`; one ⌘Z undoes each. Return after `if x` indents; ⌘/, Tab/⇧Tab on a
    multi-line selection, ⌥↑↓ and ⇧⌥↓ work and each undoes in one step. Caret beside a bracket
    tints its partner; guides and the current-line band follow scrolling and don't eat clicks
14. Script Manager: collapse the sidebar (⌃⌘S) and relaunch — it stays collapsed. Open an
    indicator: the preview chart appears left of the code. Type — the plot updates after a pause;
    break the syntax — the banner appears and the last plot stays. Move the chart left/top/bottom
    without losing the editor's cursor. Change an input, relaunch, reopen the script — the value
    is back. The market picker offers only Crypto and Stock

Adding a new `.swift` file means four hand-edits to `project.pbxproj` (`PBXBuildFile`,
`PBXFileReference`, the group's `children`, the `Sources` phase). The project does not use
synchronized folder groups.
