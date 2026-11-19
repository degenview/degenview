# wasm-chart

Proof of concept: a Coinbase candlestick chart in the browser, fetched and drawn by Rust compiled
to WebAssembly. It is its own cargo workspace and shares the app's pure candle logic through
[`crates/market`](../../crates/market) (`degenview-market`): `Kline`, `Timeframe`, the bucket folds,
the viewport (pan, zoom, `visible_window`), `PriceFormat`, the Coinbase row parser, `normalize` and
`price_ticks`.

- `src/coinbase.rs` fetches `api.exchange.coinbase.com/products/{id}/candles` with the browser's
  `fetch` (the API allows cross-origin requests, so no proxy is needed) and hands the body to the
  crate's parser. The timeframe box takes any length the app does (`22m`, `4h`, `2D`, `1W`): Coinbase
  serves 1m, 5m, 15m, 1h, 6h and 1D, and the crate's `FoldPlan` folds the rest from the coarsest
  one that nests, exactly as the app does.
- `src/view.rs` holds the app's `ChartViewport` and turns pointer events into edits of it: drag pans
  in time (and in price once the drag strays vertically), the wheel zooms about the pointer, a
  double-click resets. It is pure and tested natively.
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

Live updates (Coinbase's WebSocket), older history beyond the first 300 candles, volume and
indicators. Coinbase returns
at most 300 candles per request.
