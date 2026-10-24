//! How the Fibonacci and Gann drawings look beyond a drawing's stroke and fill: their level lists,
//! what their labels say and the few switches each tool has. Pure, no egui types.
//!
//! A tool's levels are a short list the user edits (add, remove, reorder, restore the defaults), held
//! in a fixed-size array so [`DrawingStyle`](super::DrawingStyle) stays `Copy`. The defaults are
//! TradingView's: the Charting Library's `<Name>LineToolOverrides` pages give each tool's levels,
//! colours and switches (see `docs/fibonacci-gann-tools.md` for what is **lib** and what is
//! **inferred**).

use super::chart_type::{LineDash, Rgba};
use super::drawing_style::{
    HorizontalAlign, MAX_LEVEL_RATIO, MAX_STROKE_WIDTH, MiddleLine, VerticalAlign,
};
use super::drawing_tool::Tool;
use serde::de::Deserializer;
use serde::ser::Serializer;
use serde::{Deserialize, Serialize};

/// The most levels the first list holds (TradingView's retracement has 24 slots).
pub const MAX_LEVELS: usize = 24;
/// The most entries the second and third lists hold.
pub const MAX_SMALL_LEVELS: usize = 12;

const GRAY: Rgba = [120, 123, 134, 255];
const RED: Rgba = [242, 54, 69, 255];
const ORANGE: Rgba = [255, 152, 0, 255];
const GREEN: Rgba = [76, 175, 80, 255];
const TEAL: Rgba = [8, 153, 129, 255];
const CYAN: Rgba = [0, 188, 212, 255];
const BLUE: Rgba = [41, 98, 255, 255];
const PURPLE: Rgba = [156, 39, 176, 255];
const PINK: Rgba = [233, 30, 99, 255];
const VIOLET: Rgba = [103, 58, 183, 255];
const LIGHT_GREEN: Rgba = [129, 199, 132, 255];
const LIGHT_ORANGE: Rgba = [255, 183, 77, 255];
const SALMON: Rgba = [247, 124, 128, 255];
const NAVY: Rgba = [21, 56, 153, 204];

/// One line of a level list: where it sits as a ratio of the construction, and how it is drawn.
/// `alt` is the second number of a Gann arc (`x`, `y`): which corner the arc is centred on.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct FibLevel {
    pub ratio: f32,
    pub enabled: bool,
    pub color: Rgba,
    pub width: f32,
    pub dash: LineDash,
    #[serde(skip_serializing_if = "is_false")]
    pub alt: bool,
}

fn is_false(value: &bool) -> bool {
    !*value
}

impl Default for FibLevel {
    fn default() -> Self {
        Self {
            ratio: 1.0,
            enabled: true,
            color: GRAY,
            width: 2.0,
            dash: LineDash::Solid,
            alt: false,
        }
    }
}

/// An editable list of up to `N` levels, in the order the user keeps them. It stores as a plain
/// list, so a row holds only the levels it has.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LevelSet<const N: usize> {
    len: u8,
    items: [FibLevel; N],
}

impl<const N: usize> Default for LevelSet<N> {
    fn default() -> Self {
        Self {
            len: 0,
            items: [FibLevel::default(); N],
        }
    }
}

impl<const N: usize> LevelSet<N> {
    pub fn as_slice(&self) -> &[FibLevel] {
        &self.items[..usize::from(self.len)]
    }

    pub fn as_mut_slice(&mut self) -> &mut [FibLevel] {
        let len = usize::from(self.len);
        &mut self.items[..len]
    }

    pub fn len(&self) -> usize {
        usize::from(self.len)
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn is_full(&self) -> bool {
        usize::from(self.len) >= N
    }

    /// The levels, truncated to the capacity.
    pub fn from_levels(levels: &[FibLevel]) -> Self {
        let mut set = Self::default();
        for level in levels.iter().take(N) {
            set.push(*level);
        }
        set
    }

    /// Adds a level at the end; `false` when the list is full.
    pub fn push(&mut self, level: FibLevel) -> bool {
        if self.is_full() {
            return false;
        }
        self.items[usize::from(self.len)] = level;
        self.len += 1;
        true
    }

    pub fn remove(&mut self, index: usize) {
        let len = usize::from(self.len);
        if index >= len {
            return;
        }
        self.items.copy_within(index + 1..len, index);
        self.len -= 1;
    }

    /// Moves the level at `index` one place up (`-1`) or down (`1`) the list.
    pub fn shift(&mut self, index: usize, by: i8) {
        let len = usize::from(self.len);
        let target = index as i64 + i64::from(by);
        if index >= len || target < 0 || target >= len as i64 {
            return;
        }
        self.items.swap(index, target as usize);
    }

    /// The enabled levels with a usable ratio, in the list's order.
    pub fn active(&self) -> impl Iterator<Item = &FibLevel> {
        self.as_slice()
            .iter()
            .filter(|level| level.enabled && level.ratio.is_finite())
    }

    /// Every ratio finite and within the range a level may sit at, every width in range, and no
    /// two levels on the same ratio (the later one is dropped): a damaged row cannot break the
    /// renderer or draw one line twice.
    pub fn sanitized(self) -> Self {
        let mut clean = Self::default();
        for level in self.as_slice() {
            let mut level = *level;
            if !level.ratio.is_finite() {
                continue;
            }
            level.ratio = level.ratio.clamp(-MAX_LEVEL_RATIO, MAX_LEVEL_RATIO);
            level.width = if level.width.is_finite() {
                level.width.clamp(0.5, MAX_STROKE_WIDTH)
            } else {
                2.0
            };
            let twin = clean
                .as_slice()
                .iter()
                .any(|kept| kept.ratio == level.ratio && kept.alt == level.alt);
            if !twin {
                clean.push(level);
            }
        }
        clean
    }
}

impl<const N: usize> Serialize for LevelSet<N> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_seq(self.as_slice())
    }
}

impl<'de, const N: usize> Deserialize<'de> for LevelSet<N> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let levels = Vec::<FibLevel>::deserialize(deserializer)?;
        Ok(Self::from_levels(&levels))
    }
}

/// The primary level list: a retracement's 24 slots.
pub type Levels = LevelSet<MAX_LEVELS>;
/// The second and third lists.
pub type SmallLevels = LevelSet<MAX_SMALL_LEVELS>;

/// Which sides of a fan or a box print their level values.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct LabelSides {
    pub left: bool,
    pub right: bool,
    pub top: bool,
    pub bottom: bool,
}

impl Default for LabelSides {
    fn default() -> Self {
        Self {
            left: true,
            right: true,
            top: true,
            bottom: true,
        }
    }
}

/// The look of a Fibonacci or Gann drawing past its stroke and fill.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct FibStyle {
    /// The main levels: a retracement's, a channel's, a circle's, a fan's price levels.
    pub levels: Levels,
    /// A fan's or a box's time levels; a Gann square's fan lines.
    pub second: SmallLevels,
    /// A Gann square's arcs.
    pub third: SmallLevels,
    /// Flip the construction (a retracement's two ends, a fan's direction, a box's corners).
    pub reverse: bool,
    /// Place the levels on a logarithmic chart by the ratio of the prices rather than their
    /// difference. Only a retracement and an extension have it, and only a log chart reads it.
    pub log_levels: bool,
    /// Print each level's ratio.
    pub show_labels: bool,
    /// Print each level's price beside its ratio.
    pub show_prices: bool,
    /// Print ratios as percentages (`61.8%`).
    pub as_percent: bool,
    pub label_h: HorizontalAlign,
    pub label_v: VerticalAlign,
    pub sides: LabelSides,
    /// The line joining the anchors.
    pub trend_line: bool,
    /// A speed arc's missing half.
    pub full_circles: bool,
    /// A spiral's direction.
    pub counterclockwise: bool,
    /// A box's corner rays.
    pub angles: bool,
    /// A Gann square's time range, price range and ratio.
    pub readout: bool,
    /// A fan's grid.
    pub grid: MiddleLine,
    /// A Gann square's price per bar; `0` takes it from the two anchors.
    pub scale: f32,
    /// A box's second fill (the vertical one) and a Gann square's arc fill.
    pub second_fill: bool,
}

impl Default for FibStyle {
    fn default() -> Self {
        Self {
            levels: Levels::default(),
            second: SmallLevels::default(),
            third: SmallLevels::default(),
            reverse: false,
            log_levels: false,
            show_labels: true,
            show_prices: true,
            as_percent: false,
            label_h: HorizontalAlign::Left,
            label_v: VerticalAlign::Bottom,
            sides: LabelSides::default(),
            trend_line: true,
            full_circles: false,
            counterclockwise: false,
            angles: false,
            readout: false,
            grid: MiddleLine {
                enabled: true,
                color: NAVY,
                width: 1.0,
                dash: LineDash::Solid,
            },
            scale: 0.0,
            second_fill: true,
        }
    }
}

/// The most a Gann square's price per bar may be set to.
pub const MAX_SCALE: f32 = 1.0e9;

impl FibStyle {
    /// The style a new `tool` drawing starts with, or `None` for a tool that is not one of these.
    pub fn for_tool(tool: Tool) -> Option<Self> {
        let mut style = Self::default();
        match tool {
            Tool::FibRetracement | Tool::FibExtension => {
                style.levels = retracement_levels();
            }
            Tool::FibChannel => {
                style.levels = retracement_levels();
                style.label_v = VerticalAlign::Middle;
            }
            // The time tools have no prices to print, and their labels sit right of the line, at
            // the bottom of the plot.
            Tool::FibTimeZone => {
                style.levels = time_zone_levels();
                style.label_h = HorizontalAlign::Right;
                style.show_prices = false;
            }
            Tool::FibTrendTime => {
                style.levels = trend_time_levels();
                style.label_h = HorizontalAlign::Right;
                style.show_prices = false;
            }
            Tool::FibSpeedFan => {
                style.levels = fan_levels();
                style.second = fan_levels();
                style.sides = LabelSides::default();
            }
            Tool::FibCircles | Tool::FibSpeedArcs => {
                style.levels = circle_levels();
            }
            Tool::FibWedge => {
                style.levels = wedge_levels();
            }
            Tool::Pitchfan => {
                style.levels = pitchfan_levels();
            }
            // The spiral has no levels, only a line.
            Tool::FibSpiral => {}
            _ => return None,
        }
        Some(style)
    }

    /// The lists and switches in range.
    pub fn sanitized(mut self) -> Self {
        self.levels = self.levels.sanitized();
        self.second = self.second.sanitized();
        self.third = self.third.sanitized();
        self.grid.width = if self.grid.width.is_finite() {
            self.grid.width.clamp(0.5, MAX_STROKE_WIDTH)
        } else {
            1.0
        };
        self.scale = if self.scale.is_finite() {
            self.scale.clamp(0.0, MAX_SCALE)
        } else {
            0.0
        };
        self
    }
}

fn level(ratio: f32, color: Rgba, enabled: bool) -> FibLevel {
    FibLevel {
        ratio,
        enabled,
        color,
        width: 2.0,
        dash: LineDash::Solid,
        alt: false,
    }
}

/// The retracement's, the extension's and the channel's 24 slots: the first eleven on.
pub fn retracement_levels() -> Levels {
    Levels::from_levels(&[
        level(0.0, GRAY, true),
        level(0.236, RED, true),
        level(0.382, ORANGE, true),
        level(0.5, GREEN, true),
        level(0.618, TEAL, true),
        level(0.786, CYAN, true),
        level(1.0, GRAY, true),
        level(1.618, BLUE, true),
        level(2.618, RED, true),
        level(3.618, PURPLE, true),
        level(4.236, PINK, true),
        level(1.272, ORANGE, false),
        level(1.414, RED, false),
        level(2.272, ORANGE, false),
        level(2.414, GREEN, false),
        level(2.0, TEAL, false),
        level(3.0, CYAN, false),
        level(3.272, GRAY, false),
        level(3.414, BLUE, false),
        level(4.0, RED, false),
        level(4.272, PURPLE, false),
        level(4.414, PINK, false),
        level(4.618, ORANGE, false),
        level(4.764, TEAL, false),
    ])
}

/// The speed fan's seven ratios of the box, for each of its two families of rays.
pub fn fan_levels<const N: usize>() -> LevelSet<N> {
    LevelSet::from_levels(&[
        level(0.0, GRAY, true),
        level(0.25, ORANGE, true),
        level(0.382, CYAN, true),
        level(0.5, GREEN, true),
        level(0.618, TEAL, true),
        level(0.75, BLUE, true),
        level(1.0, GRAY, true),
    ])
}

/// The circles' and the speed arcs' eleven radii, all drawn.
pub fn circle_levels() -> Levels {
    Levels::from_levels(&[
        level(0.236, RED, true),
        level(0.382, ORANGE, true),
        level(0.5, TEAL, true),
        level(0.618, GREEN, true),
        level(0.786, CYAN, true),
        level(1.0, GRAY, true),
        level(1.618, BLUE, true),
        level(2.618, PINK, true),
        level(3.618, BLUE, true),
        level(4.236, PINK, true),
        level(4.618, RED, true),
    ])
}

/// The wedge's eleven radii, the first six drawn.
pub fn wedge_levels() -> Levels {
    Levels::from_levels(&[
        level(0.236, RED, true),
        level(0.382, ORANGE, true),
        level(0.5, GREEN, true),
        level(0.618, TEAL, true),
        level(0.786, CYAN, true),
        level(1.0, GRAY, true),
        level(1.618, BLUE, false),
        level(2.618, RED, false),
        level(3.618, VIOLET, false),
        level(4.236, PINK, false),
        level(4.618, PINK, false),
    ])
}

/// The pitchfan's nine rays on either side of its median, two drawn.
pub fn pitchfan_levels() -> Levels {
    Levels::from_levels(&[
        level(0.25, LIGHT_ORANGE, false),
        level(0.382, LIGHT_GREEN, false),
        level(0.5, CYAN, true),
        level(0.618, TEAL, false),
        level(0.75, CYAN, false),
        level(1.0, BLUE, true),
        level(1.5, PURPLE, false),
        level(1.75, PINK, false),
        level(2.0, SALMON, false),
    ])
}

/// The time zone's eleven lines: the first in grey, the rest in blue.
pub fn time_zone_levels() -> Levels {
    let multiples = [0.0, 1.0, 2.0, 3.0, 5.0, 8.0, 13.0, 21.0, 34.0, 55.0, 89.0];
    Levels::from_levels(
        &multiples.map(|multiple| level(multiple, if multiple == 0.0 { GRAY } else { BLUE }, true)),
    )
}

/// The trend-based time's eleven lines, the half off.
pub fn trend_time_levels() -> Levels {
    Levels::from_levels(&[
        level(0.0, GRAY, true),
        level(0.382, RED, true),
        level(0.5, LIGHT_GREEN, false),
        level(0.618, GREEN, true),
        level(1.0, TEAL, true),
        level(1.382, CYAN, true),
        level(1.618, GRAY, true),
        level(2.0, BLUE, true),
        level(2.382, PINK, true),
        level(2.618, PURPLE, true),
        level(3.0, VIOLET, true),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_level_set_adds_removes_and_reorders() {
        let mut set = SmallLevels::default();
        for ratio in [1.0, 2.0, 3.0] {
            assert!(set.push(level(ratio, GRAY, true)));
        }
        set.shift(0, 1);
        let ratios: Vec<f32> = set.as_slice().iter().map(|l| l.ratio).collect();
        assert_eq!(ratios, [2.0, 1.0, 3.0]);
        set.remove(1);
        let ratios: Vec<f32> = set.as_slice().iter().map(|l| l.ratio).collect();
        assert_eq!(ratios, [2.0, 3.0]);
        set.shift(1, 1);
        set.shift(0, -1);
        assert_eq!(set.len(), 2, "moves past either end do nothing");
    }

    #[test]
    fn a_full_set_refuses_another_level() {
        let mut set = SmallLevels::default();
        for index in 0..MAX_SMALL_LEVELS {
            assert!(set.push(level(index as f32, GRAY, true)));
        }
        assert!(set.is_full());
        assert!(!set.push(level(99.0, GRAY, true)));
    }

    #[test]
    fn a_level_set_stores_as_a_plain_list() {
        let set = SmallLevels::from_levels(&[level(0.5, GREEN, true), level(1.0, GRAY, false)]);
        let json = serde_json::to_value(set).unwrap();
        assert_eq!(json.as_array().map(Vec::len), Some(2));
        let back: SmallLevels = serde_json::from_value(json).unwrap();
        assert_eq!(back, set);
    }

    #[test]
    fn a_stored_list_longer_than_the_set_is_cut_to_it() {
        let rows: Vec<FibLevel> = (0..40).map(|i| level(i as f32, GRAY, true)).collect();
        let set: SmallLevels = serde_json::from_value(serde_json::to_value(rows).unwrap()).unwrap();
        assert_eq!(set.len(), MAX_SMALL_LEVELS);
    }

    #[test]
    fn sanitizing_drops_bad_and_repeated_ratios_and_clamps_the_rest() {
        let mut bad = level(f32::NAN, GRAY, true);
        bad.width = 5.0;
        let set = SmallLevels::from_levels(&[
            bad,
            level(0.5, GRAY, true),
            level(0.5, RED, true),
            level(1.0e9, GRAY, true),
            FibLevel {
                width: f32::INFINITY,
                ratio: 2.0,
                ..FibLevel::default()
            },
        ])
        .sanitized();
        let ratios: Vec<f32> = set.as_slice().iter().map(|l| l.ratio).collect();
        assert_eq!(ratios, [0.5, MAX_LEVEL_RATIO, 2.0]);
        assert_eq!(
            set.as_slice()[0].color,
            GRAY,
            "the first of two twins stays"
        );
        assert_eq!(set.as_slice()[2].width, 2.0);
    }

    #[test]
    fn the_time_tools_default_to_the_librarys_levels() {
        let zone: Vec<f32> = time_zone_levels().active().map(|l| l.ratio).collect();
        assert_eq!(
            zone,
            [0.0, 1.0, 2.0, 3.0, 5.0, 8.0, 13.0, 21.0, 34.0, 55.0, 89.0]
        );
        let trend = trend_time_levels();
        assert_eq!(trend.len(), 11);
        assert_eq!(trend.active().count(), 10, "the 0.5 is off");
        // Each tool's own style starts with its own levels.
        assert_eq!(
            FibStyle::for_tool(Tool::FibTimeZone).unwrap().levels,
            time_zone_levels()
        );
        assert_eq!(
            FibStyle::for_tool(Tool::FibTrendTime).unwrap().levels,
            trend_time_levels()
        );
        assert_eq!(
            FibStyle::for_tool(Tool::FibChannel).unwrap().label_v,
            VerticalAlign::Middle
        );
        assert!(FibStyle::for_tool(Tool::Rectangle).is_none());
    }

    #[test]
    fn the_other_tools_default_to_the_librarys_levels() {
        let ratios = |levels: &[FibLevel]| -> Vec<f32> { levels.iter().map(|l| l.ratio).collect() };
        assert_eq!(
            ratios(fan_levels::<12>().as_slice()),
            [0.0, 0.25, 0.382, 0.5, 0.618, 0.75, 1.0]
        );
        let circles = circle_levels();
        assert_eq!(circles.len(), 11);
        assert_eq!(circles.active().count(), 11);
        assert_eq!(wedge_levels().active().count(), 6);
        let fan = pitchfan_levels();
        assert_eq!(fan.len(), 9);
        let on: Vec<f32> = fan.active().map(|l| l.ratio).collect();
        assert_eq!(on, [0.5, 1.0]);
        // The fan has both its families, and the spiral none.
        let speed = FibStyle::for_tool(Tool::FibSpeedFan).unwrap();
        assert_eq!(speed.levels.len(), 7);
        assert_eq!(speed.second.len(), 7);
        assert!(
            FibStyle::for_tool(Tool::FibSpiral)
                .unwrap()
                .levels
                .is_empty()
        );
    }

    #[test]
    fn the_retracement_defaults_are_the_libraries_twenty_four_slots() {
        let levels = retracement_levels();
        assert_eq!(levels.len(), 24);
        assert_eq!(levels.active().count(), 11);
        let ratios: Vec<f32> = levels.active().map(|l| l.ratio).collect();
        assert_eq!(
            ratios,
            [
                0.0, 0.236, 0.382, 0.5, 0.618, 0.786, 1.0, 1.618, 2.618, 3.618, 4.236
            ]
        );
    }
}
