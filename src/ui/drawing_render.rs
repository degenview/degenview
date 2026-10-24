//! Painter helpers for chart drawings. Everything here works in screen space
//! (`Pos2`) so it is independent of the drawing model: `chart_card` projects
//! anchors through its time/price scales and hands the results to these helpers.

use crate::model::{Anchor, Kline, PriceFormat};
use crate::ui::theme;
use eframe::egui::{self, Align2, Color32, FontId, Painter, Pos2, Rect, Stroke, StrokeKind, Vec2};

pub const HANDLE_RADIUS: f32 = 4.5;
/// Pointer distance (px) within which a drawing counts as hovered.
pub const HIT_TOLERANCE: f32 = 6.0;

/// Which part of a two-corner ruler the pointer is over.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RulerPart {
    Corner(RulerCorner),
    Edge,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RulerCorner {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

impl RulerCorner {
    pub const ALL: [RulerCorner; 4] = [
        RulerCorner::TopLeft,
        RulerCorner::TopRight,
        RulerCorner::BottomLeft,
        RulerCorner::BottomRight,
    ];

    pub fn is_left(self) -> bool {
        matches!(self, Self::TopLeft | Self::BottomLeft)
    }

    pub fn is_top(self) -> bool {
        matches!(self, Self::TopLeft | Self::TopRight)
    }

    pub fn position(self, rect: Rect) -> Pos2 {
        Pos2::new(
            if self.is_left() {
                rect.left()
            } else {
                rect.right()
            },
            if self.is_top() {
                rect.top()
            } else {
                rect.bottom()
            },
        )
    }
}

/// What a ruler measures. Pure data so the numbers are testable without a painter.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RulerReadout {
    pub price_delta: f64,
    pub percent: f64,
    pub bars: usize,
    pub span_secs: i64,
}

impl RulerReadout {
    /// `start`/`end` are the anchors in drawing order; `bars` is inclusive of both ends.
    pub fn new(start_price: f64, end_price: f64, bars: usize, span_secs: i64) -> Self {
        let price_delta = end_price - start_price;
        let percent = if start_price != 0.0 {
            price_delta / start_price * 100.0
        } else {
            0.0
        };
        Self {
            price_delta,
            percent,
            bars,
            span_secs: span_secs.abs(),
        }
    }

    /// A flat move counts as up, like the Swift card header's 0% change.
    pub fn is_upward(&self) -> bool {
        self.price_delta >= 0.0
    }

    fn sign(&self) -> &'static str {
        if self.price_delta < 0.0 { "-" } else { "+" }
    }

    pub fn percent_text(&self) -> String {
        format!("{}{:.2}%", self.sign(), self.percent.abs())
    }

    pub fn price_text(&self, format: PriceFormat) -> String {
        format!("{}{}", self.sign(), format.format(self.price_delta.abs()))
    }

    pub fn bars_text(&self) -> String {
        if self.bars == 1 {
            "1 bar".into()
        } else {
            format!("{} bars", self.bars)
        }
    }

    /// Empty under a minute, which the smallest candle still outlasts.
    pub fn duration_text(&self) -> String {
        duration(self.span_secs)
    }

    /// "23 bars · 1d 14h", or just the bars when there is no duration to report.
    pub fn coverage_text(&self) -> String {
        let duration = self.duration_text();
        if duration.is_empty() {
            self.bars_text()
        } else {
            format!("{} · {duration}", self.bars_text())
        }
    }
}

/// What an arrow measures between its two ends: the ruler's numbers plus the signed period
/// count and the on-screen angle. Pure, so the text is tested without a painter.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ArrowReadout {
    pub ruler: RulerReadout,
    /// The signed number of bars from the first end to the second (negative when it points back).
    pub periods: i64,
    /// The arrow's direction on screen in degrees, counter-clockwise from the +x axis
    /// (so an arrow rising to the right reads positive). This is the visual angle; TradingView
    /// does not document its own formula.
    pub angle_deg: f32,
}

impl ArrowReadout {
    pub fn new(
        start_price: f64,
        end_price: f64,
        bars: usize,
        periods: i64,
        span_secs: i64,
        start: Pos2,
        end: Pos2,
    ) -> Self {
        let delta = end - start;
        let angle_deg = if delta.length() > 1e-3 {
            (-delta.y).atan2(delta.x).to_degrees()
        } else {
            0.0
        };
        Self {
            ruler: RulerReadout::new(start_price, end_price, bars, span_secs),
            periods,
            angle_deg,
        }
    }

    /// The text lines for the enabled statistics, price and percent first.
    pub fn lines(
        &self,
        stats: &crate::model::drawing_style::ArrowStats,
        format: PriceFormat,
    ) -> Vec<String> {
        let mut lines = Vec::new();
        match (stats.price_range, stats.percent) {
            (true, true) => lines.push(format!(
                "{} ({})",
                self.ruler.price_text(format),
                self.ruler.percent_text()
            )),
            (true, false) => lines.push(self.ruler.price_text(format)),
            (false, true) => lines.push(self.ruler.percent_text()),
            (false, false) => {}
        }
        let mut span = Vec::new();
        if stats.bars {
            span.push(self.ruler.bars_text());
        }
        if stats.time {
            let duration = self.ruler.duration_text();
            if !duration.is_empty() {
                span.push(duration);
            }
        }
        if !span.is_empty() {
            lines.push(span.join(" · "));
        }
        let mut shape = Vec::new();
        if stats.distance {
            shape.push(format!("{:+} periods", self.periods));
        }
        if stats.angle {
            shape.push(format!("{:.0}°", self.angle_deg));
        }
        if !shape.is_empty() {
            lines.push(shape.join(" · "));
        }
        lines
    }
}

/// "1d 14h", "3h 5m", "42m"; empty below one minute.
pub fn duration(secs: i64) -> String {
    let minutes = secs / 60;
    if minutes < 1 {
        return String::new();
    }
    let (days, hours, mins) = (minutes / 1440, (minutes / 60) % 24, minutes % 60);
    match (days, hours, mins) {
        (0, 0, m) => format!("{m}m"),
        (0, h, 0) => format!("{h}h"),
        (0, h, m) => format!("{h}h {m}m"),
        (d, 0, _) => format!("{d}d"),
        (d, h, _) => format!("{d}d {h}h"),
    }
}

/// Maps candle open times to a continuous index and back. Between candles it
/// interpolates linearly; outside the data it extrapolates at the average interval,
/// so a drawing anchored in the future or before the first bar still has a position.
pub struct TimeIndex<'a> {
    candles: &'a [Kline],
    interval: f64,
}

impl<'a> TimeIndex<'a> {
    pub fn new(candles: &'a [Kline]) -> Self {
        let interval = match (candles.first(), candles.last()) {
            (Some(first), Some(last)) if candles.len() > 1 && last.open_time > first.open_time => {
                (last.open_time - first.open_time) as f64 / (candles.len() - 1) as f64
            }
            _ => 60_000.0,
        };
        Self { candles, interval }
    }

    pub fn len(&self) -> usize {
        self.candles.len()
    }

    /// The candle at `index`, if there is one.
    pub fn bar(&self, index: usize) -> Option<&'a Kline> {
        self.candles.get(index)
    }

    /// Every bar the chart draws (constructed, and cut at the replay cursor).
    pub fn bars(&self) -> &'a [Kline] {
        self.candles
    }

    /// Fractional candle index of `time_ms` (candle `i` sits at integer `i`).
    pub fn index_of(&self, time_ms: i64) -> f64 {
        let (Some(first), Some(last)) = (self.candles.first(), self.candles.last()) else {
            return 0.0;
        };
        let count = self.candles.len();
        let taken = self
            .candles
            .partition_point(|candle| candle.open_time <= time_ms);
        if taken == 0 {
            (time_ms - first.open_time) as f64 / self.interval
        } else if taken == count {
            (count - 1) as f64 + (time_ms - last.open_time) as f64 / self.interval
        } else {
            let (lower, upper) = (&self.candles[taken - 1], &self.candles[taken]);
            let width = (upper.open_time - lower.open_time).max(1) as f64;
            (taken - 1) as f64 + (time_ms - lower.open_time) as f64 / width
        }
    }

    /// Inverse of `index_of`.
    pub fn time_at(&self, index: f64) -> i64 {
        let (Some(first), Some(last)) = (self.candles.first(), self.candles.last()) else {
            return 0;
        };
        let top = (self.candles.len() - 1) as f64;
        if index <= 0.0 {
            first.open_time + (index * self.interval).round() as i64
        } else if index >= top {
            last.open_time + ((index - top) * self.interval).round() as i64
        } else {
            let lower = index.floor() as usize;
            let (a, b) = (&self.candles[lower], &self.candles[lower + 1]);
            a.open_time
                + ((index - lower as f64) * (b.open_time - a.open_time) as f64).round() as i64
        }
    }

    /// Candles covered by a span between two times, inclusive of both ends.
    pub fn bars_between(&self, first_ms: i64, second_ms: i64) -> usize {
        let a = self.index_of(first_ms).round();
        let b = self.index_of(second_ms).round();
        (b - a).abs() as usize + 1
    }
}

/// Ruler anchors with `corner` dragged to `to`, the opposite corner left in place.
/// Each axis is written to whichever stored anchor owns that edge, so the stored
/// start/end — and with them the measured direction — keep their meaning, and
/// dragging a corner past the opposite one flips the box without a special case.
pub fn resize_ruler(a: Anchor, b: Anchor, corner: RulerCorner, to: Anchor) -> (Anchor, Anchor) {
    let (mut a, mut b) = (a, b);
    if corner.is_left() == (a.time_ms <= b.time_ms) {
        a.time_ms = to.time_ms;
    } else {
        b.time_ms = to.time_ms;
    }
    if corner.is_top() == (a.price >= b.price) {
        a.price = to.price;
    } else {
        b.price = to.price;
    }
    (a, b)
}

fn with_alpha(color: Color32, alpha: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), alpha)
}

fn draw_handle(painter: &Painter, at: Pos2, color: Color32) {
    painter.circle(at, HANDLE_RADIUS, theme::surface(), Stroke::new(1.5, color));
}

/// A ruler rectangle with its readout card. Green when measured upward, red downward.
pub fn draw_ruler(
    painter: &Painter,
    plot: Rect,
    rect: Rect,
    readout: &RulerReadout,
    format: PriceFormat,
    emphasized: bool,
    dashed_draft: bool,
) {
    let color = if readout.is_upward() {
        theme::UP
    } else {
        theme::DOWN
    };
    let clipped = painter.with_clip_rect(plot);
    clipped.rect_filled(rect, 0.0, with_alpha(color, 36));
    if dashed_draft {
        let corners = [
            rect.left_top(),
            rect.right_top(),
            rect.right_bottom(),
            rect.left_bottom(),
            rect.left_top(),
        ];
        clipped.add(egui::Shape::dashed_line(
            &corners,
            Stroke::new(1.0, color),
            4.0,
            3.0,
        ));
    } else {
        clipped.rect_stroke(rect, 0.0, Stroke::new(1.0, color), StrokeKind::Inside);
    }
    if emphasized {
        for corner in RulerCorner::ALL {
            draw_handle(&clipped, corner.position(rect), color);
        }
    }

    // Readout card: centred under the box (above it when there is no room).
    let lines = [
        format!(
            "{} ({})",
            readout.percent_text(),
            readout.price_text(format)
        ),
        readout.coverage_text(),
    ];
    let card_size = Vec2::new(150.0, 38.0);
    let mut top = rect.bottom() + 6.0;
    if top + card_size.y > plot.bottom() {
        top = (rect.top() - 6.0 - card_size.y).max(plot.top());
    }
    let left = (rect.center().x - card_size.x / 2.0)
        .clamp(plot.left(), (plot.right() - card_size.x).max(plot.left()));
    let card = Rect::from_min_size(Pos2::new(left, top), card_size);
    painter.rect(
        card,
        4.0,
        theme::chrome().gamma_multiply(0.95),
        Stroke::new(1.0, color),
        StrokeKind::Inside,
    );
    painter.text(
        card.center_top() + Vec2::new(0.0, 12.0),
        Align2::CENTER_CENTER,
        &lines[0],
        FontId::monospace(10.0),
        color,
    );
    painter.text(
        card.center_bottom() - Vec2::new(0.0, 11.0),
        Align2::CENTER_CENTER,
        &lines[1],
        FontId::monospace(9.0),
        theme::text_secondary(),
    );
}

/// Hit-test a ruler rectangle: corners resize, edges move, the interior is not a
/// target so a new measurement can start inside an old one.
pub fn hit_ruler(rect: Rect, pointer: Pos2) -> Option<RulerPart> {
    for corner in RulerCorner::ALL {
        if corner.position(rect).distance(pointer) <= HIT_TOLERANCE + 2.0 {
            return Some(RulerPart::Corner(corner));
        }
    }
    let outer = rect.expand(HIT_TOLERANCE);
    let inner = rect.shrink(HIT_TOLERANCE);
    (outer.contains(pointer) && !inner.contains(pointer)).then_some(RulerPart::Edge)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readout_up_and_down() {
        let up = RulerReadout::new(100.0, 104.23, 10, 3600);
        assert!(up.is_upward());
        assert_eq!(up.percent_text(), "+4.23%");
        let down = RulerReadout::new(100.0, 90.0, 1, 30);
        assert!(!down.is_upward());
        assert_eq!(down.percent_text(), "-10.00%");
        assert_eq!(down.coverage_text(), "1 bar");
    }

    #[test]
    fn the_ruler_writes_its_price_as_the_chart_does() {
        use crate::model::SmallPrices;
        let tiny = RulerReadout::new(0.000_001, 0.000_003_78, 3, 60);
        assert_eq!(tiny.price_text(PriceFormat::AUTO), "+0.0₅278");
        let written = PriceFormat {
            decimals: None,
            small: SmallPrices::Decimals,
        };
        assert_eq!(tiny.price_text(written), "+0.00000278");
        let down = RulerReadout::new(67_432.19, 65_000.0, 3, 60);
        assert_eq!(down.price_text(PriceFormat::AUTO), "-2,432");
        let pinned = PriceFormat {
            decimals: Some(2),
            small: SmallPrices::ZeroCount,
        };
        assert_eq!(down.price_text(pinned), "-2,432.19");
    }

    #[test]
    fn an_arrow_reads_out_only_the_statistics_that_are_on() {
        use crate::model::drawing_style::ArrowStats;
        let up = ArrowReadout::new(
            100.0,
            110.0,
            11,
            10,
            36_000,
            Pos2::new(0.0, 100.0),
            Pos2::new(100.0, 0.0),
        );
        assert!((up.angle_deg - 45.0).abs() < 1e-3);
        assert!(
            up.lines(&ArrowStats::default(), PriceFormat::AUTO)
                .is_empty()
        );
        let all = ArrowStats {
            price_range: true,
            percent: true,
            bars: true,
            time: true,
            distance: true,
            angle: true,
            ..ArrowStats::default()
        };
        assert_eq!(
            up.lines(&all, PriceFormat::AUTO),
            vec![
                "+10.00 (+10.00%)".to_owned(),
                "11 bars · 10h".to_owned(),
                "+10 periods · 45°".to_owned()
            ]
        );
        let only_percent = ArrowStats {
            percent: true,
            ..ArrowStats::default()
        };
        assert_eq!(
            up.lines(&only_percent, PriceFormat::AUTO),
            vec!["+10.00%".to_owned()]
        );
        // Pointing back and down: negative periods, an angle in the third quadrant.
        let back = ArrowReadout::new(
            110.0,
            100.0,
            3,
            -2,
            30,
            Pos2::new(100.0, 0.0),
            Pos2::new(0.0, 100.0),
        );
        assert!((back.angle_deg + 135.0).abs() < 1e-3);
        let lines = back.lines(&all, PriceFormat::AUTO);
        assert!(lines[0].starts_with("-10"), "{lines:?}");
        assert!(
            lines[1] == "3 bars",
            "under a minute shows no duration: {lines:?}"
        );
        assert!(lines[2].starts_with("-2 periods"), "{lines:?}");
        // A zero-length arrow has a defined angle.
        let dot = ArrowReadout::new(1.0, 1.0, 1, 0, 0, Pos2::ZERO, Pos2::ZERO);
        assert_eq!(dot.angle_deg, 0.0);
    }

    #[test]
    fn flat_move_is_upward_and_signed() {
        let flat = RulerReadout::new(100.0, 100.0, 2, 0);
        assert!(flat.is_upward());
        assert_eq!(flat.percent_text(), "+0.00%");
    }

    #[test]
    fn zero_start_has_zero_percent() {
        assert_eq!(RulerReadout::new(0.0, 5.0, 1, 0).percent, 0.0);
    }

    #[test]
    fn durations() {
        assert_eq!(duration(59), "");
        assert_eq!(duration(42 * 60), "42m");
        assert_eq!(duration(3 * 3600 + 5 * 60), "3h 5m");
        assert_eq!(duration(3 * 3600), "3h");
        assert_eq!(duration(86_400 + 14 * 3600), "1d 14h");
        assert_eq!(duration(2 * 86_400), "2d");
    }

    #[test]
    fn coverage_joins_bars_and_duration() {
        let readout = RulerReadout::new(1.0, 2.0, 23, 86_400 + 14 * 3600);
        assert_eq!(readout.coverage_text(), "23 bars · 1d 14h");
    }

    fn candles(times: &[i64]) -> Vec<Kline> {
        times
            .iter()
            .map(|time| Kline {
                open_time: *time,
                close_time: *time + 59_999,
                open: 1.0,
                high: 1.0,
                low: 1.0,
                close: 1.0,
                volume: 0.0,
            })
            .collect()
    }

    #[test]
    fn time_index_round_trips_inside_and_outside() {
        let data = candles(&[1_000, 2_000, 3_000, 4_000]);
        let index = TimeIndex::new(&data);
        assert_eq!(index.index_of(1_000), 0.0);
        assert_eq!(index.index_of(3_500), 2.5);
        assert_eq!(index.index_of(5_000), 4.0);
        assert_eq!(index.index_of(0), -1.0);
        for time in [0, 1_000, 2_250, 4_000, 6_000] {
            assert_eq!(index.time_at(index.index_of(time)), time);
        }
    }

    #[test]
    fn bars_between_is_inclusive() {
        let data = candles(&[0, 1_000, 2_000, 3_000]);
        let index = TimeIndex::new(&data);
        assert_eq!(index.bars_between(1_000, 2_000), 2);
        assert_eq!(index.bars_between(2_000, 2_000), 1);
        assert_eq!(index.bars_between(3_000, 0), 4);
    }

    #[test]
    fn time_index_survives_empty_and_single() {
        assert_eq!(TimeIndex::new(&[]).index_of(5), 0.0);
        let one = candles(&[10_000]);
        let index = TimeIndex::new(&one);
        assert_eq!(index.index_of(10_000), 0.0);
        assert_eq!(index.time_at(1.0), 70_000);
    }

    #[test]
    fn resize_ruler_flips_past_opposite_corner() {
        let a = Anchor {
            time_ms: 100,
            price: 10.0,
        };
        let b = Anchor {
            time_ms: 200,
            price: 20.0,
        };
        // Drag the bottom-left (earliest, lowest) corner left and down.
        let (a2, b2) = resize_ruler(
            a,
            b,
            RulerCorner::BottomLeft,
            Anchor {
                time_ms: 50,
                price: 5.0,
            },
        );
        assert_eq!((a2.time_ms, a2.price), (50, 5.0));
        assert_eq!((b2.time_ms, b2.price), (200, 20.0));
        // Drag the same corner past the top-right: the anchor that owned it moves, the
        // other stays, and the box now reads as flipped.
        let (a3, b3) = resize_ruler(
            a,
            b,
            RulerCorner::BottomLeft,
            Anchor {
                time_ms: 300,
                price: 30.0,
            },
        );
        assert_eq!((a3.time_ms, a3.price), (300, 30.0));
        assert_eq!((b3.time_ms, b3.price), (200, 20.0));
    }

    #[test]
    fn ruler_hit_prefers_corners_and_skips_interior() {
        let rect = Rect::from_min_max(Pos2::new(10.0, 10.0), Pos2::new(110.0, 60.0));
        assert_eq!(
            hit_ruler(rect, Pos2::new(11.0, 11.0)),
            Some(RulerPart::Corner(RulerCorner::TopLeft))
        );
        assert_eq!(
            hit_ruler(rect, Pos2::new(60.0, 10.0)),
            Some(RulerPart::Edge)
        );
        assert_eq!(hit_ruler(rect, Pos2::new(60.0, 35.0)), None);
    }
}
