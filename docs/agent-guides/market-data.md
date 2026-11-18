# Market data guidance

## Adding a provider

1. `model::market::source::DataSource` metadata: display name, watchlist slug, prediction-market flag,
   volume support, line/candle mode.
2. Implement `service::MarketSource`; `search_tickers` if supported; a parser for a new kline shape.
   Declare its candle lengths in `model::market::natives` and plan every request through
   `DataSource::plan` (see Timeframes), so `serves` and the fetch agree.
3. Register in `service::for_source`. `supports_trending`/`trending` only if it can rank active
   markets (Kalshi cannot).
4. Add to the symbol picker (`ui::chart::symbol_picker`, Predictions tab) via `is_prediction_market()`, not provider matching.
5. `SourceLogo` under `assets/source_logos/`, the `ui::kit::source_logo::source` arm, attribution in
   `docs/source-logos.md`.

Prediction markets: lines under Predictions, REST refresh. Kalshi ids are `"SERIES/MARKET"`; cache
its series index in memory, rank locally.

Search groups `SearchResult::event` choices as a tree. Choices selected from one event make one chart:
`EventInfo::series` holds them, the first supplies symbol and candles, `ChartCard::set_series` draws
them by position with end labels. Persisted in `PersistedChart::pm_series`/`title`; not yet in
watchlists.

## Names and search

- Name a market as its exchange does (`BTCUSDT`, `BTC-USD`); `DataSource::display_symbol` only
  upper-cases. Never rebuild `BTC/USDT` labels. Asset: `DataSource::base_asset` /
  `SearchResult::base_asset`; pair: `SearchResult::base`/`quote`.
- Hits are ordered by `service::market::search_rank`; see [Symbol picker](../symbol-picker.md).
- `VENUE:SYMBOL` text (`BINANCE:BTCUSDT`, `NASDAQ:AAPL`) is read only by `model::market::venue`
  (watchlist slugs plus `US_EXCHANGES`, which are Alpaca); the watchlist import and the picker both use
  it. A new provider's slug is `DataSource::watchlist_slug`; do not add a second prefix table.

## Timeframes

- `model::Timeframe` is a value, `{ count: u16, unit: TimeUnit }` (`Minute`, `Hour`, `Day`, `Week`,
  `Month`), not a list. Limits: minutes 1-1440, hours 1-24, days 1-365, weeks 1-52, months 1-12;
  `Timeframe::new` refuses the rest (`TimeframeParseError`, whose messages are shown as they are).
  Canonical form: whole hours of minutes are hours (`60m` is `1h`, `1440m` is `24h`), `24h` stays
  apart from `1D`, and `12M` is the year. Equality, hashing and ordering (by nominal length, then
  unit) follow that form.
- Consts: `MINUTE_1` ... `YEAR_1`; `PRESETS` are the lengths providers serve natively, `LEGACY` the
  six (`1h`, `1D`, `1W`, `1M`, `3M`, `1Y`) saved data names with a variant name (see serde below). The toolbar and Pine Studio offer every timeframe through the [timeframe picker](../workspace-toolbar.md#timeframe-picker).
- `Timeframe::parse`: digits, then an optional unit, spaces allowed. No unit is minutes (`22` is
  `22m`); `m` is minutes and `M` months, the rest take either case (`h d w y`); `1Y` is `12M`, `2Y`
  is refused, seconds (`s`) are refused. It reads the old labels and provider tokens too.
- Three spellings: `Display`/`label()` for the UI (`22m`, `3h`, `2D`, `1W`, `3M`, `1Y`), `interval()`
  for the token every provider is asked for (`22m`, `3h`, `2d`, `1w`, `3M`, `1Y`: the legacy six are
  byte-identical to what providers always got) and, for saved data, serde: the six `LEGACY` values
  serialize as `"Hour"`, `"Day"`, `"Week"`, `"Month"`, `"Quarter"`, `"Year"` so older builds and
  synced layouts decode them; anything else is its label (`"22m"`). Decoding takes both.
  `nominal_ms()` is exact to weeks; months are 30 days, a quarter 91 and a year 365.
- Buckets are `model::market::bucket` and nothing else cuts them (providers, chart and Pine): UTC,
  minutes and hours restart at midnight (a `22m` day ends in a shorter bucket), days count from
  1970-01-01, weeks start on Monday, months count from 1970-01 (so `3M` is the calendar quarter,
  `12M` the year). A bucket closes 1 ms before the next opens.
- Missing interval: fold finer bars with `bucket::fold(candles, timeframe, truncated)`. Never
  substitute a nearby interval. `bucket::fold_base(target, natives)` is the coarsest native whose
  buckets nest exactly in the target's (an intraday base must divide the day, and the target when
  that is intraday; days nest in multiples of days and, as single days, in weeks and months; weeks
  in weeks, months in months), or `None`. `3M`/`1Y` are real quarters/years everywhere. Pass
  `truncated` when the series starts where a request stopped, to drop the partial first bucket.
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
  | Binance | 1m 3m 5m 15m 30m 1h 2h 4h 6h 8h 12h 1D 1W 1M | 1000 per request, paged in parallel (`fetch_klines` pages when a fold needs more). Its `3d` is left out: Binance's 3d bars start on a day that is not a multiple of three since 1970-01-01 (checked against the API), so 3D folds from 1D. Weeks start Monday, as here. Folded series are cached (`cache::policy(source, timeframe)`) and refreshed by their tail |
  | OKX | 1m 3m 5m 15m 30m 1H 2H 4H 6H 12H 1D 1W 1M 3M | Public API, no key. Asked as `…utc` bars from 6H up (its plain ones are cut at UTC+8). Left out: `8H` (no such bar), `2D`/`3D` (counted from a day that is not a multiple of the length since 1970-01-01, like Binance's `3d`: fold from 1D) and the year (folds from `3Mutc`). 300 per request: `/market/candles` for the newest, `/market/history-candles` with an `after` cursor for older (20 requests per 2 s, so pages are spaced 100 ms). Spot only. A request pages back at most 120 pages (36,000 base candles, `MAX_CANDLES`), capped before the fold (`base_candles`) so a cut-short answer drops its partial first bucket instead of keeping it. Held and tail-refreshed (`cache::policy`) |
  | MEXC | 1m 5m 15m 30m 1h 4h 8h 1D 1W 1M | Public v3 API, no key, spot only; Binance-style ids (`BTCUSDT`), but split by MEXC's own quotes (`model::market::source::split_mexc_pair`: `USDT`, `USDC`, `USD1`, `USDE`, `USDF`, `EUR`, `BRL`, `ETH`, `BTC`), so `BTCUSD1` and `SUSDT` are listed ids and not a bare asset to quote in USDT (a one-letter base before a non-stable quote, `WBTC`, still is one). Its `/klines` takes a `limit` (newest rows) **or** a `startTime` + `endTime` window and ignores an `endTime` alone, 500 rows at most: older pages are windows (`window_before`, a month taken at its shortest), 20,000 base candles a request (`Paging::max_candles`, capped before the fold). **Its oldest days, weeks and months (before about 2018-12) are cut at UTC+8 midnight**: parsing drops every candle that is not on `bucket_start_ms`, so history starts where it is on the grid. Its tokenised stocks and commodities (`GOLD(XAUT)USDC`) are not letters and digits and are left out of search. Watchlists use the REST `/ticker/24hr` (all markets in one 670 KB answer; `openPrice` is a real 24 h reference). No ticker socket on purpose: its mini ticker's change is for a time-zone day, not rolling 24 h. Held and tail-refreshed when folded (`cache::policy`) |
  | Gate.io | 1m 5m 15m 30m 1h 4h 8h 1D 1W 1M | Public v4 API, no key, spot only; ids are `BTC_USDT`. Natives are its documented intervals `1m … 8h`, `1d`, `7d` (weeks from Monday) and `30d` (the calendar month): all cut where this app cuts, so 3M and 1Y fold from months (it also answers `3m`, `2h`, `6h` and `12h`, undocumented, so they fold). 1,000 per request, paged back with a `to` cursor through `service::market::paging`. **A request may not reach 10,000 points back** in the interval asked for (HTTP 400 "Candlestick too long ago"): the cap is 8,900 base candles (`Paging::max_candles`, capped before the fold) and `reach_left` answers an older page past it as empty, so a chart does not retry. 200 requests per 10 s per IP: no lane. A 400 is an unknown market, a 403 a country it does not serve. Held and tail-refreshed when folded (`cache::policy`) |
  | Bybit | 1m 3m 5m 15m 30m 1h 2h 4h 6h 12h 1D 1W 1M | Public v5 API, no key, spot only; symbols are Binance-style `BTCUSDT`. Every bar is cut where this app cuts (weeks Monday, months calendar), so 3M and 1Y fold from months. 1,000 per request, paged back with an `end` cursor through `service::market::paging` (60 pages, capped before the fold). Refusals are HTTP 200 with a `retCode` (`10006`/`10018` are rate limits) and some countries get HTTP 403. 600 requests per 5 s per IP: no lane. Held and tail-refreshed when folded (`cache::policy`) |
  | Kraken | 1m 5m 15m 30m 1h 4h 1D | Public API, no key. `OHLC` answers only the newest 720 candles and cannot page back past them (`KRAKEN_BARS`): 12 hours of 1m, 30 days of 1h, 2 years of 1D. The chart's first 500 leave the rest for its first older page (`supports_older_history` is true, `window`), and the next page is empty; a full window drops the fragment a fold of it would start with. Its week (`10080`) starts on a Thursday and its `21600` is 15 days counted from 1970, so 1W, 1M and 1Y fold from days. Ids are `BTC/USD` on REST and websocket alike (`XBT`/`XDG` are refused: `kraken_asset`). One request every six seconds (`net::rate_limit::kraken`: measured, a burst of about nine gets through and then only about one in six seconds; faster is `EGeneral:Too many requests`), so charts and overlays refresh on the slow schedule (the socket keeps them live) on a worker lane of its own (`work_queue::KRAKEN_LANE`: the sleep must not hold the general workers; the quote adapter does not wait, since a round joins every source); refusals are HTTP 200 with an `error` code (`is_rate_limit`) |
  | Coinbase | 1m 5m 15m 1h 6h 1D | `MAX_PAGES` x 290 candles caps a request (a 5m series reaches back ~60 days). Paging is by time window, so an answer that stops at the cap with fewer candles than asked (an illiquid product) begins at the window's start, and a first bucket that opened before it is a fragment (`FoldPlan::shape_since`); an empty page is the start of the product's history, and the count rule alone applies |
  | Alpaca | 1m 5m 15m 30m 1h 1D 1W 1M | Alpaca also has `3Hour`, `4Min`, `2Month` and so on, but it does not say where those start; only lengths that divide the hour are used (an hour's start is the same in UTC and New York), the rest fold. Days, weeks and months keep Alpaca's New York-midnight stamps; a fold re-anchors them on UTC. Stocks fold intraday buckets from UTC midnight, not the session open. 10,000 bars per request |
  | DEXScreener (GeckoTerminal) | 1m 5m 15m 1h 4h 12h 1D | 1,000 per request; empty buckets are filled flat (`bucket::fill_gaps`) before a fold |
  | CoinGecko | none: prices, `model::market::samples` | `/market_chart` samples are 5 minutes for 1 day, hourly to 90 days, daily to 365; a candle is built from samples so the spacing must divide the candle: `chart_days(timeframe, limit)` is the days to ask for, or `None` (1m, and any length that is not a whole number of five minutes: 7m, 22m) |
  | Polymarket, Kalshi | lines: `model::market::line` | any timeframe: a point per timeframe (a week at most) over 60 candles, between a month and a year, at most 1,000 points (a 1m line reaches back 16 hours). Polymarket: the shortest `prices-history` window that holds the span and allows `fidelity` = the point spacing (`1w` allows 5 minutes, `1m` 10, `max` is the whole history only from 12 hours; where none holds the span and allows the fidelity the line is shorter). Kalshi: the longest of 1, 60 and 1,440 minutes that is no longer than a point, over at most 1,000 of it |
  | CoinMarketCap | none | ignores the timeframe |

- Alpaca bars close 1 ms before the next opens (days, weeks and months keep the New York midnight
  offset, so an hour off across a clock change). Intraday fetches look back at least 7 days, so a
  tail refresh (4 bars) finds the last session over a weekend; every start is clamped to
  `EARLIEST_MS`. A request is at most 10,000 bars, so a `22m` series from 1-minute bars is shorter
  than 500. Alpaca answers a time window, not a count, and its bars are sparse (IEX trades, holidays):
  a short answer is not the start of history. `shape_since` takes the window's start (the lookback, or
  the clamped range start) and drops a first bucket that opened before it.
- **Older history** (`DataSource::supports_older_history`): Binance, Coinbase, OKX, Bybit, Gate.io (to its 8,900-candle reach), MEXC and Alpaca page back at
  any length. GeckoTerminal's free API reaches back 180 days, so it pages (one page of base candles,
  folded) only while a page of the base candle (1,000 of them) is shorter than that: bases up to 4h
  (1h, 3h and 4h charts yes; 12h, 1D, 1W and 1M, whose first fetch already reaches the cap, no). A
  401 on `before_timestamp` is the end of history.
- The chart offers 1h+, but Pine's `request.security` may ask any timeframe (5m intrabars, 4h):
  `timeframe_of` turns Pine's seconds into a `Timeframe` and the provider folds it, so an odd length
  no longer fails with `PINE4021`/`PINE4022`; a length under a minute, or that is no timeframe
  (`30S`), still does. Pine states lengths in seconds with a 30-day month; `pine::time::bucket` cuts
  the buckets the same way as everything else, a length that is no timeframe flooring onto the epoch.
  `request.security_lower_tf` intrabars are folded in the Pine host from the coarsest of its own
  base list that nests (`INTRABAR_BASES`), bounded by `MAX_INTRABAR_CANDLES` base candles.
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
  count): the bucket's open, range and volume from before the socket's first kline are what the
  chart held then, and the bucket cuts where `bucket` does (a `22m` day's last bucket closes at
  midnight). How much of a live base bar that held candle already counts depends on when its
  request began (`LiveFold::as_of`, epoch ms): a bar that opened at or after it is not in it and
  counts in full, one that opened before counts only from its first live kline on, and with no time
  known only the bucket's first bar is taken to be in it. The feed connects as soon as the chart
  exists, so a kline can arrive before the first REST answer: the bucket then holds only what the
  socket saw, and `LiveFold::merge_fetched` makes the answer's candle of that bucket its held
  candle (once; a bucket that already has one is not moved by later refreshes). A chart and an
  overlay series each keep their own `LiveFold` and merge every REST answer through it; it resets
  when an answer replaces the forming bucket or the market or timeframe changes. A
  current-or-newer WebSocket bar beats an older REST snapshot
  (`live::merge_fetched`) only if the socket delivered after that request began
  (`LiveMark::guard_for`, carrying the bucket's open time): when it goes quiet, REST refreshes update
  the forming bar again.
- A chart whose last answer was `UnsupportedInterval` shows it in the usual error slot and is not
  refreshed again (`Chart::refresh_due`) until its market or the timeframe changes, which fetch at
  once. An overlay market becomes `OverlayStatus::Blocked(text)` and is not retried either.
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
- Identity is `InstrumentId::key()` (source + provider id as the provider takes it): Binance
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
- Quote timestamp = candle arrival (`Chart::last_data_ms`), never read time. Prices older than
  `watchlist_quote::maximum_age_secs` are withheld; Alpaca prices outside US hours carry
  `is_market_open = false`, so an old close cannot fill.
- In replay the quote stays live; the ticket's reference follows the replay slice. Unresolved: should
  replay block paper orders?

## Network

Every HTTP agent is built from `service::net::config_builder_for(url)` (or is a `net::agent::FollowingAgent`, used through `get_for(url)`), and every WebSocket opens through `net::ws::connect` and passes its `Route` to `ws::Health::new`, so the user's proxy applies and a feed reconnects when it changes. Never start from `ureq::Agent::config_builder()` or `Agent::new_with_defaults()`, and never open a `TcpStream` to a provider yourself. See [Network proxy](../network-proxy.md).
