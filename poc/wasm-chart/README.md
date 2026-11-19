# wasm-chart

Proof of concept: a Coinbase candlestick chart in the browser, fetched and drawn by Rust compiled
to WebAssembly. It is its own cargo workspace and shares the app's pure candle logic through
[`crates/market`](../../crates/market) (`degenview-market`): `Kline`, the Coinbase row parser,
`normalize` and `price_ticks`.

- `src/coinbase.rs` fetches `api.exchange.coinbase.com/products/{id}/candles` with the browser's
  `fetch` (the API allows cross-origin requests, so no proxy is needed) and hands the body to the
  crate's parser.
- `src/render.rs` paints candles, axes and the last price on a Canvas 2D context; `src/lib.rs`
  exports `load_chart` and `Chart::draw` through `wasm-bindgen`.
- `web/index.html` only wires the market and timeframe selectors and redraws on resize.

## Run

```bash
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.130 --locked
./build.sh serve   # http://localhost:8080
```

`cargo test` runs the URL and axis-scale tests natively; parsing and tick tests live in the crate.

## Not in scope

Live updates (Coinbase's WebSocket), pan/zoom, crosshair, volume and indicators. Coinbase returns
at most 300 candles per request.
