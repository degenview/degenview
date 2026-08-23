# DegenView

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

### Portfolio tracking

- **Dedicated Portfolio tab** — Open Portfolio from any chart workspace into one native,
  persistent tab type with no chart-creation controls. The same tab can be reordered,
  detached, merged, and restored with the rest of the macOS tab session
- **Multiple portfolios** — Create, rename, duplicate, delete, reorder, and switch between
  independent portfolios, or use **All Portfolios** for aggregated holdings and allocation
- **Transaction ledger** — Holdings derive from Decimal-valued Buy, Sell, Transfer In,
  Transfer Out, reward, fee, and adjustment events instead of stored current quantities
- **Weighted-average accounting** — Purchase fees increase cost basis; sale fees reduce net
  proceeds; transfers remove proportional basis without realizing a market sale
- **Live analytics** — Current value, allocation, 24-hour movement, average cost, realized
  and unrealized P&L, best/worst performers, and unpriced-asset status update from the
  existing market-data sources
- **Portfolio history** — Transaction-aware value snapshots support 1D, 1W, 1M, 1Y, and
  ALL ranges with time and value axes, profit/loss coloring, and an interactive crosshair
- **Holdings and transactions** — Sort holdings, inspect asset-specific history, edit,
  duplicate, delete, or remap an asset to another source-qualified instrument
- **Privacy mode** — Globally hide balances, quantities, basis, values, P&L, chart values,
  and their accessibility descriptions
- **CSV workflows** — Preview and atomically import DegenView CSV files, and export
  transactions, current holdings, or portfolio history with timezone-bearing timestamps
- **CoinMarketCap import** — Parse CoinMarketCap transaction exports, auto-map tokens to
  portfolio-currency pairs (Binance first, then CoinGecko and DEXScreener), override or
  skip mappings, deduplicate reimports, and supply historical FX for foreign fees or skip
  individual affected rows

## Paper Trading simulation model

Paper Trading is a local, persistent simulator and has no exchange-order endpoint or
credential-bearing execution adapter. The UI holds a `PaperTradingExecutionService`
directly; paper orders never route through Alpaca, Binance, or another live service.

- Market buys execute at ask and market sells at bid when both sides are available.
  The current chart feeds expose only last/bar prices, so those integrations use an
  audited `lastPriceFallback` fill source and never invent a spread.
- Quotes older than 30 seconds cannot execute market orders. Closed-market events keep
  working orders pending. The app currently has no exchange calendar, so chart sources
  only emit the open state they can establish; historical candles are never treated as
  proof that a market is open.
- Limit and stop conditions are evaluated from executable quote/trade events, not from
  pixels on the chart. There is no OHLC-touch fill path. Stop-limit orders activate at
  the stop and subsequently obey limit semantics.
- Positions use one net position per account and instrument with weighted-average cost.
  Reductions realize P&L against that average; reversals close the old side before
  opening the residual quantity on the new side. Fill records remain immutable.
- Long liquidation value uses bid and short liquidation value uses ask, falling back to
  last only when the feed has no bid/ask. P&L applies the instrument point value.
- Account-currency conversion is rejected when no reliable FX conversion is available.
  USD accounts may treat USDT/USDC quotes as USD for this simulator; this limitation is
  explicit rather than a general-purpose FX assumption.
- Fills are deterministic and complete because no connected source exposes reliable
  depth. The schema retains original, filled, and remaining quantities plus fill-level
  records so a future liquidity model can generate partial fills without migration.

A native macOS market dashboard for watching crypto, stocks, and prediction markets in
customizable candlestick and line charts. Built with SwiftUI and an AppKit `Canvas`, with
no external dependencies.

### Historical bar replay

- **No-lookahead replay** — Choose a historical candle and progressively reveal the
  market from that point. Future candles are removed at the data boundary before chart
  rendering, indicators, volume, crosshair inspection, autoscaling, or drawing snapping
- **Native replay controls** — Select a bar, date/time, random bar, or first available
  bar; then step, play/pause, change speed or interval, restart, or return to latest
- **Granular Binance and Alpaca replay** — When lower-timeframe OHLCV is available,
  DegenView paginates up to 100,000 source bars and incrementally reconstructs the active
  displayed candle. Other providers fall back to deterministic complete-chart-bar steps
  without fabricating intrabar prices
- **Deterministic partial candles** — Open comes from the first observed source bar,
  high/low from observed extremes, close from the latest observed close, and volume from
  observed volume only. Source close times—not opening times—advance the replay clock
- **Shared replay clock** — Every chart in a tab is constrained by one authoritative
  historical timestamp, including layouts with different symbols
- **Replay-safe live data** — WebSockets and automatic refresh are suspended during
  replay and restored when returning to latest
- **Session restoration** — Interrupted replay state is saved with the tab and restored
  paused, including the start/current timestamps, interval, and speed
- **Keyboard controls** — **Shift–↓** toggles Play/Pause, **Shift–→** advances one step,
  and **Escape** cancels start-point selection

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
9. Open the toolbar **Replay** menu and choose **Select bar**. Move over a chart to snap
   the orange marker to a historical candle, then click to begin. Use the replay strip to
   step, play, change speed/resolution, choose a new start, or return to latest.
10. Open the toolbar **Portfolio** menu to create a dedicated Portfolio tab. Create a
    portfolio, add transactions manually, or choose **Import from CoinMarketCap**. Review
    automatic asset mappings and historical FX issues before committing the atomic import.

### Replay data support

| Provider | Granular replay | Available behavior |
| --- | --- | --- |
| Binance | Yes | `1m`, `5m`, `15m`, `30m`, `1h`, and `1D` where finer than the chart |
| Alpaca | Yes | Minute/hour/day historical bars from the configured IEX feed |
| CoinGecko | No | Complete displayed bars |
| DEXScreener / GeckoTerminal | No | Complete displayed bars |
| Polymarket | No | Complete displayed observations |

**Auto** chooses the finest provider-supported interval that fits the loaded span within
the 100,000-source-bar replay budget. Intervals that cannot cover the span accurately are
not shown. If a granular request fails or contains no data, that chart displays a
non-blocking notice and safely falls back to complete bars.

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
│   ├── PortfolioModels.swift          # Portfolios, assets, transactions, holdings, snapshots
│   ├── ReplaySession.swift            # Replay status, clock, interval, and speed
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

## Tests

The `DegenViewTests` target covers replay selection, stepping, seeking, completion,
restoration, duplicate/missing timestamps, deterministic OHLCV aggregation, granular
partial candles, source-close-time progression, and future-data leakage. Portfolio tests
cover weighted-average basis, fees, buys/sells/transfers, realized and unrealized P&L,
multi-portfolio aggregation, history invalidation, asset remapping, privacy redaction,
CoinMarketCap parsing/mapping/FX handling, chronological import, deduplication, and
duplicate-safe quote refresh.

```bash
xcodebuild test \
  -project DegenView.xcodeproj \
  -scheme DegenView \
  -destination 'platform=macOS'
```

## License

DegenView is licensed under the [GNU General Public License v3.0 only](LICENSE)
(`GPL-3.0-only`). Copyright © 2026 Nico Oelgart.
