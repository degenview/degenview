//! What a TradingView account holds, as far as this machine's copy of the desktop app knows it,
//! and the pure rules that turn it into DegenView layouts, drawings, watchlists and scripts.
//!
//! TradingView keeps charts on its servers. The desktop app is a browser, so what it last loaded
//! is in its HTTP cache; `service::tradingview` reads that and this module understands it. A
//! [`Library`] therefore holds what was opened recently, never a whole account, and every part of
//! it can be missing. Nothing here guesses: what cannot be read or has no counterpart is reported
//! as a [`Skip`], never approximated silently. See `docs/tradingview-import.md`.
//!
//! Pure: no egui, no I/O.

pub mod color;
pub mod convert;
pub mod drawing;
pub mod grid;
pub mod interval;
pub mod library;
pub mod parse;
pub mod plan;
pub mod symbol;

#[cfg(test)]
pub(crate) mod fixtures;

pub use library::{Library, Script};
pub use symbol::TvSymbol;
