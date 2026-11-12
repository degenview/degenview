//! Order entry, as the screens see it: one vocabulary and one trait for every venue (the paper
//! account and each connected broker). See [`backend::TradingBackend`]. No drawing code lives here,
//! and nothing here reaches into `ui`.
//!
//! The trait covers what the order sheet needs: the account, the market's rules and prices, checking,
//! previewing, sending and cancelling an order. Changing an open order in place is [`edit`]: a state
//! machine the Edit sheet draws. The live orders panel and the chart's order lines still read the
//! broker host's snapshots directly (`docs/broker-trading.md` lists what is not migrated).

pub mod backend;
pub mod broker;
pub mod confirm;
pub mod draft;
pub mod edit;
pub mod model;
pub mod money;
pub mod paper;
pub mod preview;
pub mod task;

#[cfg(test)]
mod conformance_tests;
