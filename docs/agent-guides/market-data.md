# Market data guidance

## Adding a provider

A venue is one profile entry plus one directory plus a logo; no other file names it.

1. `model::market::source::DataSource` gets the variant, and `model::market::profile::table` its
   `Profile`: display name, watchlist slug (also read for `VENUE:SYMBOL` text and the Pine host, so
   there is no second table), MCP id, `Class` (which picker lists it), `kinds` (spot, perpetual or both), `Series`, volume support, how
   its ids join two assets (`PairStyle`) and the quote a bare asset gets, its native candle lengths
   (see Timeframes), history paging, which streams it has, change basis, quote polling and batch
   size, whether its charts refresh slowly (`slow_refresh`, for a small REST allowance), whether its requests wait on a rate limiter and so run on a worker of their own (`own_lane`), and its candle cache. Add it to `DataSource::ALL`; the profile tests fail until it is there.
2. `service/market/<venue>/`, one concern per file: `mod.rs` (the `MarketSource` and `Venue`
   implementations and wiring only), `rest.rs` (HTTP and paging), `parse.rs` (pure JSON, tested with
   inline samples), `markets.rs` (the market list and search), `ticker.rs` (the REST call behind a
   watchlist price and its parser), `live.rs` (`CandleProtocol`), `quotes.rs` (`QuoteProtocol`), and
   tests. Plan every request through `DataSource::plan` (see Timeframes), so `serves` and the fetch
   agree.
   A venue whose API is another's clone does not copy that venue's parsers: `service::market::shape::<api>`
   holds the shared klines, market list, ticker and socket code, and the venue's directory is a
   `Config` (hosts, row limit, which listed markets it serves) plus what differs. Only
   `shape::binance` exists so far (Binance spot, Aster perpetuals); extract a shape when a second
   venue speaks the same API, never before.
3. Register it in `service::market::venue::of`, the one place that pairs a source with its code. The
   `Venue` trait answers the candles, the optional candle and ticker sockets, REST quotes, how an HTTP
   status reads for a quote request, and `trending` where the venue can rank active markets (Kalshi
   cannot). `LiveFeed` and the quote engine stay the only socket loops: a protocol only builds a
   `net::ws::Session` and parses messages. A test fails when a profile's streams disagree with what
   its venue opens.
4. The symbol picker lists it from its `Class` (`DataSource::symbol_markets()`,
   `prediction_markets()`), not by matching on providers.
5. `SourceLogo` under `assets/source_logos/`, the `ui::kit::source_logo::source` arm, attribution in
   `docs/source-logos.md`.

Prediction markets: lines under Predictions, REST refresh. Kalshi ids are `"SERIES/MARKET"`; cache
its series index in memory, rank locally.

Search groups `SearchResult::event` choices as a tree. Choices selected from one event make one chart:
`EventInfo::series` holds them, the first supplies symbol and candles, `ChartCard::set_series` draws
them by position with end labels. Persisted in `PersistedChart::pm_series`/`title`; not yet in
watchlists.

## Names and search

- Name a market as its exchange does (`BTCUSDT`, `BTC-USD`, `AVAX:USD`); `DataSource::display_symbol` only
  upper-cases. Never rebuild `BTC/USDT` labels. Asset: `DataSource::base_asset` /
  `SearchResult::base_asset`; pair: `SearchResult::base`/`quote`.
- Hits are ordered by `service::market::search_rank`; see [Symbol picker](../symbol-picker.md).
- `VENUE:SYMBOL` text (`BINANCE:BTCUSDT`, `NASDAQ:AAPL`) is read only by `model::market::venue`
  (watchlist slugs plus `US_EXCHANGES`, which are Alpaca); the watchlist import and the picker both use
  it. A new provider's slug is its `Profile::slug`; do not add a second prefix table.
- **Market kind** (`model::market::MarketKind`, `Spot` or `Perp`): `SearchResult::kind` (a venue that
  lists perpetuals sets it on the hits) and `Profile::kinds` (what the venue lists, first = default).
  Never stored with a chart, watchlist row or recent: `Venue::kind_of(&InstrumentId)` answers it
  from the venue and the id (default: the profile's first kind), so no saved or synced format
  changes. A venue that lists both overrides `kind_of`; a test checks its answer is in `kinds`.
  Spot covers everything that is not a perpetual (stocks, predictions, indices).
- **Names a venue lists as they are** (`PairStyle::Listed`, Hyperliquid, Lighter and Deribit): ids are case-sensitive and
  kept as written (`BTC`, `kPEPE`, `xyz:SP500`, `PURR/USDC`, `@107`), no quote is added to a bare
  asset, `base_asset` drops the namespace (`xyz:SP500` is `SP500`; `kPEPE` has no artwork and shows a
  monogram) and an imported line is checked by `Listed::is_name`. A `:` in a name survives
  `VENUE:SYMBOL` (`HYPERLIQUID:xyz:SP500`: the first colon ends the prefix) and text import and
  export. Hyperliquid's perpetuals carry no quote (the collateral differs by dex) and its spot pairs
  are shown by their tokens (`HYPE/USDC`) and charted by the id it numbers them with (`@107`).
- **Search names** (`hyperliquid/names.rs`): where traders call a market by another name than the
  venue lists, the name goes after the id as a company does (`xyz:SP500 — S&P 500 (SPX)`,
  `xyz:XYZ100 — Nasdaq 100 (NDX)`), so the shared ranking applies ("S&P", "SP500", "SPX", "Nasdaq",
  "NDX", "Nasdaq 100" all find them) and no other dex's index perpetual is hidden (`km:US500` is found
  by `500` as it is). Add an entry there, not a search special case.

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
- **Natives live in the source's `Profile`** (`model::market::profile::table`), one list per candle
  source (`DataSource::natives()`; `model::market::natives` plans from them), so the planner a provider fetches by (`DataSource::plan`, a
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
  | Hyperliquid | 1m 3m 5m 15m 30m 1h 2h 4h 8h 12h 1D 3D | Perpetuals (the main dex `BTC`, the HIP-3 dexes under their prefix: trade.xyz's `xyz:SP500`, `xyz:XYZ100`) and spot pairs, public `/info` POSTs (`net::http::post_json`), no key and no region block. `candleSnapshot` answers a window (`startTime`/`endTime`) of at most ~5,000 candles, and **only the newest ~5,000 of each interval exist** (1m reaches back ~3.5 days, 1h ~7 months, 1D ~13 years): a chart is one page (`Paging::max_candles` 5,000), the first older page brings what the first fetch left and the next is empty, not an error; a page asks a window with room for missed bars and, when that holds too few of a page that is more than a refresh, for everything kept (`rest::page_before`; a refresh of a few bars never is, and a market whose whole history holds under 100 candles is remembered as thin and asked for whole in one request from then on, so a quiet market costs one request, not two to four, every refresh). **Its `1w` opens on a Thursday** (weeks counted from 1970-01-01) **and its `1M` is thirty days counted from 1970-01-01**, neither on this app's grid (Monday weeks, calendar months), so weeks, months and years fold from 1D; its `3d` is on the grid (day number divisible by three, unlike Binance's) and native; `2m` is refused. Verified against the API on 2026-11-21 (`live_hyperliquid_...` checks every native's opens). Volume is the base asset: contracts of a perpetual, coins of a spot pair. Names are case-sensitive and kept as listed (`PairStyle::Listed`: `BTC`, `kPEPE`, `xyz:SP500`, `PURR/USDC`, `@107`); a name it does not list is HTTP 500 with the body `null`, read through `net::http::post_json_reading_refusals` (an `http::Error::Refused` keeps an error status's body) and answered as `ProviderError::UnknownMarket`; any other 500 is an outage, not a missing market. 1,200 weight a minute per IP, a candle request weighing about 20 plus about 1 per 100 rows (measured: ~95 requests of 500 candles or ~25 of 5,000 in a burst, then HTTP 429, the allowance refilling at 20 a second): one chart is one or two requests, so there is no lane of its own; many charts refreshing every 5 s approach it, which the candle socket and the tail refresh (`cache::policy`, every series) keep down. A watchlist price is `metaAndAssetCtxs` per dex (`spotMetaAndAssetCtxs` for spot pairs): `markPx`, the rolling-24h `prevDayPx`, `dayNtlVlm` in dollars |
  | Aster | 1m 3m 5m 15m 30m 1h 2h 4h 6h 8h 12h 1D 1W 1M | Perpetual futures only (`PERPETUAL` and `TRADING` in `exchangeInfo`: coins, stock and ETF perpetuals such as `QQQUSDT`, `TSLAUSDT`, `XAUUSDT`; `SPXUSDT` is the SPX6900 memecoin, not the S&P 500). Public Binance-futures API, no key and no region block, configured on `service::market::shape::binance` (the parsers, paging and sockets are shared with Binance spot; the directory is hosts, limits and the 24 h ticker call). Verified against the API on 2026-11-21: 1,500 rows a request (2,000 is error -1130), paged in parallel; weeks open Monday and months on the first (native); **its `3d` opens on a day number that is 1 modulo 3 since 1970-01-01, like Binance's, so 3D folds from 1D**. A bar is stored some seconds after its minute opens (once over a minute), so a REST answer can lack the newest bar; the socket has it. Volume is the base asset. Ids `BTCUSDT`, split by `USDT`, `USDC`, `USD1` (`BTCU` and `ETHU`, BTC and ETH quoted in `U`, are `Concat::exceptions`: read whole, so a bare `BTCU` is not asked as `BTCUUSDT`, verified live: that is `-1121 Invalid symbol`); `USD1` is not one of the shared dollar quotes, so such a row shows units, not turnover. Weight 2,400 a minute (`x-mbx-used-weight-1m`), a 1,500-row page costing 10: no lane. Sockets `wss://fstream.asterdex.com/stream`: candles by topics in the URL (200 a connection, more markets get more connections; a topic it does not list is dropped silently), watchlist prices by one `SUBSCRIBE` of `<symbol>@ticker` for the first 200 markets, the rest priced from REST (its `/ticker/24hr` ignores a `symbols` list and answers all 640 markets, 230 KB at weight 40, so prices are asked every ten seconds). Sockets wait 10 minutes of silence (a stock perpetual sleeps outside its session). An unknown symbol is an HTTP 400 with `{"code":-1121,"msg":"Invalid symbol."}`, read through `net::http::get_text_reading_refusals` and shown as "Aster does not list …" (`ProviderError::UnknownMarket`). Spot (`sapi.asterdex.com`) is not served. Held and tail-refreshed when folded (`cache::policy`) |
  | Lighter | 1m 5m 15m 30m 1h 4h 12h 1D | Perpetuals only: active `perp` rows of `/orderBooks` (coins and the stock, ETF, index, currency and commodity perpetuals listed beside them: `AAPL`, `US500`, `EURUSD`, `XAU`; 210 on 2026-11-21; its `SPX` is the SPX6900 coin, the S&P 500 is `US500`). Public API at `mainnet.zklighter.elliot.ai`, no key. A market is named by its symbol (`PairStyle::Listed`, no quote added, shown upper case, looked up case-insensitively) and asked for by the number the cached list gives it (`catalog.rs`: `resolve`, which answers "Lighter does not list X" for a name it has not, after downloading the list again if it was held for a while: `Catalog::get_or_refresh_on_miss`, at most once in ten minutes; ids from 0, spot from 2048 and perpetual numbers up to 11649, so they are not positions). `/candles?market_id&resolution&start_timestamp&end_timestamp&count_back` always answers the newest 500 bars (`count_back` is required and changes nothing, nor does `start_timestamp`, so 0 is sent), and keeps the bars that open before the end's whole second: `end_timestamp` is rounded up to the second and the next page is asked with the oldest open as its end. A page before a market's first bar is an empty answer, not an error. Verified against the API on 2026-11-21: all eight resolutions open where this app cuts (500 bars of each, none off the grid, none missing; 1D at 00:00 UTC), the rest (`3m 2h 6h 8h 3d 1w 1M`) are `invalid param`, so those, weeks, months and years fold. BTC reaches back to 2025-01-17 (674 days). Volume is the base asset (`v`; `V` is the quote). **About one request a second per IP** (measured: a burst of about 60 is let through, then HTTP 429 with `{"code":23000}` until the allowance, refilling at about one a second, has recovered; 80 requests at exactly one a second all passed), so every REST call waits its slot on `net::rate_limit::lighter` (one second), a chart refreshes every 15 s (`Profile::slow_refresh`), a chart takes at most 10 pages in one request, and prices are one `/exchangeStats` call (every market, 85 KB, `last_trade_price`, `daily_price_change` in percent over a rolling 24 h, checked against the hourly bars, `daily_quote_token_volume` in dollars) every ten seconds. CloudFront caches an identical request for about 10 s, which is why the forming bar comes from the socket. Socket `wss://mainnet.zklighter.elliot.ai/stream`: `candle/<id>/<resolution>` (the snapshot, then an update twice a second; a bar names its market only by its channel number, so a message is turned into its symbol with the list held in memory) and `market_stats/<id>` (about once a second); the list-keyed `market_stats/all` pushes every market's stats each second and is not used. A connection that sends nothing is closed after two minutes (code 1000, "i/o timeout"); a text `{"type":"ping"}` is answered `{"type":"pong"}` and keeps it open (`Keepalive::Text`). **A channel Lighter does not know (an unlisted market number, an unserved resolution, an unknown channel) is answered `{"error":{"code":30005,…}}` and the connection stays**, so all of a chart's markets, and all of a watchlist's, share one connection (224 subscriptions were accepted on one). The candle session is built on the UI thread from the list **held in memory** (`Catalog::held`, never a download): while the first chart is still fetching the list there is no session and the feed is asked for again, and a market the list lacks is left out. The quote session is built on the quote thread and may download it; when it cannot, REST prices alone remain. Held and tail-refreshed (`cache::policy`, every series: a refresh is one request of 500 bars either way) |
  | Deribit | 1m 3m 5m 10m 15m 30m 1h 2h | Perpetuals only: active rows of `get_instruments?kind=future` with `settlement_period` `perpetual` (154 on 2026-11-21: the inverse `BTC-PERPETUAL` and `ETH-PERPETUAL`, and ~150 linear `<ASSET>_USDC-PERPETUAL`, which include stock, ETF and index perpetuals such as `SPY_USDC-PERPETUAL`, `QQQ_USDC-PERPETUAL`, `GOLD_USDC-PERPETUAL`). Public API at `www.deribit.com/api/v2/public`, no key, JSON-RPC answers. Names are the id of every call and are **upper case only** (`btc-perpetual` is HTTP 400): `PairStyle::Listed` keeps them as written, the requests send them upper-cased and refuse text that is not letters, digits, `-` and `_` before asking; the asset for artwork is the part before the first `_` or `-`. `get_tradingview_chart_data?instrument_name&resolution&start_timestamp&end_timestamp` answers the bars that open inside the window (**both ends inclusive**), the newest 5,001 when it holds more, and **a bar for every bucket** (a market nobody traded has flat zero-volume bars, so a short page is the start of the market); a page is the window ending a millisecond before the oldest bar, started as many lengths before as bars are wanted, so a refresh is a few bars and not 5,001. Verified against the API on 2026-11-21: of its resolutions `1 3 5 10 15 30 60 120` open where this app cuts (5,000 bars of each), **`180` and `360` open at 02:00 UTC and `720` and `1D` at 08:00 UTC** (off grid, so left out), and `240`, `1W`, `1M` are `unsupported resolution`: days, weeks and months fold from the two-hour bars, which nest in all of them (verified: its 12-hour bar of 08:00 is exactly the six two-hour bars from 08:00, open, high, low, close and volume). A chart reaches back at most ten pages (`Paging::max_candles`, 50,000 base bars, 500 weeks), then the chart scrolls to older ones. BTC-PERPETUAL reaches back to 2018-08-14. **Volume is the base asset** (`volume`: coins for the inverse and the linear alike, equal to `cost` divided by the price; `cost` is dollars and is not used); for a watchlist the day's turnover is `stats.volume_usd`, dollars. The allowance is credits, not documented in a number: measured, a burst of about 45 requests is let through, then HTTP 429 (JSON-RPC code 10028, read as the rate limit however it arrives) until they refill, and 30 requests a second for 20 seconds were all let through; a chart pages one request after another, so there is no lane. An HTTP 400 on a chart request is an instrument it does not list (the body is not readable through `net::http`), answered "Deribit does not list X". Prices: `get_book_summary_by_currency` (it refuses `currency=any`) is one call per settlement currency (`BTC`, `ETH`, `USDC`: the coin of an inverse perpetual, the quote of a linear one, read from the name by `markets::settlement_of`), 120 KB for USDC: `last`, `price_change` (percent over a rolling 24 h, as the ticker's) and `volume_usd`, every ten seconds; an HTTP 400 there is a currency it does not have, so its markets are rejected. Socket `wss://www.deribit.com/ws/api/v2`: `chart.trades.<name>.<resolution>` (the forming bar, pushed with each trade and answered once when subscribed; **a running total**, `tick` is the open) and `ticker.<name>.agg2` (about once a second; `100ms` also works unauthenticated, `raw` does not); a `public/subscribe` naming an instrument or resolution it does not know is answered with the channels it took and the rest work, so all of a feed's markets share one connection (918 channels were subscribed on one, in requests of 100 as `socket.rs` sends them: one request of 918 closes the connection, one of 750 is accepted). Kept alive by `Keepalive::JsonRpcHeartbeat`: `public/set_heartbeat` (30 s) after each connect, a `heartbeat` and a `test_request` every 30 s answered `public/test` (measured; a connection that answered them was still up after 72 s). Held and tail-refreshed when folded (`cache::policy`) |
  | HTX | 1m 5m 15m 30m 1h 4h | Spot only (formerly Huobi): the `online` rows of `/v2/settings/common/symbols` (616 on 2026-11-21 of about 2,200 listed, 1 MB, cached daily): `sc` is the name (`btcusdt`), `bc` and `qc` its assets; quotes `USDT USDC USDD KGST ETH BTC` (`HTX_QUOTES`; `husd` and `ht` pairs are offline). Public API at `api.huobi.pro`, no key. **Names are lower case on every call** (`BTCUSDT` is `invalid symbol`): the app keeps the upper-case spelling the other exchanges use, `symbol_id` lowers it and refuses text that is not letters and digits before it reaches a URL or a message. `/market/history/kline?symbol&period&size` answers the newest 1 to 2,000 candles (0 and 2,001 are `invalid-parameter`) and **ignores `from` and `to`**, so REST reaches back 33 hours of 1m, 7 days of 5m, 83 days of 1h and 333 days of 4h. Older history is asked over the socket: `{"req":"market.<name>.kline.<period>","id":…,"from":<s>,"to":<s>}` answers the candles that open in the window (both ends included, oldest first), **at most 900 lengths** (905 is `invalid from to`), back to a market's first candle (BTCUSDT's 4h to 2017-10-26: 19,879 bars). `history.rs` asks REST while the candles wanted are among the newest 2,000 (a chart's first answer and its refreshes cost no connection) and otherwise a `net::ws::Conversation`, opened on the first page that needs it and kept for the rest of the fetch, in windows of 900; a connection that fails is dropped and the next window opens another. **HTX fills the gaps of a market nobody traded with flat candles** (verified: a market with one trade has 2,000 consecutive 1m bars), so a short answer is the start of the market, which `Paging` reads it as; `PAGING` is 2,000 a page and 30,000 base candles a request, which is all of BTCUSDT's 4h, days, weeks and months in one chart request, 3.4 years of 1h, 312 days of 15m and 20 days of 1m; the chart scrolls to older ones, to the market's start. **A `req` is limited per connection** (measured: a burst of 300 gets 18 answers and then `{"status":"error","err-code":"bad-request","err-msg":"429 too many request topic is …"}`; 20 a second for 6 s all passed, 30 a second lost about a sixth of them and 50 a second half): a fetch asks one at a time, and that error is the rate limit, read by the status number its message starts with (`parse::status_prefix`), the only thing that tells it from `invalid from to`. **Its `1day` opens at 16:00 UTC, its `1week` on Sunday at 16:00 UTC and its `1mon` and `1year` at 16:00 UTC on the last day before** (UTC+8 midnight), none where this app cuts, so days, weeks, months and years fold from the four-hour bars (verified: its day of 16:00 UTC is exactly the six 4h bars from 16:00, volume included); the six natives open on this app's grid (2,000 bars of each). REST failures are HTTP 200 with `status: error` (`invalid-parameter` / `invalid symbol` for a market it does not list, answered "HTX: invalid symbol"); HTTP 403 is read as a country it does not serve (HTX is documented as unavailable to US users; not observed from where this was built). 800 requests a second per IP are allowed: no lane. Volume is the base asset (`amount`; `vol` is the quote). A watchlist price is `/market/tickers` (every market, 124 KB, every ten seconds): `close`, `amount`, `vol` (a dollar-quoted pair's turnover, else units). **Its `open` is where the 16:00 UTC day opened (59,481.46 against the 61,895.78 that `/market/detail` and the socket give for the rolling day, measured 2026-11-21), so REST reports no change** and the engine keeps the socket's reference. Socket `wss://api.huobi.pro/ws`, **every message a gzip binary frame** (`Decode::Gzip`): `market.<name>.kline.<period>` (the forming bar, pushed with each trade, `amount` a running total, `tick.id` the open) and `market.<name>.ticker` (up to every 100 ms while a market trades: `open` the price 24 hours ago, `lastPrice`, `amount`, `vol`); `market.tickers` (every market, once a second, ~50 KB) is not used. A `sub` of an unknown market is answered `{"status":"error","err-code":"bad-request","err-msg":"invalid symbol nopexyz"}` and the connection and its other subscriptions stay, so every market of a chart, and of a watchlist, share one connection (616 subscriptions were accepted on one). The server pings every five seconds with `{"ping":ts}`, answered `{"pong":ts}` by `Keepalive::EchoJson` (a client that misses two is dropped; a connection answering them was still up after 65 s). Held and tail-refreshed when folded (`cache::policy`) |
  | Bitfinex | 1m 5m 15m 30m 1h 3h 6h 12h 1D 1W 14D 1M | Spot only: the pairs of `/v2/conf/pub:list:pair:exchange` (178 real ones on 2026-11-21, cached daily; its paper-trading `TEST…` markets are left out). **Names are the pair as Bitfinex lists it, without the `t` every API puts in front** (`tBTCUSD` is `BTCUSD`): run together (`BTCUSD`, `ADAUST`) until an asset has more than three letters, then split by a colon (`AVAX:USD`, `BTC:XAUT`, `ETH2X:USD`). `PairStyle::Concat` carries that as `long_pairs` (`LongPairs`): `split` finds the quote by its suffix and drops the colon, `spelled` puts it back where an asset is long, whichever way a market was typed, so `api_symbol`, the watchlist import (`BITFINEX:AVAX:USD`: the first colon ends the venue, `PairStyle::allows_colon`) and the icon base (`base_asset`) read one rule; quotes `USD UST EUR GBP BTC ETH XAUT` (`UST` is Tether: a search for `USDT` finds those pairs). Public API at `api-pub.bitfinex.com/v2`, no key. `/candles/trade:<period>:t<PAIR>/hist?limit&end` answers up to 10,000 bars (10,001 is HTTP 500 `["error",10020,"limit: invalid"]`), **newest first** and as `[MTS, OPEN, CLOSE, HIGH, LOW, VOLUME]` (**the close comes before the high**), paged back with `end` (inclusive, so the page before a candle is `end = open - 1`; `service::market::paging`, at most three pages = 30,000 base candles per request); history reaches 2013. Every native opens where this app cuts (verified 2026-11-21 from 2020-01-01: 3h, 6h and 12h from UTC midnight, 1D at 00:00 UTC, 1W on Monday, 14D on days that are a multiple of 14 since 1970, 1M on the first); the others fold. **Rate limit** (`net::rate_limit::bitfinex`, 2.5 s a request, on a worker lane of its own, `work_queue::BITFINEX_LANE`, like Kraken's: the sleep must not hold the general workers; REST prices and the market list do not wait): measured on 2026-11-21, 80 candle requests two seconds apart were all answered, but the 34th of a run at one a second was **HTTP 429 `["error",11010,"ratelimit: error"]` with `Retry-After` of about 284 s**, and every request in those five minutes got the same. A 429 is not retried (`Retry::Never`) and comes back as `ProviderError::Network(Status(429))`, which `cache::fetch` and the chart card read as a rate limit. `[]` was **not** seen for a valid request while limited (it is what an unknown name, a range before a market's first bar and a market with no candle of the length get), and the `["error",11010,…]` body is read as the same rate limit as the HTTP status (`parse::refusal`); `rest::page_before_with` does not take an empty page for the start of a market's history until the market's newest candle has been asked for (`limit=1&end=<now>`, an `end` no earlier request used, so no cache between can answer it) and answered: if it comes back, the history ended; if it is empty too, the market is either not in the list (`Bitfinex does not list X`) or has no candle of this length, which ends the history. A limit is only what a request itself says, never an empty answer. Charts and overlays refresh on the slow schedule (the socket keeps them live) and a held series is patched by one small request (`cache::policy`, `CachedSeries::Every`). Volume is the base asset. **Sockets** `wss://api-pub.bitfinex.com/ws/2` (JSON): `{"event":"subscribe","channel":"candles","key":"trade:1m:tBTCUSD"}` and `{"event":"subscribe","channel":"ticker","symbol":"tBTCUSD"}`; a `subscribed` event gives the **channel number**, and every push is `[chanId, payload]` (snapshot of the newest 240 candles, then updates of the forming candle and the one before it, which are resent), so the connection's protocol is a `FrameReader` / `TickReader` (`CandleProtocol::reader`), a `Channels` map made on the feed's thread and `reset` each time the link comes up. Heartbeats `[chanId,"hb"]` every 15 s are data to the keepalive (a minute without any is a dead link); a quiet socket is probed with `{"event":"ping"}` (`Keepalive::Text`, answer `{"event":"pong"…`). An unknown market in a **candle** subscription is an empty snapshot, in a **ticker** one an `error` event (`symbol: invalid`, 10300); `subscribe: dup` (10301) and `subscribe: limit` (10305) are refusals of that subscription alone and the connection stays. A connection took 33 to 56 subscriptions before it refused (measured, varying), so a feed plans for the 30 of the documentation (`CHANNELS_PER_CONNECTION`: `LiveFeed` opens a connection per 30 markets; a ticker socket subscribes the first 30 and REST prices the rest, which the engine falls back to). Quotes `/v2/tickers?symbols=tBTCUSD,…` (one call, an unknown market is left out): `DAILY_CHANGE` is a rolling 24 hours (its implied open differs from the 1D candle's), so the price a day ago is `LAST_PRICE - DAILY_CHANGE`; `VOLUME` is the base asset; 60 calls one a second were all answered. `{"event":"info","code":20051}` (the server restarts) and `20061` (maintenance is over, after which Bitfinex advises subscribing again) are answered with a new connection (`Channels::take_reconnect`, read by both readers); `20060` (maintenance begins: pause) is left alone, as a client that sends nothing pauses already |
  | Bitstamp | 1m 3m 5m 15m 30m 1h 2h 4h 6h 12h 1D 3D | Spot only: the `SPOT` rows of `/api/v2/markets/` whose `trading` and `matching` are `Enabled` (251 of 272 rows on 2026-11-21: 20 are perpetuals, one has matching disabled; cached daily). **Names are lower case on every call** (`btcusd`; `BTCUSD` is HTTP 404): the app keeps the upper-case spelling, `market_id` lowers it and refuses text that is not letters and digits before it reaches a URL; quotes `USD EUR USDT USDC GBP BTC RLUSD EURC EURCV ETH` (`BITSTAMP_QUOTES`, longest first; `SUSD` is Sonic and `WEUR` Wormhole, a one-letter base before the first eight). Public API at `www.bitstamp.net/api/v2`, no key. `/ohlc/<market>/?step&limit&end`: **`limit` is required**, 1 to 1,000 (none, 0 and 1,001 are HTTP 400 `validation-error`, whose body `net::http` does not hand over), every number is a string, rows come oldest first, `end` is in seconds (milliseconds are 400) and **inclusive** (a bar opening at `end` is in), so the page before a bar is `end = open - 1`. **`start` is no lower bound once `end` is given** (the newest `limit` bars up to `end` come anyway), so it is not sent and `Paging` does the filtering. **Bitstamp fills the minutes nobody traded in with flat zero-volume bars** (a quiet market's 1,000 newest 1m bars were all flat), so a short page is the start of the market (BTCUSD's 1D begins 2011-08-18, 5,575 bars) and the page before it is empty. Every step opens where this app cuts (verified 2026-11-21: all twelve on multiples of their length from the epoch, which for the intraday ones that divide a day is UTC midnight): its `259200` is on the grid (day number divisible by three, unlike Binance's `3d`) and is exactly the fold of three 1D bars (open, high, low, close and volume of eight bars equal), so `3D` is native. `604800` and every other length are `Not a valid choice`: weeks, months and years fold from days, `8h` from 4h. An unknown market is HTTP 404 with an empty body, read as "Bitstamp does not list X". 30 pages of 1,000 (`PAGING`) are 30,000 base candles in one request. **Rate** documented as 400 a second and 10,000 per 10 minutes; 160 requests from 40 threads in two seconds were all answered, so there is no lane. **No candle or ticker channel**: socket `wss://ws.bitstamp.net`, `{"event":"bts:subscribe","data":{"channel":"live_trades_<market>"}}` (`ohlc_btcusd` is `bts:error` "Invalid channel provided."; an unknown market is acknowledged and never trades; all 252 spot markets were subscribed on one connection, so a feed shares one). A trade is `price`, `amount` (base) and `microtimestamp` (string, µs). **The live candle is built from trades** (`live.rs`: `Trades` is a `FrameReader` holding a `model::market::live::TradeFold` per market, `reset` on every link up), and the closed one-minute bar it built of BTCUSD on 2026-11-21 had the open, high, low, close and volume (to 1e-8) of the bar Bitstamp's REST made of the same minute (`live_bitstamp_serves_...`); the chart adds those trades to the bar it holds, so the bar open at connect keeps its open and range (`TradeFold`, `LiveFold::apply`). **Prices**: the last trade is the price (`quotes.rs`, `QuoteProtocol::parse`), and `/ticker/` (every market in one call, 80 KB, perpetuals included: `market_type`; `/ticker/<pair>/` of an unknown pair answers the same list) gives `open_24`, the price 24 hours ago (**`open` is where the UTC day opened**, not used; `0` for a market with no trade in 24 hours, which then has no change) and `volume`, the base asset of the 24 hours. A trade says neither, so `QuoteProtocol::ticks_carry_reference` is `false`: the quote engine asks REST for the reference of such a market (once, then at half of `stream_reference_ms`), keeps it between calls and lends it, and the last volume, to the ticks. Keepalive: the server sends nothing of its own and kept a quiet connection (one illiquid market, nothing sent by the client) open for the 15 minutes it was watched; the client's `{"event":"bts:heartbeat"}` is answered `{"event":"bts:heartbeat","channel":"","data":{"status":"success"}}` (`Keepalive::QuietText`; the answer is no data but counts as hearing Bitstamp, so a quiet market is not reconnected for want of a trade). `bts:request_reconnect` (a restart, after which the server closes the connection) was never seen; both readers answer it with a new connection (`FrameReader::reconnect`, `TickReader::reconnect`). Folded series are held and tail-refreshed (`cache::policy`) |
  | BingX | 1m 3m 5m 15m 30m 1h 2h 4h 8h | Spot only: the `status` 1 rows of `/openApi/spot/v1/common/symbols` (634 of 2,304 on 2026-11-21: 0 is delisted, 5, 10 and 25 are not trading; 680 KB, cached daily): ids `BTC-USDT` (dash pair style, `PairStyle::Separated`; a trading market whose name has a second dash is left out), `tickSize` (always a power of ten) gives the digits of its prices. Public API at `open-api.bingx.com`, no key; every answer is HTTP 200 `{"code":…,"msg":…}` (`parse::answer`): `100204` is nothing for the request (no candles in that range, the range is past what the endpoint keeps, or `symbol is not found`: one code, told apart by text only, so a candle answer of it is an empty page and a market is looked up in the list before it is asked for), `100400` a refused parameter, and a rate limit is `100410` (BingX's documented code, never provoked while building this: the app asked for no more than the headers allow) or HTTP 429, both `ProviderError::is_rate_limited`. **Two candle endpoints, one row shape** (`[open_ms, open, high, low, close, volume, close_ms, quote_volume]`, newest first, `endTime` inclusive, volume the base asset), served by `history.rs` by the age of the page's end: `/openApi/spot/v2/market/kline` (recent) answers the forming candle and a window of the present (`maximum query range` for an `endTime` older than 7 days at `1m`, 63 at `3m`, 105 at `5m`, 315 at `15m`, 630 at `30m`, 1,260 at `1h`, 2,520 at `2h`; `limit` above 1,000 is clamped to 1,000, or 1,440 at `1m`), and `/openApi/market/his/v1/kline` (history) reaches a market's first candle but its newest candle is 24 hours old (`no data found` for an `endTime` newer than that; measured 24.2 h at `1m`, 23.5 h at `1h` on 2026-11-21, twice an hour apart), refuses `limit` above 1,000 as `100400`, and holds `1m` to `15m` only from 2025-01-01 (an `endTime` older than 689 days is `exceeds the valid period`). A page that ends in the last 36 hours is the recent endpoint's, an older one the history's, so a chart pages from now to a market's first candle (BTC-USDT's `1h`: 2024-02-24) with no gap, and the two are one bar where both have it (`live_bingx_serves_…` compares 600 one-minute bars of each). **`limit` counts slots, not rows**: BingX has no candle for the minutes its own service was down (2025-11-25 04:03 to 04:12 UTC, in every market: a `3m` page across it had 997 rows), so a short page is not the start of the market. A page that is short asks once more for the whole page from before its oldest candle (at most four requests a page), and the page that is empty is the start. A market nobody trades in is filled with flat candles. **The history's prices have a digit past the tick** (`64655.599`, `61702.823000000004` for BTC-USDT's tick 0.01) and so does the candle socket's; the recent endpoint's are that number with its decimal text cut at the tick, **never rounded** (4,000 prices of each of five markets: cut matched every one, rounding half), and a double a hair under its tick is cut a tick lower (`64441.509999999995` is `64441.50`). `precision::cut` does exactly that on the text of the number (so `serde_json` is built with `float_roundtrip`, which parses the text of a number to the double it names), from the market's `tickSize`, on every history page and every socket push (the reader takes the digits from the list the chart's candle request holds in memory, `Catalog::held`, once and again only while it has none: a socket thread never downloads it), so every endpoint draws one bar; the volume is untouched. **Its `6h` and `12h` open at 04:00 UTC, its `1d` and `3d` at 16:00 UTC, its `1w` on a Sunday at 16:00 UTC and its `1M` at 16:00 UTC on the last day before** (UTC+8 midnight; checked on both endpoints), none where this app cuts, so those, years and every other length fold from the nine natives, which open on the grid (`1m` to `8h`, every bar on a multiple of its length from the epoch). The recent endpoint allows 500 requests every 10 seconds and the history endpoint another 500 (`x-ratelimit-requests-remain` counts down, `-expire` is 10000 ms): a chart's 30 pages are inside it, so there is no lane. A watchlist price is `/openApi/spot/v1/ticker/24hr` (every market, 645 rows, 206 KB, every ten seconds; one `symbol` works, several joined by commas are `symbol is not found`): `lastPrice`, `openPrice` (the price 24 hours ago, the window is rolling) and `volume` (base) or `quoteVolume` (shown for a dollar-quoted pair). Socket `wss://open-api-ws.bingx.com/market`, **every message a gzip binary frame** (`Decode::Gzip`): `{"id":…,"reqType":"sub","dataType":"BTC-USDT@kline_1min"}` (**the socket's length names are not REST's**: `1min 3min 5min 15min 30min 60min 2hour 4hour 8hour`; `1hour`, `1h`, `1d` and `1M` are refused, `socket::token`) pushes the forming candle about twice a second (`K.t` open, `v` base volume running total) and `BTC-USDT@ticker` once a second (`o` the price 24 hours ago, `c` the last, `v`, `q`). A `sub` is acknowledged `{"code":0,"msg":"SUCCESS"}` and one of an unknown market or a refused length is `{"code":100400,"msg":"dataType is error: …"}` while the connection and the other topics stay. **A connection takes 200 topics** (the 201st on is `100416`, "one websocket can subscribe to a maximum of 200 data points"; candles and prices are separate sockets): a chart shards its markets over connections of 200 (`socket::sessions`) and the price socket takes the first 200 of a watchlist, REST fills the rest. **The server pings every 30 seconds with JSON, not the text `Ping`**: `{"ping":"<id>","time":"<UTC+8 time>"}` (`Keepalive::AnswerJson`), answered `Pong`. It dropped neither a client that answered nothing (8 pings, 4 minutes) nor one that answered `Pong`, so which answer it wants is unverified; a connection answering them was still up after 65 s (`live_bingx_pings_…`). BingX restricts accounts in the US; its public data answered from where this was built and a country refusal (HTTP 403, as the other venues read it) was not observed. Held and tail-refreshed when folded (`cache::policy`) |
  | Gate.io | 1m 5m 15m 30m 1h 4h 8h 1D 1W 1M | Public v4 API, no key, spot only; ids are `BTC_USDT`. Natives are its documented intervals `1m … 8h`, `1d`, `7d` (weeks from Monday) and `30d` (the calendar month): all cut where this app cuts, so 3M and 1Y fold from months (it also answers `3m`, `2h`, `6h` and `12h`, undocumented, so they fold). 1,000 per request, paged back with a `to` cursor through `service::market::paging`. **A request may not reach 10,000 points back** in the interval asked for (HTTP 400 "Candlestick too long ago"): the cap is 8,900 base candles (`Paging::max_candles`, capped before the fold) and `reach_left` answers an older page past it as empty, so a chart does not retry. 200 requests per 10 s per IP: no lane. A 400 is an unknown market, a 403 a country it does not serve. Held and tail-refreshed when folded (`cache::policy`) |
  | Bybit | 1m 3m 5m 15m 30m 1h 2h 4h 6h 12h 1D 1W 1M | Public v5 API, no key, spot only; symbols are Binance-style `BTCUSDT`. Every bar is cut where this app cuts (weeks Monday, months calendar), so 3M and 1Y fold from months. 1,000 per request, paged back with an `end` cursor through `service::market::paging` (60 pages, capped before the fold). Refusals are HTTP 200 with a `retCode` (`10006`/`10018` are rate limits) and some countries get HTTP 403. 600 requests per 5 s per IP: no lane. Held and tail-refreshed when folded (`cache::policy`) |
  | Kraken | 1m 5m 15m 30m 1h 4h 1D | Public API, no key. `OHLC` answers only the newest 720 candles and cannot page back past them (`KRAKEN_BARS`): 12 hours of 1m, 30 days of 1h, 2 years of 1D. The chart's first 500 leave the rest for its first older page (`supports_older_history` is true, `window`), and the next page is empty; a full window drops the fragment a fold of it would start with. Its week (`10080`) starts on a Thursday and its `21600` is 15 days counted from 1970, so 1W, 1M and 1Y fold from days. Ids are `BTC/USD` on REST and websocket alike (`XBT`/`XDG` are refused: `kraken_asset`). One request every six seconds (`net::rate_limit::kraken`: measured, a burst of about nine gets through and then only about one in six seconds; faster is `EGeneral:Too many requests`), so charts and overlays refresh on the slow schedule (the socket keeps them live) on a worker lane of its own (`work_queue::KRAKEN_LANE`: the sleep must not hold the general workers; the quote adapter does not wait, since a round joins every source); refusals are HTTP 200 with an `error` code (`is_rate_limit`) |
  | Coinbase | 1m 5m 15m 1h 6h 1D | `MAX_PAGES` x 290 candles caps a request (a 5m series reaches back ~60 days). Paging is by time window, so an answer that stops at the cap with fewer candles than asked (an illiquid product) begins at the window's start, and a first bucket that opened before it is a fragment (`FoldPlan::shape_since`); an empty page is the start of the product's history, and the count rule alone applies |
  | Alpaca | 1m 5m 15m 30m 1h 1D 1W 1M | Alpaca also has `3Hour`, `4Min`, `2Month` and so on, but it does not say where those start; only lengths that divide the hour are used (an hour's start is the same in UTC and New York), the rest fold. Days, weeks and months keep Alpaca's New York-midnight stamps; a fold re-anchors them on UTC. Stocks fold intraday buckets from UTC midnight, not the session open. 10,000 bars per request |
  | DEXScreener (GeckoTerminal) | 1m 5m 15m 1h 4h 12h 1D | 1,000 per request; empty buckets are filled flat (`bucket::fill_gaps`) before a fold |
  | CoinGecko | none: prices, `model::market::samples` | `/market_chart` samples are 5 minutes for 1 day, hourly to 90 days, daily to 365; a candle is built from samples so the spacing must divide the candle: `chart_days(timeframe, limit)` is the days to ask for, or `None` (1m, and any length that is not a whole number of five minutes: 7m, 22m) |
  | Polymarket, Kalshi | lines: `model::market::line` | any timeframe: a point per timeframe (a week at most) over 60 candles, between a month and a year, at most 1,000 points (a 1m line reaches back 16 hours). Polymarket: the shortest `prices-history` window that holds the span and allows `fidelity` = the point spacing (`1w` allows 5 minutes, `1m` 10, `max` is the whole history only from 12 hours; where none holds the span and allows the fidelity the line is shorter). Kalshi: the longest of 1, 60 and 1,440 minutes that is no longer than a point, over at most 1,000 of it |
  | CoinMarketCap | none | the four gauges ignore the timeframe. `TOTAL`, `TOTAL2`, `TOTAL3` and `BTC.D` are price samples folded like CoinGecko's (`samples::fold_samples`), see **CoinMarketCap market-cap indices** below: 5-minute readings for any length that is a whole number of five minutes (5m, 15m, 90m), hourly for whole hours (1h, 4h), daily for whole days (1D, 1W, 1M, 1Y); 1m and 7m, 22m, ... answer `UnsupportedInterval` |

- **CoinMarketCap market-cap indices** (`service/market/cmc/market_caps/`, `model::market::cmc::{global,
  interval}`): `TOTAL`, `TOTAL2` (`altcoinMarketCap`, checked equal to total x (1 - BTC.D/100) within
  1e-5), `TOTAL3` (total x (1 - (BTC.D + ETH.D)/100)) and `BTC.D`, on the existing
  `DataSource::CoinMarketCap` and routed by symbol inside the CMC venue (`GlobalIndex::from_symbol`);
  they never read the Pro API key. Source: `https://api.coinmarketcap.com/data-api/v3/global-metrics/quotes/historical?format=chart_altcoin&interval={5m|1h|1d}&timeStart&timeEnd`
  (seconds, both ends inclusive), the website's own API: **unofficial and undocumented, it can break
  or go away**, so the chart says "CoinMarketCap website data (unofficial)" (`chart_feed_label`).
  No key; 403 without any `User-Agent` and 200 with any, so the shared agent's is enough. A reading
  has `timestamp`, `btcDominance`, `ethDominance` (0 before Ether) and `quote[0].totalMarketCap` /
  `altcoinMarketCap`; 5-minute readings sit on the 5-minute grid, hourly ones at :10, daily ones at
  10:10 UTC (08:10 when first measured, 10:10 on 2026-11-18 to 20: the time of day moves; 00:00 before
  about 2020; a day bucket either way). **A coarser reading trails the clock by up to its spacing**
  (today has no daily reading until 10:10 UTC, and its candle then sits still until the next day's),
  so a 1h or 1d chart appends the newest 5m readings after the last (`market_caps::with_newest`: the
  last half hour, one request kept 60 s under `cmc_global_newest`, a failed one adds nothing). History: 1d from 2013-04-29, 5m and
  1h are as deep as the windows ask. **At most 2,200 readings per request** (`status.error_code`
  `"500"`, "Too many items requested"), so requests sit on a fixed window grid of 2,000 readings
  (`Interval::windows`, at most four per chart). **Errors arrive as HTTP 200** with
  `status.error_code != "0"`: `"500"` ("The system is busy", also a refused range) is
  `ProviderError::CoinMarketCap(Status(500))`, `"429"` a rate limit, any other code its own message.
  Units: market caps in billions of dollars and the dominance in percent, plain numbers (no chart
  carries a unit; `PriceFormat` prints `2,411` and `58.49`), the symbol's title says "USD bn" / "%".
  A window that is over and settled is kept for good in `cache::lookup` (namespaces `cmc_global_*`,
  cleared with the rest by Settings ▸ Storage), the one still filling is used for 1 minute (5m), 5
  (1h) or 30 (1d) and a failed request falls back on what was kept. Its own rate lane
  (`rate_limit::cmc_global`, one request a second; twelve in a row were all answered on 2026-11-21).
  No older-history paging (`OlderHistory::Never`): a chart asks for as many candles as it shows. The
  profile's `slow_refresh` is on for the whole source.

- **Alpaca feeds** (`model::market::feed`): bars of a day and longer come from SIP (consolidated
  volume, 15 minutes late on the Basic plan, which the app cannot tell from a paid one), intraday
  bars from real-time IEX (a single venue: 2-4% of the volume). A SIP request whose `end` falls inside
  the delay is refused on Basic, so `end` is left out there (`bars_window`): Alpaca then applies the
  account's entitlement, and a caller's range is trimmed by the provider. Never cap `end` to the
  delay, which holds a paid account behind. A SIP `start` inside the delay is refused on Basic too
  (403, with or without `end`, measured on `1Day`), so `bars_window` makes no request: no bar of a
  day or longer opens that recently on any plan. Charts name their feed (`DataSource::chart_feed_label`);
  `display_name()` stays `Alpaca (IEX)` because portfolio and paper keys are built from it.
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
  any length; Hyperliquid and Kraken to the newest ~5,000 and 720 candles they keep. GeckoTerminal's free API reaches back 180 days, so it pages (one page of base candles,
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
- One `service::market::live::LiveFeed` per workspace and streamed source (`Profile::streams.candles`;
  each protocol is a `CandleProtocol` in `<venue>/live.rs` that builds a `net::ws::Session` and parses its messages): updates the forming candle, reconnects with backoff. MEXC is the sixth: its spot socket speaks Protocol Buffers only (the JSON topics are refused), read by `service::net::protobuf` (a field iterator, no dependency; `ws::Frame::Binary` carries the frames), with `spot@public.kline.v3.api.pb@<SYMBOL>@Min1` topics, at most 30 per connection (so `LiveFeed` runs one connection per 30 markets), one topic per subscribe request and `{"method":"PING"}` as keepalive. Gate.io is the fifth (`spot.candlesticks` and `spot.tickers` on `wss://api.gateio.ws/ws/v4/`: one subscribe request per market, since a request that names a market it does not know is refused whole; protocol pings keep it alive). Bybit is the fourth (`kline.<interval>.<SYMBOL>` on `wss://stream.bybit.com/v5/public/spot`: one subscribe request per topic, because a request takes at most 10 and one unknown symbol refuses all of it; `{"op":"ping"}` keepalive, whose answer `ws::Keepalive::Text` recognises by its start). Kraken is the third (websocket v2 `ohlc` with `snapshot:false`, one subscribe naming every market, kept alive by the server's one-a-second `heartbeat` and protocol pings; its `ticker` channel carries the rolling 24h `change`, which gives the price a day ago). OKX is the second stream (`candle<bar>` channels on `wss://ws.okx.com/ws/v5/business`, one subscribe message naming every market, kept alive with the text `ping`/`pong` through `ws::Keepalive::Text`; its `public` endpoint refuses candles). Binance's streams
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
- **Hyperliquid's sockets** (`wss://api.hyperliquid.xyz/ws`, JSON, `{"method":"ping"}` answered `{"channel":"pong"}`):
  `candle` for the forming candle and `activeAssetCtx` for a watchlist price (a spot pair answers on
  `activeSpotAssetCtx`), one subscription per market. **A `candle` subscription to a name it does not list
  closes the connection with no message** (`activeAssetCtx` gets an `error` message and stays), while an
  IP may hold only 100 connections and a connection 1,000 subscriptions. So the names the held market
  list names (`catalog::warm` reads it when a chart is fetched) share connections of up to 1,000
  (`live::sessions_among`), and every other name (a typo, a delisted market, or any name while no list is
  held) gets one of its own, where it cannot silence its neighbours. Watchlist prices subscribe the first
  1,000 markets on their one connection and REST prices the rest.
- **Session variants** (`net::ws`: `mod.rs` loop and health, `keepalive.rs`, `decode.rs`). Per-session
  data, never a venue loop: `Session::decode` is `Decode::None` (binary frames handed over, MEXC) or
  `Decode::Gzip` (HTX, BingX: unpacked to a text message, capped at 8 MB, an unreadable frame dropped
  without ending the socket), applied before the heartbeat is looked at because BingX gzips its pings
  too. `Keepalive` is `Frame`, `Text` (client ping, answer recognised by its start: OKX and Bybit),
  `QuietText` (the same for a feed whose market can be quiet for hours: Hyperliquid's
  `{"method":"ping"}` / `{"channel":"pong"`, Lighter, Bitstamp), `EchoJson` (server `{"ping":n}`, client
  `{"pong":n}`: HTX), `AnswerJson` (server `{"ping":"<id>","time":…}`, client the text `Pong`: BingX) and `JsonRpcHeartbeat`
  (Deribit: `public/set_heartbeat` is sent after every connect before the subscriptions, a
  `test_request` heartbeat is answered `public/test`; request ids 1000000001 and 1000000002 are
  reserved). Pure `Keepalive::inspect` / `probe` / `greeting` decide; heartbeat traffic and its
  answers are absorbed, never handed to `CandleProtocol::parse`. `Keepalive::heard_is_alive` says
  what `max_data_silence` is counted from: for `Frame` and `Text` (a dense stream: a socket that
  answers but sends nothing is stuck) the last data, for the server-driven variants and `QuietText`
  the last message of any kind, so a market nobody trades is not reconnected every ten minutes for
  want of a trade. A server that asks for a new connection (a restart, maintenance that dropped the
  subscriptions) is answered through `FrameReader::reconnect` / `TickReader::reconnect` after the
  message that asked, which `Session::run_flow` honours (`Flow::Reconnect`; `run` and `run_linked`
  only read or stop).
- **A question over a socket** (`net::ws::conversation`): `Session::converse` connects once, sends the
  keepalive's greeting and the subscriptions, and returns a `Conversation` whose `ask(request,
  patience, answer)` sends one message and returns what `answer` makes of the first message that is the
  reply, the others (pushes, other replies) left unread; the session's keepalive and decoding apply, so
  the server's pings are answered while it waits. For a venue whose history is asked over its socket
  (HTX); a failed conversation is begun again by its caller, never reconnected. A feed's `run` loop
  stays the only listener.
- **Candles from trades** (`model::market::live::TradeFold`): for a venue with no candle channel
  (Bitstamp), trades `(time_ms, price, size)` fold into the forming base candle, cut by `bucket`. A
  trade older than the bucket in progress is dropped, a newer bucket starts fresh. The candle holds
  only the trades seen since the feed began, so it is **not the bar**: `CandleProtocol::klines` says
  `Klines::Trades` and `LiveFold::apply` (given the feed's `Stream`) folds it as an increment on the
  candle the chart holds of that bar, even when the base is the chart's own length: the held open
  stays, the high and low widen, the close is the latest trade and the volume adds what traded after
  the first kline (that kline's trades are taken as already in the held candle). A REST answer
  re-seeds the held candle (`LiveFold::merge_fetched`): the trades seen when it arrived count as in
  it, an answer the socket was quiet for stands whole, close included. The fold lives as long as the
  feed, a reconnect included, because its volume only grows; the trades missed in between are what
  REST's next account of the bar restores. A venue whose
  messages need memory (a channel number that names no market; a `TradeFold` per market) implements
  `CandleProtocol::reader` / `QuoteProtocol::reader` instead of `parse`: one `FrameReader` /
  `TickReader` per connection, made by the feed's thread, told `reset` each time the link comes up
  (so what a dead connection told, such as its channel numbers, is not believed; a reader keeps what is
  no connection's: BingX's price digits, Bitstamp's trades), read through by the one loop.
- A chart whose last answer was `UnsupportedInterval` shows it in the usual error slot and is not
  refreshed again (`Chart::refresh_due`) until its market or the timeframe changes, which fetch at
  once. An overlay market becomes `OverlayStatus::Blocked(text)` and is not retried either.
- `service::cache::patched` merges a tail into a held series by open time: the tail's copy of a candle
  wins and a held candle the tail skips stays. A tail that does not reach back to the series is
  fetched whole.
- `service::cache::fetch`: a failed tail refresh falls back to the whole fetch, except a rate limit,
  which is returned.
- Visible watchlists share one ticker socket per source in `DataSource::quote_streams()` (Coinbase: last
  trade, 24-hour open, volume, it has no candle channel; OKX: its `tickers` channel on `/ws/v5/public`; Kraken: its v2 `ticker`; Bybit: `tickers.<SYMBOL>`; Gate.io: `spot.tickers`, with the price a day ago taken from its 24h change in percent). Kraken's REST ticker has only the UTC day's open (`o`), which is not a price a day ago, so its REST quote reports no change; the socket's rolling `change` gives it, and the engine keeps that reference (`stream_references`, `Config::stream_reference_ms`, 10 minutes) for a REST refresh of a streamed market whose socket went quiet. A market the socket never answered shows no change. The reverse holds for a venue whose socket sends only trades (Bitstamp, `QuoteProtocol::ticks_carry_reference` false): its REST answer's price a day ago is remembered the same way, lent to the ticks that follow, asked again when it is half as old as it may get, and a tick that gives no volume keeps the last one. `watchlist_quotes::quote_stream` holds the `QuoteProtocol` trait and the socket loop; each venue's protocol is `<venue>/quotes.rs`. It holds one
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
