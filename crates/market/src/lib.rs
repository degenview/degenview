//! Candle data shared by the DegenView app and its WebAssembly build.
//!
//! Pure logic: no I/O, threads, clocks or UI. Anything that needs the network or a window stays in
//! the app (`service/`, `ui/`) or the web crate, and calls in here.

pub mod bucket;
pub mod calendar;
pub mod coinbase;
mod kline;
pub mod live;
mod normalize;
mod ticks;
mod timeframe;

pub use kline::Kline;
pub use normalize::normalize;
pub use ticks::price_ticks;
pub use timeframe::{Section, TimeUnit, Timeframe, TimeframeParseError};
