//! Markets and their data: instruments, sources, timeframes, candles and prediction-market series.

pub use crate::bars::bucket;
pub use crate::bars::source::cmc;
pub mod feed;
pub mod history;
pub(in crate::model) mod instrument;
pub(in crate::model) use crate::bars::kline;
pub mod line;
pub mod live;
pub mod natives;
pub(in crate::model) mod pm_series;
pub mod samples;
pub(in crate::model) use crate::bars::source;
pub(in crate::model) use crate::bars::timeframe;
pub mod trend;
pub mod venue;
