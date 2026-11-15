<div align="center">

<img src="assets/brand/DegenView-512-rounded.png" width="128" alt="DegenView logo">

# DegenView: the free, open source TradingView alternative

Charts for crypto, stocks and prediction markets, with Pine Script v6 that runs on your own machine.
No account. No subscription.

[![Latest release](https://img.shields.io/github/v/release/degenview/degenview?include_prereleases&label=release)](https://github.com/degenview/degenview/releases)
[![Downloads](https://img.shields.io/github/downloads/degenview/degenview/total)](https://github.com/degenview/degenview/releases)
[![Stars](https://img.shields.io/github/stars/degenview/degenview)](https://github.com/degenview/degenview/stargazers)
[![License](https://img.shields.io/github/license/degenview/degenview)](LICENSE.md)
![Platforms](https://img.shields.io/badge/macOS%20%7C%20Windows%20%7C%20Linux-lightgrey)

[Website](https://degenview.app) · [Download](#download) · [Docs](https://docs.degenview.app) · [Compare with TradingView](https://degenview.app/tradingview-alternative/)

<br>

![DegenView showing a BTCUSDT candlestick chart with a volume panel, RSI, EMA, a Pine overlay and a MACD pane](assets/screenshots/hero.png)

</div>

## Why DegenView

- **Free and open source.** GPL-3.0, no paywalled features.
- **No account.** Install it and start charting.
- **Private.** Your scripts, alerts, watchlists and portfolios stay on your device.
- **Native and fast.** Built in Rust for macOS, Windows and Linux.

## Features

### Markets

- **Crypto** from Binance, Coinbase, CoinGecko and on-chain DEX pairs, searchable side by side.
- **US stocks and ETFs** through Alpaca. A free key is all you need.
- **Prediction markets** from Polymarket and Kalshi, charted as probabilities.
- **Market-wide gauges** such as Crypto Fear and Greed and Altcoin Season.
- **Live prices** streamed in real time, with watchlists and a quick symbol search on `⌘K` or `Ctrl+K`.

### Charting

- **18 chart types** including Candles, Heikin Ashi, Renko, Kagi, Line Break and Point and Figure.
- **80+ drawing tools**: trend lines, channels, Fibonacci, Gann, chart patterns, Elliott waves, position and forecast tools, notes and more.
- **25+ indicators** with a searchable picker and live previews.
- **Compare** any number of markets on one chart, in percent or log scale.
- **Flexible layouts**: grids of charts, colour-linked tabs, saved layouts and chart templates.
- **Bar replay**: step back in time and trade the market again with no lookahead.
- **Screenshot Studio** for sharp chart images you can copy, save or share.

### Pine Script

Write TradingView-style Pine Script v6 and run it locally. Nothing is uploaded.

- **Pine Studio** with a script library, a smart editor and a live preview chart.
- **Step debugger** and **profiler** to find bugs and slow lines fast.
- **20 bundled Pine indicators** such as Ichimoku Cloud, MACD, VWAP and Stochastic.
- **AI assistant** that edits your script, using your own API key.
- **Editor themes** to match your taste.

| Step debugger | Profiler |
| :---: | :---: |
| ![Pine Studio paused on a breakpoint, with variables, watches and the call stack](assets/screenshots/pine-debugger.png) | ![Pine Studio profiler showing per-line time bars and the hottest lines and blocks](assets/screenshots/pine-profiler.png) |

### Alerts

- **Price alerts** that cross above or below a level, once or repeating.
- **Pine alerts** fired by your own scripts.
- **Notifications** in the app and on your desktop, plus **webhooks** to reach other tools.
- **Background alerts on macOS**, even after you quit the app.

### Trading and portfolio

- **Paper trading** with market, limit and stop orders, shown right on the chart.
- **Portfolios** with a transaction ledger, profit and loss, allocation and history.
- **Multiple currencies**: USD, EUR, GBP, JPY, CHF and BTC.
- **CSV import and export**, including CoinMarketCap exports.

### Extras

- **Session statistics** that show how often gaps fill, prior-day levels break and weekdays behave, with sample sizes.
- **MCP server** so your AI assistant can read and drive DegenView. Off by default, local only and permission based.
- **Light and dark themes**, and sessions that come back as you left them.

## Download

Get the latest build from the [releases page](https://github.com/degenview/degenview/releases) or pick your platform.

| Platform | Install |
| --- | --- |
| **macOS** (Apple Silicon and Intel) | Open the `.dmg` and drag DegenView to Applications, or use Homebrew |
| **Windows** (x64 and ARM64) | Run the `-setup.exe`, or unzip the portable `.zip` |
| **Linux** (x64 and arm64) | Use the `.tar.gz` or the Flatpak bundle |

DegenView updates itself from inside the app.

<details>
<summary>Homebrew</summary>

```bash
brew install --cask degenview/degenview/degenview      # latest final release
brew install --cask degenview/degenview/degenview@rc   # latest release candidate
```

</details>

<details>
<summary>Linux notes</summary>

Unpack the tarball and add a launcher and icon:

```bash
install -Dm755 degenview "$HOME/.local/bin/degenview"
install -Dm644 degenview.png "$HOME/.local/share/icons/hicolor/512x512/apps/degenview.png"
install -Dm644 degenview.desktop "$HOME/.local/share/applications/degenview.desktop"
```

Or install the sandboxed Flatpak:

```bash
flatpak install --user DegenView-<tag>-linux-<x64|arm64>.flatpak
flatpak run es.nico.degenview
```

Flatpak bundles do not update themselves, so download the next one from the releases page.
Linux needs glibc 2.35 or newer. On an older distro, use the Flatpak.

</details>

<details>
<summary>Windows notes</summary>

The installer needs no admin rights and keeps your data when you remove it. It is not signed yet,
so SmartScreen may ask you to approve the first launch.

</details>

Want to build it yourself? See [BUILDING.md](BUILDING.md).

## Learn more

- [User guide](https://docs.degenview.app) for markets, charts, Pine Studio, alerts and more
- [BUILDING.md](BUILDING.md) to build from source and package a release
- [Architecture](docs/architecture.md) for contributors

DegenView is an independent project and is not affiliated with TradingView.

## Star History

<a href="https://www.star-history.com/?repos=degenview%2Fdegenview&type=date&legend=top-right">
 <picture>
   <source media="(prefers-color-scheme: dark)" srcset="https://api.star-history.com/chart?repos=degenview/degenview&type=date&theme=dark&legend=bottom-right" />
   <source media="(prefers-color-scheme: light)" srcset="https://api.star-history.com/chart?repos=degenview/degenview&type=date&legend=bottom-right" />
   <img alt="Star History Chart" src="https://api.star-history.com/chart?repos=degenview/degenview&type=date&legend=bottom-right" />
 </picture>
</a>

## License

DegenView is licensed under the [GNU General Public License v3.0 only](LICENSE.md)
(`GPL-3.0-only`).

Copyright © 2026 Nico Oelgart.

### Fonts

Two fonts are built into DegenView. They are not covered by the GPL. Each keeps its own licence, the
[SIL Open Font License 1.1](https://openfontlicense.org), and neither declares a Reserved Font Name. The
licence texts are in [`assets/fonts`](assets/fonts) and ship with every download (the macOS app's
`Contents/Resources/Licenses`, the Flatpak's `share/licenses`, the Linux and Windows archives and the
Windows installer's `licenses` folder).

- **JetBrains Mono** (the NL build, without ligatures, version 2.304), the Pine editor's default font.
  Copyright 2020 The JetBrains Mono Project Authors, <https://github.com/JetBrains/JetBrainsMono>.
  [Licence](assets/fonts/OFL-JetBrainsMono.txt).
- **Noto Sans Symbols 2** (a subset of version 2.008), the fallback for symbols such as ▲ ● ★ ✔ that other
  fonts lack. Copyright 2022 The Noto Project Authors, <https://github.com/notofonts/symbols>.
  [Licence](assets/fonts/OFL-NotoSansSymbols2.txt).

Where each file comes from, how to fetch it again and how it is checked: [`assets/fonts/README.md`](assets/fonts/README.md).
