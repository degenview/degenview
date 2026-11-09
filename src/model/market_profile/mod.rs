//! The shared engine under the three order-flow chart types (Volume footprint, Time price
//! opportunity, Session volume profile): pure, deterministic and independent of egui.
//!
//! What lives here, and what does not:
//!
//! - [`price_grid`]: rows are **integer tick indices**, never `f64` keys, so neighbouring bars and
//!   sessions land on the same rows and a price such as `0.1 + 0.2` does not fall in the wrong one.
//! - [`value_area`]: the point of control and value area, once for volume (shared with the Pine
//!   footprint and the volume-profile drawing) and once for TPO counts, which is a different
//!   algorithm in TradingView's documentation.
//! - [`sessions`]: day/week/month periods and custom sessions in a zone, and the TPO letter of a
//!   block of time.
//! - [`trades`]: exchange trades with an aggressor side, deduplicated by id and bounded.
//! - [`flow`]: lower-timeframe candles with the exchange's own taker-buy volume, deduplicated by open
//!   time and bounded. Side data: [`super::kline::Kline`] itself is unchanged.
//! - [`quality`]: where a number came from, so nothing estimated is shown as exchange data.
//!
//! The per-type builders (footprint, TPO, session profile) sit on top of these and arrive with the
//! chart types that draw them. Nothing here fetches, draws or reads the clock.

pub mod flow;
pub mod price_grid;
pub mod quality;
pub mod sessions;
pub mod trades;
pub mod value_area;
