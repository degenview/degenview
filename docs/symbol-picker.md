# Symbol picker

`src/ui/chart/symbol_picker/` is the one modal for choosing what goes on a chart: a market, a
prediction market, a CoinMarketCap index or a view of the user's portfolios. It opens from

- the symbol button at the left of the toolbar and ⌘K / Ctrl+K (`shortcut::SYMBOL_SEARCH`, handled in
  `workspace/input.rs`): **Chart** mode, a pick replaces the focused chart's market;
- the new-tab page's search launcher: **Add** mode, a pick adds a chart
  (`ChartWorkspace::open_add_picker`); the toolbar has no Add Chart button, ⇧Return and the row's plus
  add from Chart mode;
- the toolbar's Compare button: **Compare** mode (below);
- a watchlist's plus: **Watchlist** mode (below).

The workspace side (`src/workspace/chart/symbol_picker.rs`) opens it, keeps the highlighted market
priced, feeds it the watchlists and portfolios while their tabs show, and applies a pick. The
shared pieces it and the portfolio's asset picker are made of (recents, suggestions, artwork, result
rows, search box, header art) live in `src/ui/chart/picker_kit/`.

## What it does

- **Opens searching for the market on the focused chart** (`Chart::picker_query`): the exchange's own
  name for Binance, Coinbase, OKX, Kraken, Bybit, Gate.io, MEXC, Hyperliquid, Aster, Lighter, Deribit, HTX, Bitfinex, Alpaca and CoinGecko, the base token for a DEX pair, nothing for a
  prediction market or a CoinMarketCap gauge. The text is selected, so typing replaces it. Add,
  Compare and Watchlist modes open empty.
- **Return replaces** the focused chart's market (`replace_focused_market`, which is
  `replace_chart_market`: the chart keeps its settings, indicators and place in the grid, shows the drawings of the new market (each market has its own set), and
  its candles reload). The replacement is one undo step (`Snapshot::Market`, with the indicators when a
  prediction market or gauge, which take none, removes them); undo reloads the old market's candles. A
  watchlist click on the focused chart replaces the same way; Tab Linking's follow does not, as it is
  not this tab's own edit. **⇧Return or the row's plus adds** a chart instead. **Right-clicking a row** opens
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
  Watchlists tab, and the workspace keeps the target list (`WatchlistPane::picker_target`) only while the picker
  is open.
  - **The list's own markets** come in each frame as `Env::in_list` (the target list's instruments,
    portfolio cards by `instrument_id`), the way `Env::on_tab` carries a chart tab's. A row already in
    the list shows the check instead of the plus, the footer's Return hint reads "Already in {list}",
    and Return, ⇧Return or a click on it adds nothing and acknowledges nothing (`commit` refuses it, so
    "Added X" is never said for a duplicate; the store's `add_to_list` still answers `Ok(false)` for any
    other caller).
  - **The footer names the market** Return adds, resolved from the highlighted row: "Add Binance ·
    BTCUSDT to Favorites" (the venue as `lookup::short_name` says it, so "Alpaca (IEX)" is "Alpaca",
    then the row's name; an event's title for a prediction market).
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
    a lone choice is a row like any market, indented to line up with the events' artwork, with its
    chance left of the provider. Choices under an event carry no artwork of their own. Several
    events start folded, one alone starts open. Return, or a click, on a header folds it; on a choice
    it uses that choice. **Ticking** choices (the checkbox, or the header's for all of them) charts
    them together as the lines of one chart: Return on a choice of that event, or the footer's
    `Chart N choices` button (shown from the first tick; "Add N choices" in Watchlist mode), takes every tick (`event.series`, the first in the event's order leads
    the chart). Ticks live in one event only, and are dropped on another tab. The empty box lists the
    topic chips (`PREDICTION_TOPICS`, on at most two lines that scroll sideways) and "Trending on Polymarket" (`predictions/trending.rs`, 10 minutes
    fresh, a failure retried after 30 seconds and shown as nothing); Kalshi cannot rank its markets,
    so with it chosen only the topics show. Predictions are never kept as recents.
  - **Dashboards** (`dashboards.rs`) is a list, not a search: "Market indices" (the four CoinMarketCap
    gauges, `model::market::cmc::CHART_TYPES`) and "Portfolio" (the four card kinds, `CardKind::ALL`, for the portfolio
    the pull-down chooses). The box filters titles and descriptions. An index replaces or adds a chart
    like a market; a card only ever adds (`Output::AddCard`, a card cannot take a chart's place), and
    a check marks a card or index already on the tab (`Env::cards_on_tab`). Without a portfolio the
    cards are not offered and a note says where they come from (`wants_portfolios` feeds the list
    while the tab shows). Nothing here has a price to fetch.
- **Typed `VENUE:SYMBOL`** (`lookup.rs`, in every mode): `BINANCE:BTCUSDT`, `nasdaq: aapl`. The prefix
  is read by `model::market::venue`, the same parser the watchlist import uses (a provider's
  watchlist slug, or a US exchange, which is Alpaca's), in any case and with spaces. The symbol is the
  query and the prefix's provider the venue filter, over whatever the pull-down says (the pull-down
  then shows that venue). `BINANCE:` alone lists the venue's recent or popular markets. The Watchlists
  and Predictions tabs read only a prefix they know (a prediction title can hold a colon), Dashboards
  none. A prefix the open tab has no markets of (`BINANCE:` on Stocks) says so.
  - **Never silent.** A prefix no provider serves (`KUCOIN:BTCUSDT`, BITGET, HKEX, SSE, futures venues,
    CoinMarketCap's `CMC`) is a word of 2-12 letters or digits followed by a colon and a symbol: the
    symbol is searched on every venue of the tab (a venue picked in the pull-down does not narrow it,
    as the prefix overrides it) and listed under "KUCOIN isn't supported · matches from
    other venues", and with no match the card reads "KUCOIN isn't supported. Searched: Binance,
    Coinbase, OKX, Kraken, Bybit, Gate.io, MEXC, Hyperliquid, Aster, Lighter, Deribit, HTX, Bitfinex, DEXScreener, CoinGecko, Alpaca." A plain miss reads
    "Nothing matches “zzz”. Searched:
    …" with the venues asked (the pull-down's one, or the tab's). Return on no result does nothing.
    Two sources are never merged into one row.
- **Rows** show the market as its exchange names it (`BTCUSDT`, `BTC-USD`, `AAPL`), what it trades
  under it (`BTC / USDT`, the company, a prediction's event), the exchange and its kind (a perpetual's
  kind line ends in a tertiary ` · Perp`, from `SearchResult::kind`; spot is unmarked, and the detail panel
  says `Perp` for the venue line and the related rows), a check when
  the market is already on a chart of the tab, and a plus on the highlighted row.
- **Detail panel** (hidden below 760 px of window width): artwork, price and change from the quote feed (the percent at two decimals, with the amount
  beside it, trimmed of trailing zeros, only while both fit),
  "Open 24/7" for crypto or the stock's market state, the exchange, and "Also trades as": the other
  dollar markets of the asset on Binance, Coinbase, OKX, Kraken, Bybit, Gate.io, MEXC and Hyperliquid's spot pairs (`SymbolSearch::related`; its perpetuals have no quote and are not listed), read from the locally
  held exchange lists). Clicking one of them replaces the chart. A prediction market shows the event's
  artwork and title, its chance, the provider and what the provider says about the event
  (`EventStats`: volume, 24-hour volume, liquidity, open interest, end date, category, and its
  description under ABOUT, as many lines as fit) instead; an index or a card says what it charts.
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
are one query, and Binance, Coinbase, OKX, Kraken, Bybit, Gate.io, MEXC, Hyperliquid, Aster, Lighter, Deribit, HTX and Bitfinex match it against their own lists the same way
(`BTC-USDT` finds Binance's `BTCUSDT`, Coinbase's `BTC-USDT` and OKX's `BTC-USDT`; Kraken's legacy `XBT`
is read as `BTC`, so `xbtusd` finds its `BTC/USD`; Bitfinex's `UST` is Tether, so `btcusdt` finds its `BTCUST`). Tiers:

0. the exact market, or the asset in a dollar quote (USD, USDT, USDC, USDS, FDUSD, BUSD, TUSD, USDP,
   DAI) on every exchange;
1. the query is the asset plus the start of a dollar quote (`BTCUSD` finds `BTCUSDT`);
2. the asset in another quote (`BTCTRY`, `BTC-EUR`);
3. assets and companies that begin with the query;
4. anything that contains it;
5. the rest.

A prediction market has no pair to rank: the Predictions tab keeps each provider's own order,
Polymarket first. Inside a tier: provider order (Binance, Coinbase, OKX, Kraken, Bybit, Gate.io, MEXC, Hyperliquid, Aster, Lighter, Deribit, HTX, Bitfinex, DEXScreener, CoinGecko, Alpaca), then quote (the
exchange's main dollar quote first: USDT on Binance, OKX, Bybit, Gate.io, MEXC and HTX, USD on Coinbase, Kraken and Bitfinex; then USD, USDT, USDC...), then
the provider's own order. A coin or a share has no quote and counts as dollars. Each provider still
keeps its first 20 (30 for Alpaca) after ranking, so the dollar markets are never cut off by the
odd quotes. Hyperliquid's perpetuals have no quote either, so they rank as the asset in dollars
(`BTC` is tier 0) and carry the Perp tag; so do all of Aster's, whose ids are the pair (`BTCUSDT`, `QQQUSDT`, `BTCUSD1`); its index perpetuals are also named as traders call them
(`xyz:SP500 — S&P 500 (SPX)`, see [Market data](agent-guides/market-data.md#names-and-search)), which
the company tiers rank, so "S&P", "SPX" and "Nasdaq" find them. All of Lighter's carry the Perp tag too:
its ids are the bare names it lists (`BTC`, `1000PEPE`, `US500`, `EURUSD`), which rank as the asset in
dollars, and it has no name table (its `SPX` is the SPX6900 coin; the S&P 500 index perpetual is `US500`). Deribit's carry it too: `BTC-PERPETUAL` and `ETH-PERPETUAL` (inverse, quoted in dollars) and the linear `<ASSET>_USDC-PERPETUAL` (`SOL_USDC-PERPETUAL`, `SPY_USDC-PERPETUAL`) are ids read for their asset before the first `_` or `-`, so "btc" finds both of BTC's, the inverse first, and "spy" the share perpetual. A spot pair is shown by its tokens
(`HYPE/USDC`) and opens by the id Hyperliquid numbers it with (`@107`).

## Search

`SymbolSearch` asks each provider of the scope after a 220 ms pause through
`work_queue::start_search`: CoinGecko and GeckoTerminal searches take their provider's lane and wait
for its rate-limit slot instead of running beside its candle requests; the rest run on search workers
of their own, so a busy chart never delays the list. It keeps every
answer under `(query, provider)` so switching tabs or venues re-reads instead of re-asking, and
reports a failing provider without hiding the others. Stocks need an Alpaca key: a missing key
(`ProviderError::MissingKey`) shows a card pinned to the bottom of the list with an Open Settings
button (`Output::OpenAlpacaSettings`); any other Alpaca failure shows its message without the button.
The key is checked only when a stock is picked, never per frame. `Backend` lets tests replace the
providers.

## Verified and not

Unit tests cover the ranking, the search state, the keyboard (arrows, Return, ⇧Return, Tab, Esc),
scopes, modes and venues, the prediction grouping and ticking, the dashboards lists and filters,
recents and popular markets, the tab bar's fitting, the workspace replace/add/prefill behaviour
(including a prediction market keeping its title, artwork and lines) and layout at several widths.
The footer's key hints are `kit::hints::hint_row` and the field `kit::search_box`, shared with the
[timeframe picker](workspace-toolbar.md#timeframe-picker).

`gallery_of_the_symbol_picker` (ignored, set `DEGENVIEW_GALLERY_DIR`) writes pictures of every tab and
mode in both appearances. Not checked: Linux and Windows windows; real Polymarket, Kalshi and
CoinMarketCap answers in a running window; the "Open 24/7" line is a fact about crypto markets, not a
feed.
