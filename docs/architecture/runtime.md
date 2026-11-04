# Runtime and data flow

## Startup and windows

`main` opens `Store`, builds `eframe::NativeOptions`, and starts `DegenViewApp` in a hidden root
viewport. Each visible deferred viewport renders one `TabWindows` entry: the app-owned tab strip and
its active `ChartWorkspace`, `PortfolioTab`, or `PineStudio`. `TabWindowHost` owns grouping and
dragging; `DegenViewApp` owns `TabSlot`s and routes Settings/Alerts window actions to shared hosts.

`UpdateHost` is app-owned and projected into each window and Settings. It checks after launch and
every six hours, then exposes availability, byte progress, failure, and restart state.

## Concurrency

- UI and state coordination run on the app thread. Only `mcp/` has an async runtime.
- Blocking replaceable REST jobs use `service::work_queue` and `std::sync::mpsc`. Fetch generations
  discard stale replies.
- `BinanceLiveFeed`, `WatchlistQuoteCoordinator`, and Coinbase's ticker socket own reconnecting
  threads and channels. Hidden/occluded workspaces suspend feeds.
- App-wide state uses `Arc<Mutex<...>>`; per-tab content is `Arc<Mutex<TabContent>>`. Hold the store
  lock only for short operations. Portfolio/paper compute candidates first, then transact.

## Market and chart flow

1. A market chart owns symbol/source, candles, fetch state, and `ChartCard`.
2. `service::for_source` selects `MarketSource`; results are cached by source/symbol/interval.
3. Binance streams replace the forming candle; five-second REST refresh covers other providers and
   recovery. `merge_fetched_candles` rejects older snapshots.
4. Indicators compute over full history, trim to the visible tail, then render through
   `egui::Painter`.

CoinMarketCap index cards keep typed index points rather than forcing them into OHLCV. Portfolio
cards are non-market charts and must be skipped by market-data paths.

## Persistence and domain ownership

- `Store` opens bundled-rusqlite `degenview.sqlite` in Application Support. Document tables are
  ordered `id/position/payload` JSON; `setting` and `window_group` are keyed tables. The schema is
  idempotent with no migrations. Serde defaults evolve nested models.
- `replace_documents` refuses to replace a collection containing undecodable rows. Swift data
  imports once through `store::import::run_once`, only into empty Rust tables.
- `PortfolioLedger` serializes mutations and persists one snapshot transaction. Pure accounting
  replays weighted-average basis/P&L; historical edits invalidate only the affected suffix. Live
  quotes and cached FX update reporting projections separately.
- `PaperTradingEngine` produces candidate snapshots. `PaperHost` persists and publishes every
  result, including rejected-order domain errors, and refuses writes after initial-load failure.
  `PaperPanel` is per-window presentation over the app-wide host.
- `AlertHost` owns the pure crossing engine, persistence commands, and evaluated snapshot. Alert
  editors hold quote demand while open; banners project triggers into every workspace.
- Drawings store time/price anchors per chart. The interactive layer handles gestures/hit tests and
  reports commands; renderers project coordinates each frame
  ([drawing tools](../drawing-tools.md)).
