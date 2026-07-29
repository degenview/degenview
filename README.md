# CryptoCharts

macOS crypto candlestick chart app. SwiftUI + AppKit Canvas.

## Features

- **Candlestick charts** — Open/High/Low/Close with wicks, doji detection, colored bodies
- **3 data sources** — Binance, CoinGecko, DEXScreener (Solana/ETH pairs)
- **Real-time updates** — Binance WebSocket streams for live price data
- **6 timeframes** — 1H, 1D, 1W, 1M, 3M, 1Y
- **Scroll-to-zoom** — Adjust candle count via mouse scroll wheel
- **2 layouts** — Vertical list (drag reorder) or 2-column grid (drag-and-drop reorder)
- **Saved views** — Name and persist ticker sets, timeframe, layout, candle count
- **Unsaved changes tracking** — Auto-detects changes, inline save button
- **Crypto icons** — CoinGecko icon lookup by base symbol
- **Theme support** — System / Light / Dark
- **Multi-source search** — Add tickers from any data source with duplicate detection per source

## Architecture

```
CryptoCharts/
├── CryptoChartsApp.swift              # App entry point
├── ContentView.swift                  # Main view, toolbar, theme, drag-drop delegates
├── Model/
│   ├── KlineData.swift                # OHLCV data model, Binance + CoinGecko parsers
│   ├── TimeRange.swift                # Timeframe enum (1H-1Y), interval mapping
│   ├── DataSourceType.swift           # Binance/CoinGecko/DEXScreener enum + TickerConfig
│   ├── SavedView.swift                # Persisted view state (tickers, timeframe, layout)
│   └── SymbolInfo.swift               # Search result metadata
├── ViewModel/
│   ├── ContentViewModel.swift         # Global state: tickers, timeframes, views, WebSocket, auto-refresh
│   └── ChartViewModel.swift           # Per-chart state: fetch, cache, price tracking
├── View/
│   ├── ChartCardView.swift            # Single chart card: header, icon, price, chart, errors
│   ├── CandleChartView.swift          # AppKit Canvas renderer: candles, grid, price line, axis
│   ├── CandleChartStyle.swift         # Colors, sizing, doji threshold config
│   ├── AddTickerSheet.swift           # Multi-source ticker search sheet
│   ├── EmptyStateView.swift           # Empty state with "Add Ticker" prompt
│   ├── ScrollWheelView.swift          # Scroll wheel zoom wrapper
│   └── TimeRangePicker.swift          # Timeframe segmented picker
└── Service/
    ├── BinanceAPIService.swift        # Binance REST API + kline fetching
    ├── BinanceWebSocketService.swift  # Binance WebSocket streams
    ├── CoinGeckoAPIService.swift      # CoinGecko OHLC API
    ├── CoinGeckoService.swift         # CoinGecko icon / search API
    ├── DEXScreenerService.swift       # DEXScreener pairs API
    ├── TickerDataSource.swift         # Protocol + factory for all 3 sources
    ├── TickerStore.swift              # Persisted ticker list (UserDefaults)
    └── ViewStore.swift                # Persisted saved views (JSON file)
```

## Data Flow

1. `ContentViewModel` owns global state (ticker list, timeframe, layout, candle count)
2. Each ticker gets a `ChartViewModel` that fetches OHLC data from its source API
3. `CandleChartView` renders via AppKit `Canvas` — custom draw loop, no third-party chart lib
4. Binance tickers open WebSocket streams for real-time price updates
5. Auto-refresh timer (5s) re-fetches all charts; API responses are cached per (symbol, interval, limit) key
6. CoinGecko icons fetched async per chart card, cached by `CoinGeckoService`

## Requirements

- macOS 14+ (Sonoma)
- Xcode 16+
- Swift 6

## Build & Run

```bash
open CryptoCharts/CryptoCharts.xcodeproj
```

Then **Product → Run** (⌘R) in Xcode.

No external dependencies — all SwiftUI + AppKit native APIs. No CocoaPods, SPM, or Carthage.

## Usage

1. Click **+** to search and add tickers from Binance, CoinGecko, or DEXScreener
2. Toggle **timeframe** in the toolbar (1H–1Y)
3. Toggle **layout** between vertical list and 2-column grid
4. **Scroll** over a chart to zoom (adjust candle count)
5. Drag to **reorder** tickers
7. **Save** current view (name + state) with the save button
8. **Load** saved views from the folder menu
