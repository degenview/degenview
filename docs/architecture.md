# DegenView Architecture

## Project structure

```text
DegenView/
├── DegenViewApp.swift                 # App entry, value-based window scenes, commands
├── ContentView.swift                  # Per-tab dashboard, toolbar, layouts, sidebars
├── Model/
│   ├── KlineData.swift                # Shared OHLCV representation and API parsers
│   ├── PineModels.swift               # Pine diagnostics, inputs, persistence, visual output
│   ├── Indicators.swift               # RSI, EMA, Bollinger, and Supertrend calculations
│   ├── TimeRange.swift                # Timeframes, source intervals, visible limits
│   ├── DataSourceType.swift           # Sources, CMC chart identity, persisted card config
│   ├── ChartColumn.swift              # Persisted grid columns and legacy layout repair
│   ├── ChartTab.swift                 # Persisted per-tab state and restored session
│   ├── PortfolioModels.swift          # Portfolios, assets, transactions, holdings, snapshots
│   ├── PriceAlertModels.swift         # Alert rules, runtime state, quotes, history, settings
│   ├── ReplaySession.swift            # Replay status, clock, interval, and speed
│   ├── SavedView.swift                # Named dashboard snapshots
│   ├── FavoriteItem.swift             # Persisted app-wide market shortcuts
│   ├── Crosshair.swift                # Shared per-tab crosshair state
│   ├── TrendLine.swift                # Trend-line, ruler, and tool-selection models
│   └── FibonacciRetracement.swift     # Fib levels, style, calculator, visibility, templates
├── ViewModel/
│   ├── ContentViewModel.swift         # Per-tab charts, tools, refresh, persistence
│   ├── ChartViewModel.swift           # Fetching, caching, indicators, chart state
│   ├── AlertStore.swift               # MainActor alert UI facade and notification delivery
│   ├── TickerSearchViewModel.swift    # Parallel crypto and stock search
│   └── PolymarketSearchViewModel.swift
├── View/
│   ├── CandleChartView.swift          # AppKit Canvas candlestick renderer
│   ├── LineChartView.swift            # Prediction-market and multi-series renderer
│   ├── CoinMarketCapChartView.swift   # Fixed-scale CMC plots, season scale, sentiment gauge
│   ├── ChartPlot.swift                # Shared axes, indicators, drawings, overlays
│   ├── ChartCardView.swift            # Card header, chart, drawing editors, errors
│   ├── ChartGridDropDelegate.swift    # Column-aware chart drag/drop destinations
│   ├── PriceAlertEditor.swift         # Compact absolute/percentage rule editor
│   ├── AlertsCenterView.swift         # App-wide rule/history center and trigger banner
│   ├── ReplayControlBar.swift         # Playback, interval, timestamp, and live controls
│   ├── ChartSettingsSheet.swift       # Instrument, appearance, indicators
│   ├── AddTickerSheet.swift           # Crypto/stock/Polymarket/CMC/Portfolio picker
│   ├── ToolSidebar.swift              # Crosshair, trend-line, Fib, and ruler tools
│   ├── FavoritesSidebar.swift         # Persistent app-wide watchlist
│   ├── PortfolioDashboardView.swift   # Overview, holdings, history, imports, transaction UI
│   ├── PortfolioTabView.swift         # Dedicated non-chart native tab lifecycle
│   └── AppSettingsView.swift          # Theme, provider credentials, notifications
└── Service/
    ├── BinanceAPIService.swift        # Binance REST klines
    ├── PineEngine.swift               # Ranged lexer, AST parser, semantic compiler
    ├── PineRuntime.swift              # Sandboxed bar VM, rollback, TA and visual builtins
    ├── BinanceWebSocketService.swift  # Binance live klines
    ├── CoinGeckoAPIService.swift      # CoinGecko OHLC and market metadata
    ├── DEXScreenerService.swift       # Pair discovery and metadata
    ├── GeckoTerminalService.swift     # DEX-pair historical OHLCV
    ├── AlpacaAPIService.swift         # Alpaca search and historical bars
    ├── AlpacaWebSocketService.swift   # Alpaca live stock bars
    ├── LocalPriceAlertEngine.swift    # Serialized crossing state machine and persistence
    ├── MarketQuoteCoordinator.swift   # App-wide owner-based alert quote polling/deduplication
    ├── FXRateService.swift            # Frankfurter current/historical FX + BTC cross-rates, disk cache
    ├── BitcoinHistoryService.swift    # Bitstamp daily BTC/USD closes, disk-cached
    ├── ReplayEngine.swift             # Deterministic replay state machine and aggregation
    ├── PortfolioAccountingEngine.swift # Weighted-average basis and P&L calculations
    ├── PortfolioLedger.swift          # Actor-serialized atomic transaction ledger
    ├── PortfolioStore.swift           # Published portfolio state, quotes, history, currency projections
    ├── PortfolioCSVService.swift      # Native and CoinMarketCap CSV import/export
    ├── PortfolioAssetAutoMapper.swift # Currency-pair asset resolution for imports
    ├── PolymarketService.swift        # Event search and probability history
    ├── CoinMarketCapService.swift     # Keychain, typed API client/provider, cache and retry
    ├── IconResolver.swift             # Multi-source artwork lookup and cache
    ├── TabsStore.swift                # Tabs, saved views, and session persistence
    ├── FavoritesStore.swift           # Shared watchlist persistence
    ├── DrawingStore.swift             # Instrument-keyed trend-line and Fib persistence
    ├── DrawingUndoCoordinator.swift   # Per-window native drawing undo/redo history
    ├── WindowCoordinator.swift        # Native tab grouping and restoration
    └── JSONStore.swift                # Generic Codable JSON persistence
```

## Data flow

1. Each native window/tab renders one `ContentView` backed by its own
   `ContentViewModel`; `NSWindow.tabGroup` remains the authority for tab layout.
2. Each card owns a `ChartViewModel`. Tradable market charts fetch `KlineData` through the
   `TickerDataSource` selected by `DataSourceFactory`; CoinMarketCap cards retain typed
   index models and never force non-price metrics into OHLCV.
3. Indicator values are calculated from a warm-up buffer and trimmed to the visible
   candles before the custom Canvas renderer draws them.
4. Binance and Alpaca streams update the latest matching candle in place. The five-second
   refresh path covers other sources and recovery, while hidden tabs suspend both paths.
5. Candle responses are cached by symbol, interval, and limit. CoinGecko requests share a
   rate limiter, and its cache is flushed to disk when the app quits. The shared
   `CoinMarketCapClient` coalesces identical in-flight requests and uses a 15-minute cache
   for latest/Altcoin data and a six-hour cache for daily Fear and Greed history.
6. `TabsStore`, `FavoritesStore`, and `DrawingStore` persist independent JSON documents in
   Application Support. Alpaca and optional CoinMarketCap secrets live in Keychain rather
   than JSON; only CMC chart type, range, and display settings enter workspace state.
   Each tab and named saved view also stores ordered `ChartColumn` membership by the
   stable `TickerConfig.chartID`. Older documents without columns are repaired into the
   former two-column row-major arrangement when loaded.
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
10. Selecting a reporting currency asks `FXRateService` for current or historical
    conversions—fiat rates from Frankfurter, BTC cross-rates from `BitcoinHistoryService`—
    and `PortfolioStore` converts transactions, quotes, and history into that currency once
    and caches the result as a projection. Switching back to an already-computed currency
    reuses its cached projection instead of re-converting or re-fetching rates.
11. Each market `ChartViewModel` owns an optional Pine configuration. Compilation and
    evaluation run in a generation-checked detached task; the runtime receives only an
    immutable OHLCV/replay prefix and emits renderer-neutral visuals. Draft source is
    persisted separately from last-valid applied source, so invalid edits do not remove
    the active result. Pine outputs are never shared between tabs or cards.
12. A CMC card stores a stable `CoinMarketCapChartType` identifier in `TickerConfig`.
    `ChartViewModel.fetchCoinMarketCap` uses generation checks and task cancellation so a
    stale range response cannot replace a newer selection. CMC cards are excluded from
    replay, price alerts, WebSockets, Pine evaluation, and OHLCV-specific controls.
13. Trend lines and Fibonacci retracements retain timestamp/price anchors in
    `DrawingStore`, keyed by source-qualified instrument. Rendering projects those
    financial coordinates through the current `ChartPlot` on every layout pass, so
    timeframe changes, horizontal zoom, vertical scaling, resizing, and replay do not
    rewrite canonical drawing state.
14. Each chart window connects its `ChartViewModel` instances to one
    `DrawingUndoCoordinator` backed by the window's native `UndoManager`. Undo and redo
    apply targeted, instrument-keyed replacements through `DrawingStore`, preserving
    unrelated drawing changes made by another window while immediately updating every
    chart observing the same instrument.

## Drawing undo and redo

Drawing history is session-only and scoped to the native window/tab where an edit
originated. `ContentViewModel` attaches a coordinator to every chart it owns, including
charts added or restored after the window has opened. AppKit supplies the Edit-menu state,
descriptive Undo/Redo titles, and the standard Command-Z and Shift-Command-Z shortcuts;
Command-Y forwards `redo:` through the focused responder chain.

The coordinator records the drawing before and after each committed mutation together
with its source-qualified instrument key and array position. Applying an inverse operation
removes or restores only that drawing ID through `DrawingStore`; it never replaces an
entire historical snapshot. This matters when separate windows show the same instrument:
undoing an older action in one window cannot erase a newer, unrelated drawing created in
another.

Pointer movement remains lightweight. Trend-line endpoint drags and Fibonacci body or
anchor drags update published geometry continuously, then register one action on
mouse-up; unchanged drags register nothing. Live changes during one Fibonacci settings
sheet presentation are similarly collapsed into one edit when the sheet closes. Drafts,
crosshairs, and ruler measurements never enter drawing history because they are transient.
Store observation clears selected or edited IDs when an undo, redo, or another window
removes the corresponding drawing.

## Fibonacci retracement flow

```text
ToolSidebar
    │ arm Fib Retracement
    ▼
ContentViewModel mouse monitor
    ├── pointer → ChartPlot inverse transform → TrendAnchor(date, price)
    ├── Command modifier → replay-visible candle → nearest OHLC candidate
    ├── first click → draft Point 1
    ├── move → live draft Point 2
    └── second click → committed FibonacciRetracementDrawing
             │
             ▼
FibonacciCalculator
    ├── linear: P1 + r × (P2 − P1)
    ├── reverse: use (1 − r), without swapping stored anchors
    └── logarithmic: exp(log(P1) + r × (log(P2) − log(P1)))
             │
             ▼
ChartPlot.drawFibonacciRetracements
    ├── project anchor times and calculated prices into pixels
    ├── sort calculated prices before building adjacent fill regions
    ├── draw extensions to current viewport edges
    ├── draw level/trend lines, labels, prices, and custom text
    └── draw selection handles
```

`FibonacciRetracementDrawing` is a versioned, Codable, first-class drawing model. Each
level owns a stable UUID, Decimal ratio, visibility, color, opacity, and custom text.
Canonical ratios are not restricted to `0...1`; negative ratios and ratios above one
remain ordinary levels. `FibonacciRetracementDrawing.addLevel` enforces the documented
24-level maximum at the domain boundary, and the settings UI disables its Add Level
action at the same limit.

The renderer receives computed financial prices rather than embedding ratio math in the
Canvas loop. It converts to `Double` only at the calculation/render boundary; persisted
ratios remain Decimal. Invalid logarithmic anchors return no level geometry rather than
forming NaN or infinite paths. DegenView does not currently expose a logarithmic chart
price scale, so the log-Fib setting is disabled in the UI while its calculator and tests
remain available for that future scale mode.

Completed drawings are stored separately in `fib-drawings.json`; existing trend-line
storage remains in `drawings.json`, avoiding a migration of existing installations.
Continuous anchor/body drags update published in-memory geometry at pointer frequency and
perform one disk write on mouse-up. Both candlestick and probability line charts reuse
the same immediate-mode Fib renderer and hit-testing geometry.

The Fib settings sheet follows the main DegenView Settings layout: sidebar navigation,
page headers, material cards, and a bottom action bar. Style edits update the drawing
immediately. Coordinates edit the same canonical dates/prices used by pointer gestures;
Visibility filters rendering by `TimeRange` and stores lock/hide state without deleting
the drawing.

The current repository has no drawing clipboard/template manager, global Strong/Weak
Magnet state, logarithmic chart scale, or drawing-linked alert model. Fib does not
introduce private parallel versions of those app-wide systems. The reusable calculator,
level/style models, stable drawing/level IDs, and current-geometry lookup are structured
so those integrations can be added when their shared infrastructure exists.

`FibonacciRetracementTests` covers deterministic low-to-high and high-to-low linear
fixtures, reverse reflection/restoration, logarithmic interpolation, invalid log anchors,
the 24-level limit, arbitrary extensions, and Codable persistence round trips.
`DrawingUndoCoordinatorTests` covers exact-ID and ordering restoration, native action
titles, redo invalidation, persistence, and isolated window histories over a shared store.

## Dashboard layout and drag flow

- Vertical mode continues to use the tab's flat chart order. Grid mode renders explicit,
  equal-width `ChartColumn` stacks; switching modes preserves the grid arrangement.
- A chart drag exposes column insertion positions and, when enough width remains, a
  trailing add-column rail. Holding over that rail expands a temporary outlined column
  and shrinks the existing columns without changing model or persisted state.
- Dropping commits through `ContentViewModel`, keyed by the chart's stable `chartID`.
  Charts can move within or between columns, and a column is removed as soon as its last
  chart moves away or is deleted. New charts are assigned to the shortest column.
- Additional columns require approximately 280 points per resulting column. Existing
  columns are never removed merely because the window or Favorites sidebar narrows.

## CoinMarketCap data flow

```text
AppSettingsView
       │
       ▼
CoinMarketCapCredentialStore (macOS Keychain)
       │ read when constructing each network request
       ▼
CoinMarketCapClient
  ├── public `/public-api` root when no key exists
  ├── Pro root + `X-CMC_PRO_API_KEY` when configured
  ├── standardized status-envelope validation
  ├── bounded exponential retry with jitter for 429 and 5xx
  ├── URLSession protocol-cache support
  └── shared response cache and in-flight request coalescing
       │
       ▼
CoinMarketCapDataProvider
  ├── Altcoin Season latest
  ├── Altcoin Season historical (7d/30d/90d)
  ├── Fear and Greed latest
  └── Fear and Greed historical (`start`/`limit`, max 500 per page)
       │
       ▼
ChartViewModel (MainActor, cancellation/generation guarded)
       │
       ▼
CoinMarketCapChartView
  ├── fixed 0–100 historical plot and tooltip
  ├── Altcoin Season regime scale and supporting statistics
  └── responsive Fear and Greed speedometer
```

The client resolves Keychain state when constructing a request, so saving or removing a
key changes the next network request without restarting or rebuilding open chart models.
Keyed and keyless endpoints share response models and cache identity; authentication
affects access and rate limits, not the represented data product. HTTP 200 responses are
still rejected when CMC's embedded `status.error_code` is nonzero. The decoder accepts
both numeric and numeric-string status codes observed from the live API.

The existing five-second visible-tab refresh coordinator may ask CMC cards to refresh,
but client TTLs suppress network traffic until upstream data is stale. Hidden or occluded
tabs cancel refresh work. Manual refresh bypasses freshness while still participating in
in-flight coalescing. Fear and Greed ALL history requests sequential 500-record pages until
the API returns a short page; all historical points are normalized oldest to newest.
