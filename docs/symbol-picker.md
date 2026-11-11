# Symbol picker

`src/ui/symbol_picker/` is the modal that changes the market of the focused chart. It opens from the
symbol button at the left of the toolbar (`workspace/toolbar.rs`, `ToolbarAction::OpenSymbolSearch`)
or with ⌘K / Ctrl+K (`shortcut::SYMBOL_SEARCH`, handled in `workspace/input.rs`). The workspace side
(`src/workspace/symbol_picker.rs`) opens it, keeps the highlighted market priced and applies a pick.

## What it does

- **Opens searching for the market on the focused chart** (`Chart::picker_query`): the exchange's own
  name for Binance, Coinbase, Alpaca and CoinGecko, the base token for a DEX pair, nothing for a
  prediction market or a CoinMarketCap gauge. The text is selected, so typing replaces it.
- **Return replaces** the focused chart's market (`replace_focused_market`, which is
  `switch_chart`: the chart keeps its settings, indicators, drawings and place in the grid, and its
  candles reload). **⇧Return or the row's plus adds** a chart instead. **Right-clicking a row** opens
  a menu with `Replace <the chart's market>` (only while a chart is focused) and `Add as new chart`,
  with the keys that do the same (`list.rs`, `context_menu`); the row becomes the highlighted one.
  With no market chart focused Return adds one. The footer names what Return will do ("Replace ETHUSDT").
- **Compare** ([chart overlays](chart-overlays.md)): **⌥Return**, the compare button left of a highlighted
  row's plus, or the row menu's `Compare with <the chart's market>` draws the market over the focused
  chart (`Output::Overlay`) and closes. The toolbar's Compare button opens the picker in `PickMode::Compare`
  (`open_compare`): nothing is searched for at first, Return compares and the picker **stays open** so
  several markets can be added in a row, ⇧Return still adds a chart and closes, and the check mark means
  "already on the chart or compared" (`Env::overlaid`); picking one of those does nothing. The footer reads
  "Compare with ETHUSDT". A chart that takes no overlay (`Env::can_overlay` false) is never offered it.
- **Tabs** All, Stocks, Crypto, Watchlists (Tab cycles them) and an exchange filter ("All venues").
  All is crypto and stocks together. **Watchlists** lists the markets of the user's watchlists, one
  captioned group per list in the list's own order (`watchlists.rs`); nothing is searched, the box
  filters by ticker, asset or company, and the venue filter applies. Lists holding only prediction
  markets, indices or portfolio cards are left out. The workspace hands the lists over each frame
  while the tab shows (`wants_watchlists`). Futures, forex, commodities and ETF tabs are not offered because no provider
  here lists them; prediction markets and CoinMarketCap gauges stay in Add Chart.
- **Rows** show the market as its exchange names it (`BTCUSDT`, `BTC-USD`, `AAPL`), what it trades
  under it (`BTC / USDT`, the company), the exchange and its kind, a check when the market is already
  on a chart of the tab, and a plus on the highlighted row.
- **Detail panel** (hidden below 760 px): artwork, price and change from the quote feed, "Open 24/7"
  for crypto or the stock's market state, the exchange, and "Also trades as": the other dollar
  markets of the asset on Binance and Coinbase (`SymbolSearch::related`, read from the locally held
  exchange lists). Clicking one of them replaces the chart.
- **Empty box**: recent picks (shared with Add Chart's list), else popular markets (Add Chart's
  suggestions).

## Names

A market is named as its exchange names it everywhere: `DataSource::display_symbol` upper-cases and
nothing else. There is no `BTC/USDT` rewriting any more. A pair's parts are never guessed from the
name: `SearchResult::base` and `quote` carry what the provider listed (Binance `baseAsset`,
Coinbase `base_currency`, DEXScreener token symbols), and `DataSource::base_asset` reads the asset
out of any spelling for artwork, alerts and paper tickets. Portfolios keep their stored
`BASE/QUOTE` label (`SearchResult::pair_label`). DEXScreener's `PEPE/WETH` is the DEX's own name and
stays.

## Ranking

`service/search_rank.rs` is the one rule, used inside each provider and across all of them
(`merge_ranked`). A query is normalised to letters and digits, so `btc/usd`, `BTC-USD` and `BTC USD`
are one query, and Binance and Coinbase match it against their own lists the same way (`BTC-USDT`
finds Binance's `BTCUSDT` and Coinbase's `BTC-USDT`). Tiers:

0. the exact market, or the asset in a dollar quote (USD, USDT, USDC, USDS, FDUSD, BUSD, TUSD, USDP,
   DAI) on every exchange;
1. the query is the asset plus the start of a dollar quote (`BTCUSD` finds `BTCUSDT`);
2. the asset in another quote (`BTCTRY`, `BTC-EUR`);
3. assets and companies that begin with the query;
4. anything that contains it;
5. the rest.

Inside a tier: provider order (Binance, Coinbase, DEXScreener, CoinGecko, Alpaca), then quote (the
exchange's main dollar quote first: USDT on Binance, USD on Coinbase; then USD, USDT, USDC...), then
the provider's own order. A coin or a share has no quote and counts as dollars. Each provider still
keeps its first 20 (30 for Alpaca) after ranking, so the dollar markets are never cut off by the
odd quotes.

## Search

`SymbolSearch` asks each provider of the scope on its own thread after a 220 ms pause, keeps every
answer under `(query, provider)` so switching tabs or venues re-reads instead of re-asking, and
reports a failing provider without hiding the others. Stocks need an Alpaca key: a missing key
(`service::ALPACA_MISSING_KEY`) shows a card pinned to the bottom of the list with an Open Settings
button (`Output::OpenAlpacaSettings`); any other Alpaca failure shows its message without the button.
The key is checked only when a stock is picked, never per frame. `Backend` lets tests replace the
providers.

## Verified and not

Unit tests cover the ranking, the search state, the keyboard (arrows, Return, ⇧Return, Tab, Esc),
scopes and venues, recents and popular markets, the workspace replace/add/prefill behaviour and
layout at several widths. `gallery_of_the_symbol_picker` (ignored, set `DEGENVIEW_GALLERY_DIR`) writes
pictures. Not checked: Linux and Windows windows; the "Open 24/7" line is a fact about crypto
markets, not a feed.
