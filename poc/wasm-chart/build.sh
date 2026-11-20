#!/usr/bin/env bash
# Builds the WebAssembly module into web/pkg and, with `serve`, hosts web/ on http://localhost:8080.
#
# One-time setup:
#   rustup target add wasm32-unknown-unknown
#   cargo install wasm-bindgen-cli --version 0.2.130 --locked
set -euo pipefail
cd "$(dirname "$0")"

# rustup's cargo, not a Homebrew one that has no wasm32 standard library.
export PATH="$HOME/.cargo/bin:$PATH"

cargo build --release --locked --target wasm32-unknown-unknown
wasm-bindgen --target web --no-typescript --out-dir web/pkg \
  target/wasm32-unknown-unknown/release/wasm_chart.wasm
ls -lh web/pkg/wasm_chart_bg.wasm

if [[ "${1:-}" == "serve" ]]; then
  echo "http://localhost:8080"
  exec python3 -m http.server 8080 --bind 127.0.0.1 --directory web
fi
