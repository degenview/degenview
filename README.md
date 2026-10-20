# DegenView

[![Total downloads](https://img.shields.io/github/downloads/degenview/degenview/total)](https://github.com/degenview/degenview/releases)

A cross-platform market dashboard for watching crypto, stocks, prediction markets, and
CoinMarketCap market-wide indices in customizable charts.

Its headline feature is a **local Pine Script v6 engine** — write TradingView-style scripts,
run them on your own machine, keep every line of source on your device.

![DegenView showing four linked candlestick charts with indicators and a watchlist](assets/screenshots/screenshot.png)

## Features

### Markets and live data

- **Crypto** — Search Binance, Coinbase, CoinGecko, and DEXScreener side by side; DEX pairs
  use GeckoTerminal for historical OHLCV.
- **Stocks** — US equities through Alpaca's IEX feed (credentials stored securely in Keychain on
  macOS, or in environment variables elsewhere).
- **Prediction markets** — Polymarket and Kalshi events, charted as probabilities. No key needed.
- **CoinMarketCap indices** — Altcoin Season and Crypto Fear and Greed, historical or latest.
- **Live updates** — Binance and Coinbase crypto receive WebSocket updates; other sources
  refresh automatically.
- **Timeframes** — 1H, 1D, 1W, 1M, 3M, and 1Y candles with scroll-wheel zoom.
- **Multi-source identity** — The same symbol can be added from different sources without
  being treated as a duplicate.

### Pine Script

A TradingView-style scripting engine that runs entirely on your machine — scripts are lexed,
parsed, and evaluated over your chart's bars in-process, with no account and no source ever
leaving your device.

- **Local evaluation** — Plots, shapes, backgrounds, and candle colors render straight onto the
  chart from Pine Script v6.
- **Typed `input.*` controls** — int, float, bool, color, time, and string inputs re-evaluate
  the script live without recompiling.
- **Script Manager** — a `.pine` file library with a code editor, diagnostics, and a live
  preview chart that re-runs as you type.
- **Pine alerts** — a script's `alert()` fires on a live bar (see [Alerts](#alerts)).

### Charts and analysis

- Hand-drawn OHLC candlesticks, line series, grid, time axis, adaptive price precision,
  current-price overlay, and price-change readout — no third-party chart library.
- **Indicators** — Volume, RSI, configurable EMA, Bollinger Bands, and Supertrend flip markers.
- **Trend lines, Fibonacci retracements, and brush strokes** — anchored to time and price, with
  per-window undo/redo (⌘Z / ⇧⌘Z).
- **Ruler** — Temporary move measurement (percent, price, duration, bar count).
- **Crosshair** — Synchronized vertical inspection across all charts in a tab.
- **Price zoom** — Drag a chart's Y-axis to adjust its vertical scale.

### Portfolio tracking

- **Multiple portfolios** with a transaction ledger and weighted-average accounting.
- **Live analytics** — value, allocation, 24-hour movement, cost basis, realized/unrealized
  P&L, and best/worst performers.
- **Multi-currency reporting** — USD, EUR, GBP, JPY, CHF, and BTC, with cached FX conversion.
- **Portfolio history** — transaction-aware value snapshots across 1D–ALL ranges.
- **CSV import/export**, including CoinMarketCap transaction exports with automatic asset
  mapping.

### Alerts

- **Local price alerts** — absolute or percentage crosses-above/below, once or repeating,
  evaluated locally with no account or remote service.
- **Pine script alerts** — notify when a script's `alert()` fires on a live bar.
- **Delivery** — in-app banner, and a desktop notification on macOS, independently configurable.
- **Background delivery on macOS** — signed bundles embed a login-item agent that keeps price
  alerts, native notifications, and price-alert webhooks running after the GUI quits. Pine alerts
  remain app-only.
- **Alerts center** — search and filter active, triggered, paused, and history records.

### MCP server

An optional, local [Model Context Protocol](https://modelcontextprotocol.io) server lets an AI
assistant or automation tool inspect and drive DegenView. It is **off by default** and turned on
in Settings ▸ MCP Server.

- **Local and authenticated** — listens on `127.0.0.1` only, and every request needs a bearer
  token. A token is shown once when you generate it; only a hash is kept. Revoking a client cuts it
  off at once.
- **Fine-grained permissions** — every permission is labelled **Read** or **Write**. Set a global
  maximum, then narrow it per client; a client never gets more than the maximum, and changes apply
  immediately.
- **What clients can do** — list workspaces and charts, add charts and indicators, list and manage
  price alerts, and read Pine scripts, diagnostics, inputs, styles, reports, and alert
  subscriptions, with permissioned edits to a script's draft and saved source.

### Paper trading

A local, persistent simulator with an order ticket, market/limit/stop orders, net-position
tracking with weighted-average cost, and position/order markers on the chart. Paper orders
never route to a live exchange.

### Historical bar replay

Pick a historical candle and step through the market with no lookahead, with play/pause, a
timeline scrubber, speed and interval controls, and shared clock across every chart in a tab.

### Dashboard and persistence

- **Native tabs and windows on macOS** — independent charts, timeframe, layout, zoom, and
  drawing state per tab; drag to reorder, detach, or merge. Other platforms use a single
  workspace window.
- **Restored sessions** — tabs, order, and window grouping return on relaunch.
- **Flexible layouts** — vertical stacks or multi-column grids with drag reorder and drop
  columns.
- **Saved views** — named ticker sets with their timeframe, arrangement, zoom, source,
  indicators, and appearance.
- **Favorites sidebar** — an app-wide, reorderable watchlist.
- **Appearance** — Dark (the default), Light, and System themes.

## Requirements

- macOS (Apple Silicon or Intel), Windows (x64), or Linux (x64)
- [Rust](https://rustup.rs) 1.95 or newer (`cargo` on your `PATH`)
- A free Alpaca account and API keys only if you want stock data — set `DEGENVIEW_ALPACA_KEY_ID`
  and `DEGENVIEW_ALPACA_SECRET_KEY`, or store them in Keychain on macOS
- No CoinMarketCap API key is required; adding one in Settings is optional
  (`DEGENVIEW_CMC_API_KEY`)

## Build and run

```bash
cargo run
```

Build a release binary:

```bash
cargo build --release
```

The binary lands at `target/release/degenview` (`degenview.exe` on Windows).

On macOS you can also build a Finder-launchable app bundle:

```bash
./tools/bundle.sh
open target/DegenView.app
```

The default bundle is unsigned and uses foreground-only alert evaluation. To make the embedded
`DegenViewAlertAgent.app` available through Login Items, sign both bundles with an Apple-issued
identity, an explicit shared Keychain access group, and one provisioning profile per bundle. The
profiles (Developer ID, for the App IDs `es.nico.degenview` and `es.nico.degenview.alert-agent`)
are required: `keychain-access-groups` is a restricted entitlement, and macOS kills a signed
build at launch without a profile that grants it.

```bash
DEGENVIEW_CODESIGN_IDENTITY="Developer ID Application: Example (TEAMID)" \
DEGENVIEW_KEYCHAIN_ACCESS_GROUP="TEAMID.es.nico.degenview.shared" \
DEGENVIEW_APP_PROFILE=/path/to/DegenView.provisionprofile \
DEGENVIEW_HELPER_PROFILE=/path/to/DegenViewAlertAgent.provisionprofile \
./tools/bundle.sh
```

The helper is installed at
`DegenView.app/Contents/Library/LoginItems/DegenViewAlertAgent.app`. DegenView falls back to
foreground evaluation when registration is unavailable, awaiting approval, or its heartbeat is
stale.

The Release workflow signs the macOS builds the same way, then notarizes and staples them. It reads
these repository secrets: `MACOS_CERT_P12` (base64 `.p12` of the Developer ID Application
certificate and key), `MACOS_CERT_PASSWORD`, `MACOS_CODESIGN_IDENTITY`,
`MACOS_KEYCHAIN_ACCESS_GROUP` (`TEAMID.es.nico.degenview.shared`), `MACOS_PROFILE_APP` and
`MACOS_PROFILE_HELPER` (base64 provisioning profiles), and an App Store Connect API key
for `notarytool`: `ASC_KEY_P8` (base64 `.p8`), `ASC_KEY_ID`, `ASC_ISSUER_ID`.

On Linux the release tarball ships a `degenview.desktop` launcher and `degenview.png` icon. To
get a launcher and dock icon, install them:

```bash
install -Dm755 degenview "$HOME/.local/bin/degenview"
install -Dm644 degenview.png "$HOME/.local/share/icons/hicolor/512x512/apps/degenview.png"
install -Dm644 degenview.desktop "$HOME/.local/share/applications/degenview.desktop"
```

Releases: push a tag like `v0.1.0` and the Release workflow builds macOS (arm64 + Intel),
Windows, and Linux binaries and publishes a GitHub release you can download.

## Documentation

- [Architecture](docs/architecture.md): project structure and data flow, including the
  [MCP server](docs/architecture/mcp.md)
- [MCP remaining work](docs/mcp-remaining-work.md): what is still planned for the MCP server

## License

DegenView is licensed under the [GNU General Public License v3.0 only](LICENSE)
(`GPL-3.0-only`).

Copyright © 2026 Nico Oelgart.
