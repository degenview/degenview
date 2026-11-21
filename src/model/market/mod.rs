//! Markets and their data: instruments, sources, timeframes, candles and prediction-market series.

pub mod bucket;
pub mod cmc;
pub mod feed;
pub mod history;
pub(in crate::model) mod instrument;
pub(in crate::model) mod kind;
pub(in crate::model) mod kline;
pub mod line;
pub mod live;
pub mod natives;
pub(in crate::model) mod pm_series;
pub mod profile;
pub mod samples;
pub(in crate::model) mod source;
pub(in crate::model) mod timeframe;
pub mod trend;
pub mod venue;

pub use kind::MarketKind;
