# Portfolio cards

Grid cards that show a portfolio instead of a market. They sit in the chart grid beside market
charts (drag, reorder, save in layouts, restore), but have no candles, indicators, alerts, drawings,
scripts, replay or paper trading. The Swift app had the first three; Top Performers is new.

| Card | Shows | Range |
|---|---|---|
| Portfolio Value Chart | Total value over time: line coloured by profit against money put in, live point, dashed line at today's value, snapped crosshair with value, date and P&L vs invested | 1D 1W 1M 1Y ALL (persisted per card) |
| Portfolio Value | Today's total, the 24h change, a month of history behind it when the card is tall | none |
| Allocation | Donut (top 8 holdings, the rest folded into "Other (n)"), centre total or hovered slice, legend with share bars; hover is linked both ways | none |
| Top Performers | Holdings ranked by how their market moved, as bars from a shared zero line (gains right, losses left), with percent and amount | 1D 1W 1M 1Y ALL (persisted per card) |

Each card covers one portfolio or all of them (`PortfolioCard::portfolio_id`). Two cards are the
same card when kind and portfolio match; adding one that exists focuses it. The range is view state.

## Where things live

- `model/portfolio/card.rs`: `CardKind`, `PortfolioCard`, `CardData` (everything a card paints),
  `performance()` (the ranking). Pure.
- `host/portfolio_card_host.rs`: `PortfolioCardEnv::view` builds a `CardData` the way the Portfolio tab
  does; each card has a `CardRuntime` (its own quote feed and demand owner).
- `service/portfolio_performers.rs`: `PerformerFeed`, start prices from hourly (ranges up to 36 h)
  or daily candles, cached in memory (5 min fresh, 1 min retry after a failure).
- `ui/portfolio/cards/`: `mod.rs` (header, controls, states, shared painting) and one file per kind.
  Geometry that can be tested without a window (`bars`, `layout`, `visible_rows`, `plot_rect`) is
  pure and has unit tests.

## Look

- The header badge is neutral (an inked tile, grey glyph), like the tile of a settings card; colour is
  kept for gains, losses and chart series. The same badge is the watchlist row icon.
- Controls (summary, range switch) are one row from 420 pt of card width up. Narrower, the switch
  drops to its own row stretched across the card (`controls_height`), so the headline is never
  truncated by it.
- Portfolio Value is left-aligned: number, 24h chip, caption. The month of history goes under it on a
  tall card, beside it on a wide short one, and is left out below 170 pt (`value::layout`).
- Allocation keeps its legend beside the donut down to about 210 pt of inner width and puts it under
  the donut only when the card is taller than wide (`allocation::layout`). Legend names, bars and
  percentages sit in fixed columns so every track lines up; the bar is dropped before the name is cut.
- A value-chart axis label that the badge of today's value would sit on is not drawn.
- To look at all four in both themes: `DEGENVIEW_GALLERY_DIR=<dir> cargo test --lib
  portfolio_cards_gallery -- --ignored` (sizes, hover and the loading, down, privacy, empty and
  unpriced states). Verified that way only, not in a running window.

## Decisions

- **Top Performers ranks holdings, not the market.** A start price per holding comes from its own
  candles: the close of the last candle that ended by the start of the range. A holding bought
  after the range began is compared from its first purchase. The return needs no currency
  conversion; the amount is today's value scaled by the return (`value * r / (1 + r)`) and
  assumes the quantity held did not change in the range. Holdings with no candles are counted as
  "unpriced" in the footer instead of ranked.
- **Profit colouring follows the Overview**, not the Swift grid card: a stretch is green while the
  value is at or above the money put in. The headline change is the profit net of deposits
  (`PortfolioPeriodChange`), not the raw value change, so a deposit is never shown as a gain.
- **Allocation never repeats a colour**; the Swift grid card cycled six.
- **Privacy mode** masks amounts everywhere (values, labels, tooltips, badges); percentages and
  the shapes stay.
- **Favorites**: a card is a watchlist row whose identity is `PortfolioCard::instrument_id()`
  (CoinMarketCap provider, the card's key as symbol) so the quote pipeline treats it as
  "nothing to quote" without a new `DataSource`. The row has no quote columns and cannot be
  imported from text; the text export leaves it out.
- Cards are not exposed over MCP (`chart_summaries` filters them).

## Not done

- Per-card portfolio picker after adding (choose the portfolio when adding; remove and add again
  to change it).
- A card in a saved layout preview on the empty page is left out of the preview.
