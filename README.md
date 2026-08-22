# DegenView

macOS crypto candlestick chart app. SwiftUI + AppKit Canvas.

## Features

- **Tabs** — Each tab holds its own charts, timeframe, layout, and zoom. Native macOS
  window tabs: drag to reorder, drag a tab out to detach it into its own window, drag a
  window back onto a tab bar to merge. The tab bar and its **+** stay visible even at one
  tab; ⌘T also opens one
- **Candlestick charts** — Open/High/Low/Close with wicks, doji detection, colored bodies
- **3 data sources** — Binance, CoinGecko, DEXScreener (Solana/ETH pairs)
- **Real-time updates** — Binance WebSocket streams for live price data
- **6 timeframes** — 1H, 1D, 1W, 1M, 3M, 1Y
- **Scroll-to-zoom** — Adjust candle count via mouse scroll wheel
- **2 layouts** — Vertical list (drag reorder) or 2-column grid (drag-and-drop reorder)
- **Saved views** — Name and persist ticker sets, timeframe, layout, candle count. Shared
  across tabs; loading one into a tab renames that tab
- **Unsaved changes tracking** — Auto-detects changes; a Save Changes button appears in the toolbar
- **Crypto icons** — CoinGecko icon lookup by base symbol
- **Theme support** — System / Light / Dark
- **Multi-source search** — Add tickers from any data source with duplicate detection per source

## Architecture

```
DegenView/
├── DegenViewApp.swift                 # App entry, tab scene, New Tab command, app delegate
├── ContentView.swift                  # One tab's view: toolbar, theme, drag-drop delegates
├── Model/
│   ├── KlineData.swift                # OHLCV data model, Binance + CoinGecko parsers
│   ├── TimeRange.swift                # Timeframe enum (1H-1Y), interval mapping
│   ├── DataSourceType.swift           # Binance/CoinGecko/DEXScreener enum + TickerConfig
│   ├── SavedView.swift                # Persisted view state (tickers, timeframe, layout)
│   ├── ChartTab.swift                 # One tab's persisted state + the tabs.json snapshot
│   └── SymbolInfo.swift               # Search result metadata
├── ViewModel/
│   ├── ContentViewModel.swift         # Per-tab state: tickers, timeframe, views, WebSocket, auto-refresh
│   └── ChartViewModel.swift           # Per-chart state: fetch, cache, price tracking
├── View/
│   ├── ChartCardView.swift            # Single chart card: header, icon, price, chart, errors
│   ├── WindowAccessor.swift           # Hands the hosting NSWindow to SwiftUI
│   ├── CandleChartView.swift          # AppKit Canvas renderer: candles, grid, price line, axis
│   ├── CandleChartStyle.swift         # Colors, sizing, doji threshold config
│   ├── AddTickerSheet.swift           # Multi-source ticker search sheet
│   ├── TickerIconView.swift           # Coin icon slot with letter-monogram fallback
│   ├── EmptyStateView.swift           # Empty state with "Add Ticker" prompt
│   ├── ScrollWheelView.swift          # Scroll wheel zoom wrapper
│   └── TimeRangePicker.swift          # Timeframe segmented picker
└── Service/
    ├── BinanceAPIService.swift        # Binance REST API + kline fetching
    ├── BinanceWebSocketService.swift  # Binance WebSocket streams
    ├── CoinGeckoAPIService.swift      # CoinGecko OHLC API
    ├── CGRateLimiter.swift            # Shared CoinGecko public-tier pacing (OHLC + icons)
    ├── IconResolver.swift             # Coin icons: multi-source fallback chain + cache
    ├── DEXScreenerService.swift       # DEXScreener pairs API
    ├── TickerDataSource.swift         # Protocol + factory for all 3 sources
    ├── TabsStore.swift                # Every tab's state + window grouping (tabs.json)
    ├── WindowCoordinator.swift        # NSWindow tabbing: joins, grouping capture, restore
    └── JSONStore.swift                # Generic Codable-to-JSON persistence
```

## Data Flow

1. Each tab is a real `NSWindow` in a tab group, owning one `ContentViewModel` (ticker
   list, timeframe, layout, candle count). Only the visible tab polls — hidden tabs stop
   their refresh timer and WebSocket, so API load stays flat as tabs are added
2. Each ticker gets a `ChartViewModel` that fetches OHLC data from its source API
3. `CandleChartView` renders via AppKit `Canvas` — custom draw loop, no third-party chart lib
4. Binance tickers open WebSocket streams for real-time price updates
5. Auto-refresh timer (5s) re-fetches all charts; API responses are cached per (symbol, interval, limit) key
6. Icons resolved async per chart card by `IconResolver`, which walks a fallback chain
   (market snapshot → source-specific lookup → CoinGecko search → static icon set) and
   caches hits *and* misses; anything unresolved renders as a letter monogram

## Requirements

- macOS 14+ (Sonoma)
- Xcode 16+
- Swift 6

## Build & Run

```bash
open DegenView.xcodeproj
```

Then **Product → Run** (⌘R) in Xcode.

No external dependencies — all SwiftUI + AppKit native APIs. No CocoaPods, SPM, or Carthage.

## Usage

1. Press **⌘T** (or the **+** in the tab bar) for a new tab. It lists your saved views for
   one-click loading; rename it from the folder menu's **Rename Tab…**
2. Click **+** to search and add tickers from Binance, CoinGecko, or DEXScreener
3. Toggle **timeframe** in the toolbar (1H–1Y)
4. Toggle **layout** between vertical list and 2-column grid
5. **Scroll** over a chart to zoom (adjust candle count)
6. Drag to **reorder** tickers
7. **Save** current view (name + state) with the save button
8. **Load** saved views from the folder menu — the tab takes the view's name
9. Drag a tab out of the tab bar to **detach** it into its own window. To put windows back
   together, drag a window onto another window's tab bar, or use **File → Merge All Windows**
