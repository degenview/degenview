# AGENTS.md — CryptoCharts

Guidance for AI coding agents working in this repo.

## Project

macOS crypto candlestick chart app. SwiftUI views, AppKit Canvas rendering, REST + WebSocket data from Binance/CoinGecko/DEXScreener. Zero external dependencies.

## Build

```bash
open CryptoCharts/CryptoCharts.xcodeproj
# ⌘R in Xcode, or: xcodebuild -project CryptoCharts/CryptoCharts.xcodeproj -scheme CryptoCharts build
```

Requires Xcode 16+, macOS 14+.

## Code conventions

- **MVVM**: `Model/` — data + enums, `ViewModel/` — `@ObservableObject` state, `View/` — SwiftUI views
- **@MainActor** on all ViewModels that publish UI state
- **No SwiftUI Charts** — candles are hand-drawn via AppKit `Canvas`
- **Protocol abstraction** for data sources: `TickerDataSource` protocol, `DataSourceFactory` singleton
- **Persistence**: `TabsStore` → `tabs.json`, saved views → `views.json`, both via the
  generic `JSONStore<T>` in the app support dir
- **One `ContentViewModel` per tab** — never treat it as app-global state
- **Caching**: `ChartViewModel.fetchData` caches results keyed by (symbol, interval, limit) in a dictionary
- **WebSocket**: Only Binance tickers get live streams; connect/disconnect on ticker add/remove

## Key patterns

### Adding a new data source
1. Add case to `DataSourceType` enum
2. Create service class conforming to `TickerDataSource`
3. Register in `DataSourceFactory.service(for:)` and `allSources`
4. Add kline parser init in `KlineData` if API format differs
5. Update `AddTickerSheet` search to include new source

### Adding a new timeframe
1. Add case to `TimeRange` enum
2. Set `binanceInterval`, `dataPointLimit`, `chartTitle`, `dateFormat`
3. If Binance doesn't support the interval natively, pick closest and adjust `dataPointLimit`

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
- `nil` is not a failure state for the UI: `TickerIconView` draws a monogram, so the
  20×20 slot is occupied either way and card headers stay aligned
- Icon lookups key off `ChartViewModel.iconKey`, never `uniqueID` — `uniqueID` survives
  `updateTicker` by design and would pin the old coin's artwork to a renamed card

### WebSocket updates
- `BinanceWebSocketService.connect(symbols:interval:)` opens one combined stream
- Callback dispatches to matching `ChartViewModel.applyKlineUpdate(_:)`
- `applyKlineUpdate` updates last candle in-place (no full refetch)

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
- `WindowAccessor` is how a view gets its `NSWindow`. Three things need it: tab-group
  registration, scoping the scroll monitor, and occlusion gating
- Restore is ours, not AppKit's: windows are `isRestorable = false` and rebuilt from
  `tabs.json` by `restoreWindows(adopted:using:)`
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
- `isHydrating` guards `syncTab()` during `init` — `layoutMode`'s `didSet` would otherwise
  write the tab back before `chartViewModels` is populated and erase it
- `syncTab()` writes the whole `ChartTab` back; `TabsStore` debounces the file write
- `@AppStorage("appTheme")` for theme preference
- Legacy `tickers.json` + `UserDefaults "lastViewID"` are read once by
  `TabsStore.migrateFromLegacyStorage()` and never written again

## File naming

- One type per file, filename = type name
- Services: `*Service.swift` or `*APIService.swift`
- ViewModels: `*ViewModel.swift`
- Views: `*View.swift` or `*Sheet.swift` for sheets
- Stores: `*Store.swift`

## Testing

No test target yet. Manual testing flow:
1. Launch app, add BTC from Binance
2. Add same symbol from CoinGecko (different source, no duplicate rejection)
3. Switch timeframes, toggle log scale, switch layout
4. Scroll-zoom on chart, verify candle count changes
5. Save view, add a ticker, verify unsaved changes indicator
6. Load saved view, verify state restores
7. Add a DEX pair (e.g. search "BONK" on DEXScreener)
8. With one tab open, confirm the tab bar and its `+` are still visible
9. Both ⌘T and the tab bar's `+` open an empty tab named "Unnamed" — never a second view
   onto an existing tab — with the toolbar present and every saved view listed for
   one-click loading
9. Give the two tabs different timeframes and layouts; confirm neither follows the other,
   and that scrolling one doesn't zoom the other
10. Drag a tab out to detach it, then put it back with File ▸ Merge All Windows or by
    dragging the window onto a tab bar. Confirm the tab bar survives both, at one tab
11. Quit and relaunch — same tabs, same order, same window grouping

Adding a new `.swift` file means four hand-edits to `project.pbxproj` (`PBXBuildFile`,
`PBXFileReference`, the group's `children`, the `Sources` phase). The project does not use
synchronized folder groups.
