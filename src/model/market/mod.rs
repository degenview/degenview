//! Markets and their data: instruments, sources, timeframes, candles and prediction-market series.

pub use crate::bars::source::cmc;
pub mod feed;
pub mod history;
pub(in crate::model) mod instrument;
pub mod line;
pub mod natives;
pub(in crate::model) mod pm_series;
pub mod samples;
pub(in crate::model) use crate::bars::source;
pub mod trend;
pub mod venue;
