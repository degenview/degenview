# wasm-chart

Proof of concept: a candlestick chart of Coinbase, Binance, OKX, Bybit, Kraken or Gate.io in the
browser, fetched and drawn by Rust compiled to WebAssembly. It is its own cargo workspace and
shares the app's pure candle logic through [`crates/market`](../../crates/market)
(`degenview-market`): `Kline`, `Timeframe`, the bucket folds, the viewport (pan, zoom,
`visible_window`), `PriceFormat`, the exchanges' candle parsers, `normalize` and `price_ticks`.

- `src/exchange.rs` is the table of exchanges: how each spells a market (`BTC-USD`, `BTCUSDT`,
  `BTC/USD`, `BTC_USDT`), which candle lengths it serves, how many it returns, how it names a
  length (`1H`, `6Hutc`, `240`, `7d`) and which of the crate's parsers reads its answer. It is pure
  and tested natively. `src/fetch.rs` asks with the browser's `fetch`; these exchanges answer a
  cross-origin request, so there is no proxy (MEXC, whose parser is in the crate, does not).
- The timeframe box takes any length the app does (`22m`, `4h`, `2D`, `1W`). The crate's `FoldPlan`
  serves it from the exchange's own length when it has it, else folds the coarsest finer one that
  nests, exactly as the app does.
- `src/view.rs` holds the app's `ChartViewport` and turns pointer events into edits of it: drag pans
  in time (and in price once the drag strays vertically), the wheel zooms about the pointer, a
  double-click resets. It is pure and tested natively.
- `src/live.rs` keeps the chart live. Coinbase has no candle channel, so the page opens the `ticker`
  feed (`degenview_market::coinbase` builds the subscription and reads the messages) and Rust
  builds the running base bar from its trades; the crate's `LiveFold` folds that into the chart's
  candles the way the app folds live klines. Every 30 s, and after a reconnect, a REST snapshot is
  merged over the result with `LiveFold::merge_fetched`, where bars the feed delivered after the
  request began win (`LiveMark`, here over `performance.now()`). The other exchanges have no feed
  here: the page polls the same snapshot every 10 s and merges it.
- `src/render.rs` paints the bars the viewport shows (`visible_window`), the axes, the last price
  and a crosshair with the hovered bar's figures on a Canvas 2D context, and reports where it put
  the plot; `src/lib.rs` exports `load_chart` and `Chart` through `wasm-bindgen`.
- `web/index.html` only wires the market and timeframe selectors and redraws on resize.

## Run

```bash
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.130 --locked
./build.sh serve   # http://localhost:8080
```

`cargo test` runs the URL and axis-scale tests natively; parsing and tick tests live in the crate.

## Not in scope

Live trades for the exchanges other than Coinbase (their socket parsers are still in the app),
older history beyond what one request returns (300 to 1,000 candles by exchange; the chart keeps at
most 500), volume and indicators.
