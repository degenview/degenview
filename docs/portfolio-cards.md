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

- `model/portfolio_card.rs`: `CardKind`, `PortfolioCard`, `CardData` (everything a card paints),
  `performance()` (the ranking). Pure.
- `host/portfolio_card_host.rs`: `PortfolioCardEnv::view` builds a `CardData` the way the Portfolio tab
  does; each card has a `CardRuntime` (its own quote feed and demand owner).
- `service/portfolio_performers.rs`: `PerformerFeed`, start prices from hourly (ranges up to 36 h)
  or daily candles, cached in memory (5 min fresh, 1 min retry after a failure).
- `ui/portfolio_cards/`: `mod.rs` (header, controls, states, shared painting) and one file per kind.
  Geometry that can be tested without a window (`bars`, `layout`, `visible_rows`, `plot_rect`) is
  pure and has unit tests.

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
