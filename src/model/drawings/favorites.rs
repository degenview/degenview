//! What a user starred: drawing tools and cursors, and where their floating bar sits.
//!
//! The list is app-wide: one ordered set, in the order it was starred. It is kept as stored names
//! (a tool's variant name, which `drawing.prefs` and `drawing.defaults` already use; a cursor's as
//! `cursor:` and its name), so a name a newer build wrote and this one does not know is kept when
//! the list is written back and is only left out of what is shown. An older build reads a cursor's
//! name as one it does not know and keeps it the same way. The bar's place is a fraction of the
//! room the bar has to move in, so it follows a resized window instead of drifting off it.

use super::tool::Tool;
use crate::model::CursorMode;
use serde::{Deserialize, Deserializer, Serialize};

/// Something that can be starred: a drawing tool with a menu row, or a cursor.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Favorite {
    Tool(Tool),
    Cursor(CursorMode),
}

impl From<Tool> for Favorite {
    fn from(tool: Tool) -> Self {
        Self::Tool(tool)
    }
}

impl Favorite {
    /// Stored names of cursors start with this; no tool's name contains a colon.
    const CURSOR_PREFIX: &'static str = "cursor:";

    /// The stored name, or `None` for a tool that has no menu row to star it from.
    fn name(self) -> Option<String> {
        match self {
            Self::Tool(tool) => Tool::GROUPED.contains(&tool).then(|| format!("{tool:?}")),
            Self::Cursor(mode) => Some(format!("{}{mode:?}", Self::CURSOR_PREFIX)),
        }
    }

    fn named(name: &str) -> Option<Self> {
        match name.strip_prefix(Self::CURSOR_PREFIX) {
            Some(cursor) => CursorMode::ALL
                .into_iter()
                .find(|mode| format!("{mode:?}") == cursor)
                .map(Self::Cursor),
            None => Tool::GROUPED
                .iter()
                .copied()
                .find(|tool| format!("{tool:?}") == name)
                .map(Self::Tool),
        }
    }
}

/// The starred items, as stored.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct FavoriteTools {
    names: Vec<String>,
}

impl FavoriteTools {
    /// The starred items this build knows, in the order they were starred, each once.
    pub fn items(&self) -> Vec<Favorite> {
        let mut items: Vec<Favorite> = Vec::new();
        for item in self.names.iter().filter_map(|name| Favorite::named(name)) {
            if !items.contains(&item) {
                items.push(item);
            }
        }
        items
    }

    /// Stars `item`, or takes its star away when it has one. Returns whether it is starred now.
    /// A tool with no menu row cannot be starred and stays out. Names this build does not know are
    /// left where they are.
    pub fn toggle(&mut self, item: impl Into<Favorite>) -> bool {
        let Some(name) = item.into().name() else {
            return false;
        };
        if self.names.contains(&name) {
            self.names.retain(|saved| *saved != name);
            false
        } else {
            self.names.push(name);
            true
        }
    }
}

/// A rectangle the bar is placed in: its top-left corner and its size.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Region {
    pub origin: [f32; 2],
    pub size: [f32; 2],
}

/// Where the bar sits, as the share of its room (0 at the top-left corner, 1 at the bottom-right
/// one) that lies before it. Whatever the window's size, the whole bar is inside the room.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct BarPlacement {
    pub x: f32,
    pub y: f32,
}

/// The bar's remembered state on this device.
///
/// A field this build cannot read falls back to its default and the others are kept.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct BarPrefs {
    #[serde(deserialize_with = "placement_or_default")]
    pub placement: Option<BarPlacement>,
    /// Whether `placement` is a share of the whole window. A place saved before the bar could
    /// leave the charts is a share of the charts' area and has this off, so it comes back where
    /// it was left; the next move saves a window place.
    #[serde(deserialize_with = "flag_or_default")]
    pub window: bool,
    #[serde(deserialize_with = "flag_or_default")]
    pub hidden: bool,
}

fn placement_or_default<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<BarPlacement>, D::Error> {
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(serde_json::from_value(value).unwrap_or_default())
}

fn flag_or_default<'de, D: Deserializer<'de>>(deserializer: D) -> Result<bool, D::Error> {
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(serde_json::from_value(value).unwrap_or_default())
}

/// Space kept between the bar and the top of the chart area when it has not been moved.
pub const DEFAULT_TOP_GAP: f32 = 12.0;

impl BarPrefs {
    /// Whether the bar is placed in the whole window: a place saved by this build. A flag with no
    /// readable place is not one, so the bar is where an unmoved bar is, in the charts.
    pub fn in_window(&self) -> bool {
        self.window && self.placement.is_some()
    }

    /// The top-left corner of a bar of `size`, as `[x, y]`. Unmoved, the bar is centred near the
    /// top of the `chart` area; moved, it is where the share of its room puts it, the whole
    /// `window` for a place saved by this build and the chart area for an older one. A bar bigger
    /// than its room sits at the room's corner.
    pub fn resolve(&self, chart: Region, window: Region, size: [f32; 2]) -> [f32; 2] {
        let room = if self.in_window() { window } else { chart };
        let free = [
            (room.size[0] - size[0]).max(0.0),
            (room.size[1] - size[1]).max(0.0),
        ];
        let (fx, y) = match self.placement {
            Some(place) => (place.x, place.y.clamp(0.0, 1.0) * free[1]),
            None => (0.5, DEFAULT_TOP_GAP.min(free[1])),
        };
        [
            room.origin[0] + fx.clamp(0.0, 1.0) * free[0],
            room.origin[1] + y,
        ]
    }

    /// These prefs with the bar left with its top-left corner at `position` in the `window`.
    pub fn moved_to(self, position: [f32; 2], window: Region, size: [f32; 2]) -> Self {
        Self {
            placement: Some(placement_from(position, window.origin, window.size, size)),
            window: true,
            ..self
        }
    }
}

/// The placement that puts a bar of `size` with its top-left corner at `position` (clamped
/// into the area).
pub fn placement_from(
    position: [f32; 2],
    origin: [f32; 2],
    area: [f32; 2],
    size: [f32; 2],
) -> BarPlacement {
    let share = |at: f32, from: f32, span: f32, bar: f32| {
        let room = span - bar;
        if room <= 0.0 {
            0.0
        } else {
            ((at - from) / room).clamp(0.0, 1.0)
        }
    };
    BarPlacement {
        x: share(position[0], origin[0], area[0], size[0]),
        y: share(position[1], origin[1], area[1], size[1]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tool_is_starred_once_and_in_the_order_it_was_starred() {
        let mut favorites = FavoriteTools::default();
        assert!(favorites.items().is_empty());
        assert!(favorites.toggle(Tool::Brush));
        assert!(favorites.toggle(Tool::TrendLine));
        assert!(favorites.toggle(Tool::Highlighter));
        assert_eq!(
            favorites.items(),
            tools(&[Tool::Brush, Tool::TrendLine, Tool::Highlighter])
        );
        assert!(!favorites.toggle(Tool::TrendLine));
        assert_eq!(favorites.items(), tools(&[Tool::Brush, Tool::Highlighter]));
        // Starred again, it goes to the end.
        assert!(favorites.toggle(Tool::TrendLine));
        assert_eq!(
            favorites.items(),
            tools(&[Tool::Brush, Tool::Highlighter, Tool::TrendLine])
        );
    }

    fn tools(tools: &[Tool]) -> Vec<Favorite> {
        tools.iter().copied().map(Favorite::Tool).collect()
    }

    #[test]
    fn a_cursor_is_starred_beside_the_tools_in_the_same_order() {
        let mut favorites = FavoriteTools::default();
        assert!(favorites.toggle(Tool::Brush));
        assert!(favorites.toggle(Favorite::Cursor(CursorMode::Eraser)));
        assert!(favorites.toggle(Favorite::Cursor(CursorMode::Cross)));
        assert_eq!(
            favorites.items(),
            [
                Favorite::Tool(Tool::Brush),
                Favorite::Cursor(CursorMode::Eraser),
                Favorite::Cursor(CursorMode::Cross)
            ]
        );
        assert!(!favorites.toggle(Favorite::Cursor(CursorMode::Eraser)));
        assert_eq!(favorites.items().len(), 2);
        assert_eq!(
            serde_json::to_string(&favorites).unwrap(),
            r#"["Brush","cursor:Cross"]"#
        );
    }

    #[test]
    fn a_cursor_name_this_build_does_not_know_is_hidden_but_kept() {
        let mut favorites: FavoriteTools =
            serde_json::from_str(r#"["cursor:Magic","cursor:Dot","Brush"]"#).unwrap();
        assert_eq!(
            favorites.items(),
            [
                Favorite::Cursor(CursorMode::Dot),
                Favorite::Tool(Tool::Brush)
            ]
        );
        favorites.toggle(Tool::Circle);
        assert_eq!(
            serde_json::to_string(&favorites).unwrap(),
            r#"["cursor:Magic","cursor:Dot","Brush","Circle"]"#
        );
    }

    #[test]
    fn a_tool_without_a_menu_row_cannot_be_starred() {
        let mut favorites = FavoriteTools::default();
        assert!(!favorites.toggle(Tool::Cursor));
        assert!(!favorites.toggle(Tool::Ruler));
        assert!(favorites.items().is_empty());
    }

    #[test]
    fn names_this_build_does_not_know_are_hidden_but_kept() {
        let mut favorites: FavoriteTools =
            serde_json::from_str(r#"["Brush","FutureTool","Cursor","Brush"]"#).unwrap();
        assert_eq!(favorites.items(), tools(&[Tool::Brush]));
        assert!(favorites.toggle(Tool::Circle));
        let stored = serde_json::to_string(&favorites).unwrap();
        assert_eq!(
            stored,
            r#"["Brush","FutureTool","Cursor","Brush","Circle"]"#
        );
        // A list holding only names nobody knows draws no bar.
        let strangers: FavoriteTools = serde_json::from_str(r#"["FutureTool"]"#).unwrap();
        assert!(strangers.items().is_empty());
    }

    #[test]
    fn the_list_is_stored_as_plain_names() {
        let mut favorites = FavoriteTools::default();
        favorites.toggle(Tool::DoubleCurve);
        assert_eq!(
            serde_json::to_string(&favorites).unwrap(),
            r#"["DoubleCurve"]"#
        );
    }

    const CHART: Region = Region {
        origin: [44.0, 90.0],
        size: [1000.0, 560.0],
    };
    const WINDOW: Region = Region {
        origin: [0.0, 40.0],
        size: [1200.0, 700.0],
    };
    const BAR: [f32; 2] = [200.0, 36.0];

    fn at(prefs: BarPrefs) -> [f32; 2] {
        prefs.resolve(CHART, WINDOW, BAR)
    }

    #[test]
    fn an_unmoved_bar_is_centred_near_the_top_of_the_charts() {
        let [x, y] = at(BarPrefs::default());
        assert_eq!(x, CHART.origin[0] + (CHART.size[0] - BAR[0]) / 2.0);
        assert_eq!(y, CHART.origin[1] + DEFAULT_TOP_GAP);
    }

    #[test]
    fn a_moved_bar_comes_back_where_it_was_left_even_outside_the_charts() {
        // Over the toolbar: above the charts' top edge, left of their left edge.
        for position in [[300.0, 200.0], [10.0, 50.0], [900.0, 60.0]] {
            let prefs = BarPrefs::default().moved_to(position, WINDOW, BAR);
            assert!(prefs.window);
            let [x, y] = at(prefs);
            assert!(
                (x - position[0]).abs() < 0.01 && (y - position[1]).abs() < 0.01,
                "{position:?} came back at {x},{y}"
            );
        }
    }

    #[test]
    fn a_bar_dragged_past_an_edge_stays_in_the_window() {
        let prefs = BarPrefs::default().moved_to([5000.0, -300.0], WINDOW, BAR);
        assert_eq!(prefs.placement, Some(BarPlacement { x: 1.0, y: 0.0 }));
        let [x, y] = at(prefs);
        assert_eq!(x + BAR[0], WINDOW.origin[0] + WINDOW.size[0]);
        assert_eq!(y, WINDOW.origin[1]);
    }

    #[test]
    fn a_place_saved_before_the_bar_could_leave_the_charts_stays_in_the_charts() {
        // Top of the charts, as an older build saved it: a share of the charts' area.
        let old = BarPrefs {
            placement: Some(BarPlacement { x: 0.0, y: 0.0 }),
            window: false,
            hidden: false,
        };
        assert_eq!(at(old), CHART.origin);
        // Moving it again saves a window place and keeps the hidden flag.
        let hidden = BarPrefs {
            hidden: true,
            ..old
        };
        let moved = hidden.moved_to([100.0, 100.0], WINDOW, BAR);
        assert!(moved.window && moved.hidden);
    }

    #[test]
    fn a_window_flag_without_a_readable_place_leaves_the_bar_in_the_charts() {
        let prefs: BarPrefs =
            serde_json::from_str(r#"{"placement":"left","window":true}"#).unwrap();
        assert!(prefs.window && prefs.placement.is_none());
        assert!(!prefs.in_window());
        assert_eq!(at(prefs), at(BarPrefs::default()));
        assert!(at(prefs)[1] >= CHART.origin[1], "not over the toolbar");
    }

    #[test]
    fn a_smaller_window_keeps_the_bar_reachable() {
        let prefs = BarPrefs {
            placement: Some(BarPlacement { x: 1.0, y: 1.0 }),
            window: true,
            hidden: false,
        };
        let small = Region {
            origin: WINDOW.origin,
            size: [260.0, 120.0],
        };
        let [x, y] = prefs.resolve(CHART, small, BAR);
        assert!(x >= small.origin[0] && x + BAR[0] <= small.origin[0] + small.size[0] + 0.01);
        assert!(y >= small.origin[1] && y + BAR[1] <= small.origin[1] + small.size[1] + 0.01);
        // A room smaller than the bar pins the bar to its corner.
        let tiny = Region {
            origin: WINDOW.origin,
            size: [100.0, 20.0],
        };
        assert_eq!(prefs.resolve(CHART, tiny, BAR), WINDOW.origin);
    }

    #[test]
    fn a_stored_value_this_build_cannot_read_resets_only_its_own_field() {
        let prefs: BarPrefs =
            serde_json::from_str(r#"{"placement":"left","window":"yes","hidden":true}"#).unwrap();
        assert_eq!(
            prefs,
            BarPrefs {
                placement: None,
                window: false,
                hidden: true
            }
        );
        let prefs: BarPrefs = serde_json::from_str("{}").unwrap();
        assert_eq!(prefs, BarPrefs::default());
        // What an older build saved has no `window` and reads as a place in the charts.
        let old: BarPrefs =
            serde_json::from_str(r#"{"placement":{"x":0.25,"y":0.5},"hidden":false}"#).unwrap();
        assert!(!old.window);
    }
}
