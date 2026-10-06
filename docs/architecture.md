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
│   ├── DataSourceType.swift           # Sources, CMC chart identity, persisted card config
│   ├── ChartColumn.swift              # Persisted grid columns and legacy layout repair
│   ├── ChartTab.swift                 # Persisted per-tab state and restored session
│   ├── PortfolioModels.swift          # Portfolios, assets, transactions, holdings, snapshots
│   ├── RecentMarket.swift             # A picked market, remembered for the Add Chart sheet
│   ├── PortfolioStatistics.swift      # Derived stats (best/worst, 24h, extremes, fees) + PortfolioPeriodChange
│   ├── PortfolioHistoryRange.swift    # Overview chart ranges (1D … ALL) and snapshot filtering
│   ├── PriceAlertModels.swift         # Alert rules, runtime state, quotes, history, settings
│   ├── PortfolioCurrency+AlertPrice.swift # `formatAlertPrice`: magnitude-aware alert price text (also in the agent)
│   ├── ReplaySession.swift            # Replay status, clock, interval, and speed
│   ├── SavedView.swift                # Named dashboard snapshots
│   ├── LayoutSnapshot.swift           # Persisted-layout fingerprint compared for dirty state
│   ├── FavoriteItem.swift             # Persisted app-wide market shortcuts
│   ├── Crosshair.swift                # Shared per-tab crosshair state
│   ├── Script/                        # Script library: LocalScript, versions, drafts, compile records
│   ├── PreviewMarket.swift            # A crypto or stock market the Script Manager preview charts
│   ├── ScriptPreviewLayout.swift      # ChartPosition: preview chart left of / above / below the code
│   ├── TrendLine.swift                # Trend-line, ruler (rect, corners, hits, overlay state), and tool-selection models
│   ├── RulerReadout.swift             # Ruler numbers: percent, price delta, bars, duration, and their text
│   ├── FibonacciRetracement.swift     # Fib levels, style, calculator, visibility, templates
│   ├── BrushDrawing.swift             # Freehand stroke (time + price points), style, draft, overlay state, tuning
│   └── BrushGeometry.swift            # Pure screen-space maths: simplification, hit testing, smoothing
├── ViewModel/
│   ├── ContentViewModel.swift         # Per-tab charts, tools, refresh, persistence
│   ├── SavedLayoutController.swift    # Per-tab active layout, dirty/autosave, save/copy/rename/open
│   ├── ChartViewModel.swift           # Fetching, caching, indicators, chart state
│   ├── ChartLiveQuote.swift           # A chart's latest best bid/ask (own object, so book ticks don't redraw the card)
│   ├── ScriptPreviewViewModel.swift   # Script Manager preview: one chart, market, timeframe, inputs, refresh
│   ├── AlertStore.swift               # MainActor alert UI facade and notification delivery
│   ├── PineAlertStore.swift           # Pine script alert subscriptions, history, banner
│   ├── PineAlertCoordinator.swift     # Routes a chart's Pine alerts; pauses/re-arms subscriptions
│   ├── TickerSearchViewModel.swift    # Parallel crypto and stock search
│   ├── PortfolioAssetInfoViewModel.swift # Portfolio asset coin names and artwork, via IconResolver
│   └── PredictionMarketSearchViewModel.swift  # Polymarket/Kalshi event search, grouped by event
├── View/
│   ├── ChartIconView.swift            # A chart's market icon with its source logo badge
│   ├── CandleChartView.swift          # AppKit Canvas candlestick renderer
│   ├── LineChartView.swift            # Prediction-market and multi-series renderer
│   ├── CoinMarketCapChartView.swift   # Fixed-scale CMC plots, season scale, sentiment gauge
│   ├── ChartPlot.swift                # Shared axes, indicators, drawings, overlays
│   ├── ChartCardView.swift            # Card header, chart, drawing editors, errors
│   ├── ChartGridDropDelegate.swift    # Column-aware chart drag/drop destinations
│   ├── PriceAlertEditor.swift         # Compact absolute/percentage rule editor
│   ├── AlertsCenterView.swift         # Alerts window: header, IconTabBar tabs, rule/history lists
│   ├── AlertRuleRow.swift             # Price-alert card: logo, condition, target, status, actions
│   ├── AlertHistoryRow.swift          # Trigger card: price vs target, delayed/failed marks
│   ├── AlertHistoryList.swift         # History grouped by day (`AlertDayGrouping`)
│   ├── AlertAssetIcon.swift           # Coin artwork + source logo badge for alert rows
│   ├── AlertConditionChip.swift       # "Crosses above" / "Falls 5%" pill
│   ├── AlertCardStyle.swift           # Shared card chrome; AlertCardScroll / AlertDayHeader lay them out
│   ├── GlobalAlertBanner.swift        # Trigger banner overlaid on every tab
│   ├── PineAlertEditor.swift          # Create a script alert from a chart's applied script
│   ├── PineAlertListView.swift        # "Scripts" tab of the alerts center (rows use PineAlertCard)
│   ├── PaperAccountManagerView.swift  # Paper Trading panel (bottom split pane): composes PaperPanelHeader (account menu, IconTabBar, actions), PaperMetricsStrip, one table per `PaperManagerTab` (PaperPositionsTable, PaperOrdersTable, PaperOrderHistoryTable, PaperClosedTradesTable, PaperJournalTable; `PaperEmptyState` when empty) and PaperPanelFooter (AlertFooterBar: Close All / Cancel All / Export)
│   ├── PaperOrderTicketSheet.swift    # Order ticket; math and validation live in `Util/PaperOrderTicketDraft`, input parsing in `Util/PaperDecimalInput`; PaperQuoteStrip, PaperSideSelector, PaperOrderSummaryCard, PaperFormField / PaperTextField
│   ├── PaperAccountConfigurationSheet.swift # Create / reset account (validated; reset starts from the account's settings)
│   ├── PaperChartTradingOverlay.swift # Position / order markers on a chart (PaperChartMarkerPill; drag to modify)
│   ├── PaperQuoteFeed.swift           # Zero-size view behind each chart card: observes the chart and streams its last price and Binance/Coinbase best bid/ask to `PaperTradingStore.stream` (ContentView does not observe charts, so this cannot live there). Util/PaperQuoteSample decides what counts as a fresh book
│   ├── PaperQuickTradeButtons.swift   # SELL / BUY pills in a chart card header
│   └── PaperTradingStyle.swift        # Buy green / sell red, P&L colour; labels and tints in `Model/PaperTradingModels+Presentation`. PaperBadge, PaperSideChip, PaperIconButton / PaperIconGlyph are the shared bits
│   ├── ReplayControlBar.swift         # The docked strip: status, transport, speed/resolution, scrubber, clock, notice, way back to live; swaps to a hint while picking a start. Pieces: ReplayStatusChip, ReplayTransportControls, ReplayPickerMenu, ReplayScrubber, ReplayClockReadout, ReplayNoticeChip, ReplayIconButton
│   ├── ReplayStyle.swift              # Replay accent (orange), per-state colour/title, date formats
│   ├── ReplaySelectionMarker.swift    # Start-picker hover marker: line, date tag, dimmed future
│   ├── ReplayStartSheet.swift         # Date/time picker clamped to the loaded span, presets, bar-snap preview
│   ├── ChartSettingsSheet.swift       # Appearance, indicators, scripts (a chart's market is fixed: remove and re-add)
│   ├── ScriptManagerView.swift        # Script list sidebar (collapsible) + per-script workspace
│   ├── ScriptWorkspaceView.swift      # Code editor + preview chart, split left/top/bottom
│   ├── ScriptPreviewPane.swift        # Live preview: market, timeframe, zoom, chart, drawer bar
│   ├── ScriptPreviewDrawer.swift      # Inputs / Report / Problems under the preview chart
│   ├── ScriptPreviewMarketPicker.swift # Crypto-and-stock-only market popover
│   ├── PineInputsView.swift           # A script's `input.*` declarations as controls (chart settings + preview)
│   ├── SettingsCardRow.swift          # Icon, title, hint and trailing control card
│   ├── PriceAxisDragMonitor.swift     # Drag a price axis to scale candles (chart tabs and the preview)
│   ├── SplitContainer.swift           # Resizable, collapsible two-pane split (+ SplitLayout, SplitMetrics)
│   ├── AddTickerSheet.swift           # Crypto/stock/prediction-market/CMC/Portfolio picker
│   ├── ToolSidebar.swift              # Crosshair, trend-line, Fib, brush, and ruler tools
│   ├── AppToolbar.swift               # Portfolio + Script Manager title-bar buttons, shared by every tab kind
│   ├── FavoritesSidebar.swift         # Persistent app-wide watchlist
│   ├── PortfolioDashboardView.swift   # Overview, holdings, history, imports, transaction UI
│   ├── PortfolioOverviewView.swift    # Portfolio tab: balance, 24h/period change, value chart, allocation, top holdings
│   ├── RecentMarketsCard.swift        # Add Chart: the last picked markets, under the suggestions
│   ├── CandleColorPreview.swift       # Tiny candle/line preview of the chosen chart colors
│   ├── ChoiceCard.swift               # Selectable option card (icon, title, description, radio mark)
│   ├── IconTabBar.swift          # Icon + title section switcher (portfolio tabs, Add Chart sources)
│   ├── PortfolioPerformerCard.swift   # Best/worst performer card (icon, return %, total P&L)
│   ├── PortfolioCreateSheet.swift     # New-portfolio sheet: name field + base-currency tiles
│   ├── PortfolioManageSheet.swift     # Manage portfolios: rename, reorder, duplicate, delete
│   ├── PortfolioCoinMarketCapImportSheet.swift # CMC import step 1: match tickers to markets, resolve fee FX
│   ├── PortfolioImportPreviewSheet.swift # Import step 2: transaction table, errors/warnings, confirm
│   ├── SheetHeader.swift     # Icon badge + title + subtitle atop the sheets
│   ├── NoticeCard.swift      # Tinted error/warning callout with optional line list and action
│   ├── PortfolioHoldingsView.swift    # Portfolio tab: positions Table (sortable headers persist the portfolio's sort)
│   ├── PortfolioTableChrome.swift     # Shared Table look: zebra rows, hairline border, side margin
│   ├── PortfolioSearchField.swift     # Rounded search box above the portfolio tables
│   ├── PortfolioNumericCell.swift     # Right-aligned monospaced-digit table cell
│   ├── PortfolioTransactionsView.swift # Portfolio tab: searchable/filterable/sortable transactions Table
│   ├── PortfolioStatisticsView.swift  # Portfolio tab: performance, value range, activity, P&L-by-asset bars
│   ├── PortfolioAssetDetailView.swift # One asset's position stats and transactions table (sheet)
│   ├── PortfolioHistoryChart.swift    # Hand-drawn portfolio value line with hover tooltip
│   ├── PortfolioAllocationChart.swift # Allocation donut + legend (top slices, rest folded into "Other")
│   ├── PortfolioCard.swift            # Titled rounded panel used across the portfolio tabs
│   ├── PortfolioStatCard.swift        # Labelled figure with optional caption line
│   ├── PortfolioTransactionTypeBadge.swift # Tinted pill for a transaction type
│   ├── PortfolioTabView.swift         # Dedicated non-chart native tab lifecycle
│   ├── AppSettingsView.swift          # Settings window shell: badge sidebar + selected page
│   ├── AppearanceSettingsView.swift   # Settings ▸ Appearance: theme previews
│   ├── AlpacaSettingsView.swift       # Settings ▸ Alpaca: API keys with status and remove
│   ├── CoinMarketCapSettingsView.swift # Settings ▸ CoinMarketCap: optional API key, connection test
│   ├── NotificationSettingsView.swift # Settings ▸ Notifications: delivery toggles, agent health
│   ├── SettingsPage.swift             # Settings page shell + SettingsSection / SettingsCard
│   ├── SettingsField.swift            # Labelled text/secret field for settings
│   └── SettingsStatusBadge.swift      # Status dot + word, and the Save result line
├── Pine/                              # Pine Script feature: language, runtime, broker, editor, views
│   ├── Language/
│   │   ├── Lexer/                     # PineLexer (+Tokens), PineSourceLine, tokens
│   │   ├── AST/                       # Expressions, statements, typed operators, traversal helpers
│   │   ├── Parser/                    # PineParser (+Statements, +Declarations, +Expressions)
│   │   ├── Compiler/                  # PineCompiler (+Declaration, +Constants, +Inputs), PineLibraryLinker (`import`)
│   │   ├── Analysis/                  # Structure validator, type checker, builtin type tables
│   │   ├── PineBuiltins.swift         # Color and named-constant tables shared by compiler/runtime
│   │   ├── PineSymbolCatalog.swift    # Builtin variables/constants/functions/namespaces, composed from the tables above; `+Members` lists a namespace's members (editor highlighting + completion)
│   │   └── PineSymbolMetadata*.swift  # Signatures, overloads and one-line docs per builtin, one DSL line each, by family; `PineSymbolMetadataTests` guards them against the catalog and the runtime
│   ├── Runtime/
│   │   ├── PineRuntimeSession*.swift  # Bar interpreter: statements, expressions, call router, one extension per builtin family
│   │   ├── Builtins/                  # Pure math, strings, formatting, time, calendar, operators, ta.*
│   │   ├── PineExecutionHost.swift    # Actor around one controller: serialized rebuild / ingest / sync
│   │   ├── PineExecutionController.swift # Aggregator + scheduler + session for one script; history, ticks, REST reconcile
│   │   ├── PineCandleAggregator.swift # Bar lifecycle: identity, dedupe, close, gap / correction detection
│   │   └── PineExecutionScheduler.swift # Which events run which script (indicator vs strategy, calc_on_every_tick)
│   ├── Broker/                        # strategy() order book, triggers, fills, trades, equity
│   ├── Model/                         # Diagnostics, inputs, typed style enums, visual output (also in the alert agent)
│   ├── Editor/                        # Script editor text view, word ranges, diagnostic mapping; highlighting = PineSyntaxClassifier (lexer tokens + catalog + PineSourceSymbolIndex for user shadowing) → PineSyntaxTheme → PineSyntaxHighlighter
│   │                                  # Editing assistance: PineLexicalSnapshot (one lex per text version: strings, comments, bracket pairs; shared with the classifier) → PineEditorContext → pure engines (PineEditorPairing, PineIndentationEngine, PineEditorCommands, PineDelimiterMatcher) returning a PineEditorEdit → PineTextView(+Editing) applies it as one undo step. Completion and signature help: PineEditorAnalysisCache (one PineLexicalSnapshot + one PineSourceSymbolIndex per text version) → PineCompletionContext (word, member base, position, scope, PineCallSite) → PineCompletionEngine / PineSignatureResolver (pure; builtins from PineSymbolCatalog + PineSymbolMetadata, imports from PineLibraryExportDirectory over PineLibraryRegistry) → PineCompletionController → child panels (PineCompletionPanel, PineSignaturePanel); accepting is a PineEditorEdit from PineCompletionInsertion. Visual only: PineEditorDecorations (temporary attrs), PineLayoutManager → PineCurrentLineRenderer / PineIndentGuideRenderer
│   └── View/                          # PineChartLayer (+per-output drawing), script pane, strategy report
└── Service/
    ├── BinanceAPIService.swift        # Binance REST klines
    ├── ChartLiveFeed.swift            # Opens the Binance/Coinbase/Alpaca streams for a set of charts
    ├── ScriptPreviewInputsStore.swift # Input values tried in the Script Manager preview, per script
    ├── BinanceWebSocketService.swift  # Binance live klines (+ optional `@bookTicker` best bid/ask)
    ├── BookTickerCoalescer.swift      # Thins book updates to a few deliveries a second per symbol
    ├── CoinbaseAPIService.swift       # Coinbase REST candles (paged, 1w/1M folded from daily) + product search
    ├── CoinbaseWebSocketService.swift # Coinbase live trades (ticker channel → CoinbaseTick)
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
    ├── PortfolioCandleStore.swift     # SQLite-backed daily candles; fetches only what is missing
    ├── PortfolioQuoteFetcher.swift    # Current prices: one batched call per source, candles as fallback
    ├── PortfolioCSVService.swift      # Native and CoinMarketCap CSV import/export
    ├── PortfolioAssetAutoMapper.swift # Currency-pair asset resolution for imports
    ├── PredictionMarketDataSource.swift # Shared protocol: YES probability history by TimeRange
    ├── TrendingMarketsDataSource.swift # Optional capability: busiest events (Polymarket 24h volume)
    ├── PolymarketService.swift        # Event search and probability history
    ├── KalshiService.swift            # Series-index search, candlestick history, live YES ask
    ├── KalshiSeriesIndex.swift        # Cached /series list + local keyword ranking
    ├── CoinMarketCapService.swift     # Keychain, typed API client/provider, cache and retry
    ├── IconResolver.swift             # Multi-source artwork lookup and cache
    ├── TabsStore.swift                # Tabs and session persistence
    ├── SavedViewStore.swift           # Shared saved-layout library (throwing writes, recency)
    ├── FavoritesStore.swift           # Shared watchlist persistence
    ├── DrawingStore.swift             # Instrument-keyed trend-line, Fib, and brush persistence
    ├── DrawingUndoCoordinator.swift   # Per-window native drawing undo/redo history
    ├── WindowCoordinator.swift        # Native tab grouping and restoration
    ├── WindowTabIcon.swift            # Per-kind tab icon (SF Symbol in the tab's attributed title) + decorator
    ├── AppDatabase.swift              # Shared SQLite (GRDB, WAL) database
    ├── AppDatabase+Schema.swift       # Schema creation, document and setting helpers
    ├── AppDatabase+Workspace.swift    # Tabs, saved views, and drawings tables
    ├── AppDatabase+Portfolio.swift    # Portfolio ledger tables
    ├── AppDatabase+Candles.swift      # Closed daily candle cache and its coverage ranges
    ├── AppDatabase+PaperTrading.swift # Paper-trading tables
    ├── AlertRuntimePersistence.swift  # Alert snapshot and GUI→runtime command queue
    └── JSONStore.swift                # Codable JSON files for disposable caches
```

## Data flow

1. Each native window/tab renders one `ContentView` backed by its own
   `ContentViewModel`; `NSWindow.tabGroup` remains the authority for tab layout.
2. Each card owns a `ChartViewModel`. Tradable market charts fetch `KlineData` through the
   `TickerDataSource` selected by `DataSourceFactory`; CoinMarketCap cards retain typed
   index models and never force non-price metrics into OHLCV.
3. Indicator values are calculated from a warm-up buffer and trimmed to the visible
   candles before the custom Canvas renderer draws them.
4. Binance and Alpaca streams update the latest matching candle in place; Coinbase trades are
   folded into it (`applyTick`) because Coinbase publishes no candle stream. The five-second
   refresh path covers other sources and recovery, while hidden tabs suspend both paths.
5. Candle responses are cached by symbol, interval, and limit. CoinGecko requests share a
   rate limiter, and its cache is flushed to disk when the app quits. The shared
   `CoinMarketCapClient` coalesces identical in-flight requests and uses a 15-minute cache
   for latest/Altcoin data and a six-hour cache for daily Fear and Greed history.
6. User data lives in one SQLite database, `degenview.sqlite` in Application Support,
   opened through `AppDatabase` (GRDB `DatabasePool`, WAL). `TabsStore`, saved views,
   `FavoritesStore`, `DrawingStore`, `PortfolioStore`, `PaperTradingStore`, and alert
   persistence each own their tables and keep their public API; nested chart configuration
   stays a JSON payload column so it evolves through Codable defaults rather than schema
   changes. The schema is a single idempotent `createSchema`; there are no migrations. State that
   fails to load disables writes instead of being replaced by an empty value. Small caches (klines, icons, FX, BTC history, quotes) remain `JSONStore`
   files; portfolio daily candles are the exception and live in SQLite (`candle`, `candle_coverage`). Alpaca and optional CoinMarketCap secrets live in Keychain rather than the
   database; only CMC chart type, range, and display settings enter workspace state.
   `KeychainPolicy.isDisabled` (XCTest, or `DEGENVIEW_NO_KEYCHAIN=1`) makes both stores behave as
   "nothing saved" without touching Keychain, so a rebuilt ad hoc signed binary raises no access prompt.
   Each tab and named saved view also stores ordered `ChartColumn` membership by the
   stable `TickerConfig.chartID`. Older documents without columns are repaired into the
   former two-column row-major arrangement when loaded.
   `ScriptStore` keeps each user script as `Scripts/<name>.pine` and everything a plain
   file can't carry (stable id, favorite, type, revisions, draft, compile cache) in
   `ScriptMetadata/<id>/`. A file finds its id through the `com.cryptocharts.script-id`
   extended attribute, falling back to `ScriptMetadata/index.json` when an editor's atomic
   save drops it; `ScriptFolderMonitor` refreshes views when the folder changes.
   Every `ScriptStore` mutation republishes its library scripts to `PineLibraryRegistry`, the
   synchronous `PineLibraryResolver` every app-side `PineCompiler.compile` passes, so
   `import user/Library/version` resolves to a Script Manager library by name.
7. During replay, each chart retains its immutable canonical history and exposes only a
   binary-searched prefix through `replayKlines`. `ReplayEngine` owns the tab's sole
   timestamp and one cancellable playback task. Timeline position (`progress`, `barNumber`,
   `startFraction`), `stepBackward` and `seek(toFraction:)` are derived from its in-memory
   timeline; `ReplaySession` (the persisted shape) carries none of it.
8. Binance, Coinbase and Alpaca optionally conform to `GranularReplayDataSource`. Their paginated
   lower-timeframe bars are aggregated against the provider-returned displayed-bar
   boundaries, preserving stock sessions, market gaps, and DST alignment.
9. Portfolio mutations are serialized by `PortfolioLedger`, persisted as one database
   transaction, and replayed by `PortfolioAccountingEngine`. Quote ticks update live
   valuation without replaying static accounting; historical edits invalidate only the
   affected snapshot suffix. Portfolio value history is one persisted snapshot per day;
   the 1D range is instead an in-memory 30-minute series (`PortfolioStore.intradayHistory`)
   replayed from recent 15m candles (1h for Alpaca). Daily candles behind that history come
   from `PortfolioCandleStore`: closed bars are stored once and a rebuild fetches only the
   days since the newest stored bar (or an older stretch if a transaction reaches back).
   Startup is value-first: persisted quotes paint the total immediately, `PortfolioQuoteFetcher`
   refreshes prices (one `BatchQuoteDataSource` call per source — Binance `ticker/24hr`,
   CoinGecko `coins/markets`, Coinbase `stats`, DEXScreener `pairs`, Alpaca `snapshots` — with
   hourly candles as the per-asset fallback), and only then does the history rebuild start,
   so it never queues ahead of prices on a shared rate limiter. The chart has its own loading
   flag (`isLoadingHistory`); per-day snapshots are computed off the main actor.
10. Selecting a reporting currency asks `FXRateService` for current or historical
    conversions—fiat rates from Frankfurter, BTC cross-rates from `BitcoinHistoryService`—
    and `PortfolioStore` converts transactions, quotes, and history into that currency once
    and caches the result as a projection. Switching back to an already-computed currency
    reuses its cached projection instead of re-converting or re-fetching rates.
11. Each market `ChartViewModel` owns an optional Pine configuration. Compilation and
    execution run behind a `PineExecutionHost`, fed in order through an `AsyncStream` of
    `PineFeedOperation`s (rebuild, live candle, REST snapshot) and applied only if its
    generation is still current. A rebuild replays history once; afterwards each WebSocket
    tick, live bar, or REST refresh becomes an execution event, so realtime bars roll back and
    commit like TradingView's (see `pine-compatibility.md`, Execution model). The runtime
    receives only an immutable OHLCV/replay prefix and emits renderer-neutral visuals. Draft source is
    persisted separately from last-valid applied source, so invalid edits do not remove
    the active result. Pine outputs are never shared between tabs or cards. A `strategy()`
    script's broker emulator is a value inside the runtime's per-bar state, so realtime
    rollback restores its orders, positions, and trades with everything else; its report
    and any `alert()` events ride along in `PineVisualOutput`.
    The pipeline is `PineLexer` → `PineParser` → `PineCompiler` (constant folding, inputs,
    `PineStructureValidator`, `PineTypeChecker`) → `PineRuntimeSession`. The session's call
    router maps each builtin name or namespace to one handler in a per-family extension;
    the pure parts (`PineMath`, `PineStrings`, `PineTA`, `PineOperators`…) hold no session
    state. A script's `import`s are linked by `PineLibraryLinker` into the compiled program;
    `PineLibraryLinkage` flattens them for the session, which runs each library in its own
    scope (`PineRuntimeContext.scope`, names registered as `path::Name`). `Pine/Model` and `Model/Script` must stay free of compiler/runtime types: the
    alert agent target compiles them.
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
15. Price alerts are evaluated by one `AlertRuntimeHost`, in either the app or the
    login-item agent, whichever holds `alert_runtime.lock`. Both processes open the same
    database: the owner saves the alert snapshot in one transaction per change, and the GUI
    sends edits by inserting `alert_command` rows, which the owner applies and deletes.
16. Pine script alerts run in the app only (the agent has no compiler or runtime).
    `PineExecutionUpdate.alerts` (realtime executions only) reaches `PineAlertCoordinator`
    through `ChartViewModel.pineAlertHandler`; the pure `PineAlertRouter` matches it to active
    `PineAlertSubscription`s (chart, symbol, timeframe, source hash) and
    `PineAlertFrequencyGuard` admits it once per call site, bar and mode. Admitted alerts are
    recorded in `pine_alert_event` (unique `dedupe_key`, NULL for `freq_all`) and fanned out by
    `PineAlertDispatcher` to independent `PineAlertChannel`s (macOS notification, in-app banner).
    Subscriptions are the `pine_alert_subscription` document table. They are separate from the
    price-alert snapshot, so `replaceSnapshot` never touches them.

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

## Brush flow

A brush stroke is a first-class drawing, not a paint layer: `BrushDrawing` holds
`TrendAnchor` points (continuous time + price), so it follows pan, zoom, resize and timeframe
changes like a trend line. It persists through `DrawingStore` as the `brush` drawing kind and
joins drawing undo through `DrawingUndoCoordinator.recordBrush`.

- **Capture** (`ContentViewModel.handleBrush`): press, drag, release. Points are raw chart
  coordinates — no candle or OHLC snapping. A sample is kept once the pointer has moved
  `BrushTuning.sampleDistance` points, and the live stroke is only `ChartViewModel.brushDraft`.
- **Commit**: on release the draft is projected with the current plot and simplified in
  screen space (Ramer–Douglas–Peucker at `BrushTuning.simplifyTolerance`, so a pixel means the
  same on DOGE and BTC); the kept points are the original anchors. One database write, one
  undo step. A click that never travels becomes a one-point dot. Esc drops the draft with no
  write and no undo.
- **Render** (`ChartPlot+Brush`): inside the chart's clipped layer, one smoothed `Path` per
  stroke. Smoothing is midpoint-quadratic, which cannot overshoot, and vertices that turn more
  than `BrushTuning.cornerAngle` stay sharp. It is render-only — stored points are untouched.
- **Hit testing** (`BrushGeometry.hit`): screen-space distance to each segment against half the
  stroke width plus `Drawing.hitTolerance`, after an expanded bounding-box rejection.
- **Move**: the pointer's screen delta applied to the stroke as it was at mouse-down, each point
  projected, offset and inverse-projected, so nothing accumulates and a future non-linear price
  scale still works. One undo step per drag. A click without movement opens `BrushEditor`.
- The brush tool stays armed after a stroke. Strokes are also selectable and deletable under
  the crosshair. The editor sits behind the same `isShowingLineEditor` gate as the other
  editors.

## Ruler flow

The ruler is a measurement, not an annotation: `RulerRect` is not `Codable`, lives only on
`ChartViewModel.rulers`, and is dropped on tool switch, timeframe switch, and relaunch.
`ContentViewModel.handleRuler` (the same window-wide mouse monitor as the other tools) runs:

- **Draw** — mouse-down on empty plot begins a draft and remembers the press. Dragging
  rubber-bands it; on release, a pointer that travelled at least `Drawing.hitTolerance`
  commits, otherwise the draft stays open and the next click commits (click-move-click).
  ⌘ snaps to OHLC through `snappedDrawingAnchor`, shared with Fibonacci.
- **Edit** — `ChartViewModel.rulerHit` finds a corner (resize) or an edge band (move);
  the interior is deliberately not a hit so a measurement can start inside another.
  Drags recompute from the rectangle as it was at mouse-down (`RulerRect.resized` /
  `translated`). Hover and selection publish `hoveredRuler` / `selectedRulerID`, which
  show handles and drive the plot cursor (`PlotCursor` in `ChartCardView`).
- **Keys** — Esc: cancel draft → clear rulers → disarm. Delete: selected ruler, else all.

Rendering is `ChartPlot+Ruler.swift`, shared by `CandleChartView` and `LineChartView`
through one `RulerOverlayState`: a gradient box (strongest at the end edge), centred
start→end arrows, corner handles on the hovered or selected ruler, a read-out card from
`RulerReadout`, and price tags in the gutter (`drawRulerPriceTags`, outside the series
clip). Colours are the chart's own bull/bear colours.

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

Completed drawings are stored in the `drawing` table, distinguished from trend lines by
`kind`.
Continuous anchor/body drags update published in-memory geometry at pointer frequency and
perform one database write on mouse-up. Both candlestick and probability line charts reuse
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

- The dashboard is always a grid of explicit, equal-width `ChartColumn` stacks; there is
  no alternative layout mode.
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
