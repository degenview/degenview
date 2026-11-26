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
  base **and quote** on another (badge "Other exchange"). Only an exact name matches; derivatives and
  formulas are never mapped. An exchange that does not answer is not replaced by another: the symbol
  stays unmatched, with Search Again. A match the search made can be changed or skipped, one the
  user made changed or reset (`ui::kit::matching` draws these rows and the CoinMarketCap import's).
  Nothing is dropped for being unreadable: every stored chart keeps its grid place.
- **Grids** (`model::tradingview::grid`): the web app's own arrangement table; `h` is side by side,
  `v` stacked. DegenView grids are columns, so a chart spanning columns sits in the first.
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
- **Order**: cached responses are applied by the response time stored with each, so a newer delta
  deletes what an older one added.
- **Choices** survive Reload and Choose Folder. Choose Folder shows only where the app has file
  panels (`platform::has_file_dialogs`: not Windows).
- **Indicators**: Volume, one EMA at a length the app offers, and Bollinger are native; others map to bundled Pine ones or to the
  user's own script, with default settings.
- **Nothing is replaced.** A taken name gets a number.

Everything left out is listed per item in the preview (`LeftOut`).

## Layering

`model::tradingview` (pure: parse, convert, plan) ← `service::tradingview` (cache, folders, matcher)
← `host::tradingview` (flow, apply) → `ui::tradingview` (sheets, returns `Action`s).

## Verified and not

Verified on real data: macOS (31 of 32 cached drawings; the one left out is an emoji) and Windows 11
ARM64 with the MSIX app 3.5.1 signed in.

Verified against the web app's own code (3.5.1, from the cached bundles): drawing class names (its
tool registry; note `schiff_pitchfork` is `LineToolSchiffPitchfork2` and the *modified* one is
`LineToolSchiffPitchfork`), series style numbers, the grid arrangement table, built-in indicator ids,
and the Fibonacci level layout `[coeff, color, visible, text]`. To check a later version, search the
cached `static.tradingview.com` scripts for `["LineTool…",async`, `"2h":["h",0,1]` and
`@tv-basicstudies`.

**Not verified** — check against a real drawing before trusting:

- How each tool's stored points and settings map, for tools never seen in real data. Seen:
  TrendLine, Brush, Highlighter, RiskRewardLong, AnchoredVWAP, Text, TextNote, Note, PriceLabel,
  Table, Tweet, Emoji. TradingView's Basic plan saves one layout, so no scratch layout could be
  made to draw the rest in.
- Left out: ellipse, signpost and Three drives (stored shape unknown), arrow marks left and right,
  bars pattern, ghost feed, images, emoji, icons, stickers, and text or notes pinned to the screen.
- Fibonacci and Gann levels other than the retracement's and extension's are DegenView's own; the
  preview says so.
- Linux: the folder (`~/.config/TradingView`) and cache format were seen on an unpacked snap run
  under FEX; the app could not connect there, so no real Linux data has been read.
