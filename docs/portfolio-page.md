# Portfolio page

The Portfolio tab: the shared top bar (`ui/portfolio/tab/bar.rs`: portfolio picker, currency, privacy, Import or Export, Add Transaction), a section bar
(Overview, Holdings, Transactions, Statistics) and the page of the selected section. The grid
cards are a separate surface: see [portfolio cards](portfolio-cards.md).

## Where things live

- `ui/portfolio/tab/mod.rs`: the tab (`PortfolioTab::ui`), the section bar, the centred
  column (`capped`), and the figures handed to the views; `ui/portfolio/tab/import.rs`: the import
  and export sheets.
- `host/portfolio/mod.rs`: `PortfolioHost`, the ledger, its saved preferences and history rebuilds;
  `host/portfolio/import_flow.rs`: the import and export work (file dialogs, parsing, matching, the
  ledger write) the sheets drive.
- `service/portfolio/quotes.rs`: `PortfolioQuoteFeed`, the valuation the views read.
- `ui/portfolio/overview.rs`: Overview (hero, key figures, charts, lists) and Statistics.
- `ui/portfolio/holdings.rs`, `ui/portfolio/transactions.rs`, `ui/portfolio/grid.rs`: the tables.
- `ui/portfolio/empty.rs`: the pictures and introductions for a page or panel with nothing in it.
- `ui/portfolio/gallery_tests.rs`: hand-run galleries of the pages (see "Looking at it").

## Revaluation is throttled

The quote book changes several times a second while markets are busy. Every change used to
rebuild the projection (holdings, history, cards) and flip `is_updating`, which made the hero's
"Updating…" label flash. Now:

- A **price tick alone** (same ledger, selection and currency, a valuation already published)
  revalues at most once per `REFRESH_INTERVAL` (2 s). A tick inside the window is not recorded, so
  the first frame after it picks it up; `PortfolioQuoteState::refresh_pending` asks the host for
  that frame.
- `is_updating` is true only while a valuation is being built where there was none: the first
  load, or after the ledger, selection or currency changed. Those are never held back, and the
  views show them as a loading state, not as a label.
- A revaluation that produces the same projection does not repaint.
- The hero shows no "Updating…" label. A dot beside the price caption says how current the prices
  are (`PriceStatus`: green live, orange cached or partly unpriced, red failed).

## Value history

- A daily point is stamped at a local midnight and means the portfolio **at that instant**: the
  holdings after every transaction before it, priced at the close of the last daily candle that had
  finished by then (`last_finished_bar_at`). A candle still forming is never used, so a point never
  sees a price from after its holdings. Today's point is therefore priced at yesterday's close; the
  live value covers today.
- Local midnights follow the system time zone through `service::system::timezone` (`ZoneOffset`): a
  daylight-saving day is 23 or 25 hours. Non-macOS builds and tests read UTC. The transaction
  editor still shows a past time with today's offset.
- A rebuild carries `transactions_fingerprint` of the transactions it was computed from and is
  stored only while the portfolio still has exactly those (an edit made while it ran drops it).
- A rebuild that could not fetch some asset's candles is stored (so the chart has something) and
  repeated every 10 minutes (`PARTIAL_RETRY_MS`) until every asset it can price comes back.
- Candles quoted in another currency than the portfolio's are converted at each day's rate
  (`in_currency`, the same FX service as the live valuation); a day with no rate has no candle.
  A span over 1,000 days asks the provider for a time range (Binance, Coinbase, Alpaca and
  DexScreener have one; the rest give their newest 1,000). Where an asset's candles begin after
  its first transaction needs, no point is made for the days before. The 1D chart is still
  dollar-quoted markets only.
- A ledger that cannot be replayed (a sale of more than is held) is shown as an error, not as an
  empty portfolio, and its history is not rebuilt into zeros.
- Realized profit counts positions closed completely (`holdings_and_closed`), not only the open
  holdings. Quantities carry float error, so a remainder within 1e-12 of the position's peak
  quantity counts as zero.

## The bar

`ui/portfolio/tab/bar.rs` draws the tab's top bar on the shared flat bar (`ui::kit::bar`, see
[Workspace toolbar](workspace-toolbar.md)): `kit::bar::HEIGHT` (44) on `theme::chrome()`, the same
as the chart's toolbar, across the whole window (only the page below is capped at
`CONTENT_MAX_WIDTH`). Left: the portfolio menu (its name, never dropped when narrow) and the currency
menu (disabled, with the candle indicator, while a conversion rebuilds the valuation). Right, from the
edge: the privacy toggle (lit while values are hidden), Add Transaction (its label drops to the icon
when narrow) and the Import or Export menu; the last two only with a portfolio. With no portfolio at
all the bar is dropped with the section strip (`shows_header`).

## Layout

- The host puts the section bar and the page in one column at most
  `CONTENT_MAX_WIDTH` (1480) wide, centred, so a full-screen window does not stretch rows and
  tables across the screen. The pages keep `PAGE_MARGIN` (16) inside that column; the section bar
  is inset by the same amount (`aligned`), so its tabs share the
  content's left and right edges at every window width.
- Overview has two layouts. From `WIDE_MIN` (1000) of content width it puts a `SIDE_WIDTH` (400)
  column beside the main one: value chart over holdings on the left, allocation over the best and
  worst performer tiles on the right, each pair ending level. Below that everything stacks, as
  before, with the performers as short cards. Statistics key-figure rows go from four across to
  two below 640.
- Statistics: a hero (total P&L large, how it splits between unrealized and realized in the
  middle, the 24 h move at the edge), four value cards, a Value Range strip (today's value
  between the history's low and high), then P&L by Asset beside a side column (from `WIDE_MIN`):
  the Win Rate ring over the four activity cards two across, both columns ending level at the
  taller of the bars and the side column. Below `WIDE_MIN` the Win Rate, P&L and activity cards
  stack. The P&L rows share their panel's height within 30 to 54 pt; below 520 pt the percentage
  column is dropped.
- Holdings: a summary band (total value and profit; one bar of how the money divides, in the
  colours the Overview's donut gives the same assets) above the search field and the table.
  Rows are 52 pt: the 24 h move is a tinted pill, the allocation cell carries a bar against the
  largest position, and P&L is the amount over its percentage. The band shows skeletons while
  prices load and is absent for a portfolio with no holdings.
- Transactions: the same band above the toolbar (the ledger's count with buys and sells, one bar
  of the mix by type in the capsule colours, fees paid), 48 pt rows with the date over the time,
  the asset over its name, and a tick in the type's colour at the row's edge. The table shows
  100 rows per page (25, 50, 100, 250 or 500 from the footer's menu); the footer's pager appears
  once the filtered list has more than 25 rows. A change of filter, search or sort returns to
  page one. The Type and Asset menus are as wide as their longest entry. The shared table
  code (`portfolio_grid`) is unchanged apart from a `put` text helper both bands use.
- Every block has a fixed height and paints at measured positions, so nothing reflows while prices
  stream in.

## Asset picker

The Add Asset, Remap and CoinMarketCap Match sheets share `portfolio_asset_picker`. It is built
from the pieces the symbol picker shares (`ui/chart/picker_kit`): the header art for the section, and
under the suggestion chips the same Recent card (`widgets::recents_card`) over the same stored list
(`addTicker.recentMarkets`), so a pick here appears in the symbol picker and the other way round. Crypto and stock markets are
kept; prediction markets and CoinMarketCap indices are not, so the Predictions section has none.

## Import sheets

Both import sheets open with the picker kit's header art (the CoinMarketCap scene, and the Portfolio
scene for the preview) and use no accent colour: the main button is the neutral Primary footer
button, a ticker's filled button is the inverse of the sheet (light on dark, dark on light), and
colour only carries meaning (green ready or matched, orange to match or a warning, red an error).
They share `card_background`, `problem_card` (errors and warnings, scrolling when long) and
`status_dot` from `portfolio_import_preview`.

**CoinMarketCap import** (`portfolio_cmc_import`). A band says how many tickers have a market
(`tally`) with one bar split by state: matched, searching, to match, skipped. Each ticker is a
fixed 60 pt row: its initials (or the market's icon once matched), the transactions it carries, an
arrow, then the market with the exchange's logo and an Auto-matched pill, or Searching, Skipped,
Not matched; the actions follow the state. Fee exchange rates are their own card with a progress
bar, a loaded line, or one row per failure with Skip Item. The footer counts the transactions that
will import.

**Import Preview** (`portfolio_import_preview`). A band with the transaction count, buys, sells
and assets, and the Transactions page's by-type bar (`type_mix`); then errors, a failed import and
warnings as cards; then the rows in the Transactions table's own cells (the type tick, date over
time, asset over name, type capsule), newest first; the footer says Ready to import, N errors to
fix or N warnings, and carries Import N Transactions.

The header menus (portfolio, currency, Import or Export) take the labels they will show and open
exactly as wide as the longest, never narrower than their button.

## Asset detail

`portfolio_asset_detail`, the sheet a double click on a holding opens. The picker kit's header art with
"BTC Transactions" over "Bitcoin · Binance · Values in USD"; the position as one card of eight
figures (two rows of four on a hairline grid, each cell reserving the caption line so figures
line up); and the asset's transactions in the Transactions table's own cells (type tick, date
over time, type capsule), with a row menu sized to its labels. An asset whose transactions net to
zero shows a note in place of the card.

## Edit Transaction

`portfolio_transaction_editor`: the picker kit's header art with a line that says what the sheet does in
its mode, a card for the asset (icon, ticker over name, where its prices come from), the type
tabs, then the fields and the summary card. No accent colour: the main button is the neutral
Primary one whatever the type, and the quantity is selected on open instead of keeping the
last sheet's cursor.

Asset subtitles come from `asset_subtitle`: a pair reads BTC/USDT, and CoinGecko's bare tickers
get the currency they are priced in (PEPE/USD). Row menus (the ... button) are sized to their
labels with `fit_menu`.

## New Portfolio

`portfolio_sheets::show_create`: the picker kit's header art, a name field, and a grid of base-currency
tiles with no accent colour (the chosen tile is the one in full ink with a check). A name another
portfolio already has shows a warning and disables Create Portfolio (and Return), since the
ledger would refuse it; the warning line is reserved, so the sheet never changes height.

## Manage Portfolios

`portfolio_sheets::show_manage`. The sheet opens with the picker kit's header art and uses no accent
colour. Each portfolio is a fixed 68 pt card: a grip of dots (drag to reorder, a line shows where
the row lands), its base currency on a tile, its name (click to rename in place) over what it
holds, then a green Viewing pill or a grey Archived one, Duplicate and Delete. A rename that
clashes with another portfolio's name shows the reason under the field in red and is put back
instead of being sent; Escape puts the old name back; Escape with nothing focused closes the
sheet. With no portfolios the card holds the empty picture. The footer has New Portfolio\u2026 and
Done. The card, pill, status dot and problem list the import sheets use live in
`portfolio_sheet_parts`.

## Empty states

`portfolio_empty` draws them with the `Art` helpers the Pine Studio's Logs, Problems and Profile
introductions use: a disc and a card, with a badge for the page.

- **A page with no transactions** (`portfolio_empty::page`): one picture per section, three steps,
  an example row with made-up figures, and the Add Transaction and Import buttons. The portfolio
  with no portfolios yet has its own.
  With no portfolios at all the page also drops the header (picker, currency, privacy, Import or
  Export; `shows_header`) and the section bar: Create Portfolio is the only control. A ledger whose
  portfolios are all archived keeps the header, since its picker leads back to them.
- **A panel waiting for data** (`portfolio_empty::compact`): Allocation, Portfolio Value history,
  Top Holdings, the Statistics P&L and Win Rate panels, and the Holdings and Transactions tables with no rows.
  A search or filter that matches nothing keeps its glyph and message.

## Not verified

- Only looked at through the hand-run galleries and the pure tests; not in a real window at
  full-screen sizes, on a second monitor, or at other scale factors.
- The 2 s interval is a judgement, not a measurement.
