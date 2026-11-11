//! Linked charts: how one chart's horizontal navigation carries over to the others of its
//! workspace.
//!
//! Nothing here keeps a viewport. A chart's [`HorizontalViewport`] stays the one description of
//! what it shows; this module only *translates* it. The leader's change is described in market
//! time ([`HorizontalChange`]), and each follower turns that time back into its own bars
//! ([`follow`]). A bar index means nothing across symbols, timeframes or sessions, so time is the
//! one coordinate the charts share.
//!
//! # The invariant
//!
//! A viewport shows the bars between two slot edges, `right = len - right_offset` and
//! `left = right - visible`, with the Canvas right margin as free space past `right`. A chart's
//! [`TimeAxis`] maps those edges to timestamps and back, so a viewport *is* a time range
//! `[left, right]`:
//!
//! * **Position** is the time at the right edge. At the newest bar it is the end of that bar, so
//!   charts of any timeframe agree on "now", and the margin (a presentation setting) never enters.
//! * **Zoom** is how many bars span the range. Zooming keeps the right edge, so a zoom never moves
//!   the position.
//!
//! The two are synchronised independently ([`ChartSyncSettings`]):
//!
//! | Position | Zoom | A pan | A zoom |
//! | --- | --- | --- | --- |
//! | off | off | – | – |
//! | on | off | followers' right edge goes to the leader's time; their zoom is kept | – |
//! | off | on | – | followers' bar count scales by the leader's factor; their position is kept |
//! | on | on | as above | followers show the leader's time range |
//!
//! Vertical state is not described by a [`HorizontalChange`], so it cannot be carried over.

use crate::model::Kline;
use crate::model::chart::viewport::HorizontalViewport;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Which parts of horizontal navigation the charts of a workspace share. Saved with the layout.
///
/// Charts are linked in both ways unless the user unlinks them: a layout saved before this
/// existed, or one that never chose, opens linked. A chart is only ever moved by navigating
/// another one, so linking costs nothing until the user pans or zooms.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct ChartSyncSettings {
    /// Panning one chart moves the others to the same point in time.
    pub horizontal_position: bool,
    /// Zooming one chart adjusts the others.
    pub horizontal_zoom: bool,
}

impl Default for ChartSyncSettings {
    fn default() -> Self {
        Self {
            horizontal_position: true,
            horizontal_zoom: true,
        }
    }
}

impl ChartSyncSettings {
    /// Every chart navigates alone.
    #[cfg(test)]
    pub const UNLINKED: Self = Self {
        horizontal_position: false,
        horizontal_zoom: false,
    };

    /// Whether anything is linked.
    pub fn is_linked(&self) -> bool {
        self.horizontal_position || self.horizontal_zoom
    }
}

// MARK: Time

/// A chart's bars as a time scale.
///
/// Slot edges count from the oldest bar: edge `i` is where bar `i` begins, edge `len` where the
/// newest ends. Between two edges time is interpolated, so a weekend inside a stock chart is one
/// slot wide and the map stays continuous and strictly increasing. Beyond either end it continues
/// at the length of the first or last bar, which is what empty space on a chart means: no bars
/// are invented, only a position in time.
#[derive(Clone, Copy, Debug)]
pub struct TimeAxis<'a> {
    bars: &'a [Kline],
}

impl<'a> TimeAxis<'a> {
    pub fn new(bars: &'a [Kline]) -> Self {
        Self { bars }
    }

    pub fn len(&self) -> usize {
        self.bars.len()
    }

    /// Time of slot edge `index`, `0..=len`.
    fn edge(&self, index: usize) -> f64 {
        match self.bars.get(index) {
            Some(bar) => bar.open_time as f64,
            None => self
                .bars
                .last()
                .map_or(0.0, |bar| bar.open_time as f64 + Self::span(bar)),
        }
    }

    /// How long a bar lasts, never less than a millisecond.
    fn span(bar: &Kline) -> f64 {
        ((bar.close_time - bar.open_time + 1) as f64).max(1.0)
    }

    /// The timestamp (ms) at slot position `x`, `None` for an empty chart or a number no chart
    /// can place.
    pub fn time_at(&self, x: f64) -> Option<f64> {
        let (first, last) = (self.bars.first()?, self.bars.last()?);
        let len = self.bars.len() as f64;
        if !x.is_finite() {
            return None;
        }
        Some(if x <= 0.0 {
            self.edge(0) + x * Self::span(first)
        } else if x >= len {
            self.edge(self.bars.len()) + (x - len) * Self::span(last)
        } else {
            let index = x.floor() as usize;
            let (from, to) = (self.edge(index), self.edge(index + 1));
            from + (to - from) * (x - index as f64)
        })
    }

    /// The slot position of timestamp `time` (ms): the inverse of [`Self::time_at`].
    pub fn x_at(&self, time: f64) -> Option<f64> {
        let (first, last) = (self.bars.first()?, self.bars.last()?);
        let len = self.bars.len();
        if !time.is_finite() {
            return None;
        }
        let (start, end) = (self.edge(0), self.edge(len));
        Some(if time <= start {
            (time - start) / Self::span(first)
        } else if time >= end {
            len as f64 + (time - end) / Self::span(last)
        } else {
            // The bar that has begun by `time`; at least the first, as `time > start`.
            let index = self
                .bars
                .partition_point(|bar| bar.open_time as f64 <= time)
                .saturating_sub(1);
            let (from, to) = (self.edge(index), self.edge(index + 1));
            if to > from {
                index as f64 + (time - from) / (to - from)
            } else {
                index as f64
            }
        })
    }

    /// The time range a viewport shows, `None` for an empty chart.
    pub fn range_of(&self, view: &HorizontalViewport) -> Option<TimeRange> {
        let right = self.bars.len() as f64 - view.right_offset;
        Some(TimeRange {
            left: self.time_at(right - view.visible as f64)?,
            right: self.time_at(right)?,
        })
    }
}

/// A span of market time, in milliseconds since the epoch.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TimeRange {
    pub left: f64,
    pub right: f64,
}

// MARK: Change

/// How the bar count changed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Zoom {
    /// Bars across the plot before the change.
    pub from: usize,
    /// And after it.
    pub to: usize,
}

/// What one frame of the user's navigation did to a chart's horizontal viewport, in terms any
/// other chart can use. It is a few numbers, copied freely.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HorizontalChange {
    /// The time range the chart shows now.
    pub range: TimeRange,
    /// The chart holds its newest bar: the other charts go to theirs.
    pub latest: bool,
    /// The right edge moved.
    pub moved: bool,
    /// The bar count changed.
    pub zoom: Option<Zoom>,
}

impl HorizontalChange {
    /// What turned `before` into `after` on a chart with these bars, or `None` when nothing the
    /// charts share changed. The vertical viewport is not an input, so it cannot be described.
    pub fn between(
        before: &HorizontalViewport,
        after: &HorizontalViewport,
        axis: &TimeAxis,
    ) -> Option<Self> {
        let moved = (after.right_offset - before.right_offset).abs() > 1e-9;
        let zoom = (after.visible != before.visible).then_some(Zoom {
            from: before.visible,
            to: after.visible,
        });
        if !moved && zoom.is_none() {
            return None;
        }
        Some(Self {
            range: axis.range_of(after)?,
            latest: after.follows_latest(),
            moved,
            zoom,
        })
    }
}

/// How a follower scales its bar count for a leader's zoom: the burst's total factor applied to
/// the count the follower had when the burst began, so rounding never accumulates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ZoomFollow {
    pub factor: f64,
    pub start_visible: usize,
}

/// A follower's viewport after a change, and whether it got all the way there.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Followed {
    pub viewport: HorizontalViewport,
    /// False when the chart's history ends before the leader's time: it stays at its oldest
    /// bars and may try again once older ones load.
    pub reached: bool,
}

/// Where a follower goes for `change`, given its own bars and what is linked.
///
/// Only the horizontal viewport is read or written, so a chart's price scale cannot be affected.
pub fn follow(
    settings: ChartSyncSettings,
    change: &HorizontalChange,
    zoom: Option<ZoomFollow>,
    axis: &TimeAxis,
    current: &HorizontalViewport,
) -> Followed {
    let total = axis.len();
    let mut next = *current;
    let unchanged = Followed {
        viewport: next,
        reached: true,
    };
    if total == 0 {
        return unchanged;
    }
    let zooms = settings.horizontal_zoom && change.zoom.is_some();
    let moves = settings.horizontal_position && change.moved;
    // The x of the leader's right edge on this chart.
    let right = if change.latest {
        Some(total as f64)
    } else {
        axis.x_at(change.range.right)
    };

    let mut wanted_offset = None;
    if zooms && settings.horizontal_position {
        // Both linked: the follower shows the leader's time range.
        // The span is the leader's range in this chart's bars; where the right edge goes may
        // snap to the newest bar, which a lagging feed would otherwise miss by one.
        let (Some(right), Some(span)) = (
            right,
            axis.x_at(change.range.right)
                .zip(axis.x_at(change.range.left))
                .map(|(right, left)| right - left),
        ) else {
            return unchanged;
        };
        if span > 0.0 {
            next.set_visible((span.round() as usize).max(1), total);
            wanted_offset = Some(total as f64 - right);
        }
    } else if zooms && let Some(zoom) = zoom {
        // Only zoom linked: the same factor about the follower's own right edge.
        let wanted = (zoom.start_visible as f64 * zoom.factor).round();
        if wanted.is_finite() && wanted >= 1.0 {
            next.set_visible(wanted as usize, total);
        }
    }
    if moves && wanted_offset.is_none() {
        wanted_offset = right.map(|right| total as f64 - right);
    }

    let mut reached = true;
    if let Some(offset) = wanted_offset.filter(|offset| offset.is_finite()) {
        next.right_offset = offset;
        let (low, high) = next.offset_limits(total);
        // Only the older end is worth another try: older bars may still load. Space past the
        // newest bar is bounded for every chart alike.
        reached = offset <= high + 1e-6;
        next.right_offset = offset.clamp(low, high);
    }
    next.clamp(total);
    Followed {
        viewport: next,
        reached,
    }
}

// MARK: Coordination

/// One change published by the chart the user is navigating.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SyncEvent {
    /// The leader: it never applies its own event.
    pub origin: u64,
    /// Grows with every event, so a chart applies each at most once.
    pub sequence: u64,
    pub change: HorizontalChange,
}

/// The bar counts a zoom burst started from.
#[derive(Debug)]
struct ZoomBase {
    origin: u64,
    leader_start: usize,
    /// Each follower's count when it first heard of the burst.
    followers: HashMap<u64, usize>,
}

/// Carries a workspace's navigation events from the chart being navigated to the others.
///
/// A workspace owns one; it holds no viewport. Only the newest event is kept: a position is a
/// time, and a zoom is measured from the burst's start, so skipping an event loses nothing. A
/// follower's own change is never published, which is what keeps the charts from echoing each
/// other.
#[derive(Debug, Default)]
pub struct SyncHub {
    sequence: u64,
    event: Option<SyncEvent>,
    /// The newest event each chart has applied (or published).
    seen: HashMap<u64, u64>,
    zoom: Option<ZoomBase>,
    zoomed_this_frame: bool,
    /// Followers whose history ended before the leader's time, and where the leader stands.
    behind: Vec<u64>,
    target: Option<HorizontalChange>,
}

impl SyncHub {
    /// The chart `origin` was navigated by the user.
    pub fn publish(&mut self, origin: u64, change: HorizontalChange) {
        self.sequence += 1;
        self.seen.insert(origin, self.sequence);
        self.behind.retain(|id| *id != origin);
        self.event = Some(SyncEvent {
            origin,
            sequence: self.sequence,
            change,
        });
        if change.moved {
            self.target = Some(change);
        }
        match change.zoom {
            Some(zoom) => {
                self.zoomed_this_frame = true;
                if self.zoom.as_ref().is_none_or(|base| base.origin != origin) {
                    self.zoom = Some(ZoomBase {
                        origin,
                        leader_start: zoom.from,
                        followers: HashMap::new(),
                    });
                }
            }
            None => self.zoom = None,
        }
    }

    /// The newest event, for tests.
    #[cfg(test)]
    pub fn latest_event(&self) -> Option<&SyncEvent> {
        self.event.as_ref()
    }

    /// Brings chart `id`, which holds `axis`' bars and shows `view`, up to date. Returns whether
    /// its viewport changed.
    pub fn deliver(
        &mut self,
        id: u64,
        settings: ChartSyncSettings,
        axis: &TimeAxis,
        view: &mut HorizontalViewport,
    ) -> bool {
        if !settings.is_linked() {
            return false;
        }
        let fresh = self
            .event
            .filter(|event| event.origin != id && self.seen.get(&id) != Some(&event.sequence));
        let change = match fresh {
            Some(event) => {
                self.seen.insert(id, event.sequence);
                event.change
            }
            // Still short of the leader's time: older bars may have arrived.
            None if self.behind.contains(&id) => match self.target {
                Some(target) => target,
                None => return false,
            },
            None => return false,
        };
        let zoom = self.zoom_for(id, &change, view);
        let followed = follow(settings, &change, zoom, axis, view);
        self.behind.retain(|other| *other != id);
        if !followed.reached {
            self.behind.push(id);
        }
        let changed = followed.viewport != *view;
        *view = followed.viewport;
        changed
    }

    fn zoom_for(
        &mut self,
        id: u64,
        change: &HorizontalChange,
        view: &HorizontalViewport,
    ) -> Option<ZoomFollow> {
        let zoom = change.zoom?;
        let base = self.zoom.as_mut()?;
        let start_visible = *base.followers.entry(id).or_insert(view.visible);
        (base.leader_start > 0).then(|| ZoomFollow {
            factor: zoom.to as f64 / base.leader_start as f64,
            start_visible,
        })
    }

    /// Call once after every chart has been drawn: a burst of zooming ends with the first frame
    /// that did not zoom.
    pub fn end_frame(&mut self) {
        if !self.zoomed_this_frame {
            self.zoom = None;
        }
        self.zoomed_this_frame = false;
    }

    /// The linked settings changed: whatever was under way is over, and nothing jumps.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// A chart left the workspace.
    pub fn forget(&mut self, id: u64) {
        self.seen.remove(&id);
        self.behind.retain(|other| *other != id);
        if let Some(base) = self.zoom.as_mut() {
            base.followers.remove(&id);
        }
        if self.event.is_some_and(|event| event.origin == id) {
            self.event = None;
        }
    }
}

#[cfg(test)]
mod tests;
