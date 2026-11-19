//! Candle data shared by the DegenView app and its WebAssembly build.
//!
//! Pure logic: no I/O, threads, clocks or UI. Anything that needs the network or a window stays in
//! the app (`service/`, `ui/`) or the web crate, and calls in here.

pub mod coinbase;
mod kline;
mod normalize;
mod ticks;

pub use kline::Kline;
pub use normalize::normalize;
pub use ticks::price_ticks;
