//! Comparison overlays on a chart card: resolving them against the bars in view, drawing them,
//! tagging their last values and answering the crosshair.
//!
//! The card is handed each overlay's configuration and candles (`set_overlays`, from the
//! workspace's hub) and never fetches. Everything here reads only the bars in view, so the work
//! follows the screen, not the history the chart holds, and one `OverlayFrame` per view feeds the
//! price range, the lines, the tags and the readout, so they cannot disagree.
//!
//! The chart's bars are the time axis: an overlay has a value on a bar or it has none (see
//! `model::overlay::align`), and a bar without one breaks the line. Replay needs nothing of its
//! own: the bars the card is given are already cut at the replay cursor, and an overlay candle
//! can only land on one of them.

use super::{ChartCard, View};
use crate::lock::lock;
use crate::model::chart;
use crate::model::chart::kind::{ChartType, PriceSource};
use crate::model::chart::price_scale::PriceScaleMode;
use crate::model::editor_theme::Rgba;
use crate::model::overlay::align::{Aligned, align};
use crate::model::overlay::feed::{OverlayFeed, OverlayStatus};
use crate::model::overlay::scale::{Mapping, legend_value, mappings};
use crate::model::overlay::{ChartOverlay, OverlayId, OverlayList};
use crate::model::{DataSource, Kline, PriceFormat, Timeframe};
use crate::ui::chart::canvas::CanvasStyle;
use crate::ui::{
    chart::{price_text, series},
    theme,
};
use eframe::egui::{Align2, Color32, Painter, Pos2, Rect, Stroke};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// What the card holds of its overlays.
#[derive(Default)]
pub(super) struct OverlayState {
    config: OverlayList,
    /// One per overlay, in the same order.
    feeds: Vec<OverlayFeed>,
    /// What the feeds were last built for: the hub's version and the chart's market.
    stamp: Option<(u64, DataSource, String)>,
    /// Each overlay's values on the bars of the last view, so the several views of a frame (and
    /// an idle chart's every frame) align once.
    memo: Mutex<HashMap<OverlayId, Memo>>,
}

struct Memo {
    key: MemoKey,
    values: Arc<Aligned>,
}

#[derive(PartialEq)]
struct MemoKey {
    revision: u64,
    start: usize,
    end: usize,
    len: usize,
    first: i64,
    last: i64,
    source: PriceSource,
    bar_ms: i64,
}

/// Why an overlay is or is not on the chart right now.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum LineState {
    Drawn,
    Hidden,
    /// No candles yet.
    Waiting,
    /// Candles, but none on the bars in view.
    NoOverlap,
    /// Rebasing needs a reference price the chart does not have.
    NoBaseline,
    /// The overlay cannot be drawn on this chart, and why.
    Blocked(Arc<str>),
    /// Price-based chart types have no shared time axis.
    Unsupported,
}

pub(super) struct OverlayLine<'a> {
    pub overlay: &'a ChartOverlay,
    /// The overlay's value on each bar in view (`None` where it has no candle).
    pub values: Arc<Aligned>,
    pub mapping: Mapping,
    pub state: LineState,
}

impl OverlayLine<'_> {
    fn drawn(&self) -> bool {
        self.state == LineState::Drawn
    }

    /// Whether the market is meant to be on this chart, drawn or not for now (loading, nothing in
    /// view): hidden, blocked and unsupported ones are not.
    fn listed(&self) -> bool {
        !matches!(
            self.state,
            LineState::Hidden | LineState::Blocked(_) | LineState::Unsupported
        )
    }

    pub(super) fn color(&self) -> Color32 {
        color32(self.overlay.color)
    }

    /// The newest value in view, and its index among the bars in view.
    fn last(&self) -> Option<(usize, f64)> {
        self.values
            .iter()
            .enumerate()
            .rev()
            .find_map(|(index, value)| value.map(|value| (index, value)))
    }

    /// Where `value` is drawn: its height on the chart.
    fn y(&self, value: f64, scale: &super::PriceScale) -> Option<f32> {
        let y = match self.mapping {
            Mapping::Main { .. } => scale.y(self.mapping.rebased(value)?),
            Mapping::Own(fit) => scale.top + scale.height * (1.0 - fit.fraction(value) as f32),
            Mapping::Unmapped => return None,
        };
        y.is_finite().then_some(y)
    }

    /// The value as the legend, the tag and the crosshair read it: the overlay's own change in
    /// the relative scale modes (a rebased line has no price of its own there), else its price.
    pub(super) fn text(&self, value: f64, mode: PriceScaleMode, format: PriceFormat) -> String {
        match (self.mapping, mode) {
            (
                Mapping::Main { baseline, .. },
                PriceScaleMode::Percent | PriceScaleMode::IndexedTo100,
            ) => chart::price_scale::format_relative(
                mode,
                legend_value(mode, Some(baseline), value),
                None,
            ),
            _ => format.format(value),
        }
    }
}

pub(super) struct OverlayFrame<'a> {
    pub lines: Vec<OverlayLine<'a>>,
}

impl OverlayFrame<'_> {
    /// The prices the main axis has to reach for the main-scale lines to be seen.
    pub(super) fn extent(&self) -> Option<(f64, f64)> {
        let mut range: Option<(f64, f64)> = None;
        for line in self.lines.iter().filter(|line| line.drawn()) {
            if !matches!(line.mapping, Mapping::Main { .. }) {
                continue;
            }
            for price in line
                .values
                .iter()
                .flatten()
                .filter_map(|v| line.mapping.rebased(*v))
            {
                if price.is_finite() {
                    range = Some(range.map_or((price, price), |(low, high)| {
                        (low.min(price), high.max(price))
                    }));
                }
            }
        }
        range
    }
}

fn color32(color: Rgba) -> Color32 {
    Color32::from_rgba_unmultiplied(color.r, color.g, color.b, color.a)
}

/// Black or white text, whichever reads on `fill`.
fn on_color(fill: Color32) -> Color32 {
    let luminance =
        0.299 * f32::from(fill.r()) + 0.587 * f32::from(fill.g()) + 0.114 * f32::from(fill.b());
    if luminance > 170.0 {
        Color32::BLACK
    } else {
        Color32::WHITE
    }
}

impl ChartCard {
    /// Whether `set_overlays` has something new: the hub changed, the overlays were edited, or the
    /// chart's own market changed (an overlay of that market can no longer be drawn).
    pub fn overlays_stale(
        &self,
        version: u64,
        source: DataSource,
        symbol: &str,
        config: &OverlayList,
    ) -> bool {
        let state = &self.overlay;
        state.config != *config
            || state
                .stamp
                .as_ref()
                .is_none_or(|(seen, s, y)| *seen != version || *s != source || y != symbol)
    }

    /// Hands the card its overlays and their candles (`feeds[n]` is `config`'s n-th overlay).
    pub fn set_overlays(
        &mut self,
        config: OverlayList,
        feeds: Vec<OverlayFeed>,
        version: u64,
        source: DataSource,
        symbol: &str,
    ) {
        debug_assert_eq!(config.len(), feeds.len());
        let state = &mut self.overlay;
        lock(&state.memo).retain(|id, _| config.get(*id).is_some());
        state.config = config;
        state.feeds = feeds;
        state.stamp = Some((version, source, symbol.to_owned()));
    }

    /// The chart's overlays, in visual order.
    pub fn overlay_list(&self) -> &OverlayList {
        &self.overlay.config
    }

    /// The data state of one overlay.
    pub fn overlay_feed(&self, id: OverlayId) -> Option<&OverlayFeed> {
        let at = self.overlay.config.position(id)?;
        self.overlay.feeds.get(at)
    }

    /// The newest value an overlay has among all the candles it holds, for the legend: its
    /// price, and `None` until it has any.
    pub fn overlay_last_price(&self, id: OverlayId) -> Option<f64> {
        let overlay = self.overlay.config.get(id)?;
        let feed = self.overlay_feed(id)?;
        feed.candles.iter().rev().find_map(|candle| {
            let value = overlay.source.value(candle);
            (value.is_finite() && value > 0.0).then_some(value)
        })
    }

    fn bar_ms(&self) -> i64 {
        self.timeframe.unwrap_or(Timeframe::HOUR_1).nominal_ms()
    }

    /// Resolves the overlays against `bars[start..end]`. `reference` is the main series'
    /// reference price (the close of the first bar in view).
    pub(super) fn overlay_frame<'a>(
        &'a self,
        bars: &[Kline],
        start: usize,
        end: usize,
        chart_type: ChartType,
        reference: Option<f64>,
    ) -> OverlayFrame<'a> {
        let state = &self.overlay;
        if state.config.is_empty() || end <= start || end > bars.len() {
            return OverlayFrame { lines: Vec::new() };
        }
        let bar_ms = self.bar_ms();
        let supported = !chart_type.is_synthetic();
        let (first, last) = (bars[start].open_time, bars[end - 1].open_time);
        let mut memo = lock(&state.memo);
        let mut resolved: Vec<(&ChartOverlay, &OverlayFeed, Arc<Aligned>)> = Vec::new();
        for (overlay, feed) in state.config.iter().zip(&state.feeds) {
            let wanted = supported
                && overlay.visible
                && !matches!(feed.status, OverlayStatus::Blocked(_))
                && !feed.candles.is_empty();
            let values = if wanted {
                let key = MemoKey {
                    revision: feed.revision,
                    start,
                    end,
                    len: bars.len(),
                    first,
                    last,
                    source: overlay.source,
                    bar_ms,
                };
                match memo.get(&overlay.id) {
                    Some(held) if held.key == key => held.values.clone(),
                    _ => {
                        let values = Arc::new(align(
                            bars,
                            start..end,
                            bar_ms,
                            &feed.candles,
                            bar_ms,
                            overlay.source,
                        ));
                        memo.insert(
                            overlay.id,
                            Memo {
                                key,
                                values: values.clone(),
                            },
                        );
                        values
                    }
                }
            } else {
                Arc::new(Vec::new())
            };
            resolved.push((overlay, feed, values));
        }
        drop(memo);
        let items: Vec<_> = resolved
            .iter()
            .map(|(overlay, _, values)| (overlay.scale, values.as_slice()))
            .collect();
        let maps = mappings(&items, reference);
        let lines = resolved
            .into_iter()
            .zip(maps)
            .map(|((overlay, feed, values), mapping)| {
                let state = if !supported {
                    LineState::Unsupported
                } else if let OverlayStatus::Blocked(reason) = &feed.status {
                    LineState::Blocked(Arc::clone(reason))
                } else if !overlay.visible {
                    LineState::Hidden
                } else if feed.candles.is_empty() {
                    LineState::Waiting
                } else if values.iter().all(Option::is_none) {
                    LineState::NoOverlap
                } else if mapping == Mapping::Unmapped {
                    LineState::NoBaseline
                } else {
                    LineState::Drawn
                };
                OverlayLine {
                    overlay,
                    values,
                    mapping,
                    state,
                }
            })
            .collect();
        OverlayFrame { lines }
    }

    /// What the axis column has to hold for the main-scale lines' last-value tags: their texts, so
    /// the axis is as wide as the widest. Every labelled line counts, in the plot or not, so the
    /// axis does not change width as one leaves it.
    pub(super) fn overlay_tag_texts(
        &self,
        bars: &[Kline],
        view: &View,
        chart_type: ChartType,
    ) -> Vec<String> {
        let frame =
            self.overlay_frame(bars, view.start, view.end, chart_type, view.scale.reference);
        let format = self.appearance.price_format();
        frame
            .lines
            .iter()
            .filter(|line| {
                line.drawn()
                    && line.overlay.show_label
                    && matches!(line.mapping, Mapping::Main { .. })
            })
            .filter_map(|line| {
                let (_, value) = line.last()?;
                Some(line.text(value, view.scale.mode, format))
            })
            .collect()
    }

    /// The overlays' lines. Drawn under the main series, over the grid, clipped to the plot.
    pub(super) fn paint_overlay_lines(
        &self,
        painter: &Painter,
        frame: &OverlayFrame,
        view: &View,
        x_for: &dyn Fn(usize) -> f32,
    ) {
        // `x_for` takes an index among the bars in view, as the values here are.
        for line in frame.lines.iter().filter(|line| line.drawn()) {
            let points: Vec<Option<Pos2>> = line
                .values
                .iter()
                .enumerate()
                .map(|(index, value)| {
                    let y = line.y((*value)?, &view.scale)?;
                    Some(Pos2::new(x_for(index), y))
                })
                .collect();
            let stroke = Stroke::new(line.overlay.width, line.color());
            for run in series::runs(&points) {
                match run.as_slice() {
                    [only] => {
                        painter.circle_filled(
                            *only,
                            line.overlay.width.max(1.0) * 1.2,
                            stroke.color,
                        );
                    }
                    _ => series::stroke_line(painter, &run, stroke, line.overlay.dash),
                }
            }
        }
    }

    /// The last value of each line as a tag: on the axis for a main-scale line (it reads as the
    /// axis does), in the plot's right edge for one on a scale of its own (it has no axis).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn paint_overlay_tags(
        &self,
        axis_painter: &Painter,
        plot_painter: &Painter,
        plot: Rect,
        rect: Rect,
        frame: &OverlayFrame,
        view: &View,
        style: &CanvasStyle,
        last_price_tag: Option<TagBand>,
    ) {
        let half = style.tag_half_height();
        let format = self.appearance.price_format();
        let mode = view.scale.mode;
        let mut axis: Vec<(f32, Color32, String)> = Vec::new();
        let mut inside: Vec<(f32, Color32, String)> = Vec::new();
        for line in frame
            .lines
            .iter()
            .filter(|line| line.drawn() && line.overlay.show_label)
        {
            let Some((_, value)) = line.last() else {
                continue;
            };
            let Some(y) = line.y(value, &view.scale) else {
                continue;
            };
            if !plot.y_range().contains(y) {
                continue;
            }
            let tag = (y, line.color(), line.text(value, mode, format));
            match line.mapping {
                Mapping::Main { .. } => axis.push(tag),
                _ => inside.push(tag),
            }
        }
        for (y, fill, text) in spread(axis, half, plot, last_price_tag) {
            let tag = Rect::from_min_max(
                Pos2::new(plot.right() + 3.0, y - half),
                Pos2::new(rect.right() - 3.0, y + half),
            );
            axis_painter.rect_filled(tag, 3.0, fill);
            price_text::paint(
                axis_painter,
                tag.center(),
                Align2::CENTER_CENTER,
                &text,
                style.price_font(),
                on_color(fill),
            );
        }
        for (y, fill, text) in spread(inside, half, plot, None) {
            let galley =
                plot_painter.layout_no_wrap(text.clone(), style.price_font(), Color32::WHITE);
            let width = galley.size().x + 10.0;
            let tag = Rect::from_min_max(
                Pos2::new(plot.right() - 4.0 - width, y - half),
                Pos2::new(plot.right() - 4.0, y + half),
            );
            plot_painter.rect_filled(tag, 3.0, fill);
            price_text::paint(
                plot_painter,
                tag.center(),
                Align2::CENTER_CENTER,
                &text,
                style.price_font(),
                on_color(fill),
            );
        }
    }

    /// A ring on each listed line at the bar under the crosshair. The card of values is the
    /// overlay card's (`overlay_card.rs`).
    pub(super) fn paint_overlay_rings(
        &self,
        painter: &Painter,
        plot: Rect,
        frame: &OverlayFrame,
        view: &View,
        index: usize,
        x: f32,
    ) {
        for line in frame.lines.iter().filter(|line| line.listed()) {
            if let Some(value) = line.values.get(index).copied().flatten()
                && let Some(y) = line.y(value, &view.scale)
                && plot.contains(Pos2::new(x, y))
            {
                painter.circle(
                    Pos2::new(x, y),
                    3.5,
                    theme::surface(),
                    Stroke::new(2.0, line.color()),
                );
            }
        }
    }
}

/// Where the last price's tag stands on the axis, top to bottom: one row of the price, or two
/// with the countdown under it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct TagBand {
    pub top: f32,
    pub bottom: f32,
}

/// One tag on the axis: where it is centred, what it is filled with, what it says.
type AxisTag = (f32, Color32, String);

/// Pushes tags apart so neighbours do not stack, keeping them in the plot and clear of `avoid`
/// (the last price's tag, which the chart itself draws); top to bottom.
///
/// The room the tags have is the plot less `avoid`: the stretch above it and the stretch below
/// it. A tag keeps the side of `avoid` it was on; one that its side has no room for goes to the
/// other, nearest first, so no tag is ever pushed back over `avoid` by the edge of the plot. Only
/// when there are more tags than the axis holds do they stack over the whole plot, as when there
/// is nothing to avoid.
pub(super) fn spread(
    tags: Vec<AxisTag>,
    half: f32,
    plot: Rect,
    avoid: Option<TagBand>,
) -> Vec<AxisTag> {
    let Some(band) = avoid else {
        return stack(tags, half, plot);
    };
    let above = Span::new(plot.top(), band.top - 1.0);
    let below = Span::new(band.bottom + 1.0, plot.bottom());
    let (above_room, below_room) = (above.capacity(half), below.capacity(half));
    let middle = (band.top + band.bottom) / 2.0;
    let (mut upper, mut lower): (Vec<_>, Vec<_>) = tags.into_iter().partition(|tag| tag.0 < middle);
    // Nearest to the band first on both sides.
    upper.sort_by(|a, b| b.0.total_cmp(&a.0));
    lower.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut over_upper = upper.split_off(above_room.min(upper.len()));
    let mut over_lower = lower.split_off(below_room.min(lower.len()));
    let spill_down = over_upper.len().min(below_room - lower.len());
    let spill_up = over_lower.len().min(above_room - upper.len());
    lower.extend(over_upper.drain(..spill_down));
    upper.extend(over_lower.drain(..spill_up));
    if !over_upper.is_empty() || !over_lower.is_empty() {
        let all = upper
            .into_iter()
            .chain(lower)
            .chain(over_upper)
            .chain(over_lower);
        return stack(all.collect(), half, plot);
    }
    let mut placed = stack(upper, half, above.rect(plot));
    placed.extend(stack(lower, half, below.rect(plot)));
    placed.sort_by(|a, b| a.0.total_cmp(&b.0));
    placed
}

/// A stretch of the axis between two heights.
struct Span {
    top: f32,
    bottom: f32,
}

impl Span {
    fn new(top: f32, bottom: f32) -> Self {
        Self { top, bottom }
    }

    /// How many tags of height `2 * half` fit, one point apart.
    fn capacity(&self, half: f32) -> usize {
        let length = self.bottom - self.top;
        if length < 2.0 * half {
            0
        } else {
            ((length + 1.0) / (2.0 * half + 1.0)).floor() as usize
        }
    }

    /// The plot narrowed to this stretch, which is all `stack` reads of it.
    fn rect(&self, plot: Rect) -> Rect {
        Rect::from_min_max(
            Pos2::new(plot.left(), self.top),
            Pos2::new(plot.right(), self.bottom.max(self.top)),
        )
    }
}

/// Spreads tags that sit closer than a tag is tall, top to bottom, inside the plot: pushed down
/// from the top, then back up from the bottom, so a pile at an edge keeps its spacing instead
/// of collapsing onto one height.
fn stack(mut tags: Vec<AxisTag>, half: f32, plot: Rect) -> Vec<AxisTag> {
    let pitch = 2.0 * half + 1.0;
    let (lowest, highest) = (plot.top() + half, plot.bottom() - half);
    tags.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut floor = lowest;
    for tag in &mut tags {
        tag.0 = tag.0.max(floor);
        floor = tag.0 + pitch;
    }
    let mut ceiling = highest;
    for tag in tags.iter_mut().rev() {
        tag.0 = tag.0.min(ceiling);
        ceiling = tag.0 - pitch;
    }
    // More tags than the plot holds: they overlap, but stay in it.
    for tag in &mut tags {
        tag.0 = tag.0.clamp(lowest, highest.max(lowest));
    }
    tags
}
