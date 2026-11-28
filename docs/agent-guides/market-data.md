# Market data guidance

## Adding a provider

1. `model::market::source::DataSource` metadata: display name, watchlist slug, prediction-market flag,
   volume support, line/candle mode.
2. Implement `service::MarketSource`; `search_tickers` if supported; a parser for a new kline shape.
   Declare its candle lengths in `model::market::natives` and plan every request through
   `DataSource::plan` (see Timeframes), so `serves` and the fetch agree. A REST venue whose kline
   request answers a fixed number of rows older than a cursor (OKX, Bybit, MEXC, Gate.io, Oanda)
   implements `service::market::kline_endpoint::KlineEndpoint` and calls `latest` and `range`
   from its `fetch_klines` and `fetch_klines_range`: it declares its `Paging`, how it names a
   market, how one page is asked for (`before` `None` is the newest, `Some(ms)` the candles opening
   before it) and, if its history is limited, `reach`. The plan, the cap before the fold, the
   pages and the shape of the answer are written once there.
3. Register in `service::for_source`. `supports_trending`/`trending` only if it can rank active
   markets (Kalshi cannot).
4. Add to the symbol picker (`ui::chart::symbol_picker`, Predictions tab) via `is_prediction_market()`, not provider matching.
   Pass the provider's asset names with `SearchResult::named` when its search lists them, return
   them from `MarketSource::asset_names` when they describe assets other venues leave unnamed, and decide
   in `service::market::names::Directory` what describes its markets otherwise
   ([Asset names](../symbol-picker.md#asset-names)).
5. `SourceLogo` under `assets/source_logos/`, the `ui::kit::source_logo::source` arm, attribution in
   `docs/source-logos.md`.

Prediction markets: lines under Predictions, REST refresh. Kalshi ids are `"SERIES/MARKET"`; cache
its series index in memory, rank locally.

Search groups `SearchResult::event` choices as a tree. Choices selected from one event make one chart:
`EventInfo::series` holds them, the first supplies symbol and candles, `ChartCard::set_series` draws
them by position with end labels. Persisted in `PersistedChart::pm_series`/`title`; not yet in
watchlists.

## Names and search

- Binance and Bybit ids join base and quote, and a bare asset is quoted in USDT. Telling the two
  apart is `split_binance_pair`: every quote the exchanges list or have listed belongs in
  `BINANCE_QUOTES` (a missing one asks for a market that does not exist, and a halted market's
  history is still served), and an asset whose name ends like a quote
  (`PENGU`) in `ASSETS_READING_AS_PAIRS`.
- Name a market as its exchange does (`BTCUSDT`, `BTC-USD`); `DataSource::display_symbol` only
  upper-cases. Never rebuild `BTC/USDT` labels. Asset: `DataSource::base_asset` /
  `SearchResult::base_asset`; pair: `SearchResult::base`/`quote_asset()`.
- Hits are ordered by `service::market::search_rank`; see [Symbol picker](../symbol-picker.md).
- A venue with no search endpoint hands over its whole list of markets and the app searches it.
  The list is a private `catalog::Catalog` `static` with its own disk namespace, never renamed: that
  would discard users' cached lists. Its item implements `catalog::search::ListedMarket`: the
  `SOURCE` every hit names, how many hits it keeps (`HITS`; `WIDE_HITS` where the venue keeps
  more), its id, the two sides and the names the venue gives them. `search_tickers` is one call to
  `catalog::search::find(query, list)` (`asset_names`: `catalog::search::asset_names`), where a query
  is normalised (`BTC-USDT`, `btc/usdt` and `BTCUSDT` are one), matched, ranked and cut. Nothing
  else is registered: a catalog adds itself to `catalog::registry` when it first keeps a list, and
  Settings ▸ Storage clears every list in memory through that, so a catalog is always a `static`. A
  download that panics counts as a failed attempt (the held list is served, the provider left alone
  for the back-off). A search that does not fit (Binance's two lists, Alpaca's company names)
  matches on its own and still ranks through `rank_results`.
- `VENUE:SYMBOL` text (`BINANCE:BTCUSDT`, `NASDAQ:AAPL`) is read only by `model::market::venue`
  (watchlist slugs plus `US_EXCHANGES`, which are Alpaca); the watchlist import and the picker both use
  it. A new provider's slug is `DataSource::watchlist_slug`; do not add a second prefix table.

## Timeframes

- `model::Timeframe` is a value, `{ count: u16, unit: TimeUnit }` (`Second`, `Minute`, `Hour`, `Day`,
  `Week`, `Month`), not a list. Limits: seconds 1-59 (a minute is `1m`), minutes 1-1440, hours 1-24, days 1-365, weeks 1-52, months 1-12;
  `Timeframe::new` refuses the rest (`TimeframeParseError`, whose messages are shown as they are).
  Canonical form: whole hours of minutes are hours (`60m` is `1h`, `1440m` is `24h`), `24h` stays
  apart from `1D`, and `12M` is the year. Equality, hashing and ordering (by nominal length, then
  unit) follow that form.
- Consts: `MINUTE_1` ... `YEAR_1`; `PRESETS` are the lengths providers serve natively, `LEGACY` the
  six (`1h`, `1D`, `1W`, `1M`, `3M`, `1Y`) saved data names with a variant name (see serde below). The toolbar and Pine Studio offer every timeframe through the [timeframe picker](../workspace-toolbar.md#timeframe-picker).
- `Timeframe::parse`: digits, then an optional unit, spaces allowed. No unit is minutes (`22` is
  `22m`); `m` is minutes and `M` months, the rest take either case (`s h d w y`); `1Y` is `12M`, `2Y`
  is refused. It reads the old labels and provider tokens too.
- Three spellings: `Display`/`label()` for the UI (`30s`, `22m`, `3h`, `2D`, `1W`, `3M`, `1Y`), `interval()`
  for the token every provider is asked for (`30s`, `22m`, `3h`, `2d`, `1w`, `3M`, `1Y`: the legacy six are
  byte-identical to what providers always got) and, for saved data, serde: the six `LEGACY` values
  serialize as `"Hour"`, `"Day"`, `"Week"`, `"Month"`, `"Quarter"`, `"Year"` so older builds and
  synced layouts decode them; anything else is its label (`"22m"`). Decoding takes both.
  `nominal_ms()` is exact to weeks; months are 30 days, a quarter 91 and a year 365.
- Buckets are `model::market::bucket` and nothing else cuts them (providers, chart and Pine), on the
  market's `bucket::Grid` (`DataSource::grid`). `Grid::Utc` is every crypto source: seconds, minutes
  and hours restart at midnight (a `22m` day ends in a shorter bucket), days count from 1970-01-01,
  weeks start on Monday and count from the first Monday of each year (a year's last multi-week
  bucket can be short; the days before that Monday belong to the previous year's), months count
  from January each year (so `3M` is the calendar quarter, `12M` the year), both as TradingView
  anchors them (2026-11-26, `time("2W" | "3W" | "4W" | "2M" | "5M")` on `BINANCE:BTCUSDT`). A bucket
  closes 1 ms before the next opens. Pine's `time_close` of a several-week bar on a 24/7 market is its bucket's end, the short one
  included (`time::bucket::close_time`). Multi-**day** buckets are known to differ: TradingView counts
  them from 1 January (a stock's in trading days), which providers' own multi-day candles and gap
  filling do not yet follow.
  `Grid::UsEquities` is a US stock (Alpaca): its **intraday** buckets anchor at the session open,
  09:30 New York (`model::market::session`, which holds the clock changes), every session restarts
  them, the last one is cut at the 16:00 close (1h is `09:30 10:30 … 15:30`, 4h `09:30 13:30`, as
  TradingView cuts `NASDAQ:AAPL`; measured 2026-11-25), and a candle that opens outside the session
  (pre and post market) is left out, in native series too. Days, weeks and months are as on the UTC
  grid. Holidays are not modelled (they hold no bar) and neither are early closes: the last bucket
  of a 13:00 session still closes at 16:00 (`time_close` of that day's last candle is later than
  TradingView's).
- Missing interval: fold finer bars with `Grid::fold(candles, timeframe, truncated)` (a plan does,
  `FoldPlan::fold`). Never substitute a nearby interval. `Grid::fold_base(target, natives)` is the
  coarsest native whose buckets nest exactly in the target's (an intraday base must divide the day,
  and the target when that is intraday; on the session grid also the open's offset into the day and
  the session's length, so Alpaca's clock-hour bars never serve a stock's hour; days nest in
  multiples of days and, as single days, in weeks and months; weeks in weeks, months in months), or
  `None`. `3M`/`1Y` are real quarters/years everywhere. Pass `truncated` when the series starts
  where a request stopped, to drop the partial first bucket. A grid that leaves candles out asks the
  provider for more (`Grid::fetch_slack`, three times as many on the session grid): the newest `n`
  base candles can be all pre or post market.
- **Every source serves any timeframe, or says it cannot.** `MarketSource::fetch_klines` and
  `fetch_klines_range` take a `Timeframe` (never a token string) and answer
  `ProviderError::UnsupportedInterval { provider, timeframe }`, "Polymarket has no 1m history"
  (`is_unsupported_interval()`; decide on the variant, never the text). The timeframe picker offers the
  presets and any timeframe typed or pinned (`1m`, `22m`, `3h`, `2D`, `2W`, `6M`), and warns before a
  chart whose source `serves` says no.
- **Natives live in `model::market::natives`**, one list per candle source
  (`DataSource::natives()`), so the planner a provider fetches by (`DataSource::plan`, a
  `bucket::FoldPlan`: target, base, `base_count(limit)`, `shape`) and the capability check the UI asks
  (`DataSource::serves`) read one source of truth; `model` cannot import `service`. A plan asks for
  `(limit + 1) × per_target` base candles (the extra bucket is the fragment the request's start cuts),
  capped by the provider's own limit and paging, folds with `truncated` set when the answer filled
  the request, and trims to `limit`. A cap gives fewer bars, never other ones.

  | Source | Natives | Notes |
  | --- | --- | --- |
  | Binance | 1s 1m 3m 5m 15m 30m 1h 2h 4h 6h 8h 12h 1D 1W 1M | Spot and USDⓈ-M perpetuals (below; perpetuals have no `1s`). `1s` klines are dense (a second with no trade is a flat zero-volume bar) and reach back at least a year (checked); a `45s` chart of 500 candles is about 23 requests. 1000 per request, paged in parallel (`fetch_klines` pages when a fold needs more). Its `3d` is left out: Binance's 3d bars start on a day that is not a multiple of three since 1970-01-01 (checked against the API), so 3D folds from 1D. Weeks start Monday, as here. Folded series are cached (`cache::policy(source, timeframe)`) and refreshed by their tail |
  | OKX | 1s 1m 3m 5m 15m 30m 1H 2H 4H 6H 12H 1D 1W 1M 3M | Public API, no key. `1s` is asked as `1s` and is dense, with history months deep (checked 100 days) through `/market/history-candles`; live is the `candle1s` channel. Asked as `…utc` bars from 6H up (its plain ones are cut at UTC+8). Left out: `8H` (no such bar), `2D`/`3D` (counted from a day that is not a multiple of the length since 1970-01-01, like Binance's `3d`: fold from 1D) and the year (folds from `3Mutc`). 300 per request: `/market/candles` for the newest, `/market/history-candles` with an `after` cursor for older (20 requests per 2 s, so pages are spaced 100 ms). Spot only. A request pages back at most 120 pages (36,000 base candles, `MAX_CANDLES`), capped before the fold (`base_candles`) so a cut-short answer drops its partial first bucket instead of keeping it. Held and tail-refreshed (`cache::policy`) |
  | MEXC | 1m 5m 15m 30m 1h 4h 8h 1D 1W 1M | Public v3 API, no key, spot only; Binance-style ids (`BTCUSDT`), but split by MEXC's own quotes (`model::market::source::split_mexc_pair`: `USDT`, `USDC`, `USD1`, `USDE`, `USDF`, `EUR`, `BRL`, `ETH`, `BTC`), so `BTCUSD1` and `SUSDT` are listed ids and not a bare asset to quote in USDT (a one-letter base counts only before a stablecoin or fiat quote, so `WBTC` is still an asset, as are the names in `ASSETS_READING_AS_PAIRS`). Its `/klines` takes a `limit` (newest rows) **or** a `startTime` + `endTime` window and ignores an `endTime` alone, 500 rows at most: older pages are windows (`window_before`, a month taken at its shortest), 20,000 base candles a request (`Paging::max_candles`, capped before the fold). **Its oldest days, weeks and months (before about 2018-12) are cut at UTC+8 midnight**: parsing drops every candle that is not on `bucket_start_ms`, so history starts where it is on the grid. Its tokenised stocks and commodities (`GOLD(XAUT)USDC`) are not letters and digits and are left out of search. Watchlists use the REST `/ticker/24hr` (all markets in one 670 KB answer; `openPrice` is a real 24 h reference). No ticker socket on purpose: its mini ticker's change is for a time-zone day, not rolling 24 h. Held and tail-refreshed when folded (`cache::policy`) |
  | Gate.io | 10s 1m 5m 15m 30m 1h 4h 8h 1D 1W 1M | Seconds: only `10s`, so 30s folds from it and 1s, 5s, 15s and 45s are not served. Public v4 API, no key, spot only; ids are `BTC_USDT`. Natives are its documented intervals `1m … 8h`, `1d`, `7d` (weeks from Monday) and `30d` (the calendar month): all cut where this app cuts, so 3M and 1Y fold from months (it also answers `3m`, `2h`, `6h` and `12h`, undocumented, so they fold). 1,000 per request, paged back with a `to` cursor through `service::market::kline_endpoint`. **A request may not reach 10,000 points back** in the interval asked for (HTTP 400 "Candlestick too long ago"): the cap is 8,900 base candles (`Paging::max_candles`, capped before the fold) and its endpoint's `reach` (`reach_left`) answers an older page past it as empty, so a chart does not retry. 200 requests per 10 s per IP: no lane. A 400 is an unknown market, a 403 a country it does not serve. Held and tail-refreshed when folded (`cache::policy`) |
  | Bybit | 1m 3m 5m 15m 30m 1h 2h 4h 6h 12h 1D 1W 1M | Public v5 API, no key, spot only; symbols are Binance-style `BTCUSDT`. Every bar is cut where this app cuts (weeks Monday, months calendar), so 3M and 1Y fold from months. 1,000 per request, paged back with an `end` cursor through `service::market::kline_endpoint` (60 pages, capped before the fold). Refusals are HTTP 200 with a `retCode` (`10006`/`10018` are rate limits) and some countries get HTTP 403. 600 requests per 5 s per IP: no lane. Held and tail-refreshed when folded (`cache::policy`) |
  | Kraken | 1m 5m 15m 30m 1h 4h 1D | Public API, no key. `OHLC` answers only the newest 720 candles and cannot page back past them (`KRAKEN_BARS`): 12 hours of 1m, 30 days of 1h, 2 years of 1D. The chart's first 500 leave the rest for its first older page (`supports_older_history` is true, `window`), and the next page is empty; a full window drops the fragment a fold of it would start with. Its week (`10080`) starts on a Thursday and its `21600` is 15 days counted from 1970, so 1W, 1M and 1Y fold from days. Ids are `BTC/USD` on REST and websocket alike (`XBT`/`XDG` are refused: `kraken_asset`). One request every six seconds (`net::rate_limit::kraken`: measured, a burst of about nine gets through and then only about one in six seconds; faster is `EGeneral:Too many requests`), so charts and overlays refresh on the slow schedule (the socket keeps them live) on a worker lane of its own (`work_queue::KRAKEN_LANE`: the sleep must not hold the general workers; the quote adapter does not wait, since a round joins every source); refusals are HTTP 200 with an `error` code (`is_rate_limit`) |
  | Coinbase | 1m 5m 15m 1h 6h 1D | `MAX_PAGES` x 290 candles caps a request (a 5m series reaches back ~60 days). Paging is by time window, so an answer that stops at the cap with fewer candles than asked (an illiquid product) begins at the window's start, and a first bucket that opened before it is a fragment (`FoldPlan::shape_since`); an empty page is the start of the product's history, and the count rule alone applies. **Coinbase answers one candle address from its cache for 300 s** (`max-age=300`, `no-cache` ignored; measured 2026-11-27), so the newest page is asked for with `&end=<now>`: the API ignores a bound given alone, and the answer is current. An `end` after now with a `start` pads flat zero-volume rows for periods not begun |
  | Alpaca | 1m 5m 15m 30m 1h 1D 1W 1M | Alpaca also has `3Hour`, `4Min`, `2Month` and so on, but it does not say where those start; only lengths that divide the hour are used, the rest fold. Its `1Hour` is cut on the clock hour (09:00 to 10:00 New York), not where a stock's session grid cuts (09:30), so a stock's hours, `2h`, `4h` and the rest fold from its `30Min` bars (`Grid::UsEquities`) and the native hour is never the base. Intraday answers hold pre and post market bars; the session grid drops them, so a request asks for three times the base bars it shows. Days, weeks and months keep Alpaca's New York-midnight stamps; a fold re-anchors them on UTC. 10,000 bars per request |
  | DEXScreener (GeckoTerminal) | 1m 5m 15m 1h 4h 12h 1D | 1,000 per request; empty buckets are filled flat (`bucket::fill_gaps`) before a fold |
  | CoinGecko | none: prices, `model::market::samples` | `/market_chart` samples are 5 minutes for 1 day, hourly to 90 days, daily to 365; a candle is built from samples so the spacing must divide the candle: `chart_days(timeframe, limit)` is the days to ask for, or `None` (1m, and any length that is not a whole number of five minutes: 7m, 22m). A sample is the price at its instant: the one on a boundary closes the candle that ends there (checked against Binance on 5 minutes, hours and days). A candle opens at the previous close, so an answer's first bucket, which has none, is left out. The last sample is the price now, so the newest candle moves until the next one starts; when that sample has passed a boundary whose own sample is not published yet (seen: over two minutes late), it stands in for it, so no candle is skipped |
  | Polymarket, Kalshi | lines: `model::market::line` | any timeframe: a point per timeframe (a week at most) over 60 candles, between a month and a year, at most 1,000 points (a 1m line reaches back 16 hours). Polymarket: the shortest `prices-history` window that holds the span and allows `fidelity` = the point spacing (`1w` allows 5 minutes, `1m` 10, `max` is the whole history only from 12 hours; where none holds the span and allows the fidelity the line is shorter). Kalshi: the longest of 1, 60 and 1,440 minutes that is no longer than a point, over at most 1,000 of it |
  | CoinMarketCap | none | ignores the timeframe |

- **Binance has two markets under one source.** `BTCUSDT` is the spot pair, `BTCUSDT.P` its
  USDⓈ-M perpetual. The suffix is read only by `model::market::binance` and
  `InstrumentId::market()`; a `Listing` (source and market) decides candle lengths, live feeds,
  overlay streams, quote batches and backoff. Never test a symbol for `.P` elsewhere. What differs
  between the two APIs is data in `service::market::binance::endpoints`, not branches. Futures
  facts (checked 2026-11-27): klines have no `1s`; kline sockets answer only under
  `fstream.binance.com/market/` (other paths connect and stay silent); the 24h ticker takes one
  `symbol` or none (`symbols` is ignored), so one watched perpetual is asked by name (HTTP 400
  for a name never listed, HTTP 200 with `{}` for a contract that no longer trades) and more as the whole list; `exchangeInfo` takes no filter. The futures list keeps
  `PERPETUAL` and `TRADIFI_PERPETUAL` contracts whose name the app reads back unchanged. Each
  market has its own catalog; a failed spot list fails the search, a failed futures list only
  costs the perpetuals. A perpetual is never offered as
  a portfolio asset. Pine's `syminfo.type` is `crypto` for both (TradingView not checked).
- **Oanda has two environments** (`model::market::oanda::OandaEnvironment`), live
  (`api-fxtrade.oanda.com`) and practice (`api-fxpractice.oanda.com`), and a token works only on its
  own: the other answers 401. The choice is saved with the account id and token (items without it are
  live), `DEGENVIEW_OANDA_ENVIRONMENT=practice` goes with the other environment variables, and every
  request is built by `service::market::oanda::api_request`, which takes the host from the login that
  signs it. Do not build an Oanda URL anywhere else. One answer uses one login: it is looked up once
  and passed down (`klines_range`), never per page, and what is kept between answers (the watchlist's
  day-ago references) is kept per environment.
- **Oanda history** uses UTC-aligned midpoint candles. Range reads feed `to` and `count` into
  the shared backward pager so a market closure consumes no page slots. The adapter requests an
  extra row and drops any candle at the exclusive end cursor. A history request is capped at
  40 pages, before folding. The watchlist publishes batched midpoint quotes independently of
  history. Its background job fetches the UTC hour covering 24 hours ago through the same candle
  service. An empty hour leaves the 24-hour change blank until the cutoff advances.

- **Alpaca feeds** (`model::market::feed`): bars of a day and longer come from SIP (consolidated
  volume, 15 minutes late on the Basic plan, which the app cannot tell from a paid one), intraday
  bars from real-time IEX (a single venue: 2-4% of the volume). A SIP request whose `end` falls inside
  the delay is refused on Basic, so `end` is left out there (`bars_window`): Alpaca then applies the
  account's entitlement, and a caller's range is trimmed by the provider. Never cap `end` to the
  delay, which holds a paid account behind. A SIP `start` inside the delay is refused on Basic too
  (403, with or without `end`, measured on `1Day`), so `bars_window` makes no request: no bar of a
  day or longer opens that recently on any plan. Charts name their feed (`DataSource::chart_feed_label`);
  `display_name()` stays `Alpaca (IEX)` because portfolio and paper keys are built from it.
- **A US stock differs from TradingView in these ways** (as of 2026-11-25; `Grid::UsEquities`, probe A8 in
  `report-dont-commit/verify/README.md`, TradingView `NASDAQ:AAPL` on its default regular trading hours,
  `RTH` in its status bar):
  - *Extended hours are never drawn.* Regular hours are TradingView's default, so the candles agree.
    TradingView's own "extended trading hours" setting is not supported, and its candle structure in
    that mode has not been measured (it probably anchors and cuts differently, so do not build
    pre and post market buckets on a guess: measure it first).
  - *Early closes are not modelled* (13:00 on the day after Thanksgiving, 24 December, …): candles
    stop where the data does, but the last bucket still closes at 16:00 (`time_close` of that
    candle is later than TradingView's 13:00). Pinned by
    `service::market::alpaca::tests::an_early_close_is_not_modelled…`.
  - *Holidays are not modelled.* They hold no bar, so nothing is built for them; a week or month
    whose last weekday is a holiday closes one bar later than TradingView's (the next bucket's first
    bar completes it).
  - *A daily, weekly or monthly bar keeps Alpaca's New York-midnight stamp* (TradingView stamps a
    daily stock bar `09:30`, closing at `16:00`, measured on a 1D chart). From Pine on an intraday
    stock chart `time_close("D")` is 16:00 as on TradingView, but `time("D")` is the UTC midnight
    (the evening before in New York): TradingView's value from an intraday chart was not measured.
    Scripts that only compare it between bars (`time("D") != time("D")[1]`) are not affected.
  - *The feed.* Alpaca's intraday bars are IEX (2 to 4% of the volume), TradingView's are
    consolidated: candle times and structure agree, prices and volumes do not.
- Alpaca bars close 1 ms before the next opens (days, weeks and months keep the New York midnight
  offset, so an hour off across a clock change). Intraday fetches look back at least 7 days, so a
  tail refresh (4 bars) finds the last session over a weekend; every start is clamped to
  `EARLIEST_MS`. A request is at most 10,000 bars, so a `22m` series from 1-minute bars is shorter
  than 500. Alpaca answers a time window, not a count, and its bars are sparse (IEX trades, holidays):
  a short answer is not the start of history. `shape_since` takes the window's start (the lookback, or
  the clamped range start) and drops a first bucket that opened before it. A bucket's first base slot
  is its UTC start plus the offset the provider stamps its bars with (New York midnight for days and
  months), so a window that began at the first of those bars keeps its first bucket; an answer that is
  only a fragment of one bucket is empty.
- **Older history** (`DataSource::supports_older_history`): Binance, Coinbase, OKX, Bybit, Gate.io (to its 8,900-candle reach), MEXC and Alpaca page back at
  any length. GeckoTerminal's free API reaches back 180 days, so it pages (one page of base candles,
  folded) only while a page of the base candle (1,000 of them) is shorter than that: bases up to 4h
  (1h, 3h and 4h charts yes; 12h, 1D, 1W and 1M, whose first fetch already reaches the cap, no). A
  401 on `before_timestamp` is the end of history.
- The chart offers 1h+, but Pine's `request.security` may ask any timeframe (5m intrabars, 4h):
  `timeframe_of` turns Pine's seconds into a `Timeframe` and the provider folds it, so an odd length
  no longer fails with `PINE4021`/`PINE4022`; a length that is no timeframe (90.5 seconds) still
  does, and one under a minute (`30S`) needs a source with second candles (Binance, OKX, Gate.io). Pine states lengths in seconds with a 30-day month; `pine::time::bucket` cuts
  the buckets the same way as everything else, a length that is no timeframe flooring onto the epoch.
  `request.security_lower_tf` intrabars are fetched in the base candle of the chart's own source
  (`DataSource::plan(length).base()`, so 30 s is 10 s candles on Gate.io and 1 s on Binance) and folded
  with that plan in the Pine host, bounded by `MAX_INTRABAR_CANDLES` base candles. The fetch starts at
  the opening of the target bucket that holds the window's start, and a first bucket the history
  still starts inside of (a history cap) is dropped, not shown as a whole candle with its first seconds
  missing (`fold_intrabars`). A source with no
  plan for the length falls back to its own series of that length. A fetch that fails with a
  transient error (`ProviderError::is_transient`: a dropped connection, 408/425/429/5xx) is asked
  again twice (`service::market::retry::while_transient`, 0.5 s and 1.5 s); one that still fails
  gives no intrabars (`request.security_lower_tf` returns empty arrays, `request.security` to a finer
  timeframe raises PINE4021) and the session asks again after 15 s or at the next rebuild, rather than
  answering with the newest candles of that length, which would leave older bars empty without a word.
- `service::fetch_klines` runs every provider through `service::normalize` (ascending, one candle per
  open time, finite prices, range holds open and close). GeckoTerminal lists only traded intervals;
  `bucket::fill_gaps` adds flat zero-volume candles for even spacing. Volume units differ (base asset,
  shares, USD); CoinGecko has none.
- GeckoTerminal charts a pool's base token, the contract sorting first on chain, so a pool can read
  "USDT / MCADE"; DEXScreener (DEX search) lists the traded token first. `geckoterminal::side_for`
  reads the pool's token symbols once, remembers, and asks `token=quote` when the base is a stablecoin
  or wrapped native coin (`QUOTE_ASSETS`) and the quote is not; else base. Without it the chart shows
  the stablecoin at ~$1. Unreadable tokens: base, unremembered.
- Prediction-market lines: `model::market::line::window(timeframe)` (span and point spacing), read by
  `polymarket::pm_window` and `kalshi::kalshi_window`; the table above has the rules.

## Caches and live data

- `service::cache::fetch`: slow providers' candles in memory by `(source, symbol, timeframe)`
  (`cache::policy(source, timeframe)` lists them and TTLs: Binance only for a folded timeframe, where
  a series takes a dozen requests); refreshes a held series with only its last `TAIL_LIMIT`
  candles; concurrent requests for one series share one call. Add a provider to `policy` only if a
  whole fetch is slow; its `fetch_klines` must honour a small `limit` cheaply. `service::net::rate_limit`
  spaces CoinGecko and GeckoTerminal; `work_queue::submit_for` gives each its own worker; submit chart
  requests through it.
- Regenerable slow data persists in the OS cache directory, never the data directory, via
  `service::cache::disk`/`cache::lookup` (TTL, stale-while-revalidate, stale-on-error). Discard corrupt
  files. New caches join `cache::control::clear`; writes use `DiskCache::write(..., generation)` so an
  in-flight write cannot undo a clear. Tests use `cache::disk::testing::TempCache`; the global cache is
  disabled in tests.
- `service::image::icon` keys by `"<source>:<ticker>"`, never chart id: static CDN (unthrottled HEAD), then
  rate-limited CoinGecko search, then a letter monogram.
- One `service::market::live::LiveFeed` per workspace and streamed source (`DataSource::CANDLE_STREAMS`;
  each protocol is a module under `live/` that builds a `net::ws::Session` and parses its messages): updates the forming candle, reconnects with backoff. MEXC is the sixth: its spot socket speaks Protocol Buffers only (the JSON topics are refused), read by `service::net::protobuf` (a field iterator, no dependency; `ws::Frame::Binary` carries the frames), with `spot@public.kline.v3.api.pb@<SYMBOL>@Min1` topics, at most 30 per connection (so `LiveFeed` runs one connection per 30 markets), one topic per subscribe request and `{"method":"PING"}` as keepalive. Gate.io is the fifth (`spot.candlesticks` and `spot.tickers` on `wss://api.gateio.ws/ws/v4/`: one subscribe request per market, since a request that names a market it does not know is refused whole; protocol pings keep it alive). Bybit is the fourth (`kline.<interval>.<SYMBOL>` on `wss://stream.bybit.com/v5/public/spot`: one subscribe request per topic, because a request takes at most 10 and one unknown symbol refuses all of it; `{"op":"ping"}` keepalive, whose answer `ws::Keepalive::Text` recognises by its start). Kraken is the third (websocket v2 `ohlc` with `snapshot:false`, one subscribe naming every market, kept alive by the server's one-a-second `heartbeat` and protocol pings; its `ticker` channel carries the rolling 24h `change`, which gives the price a day ago). OKX is the second stream (`candle<bar>` channels on `wss://ws.okx.com/ws/v5/business`, one subscribe message naming every market, kept alive with the text `ping`/`pong` through `ws::Keepalive::Text`; its `public` endpoint refuses candles). Binance's streams
  the *base* candle the timeframe is served from (`DataSource::Binance.plan(timeframe).base()`:
  `@kline_1m` for a 22-minute chart, `@kline_1M` for `3M` and `1Y`; none for a timeframe Binance cannot
  reach). Charts and compared markets fold a live kline with `model::market::live::LiveFold::apply`
  only, and every chart of the symbol takes it. For the timeframe's own length that is
  `apply_live` (replace the newest bar or append; a kline older than the newest is dropped, so candles
  stay ascending). For a folded one the forming bucket is rebuilt from the latest state of each base
  bar it holds (a kline is a running total, resent until its bar closes, so adding would double
  count), and the bucket cuts where `bucket` does (a `22m` day's last bucket closes at midnight).
  What the socket did not see of the bucket (the bars before it connected, or while it was quiet)
  comes from REST: `service::fetch_klines_live` asks, after the folded answer, for the base candles
  of the newest bucket (one extra small request per refresh, only for a folded timeframe on a
  streamed source; `FoldPlan::newest_bucket`), and `LiveFold::merge_fetched` merges them with the
  socket's bars per bar. One rule takes a bar's state from either side: it replaces the stored one
  unless it has less volume (a running total only grows; on a tie the arriving state wins), so
  nothing is counted twice and neither side rolls the other back. A bucket the socket has not
  reached starts from the bars; one newer than them is left alone. The merged series takes the
  forming candle in place of its newest candle only, so REST's finished copy of an earlier bucket
  stays. If that request fails, or the answer has no bars, the held-candle rule is the fallback:
  the bucket's open, range and volume are what the chart held when the socket's first kline
  arrived, and that kline's volume is not counted twice (`first_volume`). The feed connects as
  soon as the chart exists, so a kline can arrive before the first REST answer. A chart and an
  overlay series each keep their own `LiveFold` and merge every REST answer through it; it resets
  when an answer replaces the forming bucket (no socket bar newer than the request) or the market
  or timeframe changes. A current-or-newer WebSocket bar beats an older REST snapshot
  (`live::merge_fetched`) only if the socket delivered after that request began
  (`LiveMark::guard_for`, carrying the bucket's open time): when it goes quiet, REST refreshes update
  the forming bar again.
- A chart whose last answer was `UnsupportedInterval` shows it in the usual error slot and is not
  refreshed again (`Chart::refresh_due`) until its market or the timeframe changes, which fetch at
  once. An overlay market becomes `OverlayStatus::Blocked(text)` and is not retried either.
- `service::cache::patched` merges a tail into a held series by open time: the tail's copy of a candle
  wins and a held candle the tail skips stays. A tail that does not reach back to the series is
  fetched whole.
- `service::cache::fetch`: a failed tail refresh falls back to the whole fetch, except a rate limit,
  which is returned.
- Visible watchlists share one ticker socket per source in `DataSource::QUOTE_STREAMS` (Coinbase: last
  trade, 24-hour open, volume, it has no candle channel; OKX: its `tickers` channel on `/ws/v5/public`; Kraken: its v2 `ticker`; Bybit: `tickers.<SYMBOL>`; Gate.io: `spot.tickers`, with the price a day ago taken from its 24h change in percent). Kraken's REST ticker has only the UTC day's open (`o`), which is not a price a day ago, so its REST quote reports no change; the socket's rolling `change` gives it, and the engine keeps that reference (`stream_references`, `Config::stream_reference_ms`, 10 minutes) for a REST refresh of a streamed market whose socket went quiet. A market the socket never answered shows no change. `watchlist_quotes::quote_stream` holds one
  module per protocol; the engine keeps per-source products, and REST fills a product the socket has
  not answered. A new socket is a `Session` plus a parser, never another copy of the connect/read/backoff loop.

## Watchlists

- `WatchlistStore` is authoritative: one ordered document row per list, flat instrument and section
  entries. Validate, persist in one transaction, then publish. Never overwrite a list that failed to
  decode. Favorites is the `isFavorites` list; Swift favorite rows import once.
- Identity is `InstrumentId::key()` (source + `canonical_symbol()`): Binance
  `BTC`/`btcusdt`/`BTC/USDT` are one market, as are Coinbase `ETH`/`ETH/USD`/`eth-usd`. Use the key,
  not the struct's derived `Eq`/`Hash`, for membership, flags, quote lookup and sync merging. Older
  spellings collapse and re-key on load. Chain is metadata; ticker text is not identity. CoinMarketCap
  portfolio cards are not instruments.
- Flags: global per market in `watchlist.flags`, synced by the Watchlists switch (not Preferences),
  merged per key; `sync::record::policy::category_of` / `merge_of` decide per setting key. Sort/filter is
  presentation-only, within sections, via `watchlist_layout`.
- Quotes are transient. `WatchlistQuoteCoordinator` merges demand by `InstrumentId`, batches per
  source, shares Coinbase's socket, backs off per source, fills `WatchlistQuoteBook`; never persists
  quotes or uses the alert quote path.
- A sidebar requests quotes only for market rows it draws (`WatchlistSidebarState::quote_demand`):
  not other lists, filtered-out rows, collapsed sections or portfolio cards. The highlighted row is
  always drawn, so Enter/Delete and the detail card never act on a hidden one.
- Sidebar demand is a lease (`set_sidebar_demand`, `SIDEBAR_LEASE_MS`): renewed each drawn frame,
  lapses 3 s after the last, so background tabs and hidden/covered windows stop polling. Closing the
  sidebar suspends it at once. The coordinator requests a repaint at half lease so an idle visible
  sidebar renews. Other owners (alerts, portfolio cards) use `set_demand`, which never lapses.
- `provider::classify_status`: only a documented symbol-level answer (Binance/Alpaca 400, Coinbase
  404) is `Rejected`, making the engine bisect for the bad symbol within `ISOLATION_BUDGET` requests
  per round; credential, region, endpoint and ban statuses back off the whole source. A rate limit
  stops the round's remaining requests. Providers return an `Outcome` (quotes, rejected, failed
  requests with symbols), so a failed page or product marks only its rows stale.
- Unrequested for `Config::idle_grace_ms` (10 min), a market is forgotten: quote, confirmation time,
  refusal, Coinbase bookkeeping, book entry. Resolved DEX networks (`chains`) stay. A returning market
  shows no old price until fetched.
- Store mutations return `Result<bool, WatchlistError>` (`Ok(false)` = nothing changed or written)
  and run on a copy of one list (`WatchlistStore::modify`), so a refused change leaves it intact; the
  workspace shows why (`report_watchlist`). Names: `Watchlist::validated` (whitespace collapsed, no
  control characters, `MAX_NAME_CHARS`). Limits `MAX_ENTRIES_PER_LIST` and `MAX_LISTS`, shared with
  the sync validator. `Watchlist::touch` stamps `updated_at_ms` on content edits (rows, names), not
  sort, column or collapse. Text import caps at `MAX_IMPORT_BYTES`/`MAX_IMPORT_LINES` and reports
  early stops; export name is `export_file_name(name)`.
- On load `WatchlistStore::repair` demotes a second Favorites list, re-ids repeated row ids,
  collapses repeated markets; nothing else is deleted.
- Selecting a row focuses or opens its market in that workspace; highlighting mirrors the focused
  chart.
- Rows chart the market's last month (`WatchlistDisplaySettings::shows_chart`, per list, on by
  default and for lists saved before it; Columns ▸ Show 1M Chart). Closes come from the app-wide
  `service::market::trend::month()` loader (30 calendar days of `1d` candles, fresh for 10 minutes),
  asked only for the rows the sidebar draws (`WatchlistSidebarState::trend_demand`), none when the
  chart is off. The panel's minimum width (`style::minimum_panel_width`) makes room for it.

## Paper-trading quotes

- Paper orders fill from chart prices, no own feed. `sync_paper_quotes` hands each rendered market
  chart's last close to `PaperHost`, panel open or not (closed: only instruments with a working order
  or position). Nothing fills while the app is quit/asleep or no chart for the instrument renders.
- A chart whose bars may be late (`DataSource::bars_are_real_time`: Alpaca on SIP, 1D and longer)
  never prices an order with its close. `sync_paper_quotes` takes the real-time IEX snapshot from
  the watchlist quote book instead (demand owner `paper_quote_owner`), and withholds the price
  while that snapshot is not current.
- Quote timestamp = candle arrival (`Chart::last_data_ms`), never read time. Prices older than
  `watchlist_quote::maximum_age_secs` are withheld; Alpaca prices outside US hours carry
  `is_market_open = false`, so an old close cannot fill.
- In replay the quote stays live; the ticket's reference follows the replay slice. Unresolved: should
  replay block paper orders?

## Network

Every HTTP agent is built from `service::net::config_builder_for(url)` (or is a `net::agent::FollowingAgent`, used through `get_for(url)`), and every WebSocket opens through `net::ws::connect` and passes its `Route` to `ws::Health::new`, so the user's proxy applies and a feed reconnects when it changes. Never start from `ureq::Agent::config_builder()` or `Agent::new_with_defaults()`, and never open a `TcpStream` to a provider yourself. See [Network proxy](../network-proxy.md).
