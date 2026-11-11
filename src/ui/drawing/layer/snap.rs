//! The magnet: pulls an anchor to the open, high, low or close of the nearest bar.
//!
//! It reads the projection's bars, which are the chart's constructed series (Renko bricks, Heikin
//! Ashi candles…) already cut at the replay cursor, so it follows the active chart type and can
//! never reach a bar the replay has not shown yet.

use super::Projection;
use crate::model::{Anchor, MagnetMode};
use eframe::egui::Pos2;

/// How near (px) the pointer must be to a price of a bar for the weak magnet to take it.
pub(super) const WEAK_RADIUS: f32 = 20.0;

/// The anchor `pointer` snaps to under `mode`, or `None` when it stays where it is (magnet off,
/// no bars, over the blank space beyond the data, or, for the weak magnet, too far away).
pub(super) fn snap(pointer: Pos2, proj: &Projection, mode: MagnetMode) -> Option<Anchor> {
    if mode == MagnetMode::Off || proj.times.len() == 0 || !pointer.x.is_finite() {
        return None;
    }
    let index = proj.times.index_of(proj.time(pointer.x)).round();
    if index < 0.0 || index >= proj.times.len() as f64 {
        return None;
    }
    let bar = proj.times.bar(index as usize)?;
    let x = proj.x(bar.open_time);
    let (price, y) = [bar.open, bar.high, bar.low, bar.close]
        .into_iter()
        .filter(|price| price.is_finite())
        .map(|price| (price, proj.scale.y(price)))
        .filter(|(_, y)| y.is_finite())
        .min_by(|a, b| (a.1 - pointer.y).abs().total_cmp(&(b.1 - pointer.y).abs()))?;
    if mode == MagnetMode::Weak && Pos2::new(x, y).distance(pointer) > WEAK_RADIUS {
        return None;
    }
    Some(Anchor {
        time_ms: bar.open_time,
        price,
    })
}
