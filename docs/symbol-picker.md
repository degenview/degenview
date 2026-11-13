# Symbol picker

`src/ui/chart/symbol_picker/` is the one modal for choosing what goes on a chart: a market, a
prediction market, a CoinMarketCap index or a view of the user's portfolios. It opens from

- the symbol button at the left of the toolbar and ⌘K / Ctrl+K (`shortcut::SYMBOL_SEARCH`, handled in
  `workspace/input.rs`): **Chart** mode, a pick replaces the focused chart's market;
- the toolbar's **Add Chart** button and the new-tab page's search launcher: **Add** mode, a pick
  adds a chart (`ChartWorkspace::open_add_picker`);
- the toolbar's Compare button: **Compare** mode (below);
- a watchlist's plus: **Watchlist** mode (below).

The workspace side (`src/workspace/chart/symbol_picker.rs`) opens it, keeps the highlighted market
priced, feeds it the watchlists and portfolios while their tabs show, and applies a pick. The
shared pieces it and the portfolio's asset picker are made of (recents, suggestions, artwork, result
rows, search box, header art) live in `src/ui/chart/picker_kit/`.

## What it does

- **Opens searching for the market on the focused chart** (`Chart::picker_query`): the exchange's own
  name for Binance, Coinbase, Alpaca and CoinGecko, the base token for a DEX pair, nothing for a
  prediction market or a CoinMarketCap gauge. The text is selected, so typing replaces it. Add,
  Compare and Watchlist modes open empty.
- **Return replaces** the focused chart's market (`replace_focused_market`, which is
  `switch_chart`: the chart keeps its settings, indicators, drawings and place in the grid, and its
  candles reload). **⇧Return or the row's plus adds** a chart instead. **Right-clicking a row** opens
  a menu with `Replace <the chart's market>` (only in Chart mode with a chart focused) and
  `Add as new chart`, with the keys that do the same (`list.rs`, `context_menu`); the row becomes the
  highlighted one. With no market chart focused, or in Add mode, Return adds one. The footer names
  what Return will do ("Replace ETHUSDT", "Add chart", "Add card", "Add to Favorites").
- **Compare** ([chart overlays](chart-overlays.md)): **⌥Return**, the compare button left of a highlighted
  row's plus, or the row menu's `Compare with <the chart's market>` draws the market over the focused
  chart (`Output::Overlay`) and closes. The toolbar's Compare button opens the picker in `PickMode::Compare`
  (`open_compare`): nothing is searched for at first, Return compares and the picker **stays open** so
  several markets can be added in a row, ⇧Return still adds a chart and closes, and the check mark means
  "already on the chart or compared" (`Env::overlaid`); picking one of those does nothing. The footer reads
  "Compare with ETHUSDT". A chart that takes no overlay (`Env::can_overlay` false) is never offered it.
  Only crypto and stock markets can be drawn over a chart: Compare offers only the All, Stocks, Crypto
  and Watchlists tabs (`Scope::tabs`), and ⌥Return on a prediction market or an index adds it as in
  Chart mode.
- **Watchlist mode** (`open_for_watchlist(name)`): a pick follows the market in that list
  (`Output::Watchlist`, `WatchlistCard`) and the picker **stays open** so several can be added in a
  row; the footer says what was added. It records no recents, offers no Replace or Compare, has no
  Watchlists tab, and the workspace keeps the target list (`add_to_watchlist`) only while the picker
  is open.
- **Tabs** All, Stocks, Crypto, Predictions, Dashboards, Watchlists (Tab cycles the tabs of the mode)
  and a filter pull-down at their right: the exchange ("All venues"), the prediction provider, or on
  Dashboards the portfolio ("All Portfolios"). All is crypto and stocks together; prediction markets
  and indices never appear in All or Watchlists. Futures, forex, commodities and ETF tabs are not
  offered because no provider here lists them.
  - **Watchlists** lists the markets of the user's watchlists, one captioned group per list in the
    list's own order (`watchlists.rs`); nothing is searched, the box filters by ticker, asset or
    company, and the venue filter applies. Lists holding only prediction markets, indices or portfolio
    cards are left out. The workspace hands the lists over each frame while the tab shows
    (`wants_watchlists`).
  - **Predictions** (`predictions/`) searches Polymarket and Kalshi (both, or the one the filter
    names) and groups the answers by event. An event with several choices is a header row (artwork,
    title, "N choices · the likeliest", chevron, checkbox) over its choices (checkbox, label, chance);
    a lone choice is a row like any market, indented to line up with the events' artwork. Several
    events start folded, one alone starts open. Return, or a click, on a header folds it; on a choice
    it uses that choice. **Ticking** choices (the checkbox, or the header's for all of them) charts
    them together as the lines of one chart: Return on a choice of that event, or the footer's
    `Chart N choices` button, takes every tick (`event.series`, the first in the event's order leads
    the chart). Ticks live in one event only, and are dropped on another tab. The empty box lists the
    topic chips (`PREDICTION_TOPICS`) and "Trending on Polymarket" (`predictions/trending.rs`, 10 minutes
    fresh, a failure retried after 30 seconds and shown as nothing); Kalshi cannot rank its markets,
    so with it chosen only the topics show. Predictions are never kept as recents.
  - **Dashboards** (`dashboards.rs`) is a list, not a search: "Market indices" (the four CoinMarketCap
    gauges, `cmc::CHART_TYPES`) and "Portfolio" (the four card kinds, `CardKind::ALL`, for the portfolio
    the pull-down chooses). The box filters titles and descriptions. An index replaces or adds a chart
    like a market; a card only ever adds (`Output::AddCard`, a card cannot take a chart's place), and
    a check marks a card or index already on the tab (`Env::cards_on_tab`). Without a portfolio the
    cards are not offered and a note says where they come from (`wants_portfolios` feeds the list
    while the tab shows). Nothing here has a price to fetch.
- **Rows** show the market as its exchange names it (`BTCUSDT`, `BTC-USD`, `AAPL`), what it trades
  under it (`BTC / USDT`, the company, a prediction's event), the exchange and its kind, a check when
  the market is already on a chart of the tab, and a plus on the highlighted row.
- **Detail panel** (hidden below 760 px of window width): artwork, price and change from the quote feed,
  "Open 24/7" for crypto or the stock's market state, the exchange, and "Also trades as": the other
  dollar markets of the asset on Binance and Coinbase (`SymbolSearch::related`, read from the locally
  held exchange lists). Clicking one of them replaces the chart. A prediction market shows the event's
  artwork and title, its chance and the provider instead; an index or a card says what it charts.
- **Empty box**: recent picks (shared with the new-tab page and the portfolio asset picker), else
  popular markets (`picker_kit::suggestions`).
- **Tab bar width** (`fit_tabs`): the six tabs and the pull-down do not fit at the 520 px minimum
  width with roomy padding, so the row gives up room in order: tab padding 28 → 20, the pull-down to
  its icon, tab padding 14. The tab titles are measured, not guessed.

## Names

A market is named as its exchange names it everywhere: `DataSource::display_symbol` upper-cases and
nothing else. There is no `BTC/USDT` rewriting any more. A pair's parts are never guessed from the
name: `SearchResult::base` and `quote` carry what the provider listed (Binance `baseAsset`,
Coinbase `base_currency`, DEXScreener token symbols), and `DataSource::base_asset` reads the asset
out of any spelling for artwork, alerts and paper tickets. Portfolios keep their stored
`BASE/QUOTE` label (`SearchResult::pair_label`). DEXScreener's `PEPE/WETH` is the DEX's own name and
stays.

## Ranking

`service/market/search_rank.rs` is the one rule, used inside each provider and across all of them
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

A prediction market has no pair to rank: the Predictions tab keeps each provider's own order,
Polymarket first. Inside a tier: provider order (Binance, Coinbase, DEXScreener, CoinGecko, Alpaca), then quote (the
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
scopes, modes and venues, the prediction grouping and ticking, the dashboards lists and filters,
recents and popular markets, the tab bar's fitting, the workspace replace/add/prefill behaviour
(including a prediction market keeping its title, artwork and lines) and layout at several widths.
`gallery_of_the_symbol_picker` (ignored, set `DEGENVIEW_GALLERY_DIR`) writes pictures of every tab and
mode in both appearances. Not checked: Linux and Windows windows; real Polymarket, Kalshi and
CoinMarketCap answers in a running window; the "Open 24/7" line is a fact about crypto markets, not a
feed.
