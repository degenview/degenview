//! Candle data shared by the DegenView app and its WebAssembly build.
//!
//! Pure logic: no I/O, threads, clocks or UI. Anything that needs the network or a window stays in
//! the app (`service/`, `ui/`) or the web crate, and calls in here.
//!
//! Each exchange module reads that exchange's answers into [`Kline`]s and reports what it could
//! not read as a [`ParseError`]; fetching them is the caller's.

pub mod binance;
pub mod bucket;
pub mod bybit;
pub mod calendar;
pub mod coinbase;
mod error;
mod fields;
pub mod format;
pub mod gateio;
pub mod geckoterminal;
mod kline;
pub mod kraken;
pub mod live;
pub mod mexc;
mod normalize;
pub mod okx;
pub mod price_format;
mod ticks;
mod timeframe;
pub mod viewport;

pub use error::ParseError;
pub use kline::Kline;
pub use normalize::normalize;
pub use ticks::price_ticks;
pub use timeframe::{Section, TimeUnit, Timeframe, TimeframeParseError};
