# Market data guidance

## Adding a provider

1. Add `model::source::DataSource` metadata: display name, watchlist slug, prediction-market flag,
   volume support, and line/candle mode.
2. Implement `service::MarketSource`; add `search_tickers` when supported and a parser when the
   kline shape is new.
3. Register it in `service::for_source`. Add `supports_trending`/`trending` only when it can rank
   active markets; Kalshi cannot.
4. Add it to `ui::add_chart_sheet`, using `is_prediction_market()` rather than provider matching.
5. Add its `SourceLogo` under `assets/source_logos/`, the `ui::source_logo::source` arm, and
   attribution in `docs/source-logos.md`.

Prediction markets render lines under Predictions and refresh over REST. Kalshi ids are
`"SERIES/MARKET"`; cache its series index in memory and rank locally.

Provider search groups `SearchResult::event` choices as a tree. Selected choices from one event
become one chart: `EventInfo::series` holds the choices, the first choice supplies the symbol and
candles, and `ChartCard::set_series` draws choices by position with end labels.
`PersistedChart::pm_series`/`title` persist them; watchlists do not yet.

## Timeframes

- Add the `Timeframe` case, its `Timeframe::interval` token (`1h`, `1d`, `1w`, `1M`, `3M`, `1Y`), and
  display/date helpers. Every provider is asked for that token.
- If a provider lacks an interval, fold finer bars with `service::fold_calendar` (Binance's
  boundaries: weeks from Monday UTC, calendar months/quarters/years; a bar closes a millisecond
  before the next opens); never substitute a nearby interval. `3M` and `1Y` are real quarters and
  years everywhere. Binance and Alpaca fold monthly bars, Coinbase and GeckoTerminal fold daily
  bars, and CoinGecko builds them from its price series. Pass `truncated` when the series starts
  where a request stopped, so the fragment of its first bucket is dropped.
- `service::fetch_klines` passes every provider through `service::normalize` (ascending, one
  candle per open time, finite prices, a range that holds its open and close). GeckoTerminal lists
  only intervals that traded, so `fill_gaps` adds flat zero-volume candles to keep bars evenly
  spaced. Volume units differ by source (base asset, shares, USD) and CoinGecko has none.
- Prediction markets use `effective_span_days` and `line_chart_point_count` in `service/mod.rs`.

## Caches and live data

- `service::cache::fetch` keeps the slow providers' candles in memory by `(source, symbol,
  interval)` (`cache::policy` lists them and their TTLs), refreshes a held series by fetching only its
  last `TAIL_LIMIT` candles, and makes concurrent requests for one series share one call. Add a
  provider to `policy` only if a whole fetch is slow; its `fetch_klines` must honour a small
  `limit` cheaply. `service::rate_limit` spaces CoinGecko and GeckoTerminal work, and
  `work_queue::submit_for` gives each its own worker; submit chart requests through it.
- Persist regenerable slow data in the OS cache directory, never the data directory, through
  `service::disk_cache`/`lookup_cache`, with TTL, stale-while-revalidate, and stale-on-error.
  Discard corrupt cache files. New caches must participate in `cache_control::clear`; writes use
  `DiskCache::write(..., generation)` so an in-flight write cannot undo a clear. Tests use
  `disk_cache::testing::TempCache`; the global cache is disabled in tests.
- `service::icon` caches by `"<source>:<ticker>"`, never chart id. It tries the static CDN with an
  unthrottled HEAD, then rate-limited CoinGecko search. A miss falls back to a letter monogram.
- Each workspace owns one `BinanceLiveFeed`; it updates the forming candle and reconnects with
  backoff. A current-or-newer WebSocket bar beats an older REST snapshot.
- Visible watchlists share one Coinbase ticker socket for last trade, 24-hour open, and volume.
  Coinbase has no candle channel.

## Watchlists

- `WatchlistStore` is authoritative: one ordered document row per list containing flat instrument
  and section entries. Validate, persist in one transaction, then publish. Do not overwrite a list
  that failed to decode. Favorites are the `isFavorites` list; Swift favorite rows import once.
- Identity is `InstrumentId` (source plus provider id); chain is metadata and ticker text is not an
  identity. CoinMarketCap portfolio cards are not instruments.
- Flags are global by market in `watchlist.flags`. They sync with the Watchlists switch (not
  Preferences) and merge per key; `sync::policy::category_of` / `merge_of` decide per setting key.
  Sorting/filtering is presentation-only and stays within sections through `watchlist_layout`.
- Quotes are transient. `WatchlistQuoteCoordinator` merges demand by `InstrumentId`, batches each
  source, shares Coinbase's socket, backs off per source, and fills `WatchlistQuoteBook`. It neither
  persists quotes nor uses the alert quote path.
- Selecting a row focuses or opens its market in that workspace; highlighting mirrors the focused
  chart.
