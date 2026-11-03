# Portfolio page

The Portfolio tab: a header (portfolio picker, currency, privacy, import/export), a section bar
(Overview, Holdings, Transactions, Statistics) and the page of the selected section. The grid
cards are a separate surface: see [portfolio cards](portfolio-cards.md).

## Where things live

- `host/portfolio_host.rs`: the tab (`PortfolioTab::ui`), the header, the section bar, the centred
  column (`capped`), and the figures handed to the views.
- `service/portfolio_quotes.rs`: `PortfolioQuoteFeed`, the valuation the views read.
- `ui/portfolio_overview.rs`: Overview (hero, key figures, charts, lists) and Statistics.
- `ui/portfolio_holdings.rs`, `ui/portfolio_transactions.rs`, `ui/portfolio_grid.rs`: the tables.
- `ui/portfolio_empty.rs`: the pictures and introductions for a page or panel with nothing in it.
- `ui/portfolio_gallery_tests.rs`: hand-run galleries of the pages (see "Looking at it").

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

## Layout

- The host puts the header, the section bar and the page in one column at most
  `CONTENT_MAX_WIDTH` (1480) wide, centred, so a full-screen window does not stretch rows and
  tables across the screen. The pages keep `PAGE_MARGIN` (16) inside that column; the header and
  the section bar are inset by the same amount (`aligned`), so their buttons and tabs share the
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

The Add Asset, Remap and CoinMarketCap Match sheets share `portfolio_asset_picker`, the Add Chart
sheet's twin. It opens with Add Chart's header art for the section, and under the suggestion chips
it shows the same Recent card (`add_chart_sheet::recents_card`) over the same stored list
(`addTicker.recentMarkets`): a pick in either sheet appears in both. Crypto and stock markets are
kept; prediction markets and CoinMarketCap indices are not, so the Predictions section has none.

## Import sheets

The CoinMarketCap import sheet (`portfolio_cmc_import`) opens with the same header art as Add Chart
(its CoinMarketCap scene) and the app's blue (`SYSTEM_ACCENT`), not the old pink `ACCENT`. Under
the header a band says how many tickers have a market (`tally`) with one bar split by state:
matched, searching, to match, skipped. Each ticker is a fixed 60 pt row: its initials (or the
market's icon once matched), the transactions it carries, an arrow, then the market with the
exchange's logo and an Auto-matched pill, or Searching, Skipped, Not matched; the actions follow
the state (Match is the filled button only for a ticker with no market). Fee exchange rates are
their own card with a progress bar, a loaded line, or one row per failure with Skip Item. The
footer counts the transactions that will import. The Import Preview sheet, the next step, shares
the blue header and the footer buttons.

The header menus (portfolio, currency, Import or Export) take the labels they will show and open
exactly as wide as the longest, never narrower than their button.

## Empty states

`portfolio_empty` draws them with the `Art` helpers the Script Manager's Logs, Problems and Profile
introductions use: a disc and a card, with a badge for the page.

- **A page with no transactions** (`portfolio_empty::page`): one picture per section, three steps,
  an example row with made-up figures, and the Add Transaction and Import buttons. The portfolio
  with no portfolios yet has its own.
- **A panel waiting for data** (`portfolio_empty::compact`): Allocation, Portfolio Value history,
  Top Holdings, the Statistics P&L and Win Rate panels, and the Holdings and Transactions tables with no rows.
  A search or filter that matches nothing keeps its glyph and message.

## Not verified

- Only looked at through the hand-run galleries and the pure tests; not in a real window at
  full-screen sizes, on a second monitor, or at other scale factors.
- The 2 s interval is a judgement, not a measurement.
