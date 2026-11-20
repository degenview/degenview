//! Price bars and what is built from them, below both [`crate::model`] and [`crate::pine`]: the
//! sources bars come from, chart types and the bars they construct (Heikin Ashi, Renko, Kagi, …),
//! volume profiles, and the `{{placeholder}}` text alerts render from a bar. The bar itself,
//! timeframes and their calendar buckets are `degenview_market`'s (shared with the web build) and
//! re-exported here, so the Pine engine reads everything it needs of bars from one module.
//!
//! The engine imports this and nothing of `model`, so the two no longer import each other. `model`
//! re-exports every module here at its old path, so the rest of the app is unchanged. Pure logic:
//! no I/O and nothing of the app's own beyond `format`.

pub mod alert_message;
pub mod kind;
pub mod source;
pub mod synthetic;
pub mod volume_profile;

pub use degenview_market::Kline;
pub use degenview_market::{TimeUnit, Timeframe, bucket};
pub use kind::ChartKind;
pub use source::DataSource;
