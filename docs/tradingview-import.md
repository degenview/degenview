# Import from TradingView

Brings layouts, drawings, watchlists and the user's own Pine scripts over from the TradingView
**desktop app** on the same computer. Entry points: the toolbar's layout menu, the new-tab page, and
File ▸ Import from TradingView on macOS.

## Where the data comes from

TradingView keeps charts on its servers and has no export for drawings or layouts. The desktop app
is a Chromium shell, so what it last loaded sits in its HTTP cache. We read that: offline, read-only,
no sign-in, no TradingView API.

Consequences, which the sheet says in words:

- Only what the app opened recently on this computer is there. Chromium evicts old entries.
- A cached response can be older than the server: a drawing deleted since may still be there.
- The format is TradingView's own and undocumented. Any update can change it. Parsers treat every
  field as optional, and a document that no longer has the expected shape contributes nothing.

### Privacy

`service::tradingview` opens only responses whose address `route()` accepts: chart pages, a layout's
drawing sources, the account's own watchlists, its own Pine sources and the saved-script list.
Cookies, tokens, profile and billing responses are never opened. Addresses carry a JWT in their
query: they are not logged, stored or shown. `route()` is tested both ways; widen it with care.

### Folders and formats

| Platform | Data folder | Cache format |
| --- | --- | --- |
| macOS | `~/Library/Application Support/TradingView` (App Store build: under `~/Library/Containers/com.tradingview.tradingviewapp.desktop`) | simple |
| Windows | `%LOCALAPPDATA%\Packages\TradingView.Desktop_*\LocalCache\Roaming\TradingView` (MSIX), else `%APPDATA%\TradingView` | blockfile |
| Linux | `~/snap/tradingview/current/.config/TradingView`, `~/.config/TradingView` | simple |

The cache is `Cache/Cache_Data` inside it. "Choose Folder…" reads any other folder, which is also how
a sandboxed (Flatpak) DegenView gets access: through the portal. Bodies are gzip, brotli or zstd
(Windows is mostly zstd). Both formats are read while TradingView runs.

## What is read

| Response | Gives |
| --- | --- |
| Chart page (`/chart/` or `/chart/<id>/`) | `initData.uid` (layout id: Windows opens `/chart/` with none in the address) and `initData.content`: name, grid, per chart symbol, interval, style, price step, comparisons, indicators |
| `charts-storage …/layout/<id>/sources` | Drawings, as deltas: `null` deletes. Applied oldest first |
| `/api/v1/symbols_list/{all,custom,active}` | Watchlists (`###` entries are sections). Colour-flag lists are skipped |
| `pine-facade/get/USER;…`, `…/list?filter=saved` | Script sources (newest version) and saved names |

A layout whose page is not cached is rebuilt from its drawings: one chart per symbol, at the interval
most of its drawings were made on.

## How it maps

- **Symbols** (`service::tradingview::matcher`): the same market on its own exchange, else the same
  base **and quote** on another (badge "Other exchange"). Only an exact name matches; derivatives are
  never mapped. The user can change or skip any symbol, like the CoinMarketCap import
  (`ui::kit::matching` draws both).
- **Layouts** become saved layouts. A DegenView layout has one timeframe, TradingView one per chart:
  the first chart's is used and the preview offers the others. Charts TradingView keeps but its grid
  hides are not imported.
- **Drawings**: `model::tradingview::convert::TOOLS` is the one table. TradingView stores placement
  points, which our tools are built from. A time is `time_t + offset × bar`. A drawing belongs to a
  layout and a symbol there, to a chart and a market here: it goes to the first chart showing its
  symbol, else to the first chart under that market.
- **Positions** store distances in price steps. The step comes from a chart's `pricescale`; without
  one the distance follows from the drawing's own risk figures `(accountSize − amountStop) / qty`;
  otherwise the position is left out.
- **Indicators**: Volume, EMA and Bollinger are native; others map to bundled Pine ones or to the
  user's own script, with default settings.
- **Nothing is replaced.** A taken name gets a number.

Everything left out is listed per item in the preview (`LeftOut`).

## Layering

`model::tradingview` (pure: parse, convert, plan) ← `service::tradingview` (cache, folders, matcher)
← `host::tradingview` (flow, apply) → `ui::tradingview` (sheets, returns `Action`s).

## Verified and not

Verified on real data: macOS (31 of 32 cached drawings; the one left out is an emoji) and Windows 11
ARM64 with the MSIX app 3.5.1 signed in.

**Inferred, not verified** — check against a real account before trusting:

- Class names of tools never seen in real data. Seen: TrendLine, Brush, Highlighter, RiskRewardLong,
  AnchoredVWAP, Text, TextNote, Note, PriceLabel, Table, Tweet, Emoji. The others follow
  TradingView's Charting Library names.
- Grid codes: `h` read as stacked rows, `v` as columns side by side.
- Chart style numbers above 1, and indicator ids other than `Volume` and `Overlay`.
- Fibonacci level fields.
- Linux: the folders and the snap app (amd64 only).
