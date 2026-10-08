# Watchlists

Watchlists are global: every window and tab shows the same lists, and none belongs to a chart tab or a
saved layout. Which list a window shows, its filter and its highlighted row are local to that window.

## What a list holds

A watchlist is a name plus one flat, ordered list of entries. An entry is a market or a section
divider. A section owns the markets after it up to the next section; markets before the first section
sit at the root. Deleting a section removes only the heading, so its markets join the section above
(or the root). One market can be in many lists, and once in each list.

**Favorites** is the list the chart-card star adds to. It can be renamed but not deleted. Right-click
the star, a watchlist row, or a search result for *Add to Watchlist*, with a check beside every list
that holds the market.

## Identity

A market is an `InstrumentID`: the data source plus that provider's own id (`BTCUSDT` on Binance,
`BTC-USD` on Coinbase, a CoinGecko coin id, a DEX pair address, a Polymarket token id, a Kalshi
`SERIES/MARKET`). Equality ignores case where the provider does, and ignores a DEX pair's `chain`, which
the quote endpoints need but the address already makes unique. A display title is never identity.
CoinMarketCap index charts are not markets and can't be added.

Drawings, price alerts and paper trading keep the key formats they already had; they are not unified
here.

## Flags

Six colours, one per market, shared by every list holding it (stored under the `watchlist.flags`
setting). Filtering by flag is a view over that map, not a copy of the markets.

## Sorting, filtering, columns

Sorting orders the markets inside each section; section headings stay where they are, and the stored
order is never rewritten. Click a column heading to sort (largest first for numbers, A to Z for
symbols), again to flip, again to return to Manual. Filtering matches name, symbol and provider and
hides sections left empty. Drag symbols (or a whole section by its heading) to reorder: between sections, to
the top or end of a section, into an empty one. Dragging is turned off while a sort or filter is active. Sort, columns and the
subtitle line are saved per list.

## Quotes

Prices come from `WatchlistQuoteCoordinator`, not from charts: a market in three lists or two windows is
requested once, with one batched request per source every 5 seconds (30 for prediction markets), plus
one shared Coinbase socket. Nothing is requested while no sidebar is visible. A source that fails or is
rate limited backs off alone and keeps its last prices, marked stale; a market the provider rejects sits
out for ten minutes. A value a provider doesn't supply is a dash, never zero.

| Source | Change is measured against |
|---|---|
| Binance, Coinbase, CoinGecko, DEXScreener | rolling 24 hours |
| Alpaca (stocks) | previous regular-session close |
| Polymarket, Kalshi | the start of the past day; shown in percentage points |

A price is "current" while DegenView has refreshed it recently, whatever time the provider says it last
traded: a few poll intervals (45 seconds for Binance and stocks, 90 for Coinbase, 2 minutes for CoinGecko and
prediction markets). Only a price that has stopped refreshing is dimmed and labelled stale. A stock outside
US hours reads "Market closed" and is not dimmed (its close is its current price). A failed source is retried
after 5 seconds, backing off to 30 (60 if rate limited); Coinbase products the socket goes quiet on are
refreshed from REST every 30 seconds. A market with no price and a reason (a rate limit, a missing key) shows
a warning icon with the reason instead of a bare dash. The price is green when the market is up and red when
it is down.

Volume is dollar turnover where the provider reports it (Binance on dollar pairs, Coinbase priced from
base volume, DEXScreener, CoinGecko), shares for stocks, and units otherwise. Direction is shown with an
arrow as well as colour.

Limits: stock market hours ignore holidays; a chart and the sidebar don't share a socket; a DEX favorite
saved before chains were kept has its network looked up once.

## Charts

A row click goes to the tab's **focused chart**: the card last clicked, else the first market chart. A
market already on screen takes focus instead. Switching keeps the card's place, timeframe, colours and
indicators; it resets what belongs to the old market: the pending fetch, selections, drafts, Pine output,
a fixed price precision, and volume bars if the new market has none. Drawings belong to a market, so
each market shows its own. Price alerts, Pine alerts and paper trading are not changed; a Pine alert
stays bound to the market it was made on and does not fire for the new one. The switch changes the
saved layout (it shows "(unsaved)"); editing a watchlist never does.

*Add as New Chart* uses the normal add flow. *Open in New Tab* opens the market in its own tab.

## Import and export

One entry per line, `SOURCE:SYMBOL`, with `###Name` lines for sections:

```
###Majors
BINANCE:BTCUSDT
COINBASE:ETH-USD
DEXSCREENER:solana/<pair address>
POLYMARKET:<token id>|Market title
```

Sources: `BINANCE`, `COINBASE` (BASE-QUOTE), `COINGECKO` (coin id), `DEXSCREENER` (`chain/address`),
`ALPACA`, `POLYMARKET` and `KALSHI` (both need a `|title`). A single comma-separated line, as TradingView
exports, is read too. Best effort: `NASDAQ:`, `NYSE:`, `AMEX:`, `ARCA:` and `BATS:` map to Alpaca stocks;
anything else (futures, perpetuals, other exchanges, a concatenated Coinbase pair like `BTCUSD`) is
reported as skipped, never guessed. Valid rows are kept when others fail, and the sheet lists what was
skipped and why.

## Saved data

Lists live in the `watchlist` table of `degenview.sqlite`. On first launch with this version the old
`favorite` rows become the "Favorites" list (order and ids kept); those rows are left in place and never
read again. If the table can't be read, changes are turned off and the sidebar says so, rather than
writing over it.
