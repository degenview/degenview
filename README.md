# DegenView

A native macOS market dashboard for watching crypto, stocks, and prediction markets in
customizable candlestick and line charts. Built with SwiftUI and an AppKit `Canvas`, with
no external dependencies.

![DegenView showing crypto, stock, and prediction-market charts](resources/screenshot.png)

## Features

### Markets and live data

- **Crypto** — Search Binance, CoinGecko, and DEXScreener side by side. DEX pairs use
  GeckoTerminal for historical OHLCV data
- **Stocks** — Search and chart US equities through Alpaca's IEX feed (API credentials
  are stored securely in Keychain)
- **Prediction markets** — Search Polymarket events, chart probabilities as percentages,
  and toggle the outcome series shown for multi-outcome markets
- **Live updates** — Binance crypto and Alpaca stock charts receive WebSocket updates;
  other sources refresh automatically
- **Six timeframes** — 1H, 1D, 1W, 1M, 3M, and 1Y, with scroll-wheel zoom to change the
  visible candle count
- **Multi-source identity** — The same symbol can be added from different sources without
  being treated as a duplicate

### Charts and analysis

- **Native chart renderer** — Hand-drawn OHLC candlesticks, wicks, doji candles, line
  series, grid, time axis, adaptive price precision, current-price overlay, and price
  change readout; no SwiftUI Charts or third-party chart library
- **Indicators** — Per-chart volume, RSI (14), configurable EMA, Bollinger Bands, and
  confirmed bullish/bearish Supertrend flip markers
- **Per-chart appearance** — Custom bullish and bearish colors plus automatic or fixed
  Y-axis decimal precision
- **Independent price zoom** — Drag a chart's Y-axis to adjust its vertical scale
- **Crosshair** — Synchronized vertical inspection across all charts in a tab, with the
  hovered chart's price and time readout
- **Trend lines** — Draw, select, move, recolor, resize, and delete lines. Drawings are
  anchored to time and price and shared by every chart of the same instrument
- **Ruler** — Measure a move's price change, percentage, duration, and bar count with a
  green/red rectangle; measurements are intentionally temporary

### Dashboard and persistence

- **Native macOS tabs and windows** — Every tab owns independent charts, timeframe,
  layout, zoom, refresh lifecycle, and drawing-tool state. Drag tabs to reorder or detach,
  drag windows onto a tab bar to merge, or use **File → Merge All Windows**
- **Restored sessions** — Tabs, ordering, names, window grouping, and window frames return
  on relaunch. The tab bar and its **+** remain visible even with one tab
- **Two layouts** — A vertical chart stack or responsive two-column grid, both with
  drag-and-drop ticker reordering
- **Saved views** — Save and reload named ticker sets with their timeframe, layout, zoom,
  source, indicator, and appearance settings. Saved views are shared across tabs
- **Unsaved-change tracking** — A contextual toolbar action appears when a loaded view has
  changed
- **Favorites sidebar** — Keep an app-wide, persistent, reorderable watchlist and open any
  favorite in the current tab
- **Coin and company artwork** — Multi-stage icon lookup with disk caching and a monogram
  fallback, so every card header remains aligned
- **Appearance** — System, Light, and Dark themes
- **Efficient background behavior** — Hidden tabs suspend polling and live streams;
  responses and rate-limited CoinGecko data are cached

## Requirements

- macOS 14 or later
- Xcode 16 or later
- Swift 6
- A free Alpaca account and API keys only if you want stock data

## Build and run

```bash
open DegenView.xcodeproj
```

Then choose **Product → Run** (⌘R) in Xcode. You can also build from Terminal:

```bash
xcodebuild -project DegenView.xcodeproj -scheme DegenView build
```

DegenView uses only native SwiftUI, AppKit, URLSession, and WebSocket APIs—there are no
CocoaPods, Swift Package Manager, or Carthage dependencies.

## Usage

1. Click the toolbar **+** and choose Crypto, Stocks, or Polymarket. Crypto search fans
   out across Binance, CoinGecko, and DEXScreener; stock search requires Alpaca keys in
   **Settings → Alpaca**.
2. Pick a timeframe in the toolbar and scroll over a chart to zoom its history. Drag the
   price axis to zoom vertically.
3. Open a chart's gear menu to change its instrument, colors, decimal precision, and
   technical indicators, or remove it.
4. Use the left tool strip for the synchronized crosshair, persistent trend lines, and
   temporary ruler measurements.
5. Switch between the vertical and two-column layouts, then drag cards to reorder them.
6. Save the dashboard as a named view. Use the folder menu to load or delete views and to
   rename the current tab.
7. Toggle the Favorites sidebar, add instruments with its **+**, reorder them by dragging,
   and click one to open it in the current tab.
8. Press **⌘T** or use the tab bar's **+** for an empty tab. Drag tabs out into windows or
   merge them again through the tab bar or **File → Merge All Windows**.

## Architecture

```text
DegenView/
├── DegenViewApp.swift                 # App entry, value-based window scenes, commands
├── ContentView.swift                  # Per-tab dashboard, toolbar, layouts, sidebars
├── Model/
│   ├── KlineData.swift                # Shared OHLCV representation and API parsers
│   ├── Indicators.swift               # RSI, EMA, Bollinger, and Supertrend calculations
│   ├── TimeRange.swift                # Timeframes, source intervals, visible limits
│   ├── DataSourceType.swift           # Sources and persisted ticker configuration
│   ├── ChartTab.swift                 # Persisted per-tab state and restored session
│   ├── SavedView.swift                # Named dashboard snapshots
│   ├── FavoriteItem.swift             # Persisted app-wide market shortcuts
│   ├── Crosshair.swift                # Shared per-tab crosshair state
│   └── TrendLine.swift                # Trend-line and ruler models
├── ViewModel/
│   ├── ContentViewModel.swift         # Per-tab charts, tools, refresh, persistence
│   ├── ChartViewModel.swift           # Fetching, caching, indicators, chart state
│   ├── TickerSearchViewModel.swift    # Parallel crypto and stock search
│   └── PolymarketSearchViewModel.swift
├── View/
│   ├── CandleChartView.swift          # AppKit Canvas candlestick renderer
│   ├── LineChartView.swift            # Prediction-market and multi-series renderer
│   ├── ChartPlot.swift                # Shared axes, indicators, drawings, overlays
│   ├── ChartCardView.swift            # Card header, chart, editor, errors
│   ├── ChartSettingsSheet.swift       # Instrument, appearance, indicators
│   ├── AddTickerSheet.swift           # Crypto/stock/Polymarket search
│   ├── ToolSidebar.swift              # Crosshair, trend-line, and ruler tools
│   ├── FavoritesSidebar.swift         # Persistent app-wide watchlist
│   └── AppSettingsView.swift          # Theme and Alpaca credentials
└── Service/
    ├── BinanceAPIService.swift        # Binance REST klines
    ├── BinanceWebSocketService.swift  # Binance live klines
    ├── CoinGeckoAPIService.swift      # CoinGecko OHLC and market metadata
    ├── DEXScreenerService.swift       # Pair discovery and metadata
    ├── GeckoTerminalService.swift     # DEX-pair historical OHLCV
    ├── AlpacaAPIService.swift         # Alpaca search and historical bars
    ├── AlpacaWebSocketService.swift   # Alpaca live stock bars
    ├── PolymarketService.swift        # Event search and probability history
    ├── IconResolver.swift             # Multi-source artwork lookup and cache
    ├── TabsStore.swift                # Tabs, saved views, and session persistence
    ├── FavoritesStore.swift           # Shared watchlist persistence
    ├── DrawingStore.swift             # Instrument-keyed trend-line persistence
    ├── WindowCoordinator.swift        # Native tab grouping and restoration
    └── JSONStore.swift                # Generic Codable JSON persistence
```

## Data flow

1. Each native window/tab renders one `ContentView` backed by its own
   `ContentViewModel`; `NSWindow.tabGroup` remains the authority for tab layout.
2. Each card owns a `ChartViewModel`, which fetches a shared `KlineData` representation
   through the `TickerDataSource` selected by `DataSourceFactory`.
3. Indicator values are calculated from a warm-up buffer and trimmed to the visible
   candles before the custom Canvas renderer draws them.
4. Binance and Alpaca streams update the latest matching candle in place. The five-second
   refresh path covers other sources and recovery, while hidden tabs suspend both paths.
5. Candle responses are cached by symbol, interval, and limit. CoinGecko requests share a
   rate limiter, and its cache is flushed to disk when the app quits.
6. `TabsStore`, `FavoritesStore`, and `DrawingStore` persist independent JSON documents in
   Application Support; Alpaca secrets live in Keychain rather than JSON.

## License

DegenView is licensed under the [GNU General Public License v3.0 only](LICENSE)
(`GPL-3.0-only`). Copyright © 2026 Nico Oelgart.
