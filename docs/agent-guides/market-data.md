# Market data guidance

## Adding a provider

1. `model::market::source::DataSource` metadata: display name, watchlist slug, prediction-market flag,
   volume support, line/candle mode.
2. Implement `service::MarketSource`; `search_tickers` if supported; a parser for a new kline shape.
3. Register in `service::for_source`. `supports_trending`/`trending` only if it can rank active
   markets (Kalshi cannot).
4. Add to `ui::chart::add::sheet` via `is_prediction_market()`, not provider matching.
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

## Timeframes

- Add the `Timeframe` case, its `Timeframe::interval` token (`1h`, `1d`, `1w`, `1M`, `3M`, `1Y`) and
  display/date helpers. Every provider is asked for that token.
- Missing interval: fold finer bars with `service::fold_calendar` (Binance boundaries: weeks from
  Monday UTC, calendar months/quarters/years; a bar closes 1 ms before the next opens). Never
  substitute a nearby interval. `3M`/`1Y` are real quarters/years everywhere. Binance and Alpaca fold
  monthly bars, Coinbase and GeckoTerminal daily, CoinGecko builds from its price series. Pass
  `truncated` when the series starts where a request stopped, to drop the partial first bucket.
- The chart offers 1H+, but Pine's `request.security` may ask any timeframe (5m intrabars, 4h), so
  token tables must cover minutes or the script fails with `PINE4021`. Coinbase: 1m/5m/15m native,
  folds 30m (from 15m) and 4h (from 1h); pages stop at `MAX_PAGES` (60 x 290 candles), so 5m
  intrabars reach back ~60 days.
- `service::fetch_klines` runs every provider through `service::normalize` (ascending, one candle per
  open time, finite prices, range holds open and close). GeckoTerminal lists only traded intervals;
  `fill_gaps` adds flat zero-volume candles for even spacing. Volume units differ (base asset,
  shares, USD); CoinGecko has none.
- GeckoTerminal charts a pool's base token, the contract sorting first on chain, so a pool can read
  "USDT / MCADE"; DEXScreener (DEX search) lists the traded token first. `geckoterminal::side_for`
  reads the pool's token symbols once, remembers, and asks `token=quote` when the base is a stablecoin
  or wrapped native coin (`QUOTE_ASSETS`) and the quote is not; else base. Without it the chart shows
  the stablecoin at ~$1. Unreadable tokens: base, unremembered.
- Prediction markets use `effective_span_days` and `line_chart_point_count` in `service/mod.rs`.

## Caches and live data

- `service::cache::fetch`: slow providers' candles in memory by `(source, symbol, interval)`
  (`cache::policy` lists them and TTLs); refreshes a held series with only its last `TAIL_LIMIT`
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
- One `BinanceLiveFeed` per workspace: updates the forming candle, reconnects with backoff. A
  current-or-newer WebSocket bar beats an older REST snapshot.
- Visible watchlists share one Coinbase ticker socket (last trade, 24-hour open, volume); Coinbase
  has no candle channel.

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

## Paper-trading quotes

- Paper orders fill from chart prices, no own feed. `sync_paper_quotes` hands each rendered market
  chart's last close to `PaperHost`, panel open or not (closed: only instruments with a working order
  or position). Nothing fills while the app is quit/asleep or no chart for the instrument renders.
- Quote timestamp = candle arrival (`Chart::last_data_ms`), never read time. Prices older than
  `watchlist_quote::maximum_age_secs` are withheld; Alpaca prices outside US hours carry
  `is_market_open = false`, so an old close cannot fill.
- In replay the quote stays live; the ticket's reference follows the replay slice. Unresolved: should
  replay block paper orders?
