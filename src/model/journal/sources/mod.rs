//! Adapters that turn a trading source's own records into journal executions. Only the paper
//! source exists; a broker or file import is a new module here implementing
//! [`TradingActivitySource`](super::source::TradingActivitySource).

pub mod paper;
