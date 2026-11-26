# Import from TradingView

Brings layouts, drawings, watchlists and the user's own Pine scripts over from the TradingView
**desktop app** on the same computer. Entry points: the toolbar's layout menu, the new-tab page, and
File ▸ Import from TradingView on macOS.

## The offer at launch

The first launch that finds something to bring over asks, once, whether to import it, and lists how
many layouts (with their drawings), watchlists and scripts there are
(`host::tradingview::offer`, `ui::tradingview::offer`). Yes opens the import over the focused chart
tab.

- It counts as made as soon as it is on screen: the `tradingview.offered` setting, local to this
  computer. Opening the import by hand first sets it too.
- While there is nothing to offer, each launch looks again.
- The look runs off the frame and keeps only the counts.
- Unasked, no folder is opened that the system guards with a question of its own: on macOS the App
  Store build's container is left alone (`locate::unguarded_data_folders`). A Flatpak cannot see
  TradingView's folder at all. Neither gets the offer; the menu still works.
- It appears unasked, so Enter does nothing until a button is reached with Tab. Esc declines.

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
a sandboxed (Flatpak) DegenView gets access: through the portal, and the sheet says so there
(`ScanError::Sandboxed`). Bodies are gzip, brotli or zstd
(Windows is mostly zstd). Both formats are read while TradingView runs.

## What is read

| Response | Gives |
| --- | --- |
| Chart page (`/chart/` or `/chart/<id>/`) | `initData.uid` (layout id: Windows opens `/chart/` with none in the address) and `initData.content`: name, grid, per chart symbol, interval, style, price step, comparisons, indicators |
| `charts-storage …/layout/<id>/sources` | Drawings, as deltas: `null` deletes. Applied oldest first |
| `/api/v1/symbols_list/{all,custom,active}` | Watchlists (`###` entries are sections). Colour-flag lists are skipped |
| `pine-facade/get/USER;…`, `…/list?filter=saved` | Script sources (newest version) and saved names |

A newer `symbols_list/all/` or saved list that no longer names a list or script means it was deleted.
An empty or failed answer says nothing (a signed-out account's), and a listing speaks only for its
own data folder. Two accounts used in one installation cannot be told apart.

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
  `v` stacked. DegenView grids are columns, so a chart spanning columns sits in the first, and its
  cards are one height in every column: a chart TradingView draws full height beside two stacked ones
  is half height here.
- **Layouts** become saved layouts. A DegenView layout has one timeframe, TradingView one per chart:
  the first chart's is used and the preview offers the others. Charts TradingView keeps but its grid
  hides are not imported.
- **Drawings**: `model::tradingview::convert::TOOLS` is the one table. TradingView stores placement
  points, which our tools are built from. A time is `time_t + offset × bar`. A drawing belongs to a
  layout and a symbol there, to a chart and a market here: it goes to the first chart showing its
  symbol, else the first opening on that market, else to the first chart under its own market, and
  the preview says that it shows once the chart is switched there.
- **Settings** (`convert/style.rs`) carried over beyond colour, width and text: line and box labels'
  side (TradingView's `vertLabelsAlign` names the edge of the words that meets the line, so `bottom`
  is *above* it), line and arrow measurements, parallel channel lines (`level1`…`level7`) and label,
  pitchfork kind (`style`), median and levels, regression trend, measurements' contents, volume
  profile, anchored VWAP (percentage bands have no counterpart), fib levels where TradingView numbers
  them as DegenView does, Elliott degree, table alignment, and the tools that name a setting their own
  way (`arrowColor`, `flagColor`, `bordercolor`, `lineStyle`). The speed fan's and the Gann tools'
  levels are DegenView's own, and the preview says so. To list a tool's settings, open the public
  embed widget (`s.tradingview.com/widgetembed`), take webpack's `require` from
  `webpackChunktradingview`, and call `getLineTool(name).createProperties().state()`.
- **Posts** have no price. TradingView sets one beside its bar: `position` percent of the way from
  the high (low when negative) to the chart's edge; the stored point price means nothing.
  `model::tradingview::placement` applies that to the candles 75 bars either side, fetched from the
  matched market, and the preview says the height is estimated. Without those candles the post is
  left out (Reload or Search Again asks again). A provider with no time-range endpoint (CoinGecko)
  cannot place them.
- **Pinned** drawings (`anchored: true`) sit at a place on the screen and are left out.
- **Script names** are made into file names every system accepts. A watchlist none of whose symbols
  has a market is not added.
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

Verified on every drawing in real data (34, 12 tool types): each stored setting checked against
TradingView's code, the imported layout looked at in the app, and both real scripts compile. Every
grid code, interval string and mapped indicator opens as written. TradingView itself was not open
beside it for a side-by-side comparison.

Verified in a Flatpak sandbox (Fedora, aarch64; this build run with the installed app's permissions,
on X11, Secret portal off): picking a folder in the XDG portal, including a hidden one under
`~/snap/tradingview/current` (a symlink), and a whole import through the document portal. The packaged
bundle itself was built by CI but not run.

Verified with a made-up data folder: a whole import on Linux (Fedora, aarch64, a bare X server),
including picking the folder in the `zenity` panel `rfd` falls back to without a portal. Also: sync
accepts everything an import writes (a test holds it), and mutation fuzzing of the cache readers,
document parsers and drawing conversion found no panic.

**Not verified** — check against a real drawing before trusting:

- Picking a folder in the macOS panel.

- How each tool's stored points and settings map, for tools never seen in real data. Seen:
  TrendLine, Brush, Highlighter, RiskRewardLong, AnchoredVWAP, Text, TextNote, Note, PriceLabel,
  Table, Tweet, Emoji. TradingView's Basic plan saves one layout, so no scratch layout could be
  made to draw the rest in.
- Left out: ellipse, signpost and Three drives (stored shape unknown), arrow marks left and right,
  bars pattern, ghost feed, images, emoji, icons, stickers, and text or notes pinned to the screen.
- The settings of tools never seen in real data are read from TradingView's defaults, not from a
  drawing the web app saved.
- Linux: the folder (`~/.config/TradingView`) and cache format were seen on an unpacked snap run
  under FEX; the app could not connect there, so no real Linux data has been read.
