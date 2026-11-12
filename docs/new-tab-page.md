# New-tab page

What a chart tab shows before it has a chart: `ui/workspace/empty_state/` draws it,
`workspace/empty_page/` feeds it. Read this before changing either.

## Layout

A title, a pill search launcher (a button: it opens Add Chart, where typing happens), a tab bar and
the selected tab's content. No picture at the top. Title and search are at most 760 pt wide, the tab
content at most 1000 pt in up to five columns.

| Tab | Content |
| --- | --- |
| Recent | A list card: artwork with its data-source badge, symbol, source, week sparkline. "Clear" link, "Remove from Recent" context menu. Not filtered against favorites (they are separate tabs now). |
| Favorites | Market tiles in sidebar order, at most `ITEM_LIMIT`; "Show N more" opens the watchlist sidebar. |
| Suggestions | Ten crypto tiles (Binance, quoted in USDT), plus ten stocks and ETFs under their own caption only when an Alpaca feed is configured. |
| Layouts | One card per saved layout: name, "N charts · TF", first three tickers. No thumbnail: layouts store no image. |

The tab bar is `kit::icon_tabs`, the same pill track as Add Chart and the asset picker. The page opens
on the first of Recent, Favorites, Layouts that has something in it, and on Suggestions for someone
with nothing. The person's pick lasts until a chart replaces the page (`EmptyPageCache::reset`).
An empty tab says why, in a note card.

Tiles and the search box are opaque (page colour under the card fill) so the backdrop never shows
through them. The title is `TITLE2 + 11`, larger than the `TITLE2 + 3` page title of settings pages: it is a landing page.

## Sparklines

Each market tile and row shows its last seven daily closes, green when the week ended at or above where
it began, red otherwise. `empty_state::show` reports the markets it drew (`EmptyStateOutput::wanted`);
`TrendLoader` reads each through `service::fetch_klines` (`1d`, 8 candles, on the provider's work-queue
lane) and keeps the result: fresh for 10 minutes, a market with no history retried after 5. A failed
refresh keeps the week already shown. States: not read yet = faint flat line, no history = nothing,
else the week. Only markets of the shown tab are read. Keyed by `InstrumentId::key()`; the
provider id is `InstrumentId::api_symbol()`.

## Backdrop

`backdrop.rs`: a trend line over the top of the page, behind the title, in the colour of a rising market. It is a pure
function of the clock (smooth value noise over hashed lattice points, two octaves), scrolling left with the
newest price at the right, so it keeps no state and never repeats. The line wanders in the top 122 pt and the page content
starts below it; the fade fill reaches 340 pt down. The fill is a vertex-coloured mesh, the one gradient on
this page, asked for as decoration; cards stay flat. While the window is focused it asks for a frame every
33 ms; unfocused it keeps its last frame and costs nothing. There is no reduced-motion setting in the app yet.

## Checking it

`DEGENVIEW_GALLERY_DIR=<dir> cargo test --lib empty_state -- --ignored` writes every tab, with and without data, dark
and light, wide and narrow. Headless pictures show fallback artwork for tickers (icons load from the network).
