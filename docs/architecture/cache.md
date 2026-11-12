# Cache architecture

Regenerable data lives in `store::cache_dir`, never the user-data folder: macOS
`~/Library/Caches/es.nico.degenview`, Windows `%LOCALAPPDATA%\es.nico.degenview\Cache`, and Linux
`$XDG_CACHE_HOME` or `~/.cache`. `DEGENVIEW_CACHE_DIR` overrides it. Missing, unreadable, or invalid
files are misses.

| Path | Data | Owner |
| --- | --- | --- |
| `images/<prefix>/<url-hash>` | Logo bytes | `image::loader` |
| `lookups/<namespace>.json` | Icon URLs, CoinGecko index, provider catalogs | `cache::lookup` |
| `fx/*.json` | Fiat and BTC history | `fx_rates` |

- `CachedImageLoader` serves memory → disk → dedicated network workers. After 30 days it shows stale
  data while refreshing; downloaded bodies must identify as images; failures retry after three
  minutes.
- `icon` caches resolved artwork for 30 days, clean misses for one day, and never caches failures.
  CoinGecko's top 250 and names last one day.
- Binance, Coinbase, and Alpaca catalogs refresh daily, search locally, and serve stale on error.
- Ten seconds after launch, `cache::disk::prune` removes images older than 90 days, then oldest files
  until below 75% of 128 MB. Lookup entries expire after 90 days.
- Settings ▸ Storage clears only these cache areas, increments the write generation, drops all
  in-memory layers, and forgets egui textures off the UI thread. In-flight old-generation writes
  are discarded.
- Candle cache (`service::cache`) is memory-only (not persisted, so a restart refetches), keyed by
  source/symbol/interval, at most 64 series, and holds only the slow providers (`cache::policy`:
  CoinGecko, DexScreener, Coinbase, Alpaca; Binance answers in one request and has a live feed).
  Fresh candles are served as held (CoinGecko 120 s, DexScreener 30 s); older ones are patched by
  fetching the last four candles (`TAIL_LIMIT`) instead of the whole series, and a tail that leaves
  a gap falls back to a whole fetch. Calls for one series share one request. `cache::control::clear`
  empties it.
- CoinGecko and GeckoTerminal share per-host rate limiters and each has a one-worker lane in
  `work_queue::submit_for`, so a request waiting for its slot never holds a general worker. Their
  charts refresh every 15 s (45 s after an error) rather than every 5 s. A pool's GeckoTerminal
  network is remembered 30 days in the `geckoterminal_networks` lookup namespace.
