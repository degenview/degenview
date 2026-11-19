//! Price bars and what is built from them, below both [`crate::model`] and [`crate::pine`]: the
//! bar itself, timeframes and their calendar buckets, the sources bars come from, chart types and
//! the bars they construct (Heikin Ashi, Renko, Kagi, …), volume profiles, and the
//! `{{placeholder}}` text alerts render from a bar.
//!
//! The Pine engine reads these and nothing else of `model`, so the two no longer import each
//! other. `model` re-exports every module here at its old path, so the rest of the app is
//! unchanged. Pure logic: no I/O and nothing of the app's own beyond `format`.

pub mod alert_message;
pub mod bucket;
pub mod kind;
pub mod source;
pub mod synthetic;
pub mod timeframe;
pub mod volume_profile;

pub use kind::ChartKind;
pub use degenview_market::Kline;
pub use source::DataSource;
pub use timeframe::{TimeUnit, Timeframe};
