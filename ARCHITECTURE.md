# DegenView Architecture

## Project structure

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
│   ├── PortfolioModels.swift          # Portfolios, assets, transactions, holdings, snapshots
│   ├── PriceAlertModels.swift         # Alert rules, runtime state, quotes, history, settings
│   ├── ReplaySession.swift            # Replay status, clock, interval, and speed
│   ├── SavedView.swift                # Named dashboard snapshots
│   ├── FavoriteItem.swift             # Persisted app-wide market shortcuts
│   ├── Crosshair.swift                # Shared per-tab crosshair state
│   └── TrendLine.swift                # Trend-line and ruler models
├── ViewModel/
│   ├── ContentViewModel.swift         # Per-tab charts, tools, refresh, persistence
│   ├── ChartViewModel.swift           # Fetching, caching, indicators, chart state
│   ├── AlertStore.swift               # MainActor alert UI facade and notification delivery
│   ├── TickerSearchViewModel.swift    # Parallel crypto and stock search
│   └── PolymarketSearchViewModel.swift
├── View/
│   ├── CandleChartView.swift          # AppKit Canvas candlestick renderer
│   ├── LineChartView.swift            # Prediction-market and multi-series renderer
│   ├── ChartPlot.swift                # Shared axes, indicators, drawings, overlays
│   ├── ChartCardView.swift            # Card header, chart, editor, errors
│   ├── PriceAlertEditor.swift         # Compact absolute/percentage rule editor
│   ├── AlertsCenterView.swift         # App-wide rule/history center and trigger banner
│   ├── ReplayControlBar.swift         # Playback, interval, timestamp, and live controls
│   ├── ChartSettingsSheet.swift       # Instrument, appearance, indicators
│   ├── AddTickerSheet.swift           # Crypto/stock/Polymarket search
│   ├── ToolSidebar.swift              # Crosshair, trend-line, and ruler tools
│   ├── FavoritesSidebar.swift         # Persistent app-wide watchlist
│   ├── PortfolioDashboardView.swift   # Overview, holdings, history, imports, transaction UI
│   ├── PortfolioTabView.swift         # Dedicated non-chart native tab lifecycle
│   └── AppSettingsView.swift          # Theme and Alpaca credentials
└── Service/
    ├── BinanceAPIService.swift        # Binance REST klines
    ├── BinanceWebSocketService.swift  # Binance live klines
    ├── CoinGeckoAPIService.swift      # CoinGecko OHLC and market metadata
    ├── DEXScreenerService.swift       # Pair discovery and metadata
    ├── GeckoTerminalService.swift     # DEX-pair historical OHLCV
    ├── AlpacaAPIService.swift         # Alpaca search and historical bars
    ├── AlpacaWebSocketService.swift   # Alpaca live stock bars
    ├── LocalPriceAlertEngine.swift    # Serialized crossing state machine and persistence
    ├── MarketQuoteCoordinator.swift   # App-wide owner-based alert quote polling/deduplication
    ├── FXRateService.swift            # Frankfurter daily FX rates and disk cache
    ├── ReplayEngine.swift             # Deterministic replay state machine and aggregation
    ├── PortfolioAccountingEngine.swift # Weighted-average basis and P&L calculations
    ├── PortfolioLedger.swift          # Actor-serialized atomic transaction ledger
    ├── PortfolioStore.swift           # Published portfolio state, quotes, history cache
    ├── PortfolioCSVService.swift      # Native and CoinMarketCap CSV import/export
    ├── PortfolioAssetAutoMapper.swift # Currency-pair asset resolution for imports
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
7. During replay, each chart retains its immutable canonical history and exposes only a
   binary-searched prefix through `replayKlines`. `ReplayEngine` owns the tab's sole
   timestamp and one cancellable playback task.
8. Binance and Alpaca optionally conform to `GranularReplayDataSource`. Their paginated
   lower-timeframe bars are aggregated against the provider-returned displayed-bar
   boundaries, preserving stock sessions, market gaps, and DST alignment.
9. Portfolio mutations are serialized by `PortfolioLedger`, persisted atomically in
   `portfolios.json`, and replayed by `PortfolioAccountingEngine`. Quote ticks update live
   valuation without replaying static accounting; historical edits invalidate only the
   affected snapshot suffix.
