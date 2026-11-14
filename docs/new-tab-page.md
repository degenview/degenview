# New-tab page

What a chart tab shows before it has a chart: `ui/workspace/empty_state/` draws it,
`workspace/empty_page/` feeds it. Read this before changing either.

## Layout

A title, a pill search launcher (a button: it opens the [symbol picker](symbol-picker.md) in Add mode, where typing happens), a tab bar and
the selected tab's content. No picture at the top. Title and search are at most 760 pt wide, the tab
content at most 1000 pt in up to five columns.

| Tab | Content |
| --- | --- |
| Recent | A list card of every remembered market (`RECENT_LIMIT`, 24): artwork with its data-source badge, symbol, source, week sparkline and, when the row is wide enough, the week's change (share over amount, coloured by direction; share only for prediction markets). The card takes the room left under it less four rows (at least three) and scrolls; only rows in view are drawn and read. "Clear" link under the card, "Remove from Recent" context menu. Not filtered against favorites (they are separate tabs now). |
| Favorites | Market tiles in sidebar order, at most `ITEM_LIMIT`; when more exist, a last "N more" tile opens the watchlist sidebar. |
| Suggestions | Ten crypto tiles (Binance, quoted in USDT) and, under a hairline, ten stocks and ETFs (Alpaca), always. Tiles look like favorites: artwork with the source's logo on its corner and the week's sparkline at the right. Without an Alpaca key a stock tile has no sparkline, and still adds the chart, which then shows the chart's own no-data state. |
| Layouts | One card per saved layout: name, "N charts · TF", first three tickers. No thumbnail: layouts store no image. |

The tab bar is `kit::icon_tabs`, the same pill track as the asset picker. The page opens
on the first of Recent, Favorites, Layouts that has something in it, and on Suggestions for someone
with nothing. The person's pick lasts until a chart replaces the page (`EmptyPageCache::reset`).
An empty tab says why, in a note card.

Tiles and the search box are opaque (page colour under the card fill) so the backdrop never shows
through them. The title is `TITLE2 + 11`, larger than the `TITLE2 + 3` page title of settings pages: it is a landing page.

## Sparklines

Each market tile and row shows the daily closes of its last seven calendar days (a stock's five
sessions), green when the week ended at or above where it began, red otherwise (`kit::sparkline::paint`).
`empty_state::show` reports the markets it drew (`EmptyStateOutput::wanted`); the page's own
`service::market::trend::TrendLoader` (7 days) reads each through `service::fetch_klines` (`1d`, 8
candles, on the provider's work-queue lane) and keeps the closes that fall in the week
(`model::market::trend::closes_within`): fresh for 10 minutes, a market with no history retried
after 5. A failed refresh keeps the week already shown. States: not read yet = faint flat line, no history = nothing,
else the week. Only markets of the shown tab are read. Keyed by `InstrumentId::key()`; the
provider id is `InstrumentId::api_symbol()`.

## Backdrop

`backdrop.rs`: a trend line over the top of the page, behind the title, in the colour of a rising market. It is a pure
function of the clock (straight segments between hashed vertices, alternating peak and dip around a slowly drifting
trend, so it zigzags like a price), scrolling left with the
newest price at the right, over a faint chart grid (five horizontal lines across the strip, vertical ones every 96 pt scrolling with the line, built as vertex-coloured strips so they fade into the page), so it keeps no state and never repeats. The page starts `PAGE_TOP` (48 pt) down; the line wanders in the 122 pt under that and the page content
starts below it; the fade fill reaches 340 pt down. The fill is a vertex-coloured mesh, the one gradient on
this page, asked for as decoration; cards stay flat. It asks for a frame every
33 ms for as long as the page is drawn, focused or not (a window behind another still shows it, and a frame
every 250 ms made it crawl); the system does not draw a hidden or minimised window, so that costs nothing. There is no reduced-motion setting in the app yet.

## Checking it

`DEGENVIEW_GALLERY_DIR=<dir> cargo test --lib empty_state -- --ignored` writes every tab, with and without data, dark
and light, wide and narrow. Headless pictures show fallback artwork for tickers (icons load from the network).
