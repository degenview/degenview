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
- **Persistence**: `TickerStore` → UserDefaults, `ViewStore` → JSON file in app support dir
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

### State management
- `ContentViewModel.markChanged()` sets `hasUnsavedChanges = true` (skipped during `loadView`)
- `isApplyingView` flag prevents false unsaved-change detection on view load
- `@AppStorage("appTheme")` for theme preference
- `UserDefaults` for `lastViewID` to restore session on relaunch

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
