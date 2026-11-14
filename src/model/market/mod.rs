//! Markets and their data: instruments, sources, timeframes, candles and prediction-market series.

pub mod history;
pub(in crate::model) mod instrument;
pub(in crate::model) mod kline;
pub(in crate::model) mod pm_series;
pub(in crate::model) mod source;
pub(in crate::model) mod timeframe;
pub mod trend;
