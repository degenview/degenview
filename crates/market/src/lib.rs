//! Candle data shared by the DegenView app and its WebAssembly build.
//!
//! Pure logic: no I/O, threads, clocks or UI. Anything that needs the network or a window stays in
//! the app (`service/`, `ui/`) or the web crate, and calls in here.

pub mod bucket;
pub mod calendar;
pub mod coinbase;
pub mod format;
mod kline;
pub mod live;
mod normalize;
pub mod price_format;
mod ticks;
mod timeframe;
pub mod viewport;

pub use kline::Kline;
pub use normalize::normalize;
pub use ticks::price_ticks;
pub use timeframe::{Section, TimeUnit, Timeframe, TimeframeParseError};
