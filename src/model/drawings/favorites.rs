//! The drawing tools a user starred, and where their floating bar sits.
//!
//! The list is app-wide: one ordered set of tools, in the order they were starred. It is kept as
//! the tools' stored names (the variant names `drawing.prefs` and `drawing.defaults` already use),
//! so a name a newer build wrote and this one does not know is kept when the list is written back
//! and is only left out of what is shown. The bar's place is a fraction of the room the bar has
//! to move in, so it follows a resized window instead of drifting off it.

use super::tool::Tool;
use serde::{Deserialize, Deserializer, Serialize};

/// The starred tools, as stored.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct FavoriteTools {
    names: Vec<String>,
}

impl FavoriteTools {
    /// A tool's stored name, or `None` for one that has no menu row to star it from.
    fn name_of(tool: Tool) -> Option<String> {
        Tool::GROUPED.contains(&tool).then(|| format!("{tool:?}"))
    }

    fn tool_named(name: &str) -> Option<Tool> {
        Tool::GROUPED
            .iter()
            .copied()
            .find(|tool| format!("{tool:?}") == name)
    }

    /// The starred tools this build knows, in the order they were starred, each once.
    pub fn tools(&self) -> Vec<Tool> {
        let mut tools: Vec<Tool> = Vec::new();
        for tool in self.names.iter().filter_map(|name| Self::tool_named(name)) {
            if !tools.contains(&tool) {
                tools.push(tool);
            }
        }
        tools
    }

    /// Stars `tool`, or takes its star away when it has one. Returns whether it is starred now.
    /// A tool with no menu row cannot be starred and stays out. Names this build does not know are
    /// left where they are.
    pub fn toggle(&mut self, tool: Tool) -> bool {
        let Some(name) = Self::name_of(tool) else {
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

/// Where the bar sits, as the share of its room (0 at the top-left corner, 1 at the bottom-right
/// one) that lies before it. Whatever the window's size, the whole bar is inside the chart area.
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
    #[serde(deserialize_with = "hidden_or_default")]
    pub hidden: bool,
}

fn placement_or_default<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<BarPlacement>, D::Error> {
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(serde_json::from_value(value).unwrap_or_default())
}

fn hidden_or_default<'de, D: Deserializer<'de>>(deserializer: D) -> Result<bool, D::Error> {
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(serde_json::from_value(value).unwrap_or_default())
}

/// Space kept between the bar and the top of the chart area when it has not been moved.
pub const DEFAULT_TOP_GAP: f32 = 12.0;

impl BarPrefs {
    /// The top-left corner of a bar of `size` inside the area at `origin` of `area` size, as
    /// `[x, y]`. Unmoved, the bar is centred near the top. A bar bigger than the area sits at its
    /// corner.
    pub fn resolve(&self, origin: [f32; 2], area: [f32; 2], size: [f32; 2]) -> [f32; 2] {
        let room = [(area[0] - size[0]).max(0.0), (area[1] - size[1]).max(0.0)];
        let (fx, y) = match self.placement {
            Some(place) => (place.x, place.y.clamp(0.0, 1.0) * room[1]),
            None => (0.5, DEFAULT_TOP_GAP.min(room[1])),
        };
        [origin[0] + fx.clamp(0.0, 1.0) * room[0], origin[1] + y]
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
        assert!(favorites.tools().is_empty());
        assert!(favorites.toggle(Tool::Brush));
        assert!(favorites.toggle(Tool::TrendLine));
        assert!(favorites.toggle(Tool::Highlighter));
        assert_eq!(
            favorites.tools(),
            [Tool::Brush, Tool::TrendLine, Tool::Highlighter]
        );
        assert!(!favorites.toggle(Tool::TrendLine));
        assert_eq!(favorites.tools(), [Tool::Brush, Tool::Highlighter]);
        // Starred again, it goes to the end.
        assert!(favorites.toggle(Tool::TrendLine));
        assert_eq!(
            favorites.tools(),
            [Tool::Brush, Tool::Highlighter, Tool::TrendLine]
        );
    }

    #[test]
    fn a_tool_without_a_menu_row_cannot_be_starred() {
        let mut favorites = FavoriteTools::default();
        assert!(!favorites.toggle(Tool::Cursor));
        assert!(!favorites.toggle(Tool::Ruler));
        assert!(favorites.tools().is_empty());
    }

    #[test]
    fn names_this_build_does_not_know_are_hidden_but_kept() {
        let mut favorites: FavoriteTools =
            serde_json::from_str(r#"["Brush","FutureTool","Cursor","Brush"]"#).unwrap();
        assert_eq!(favorites.tools(), [Tool::Brush]);
        assert!(favorites.toggle(Tool::Circle));
        let stored = serde_json::to_string(&favorites).unwrap();
        assert_eq!(
            stored,
            r#"["Brush","FutureTool","Cursor","Brush","Circle"]"#
        );
        // A list holding only names nobody knows draws no bar.
        let strangers: FavoriteTools = serde_json::from_str(r#"["FutureTool"]"#).unwrap();
        assert!(strangers.tools().is_empty());
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

    const ORIGIN: [f32; 2] = [44.0, 40.0];
    const AREA: [f32; 2] = [1000.0, 600.0];
    const BAR: [f32; 2] = [200.0, 36.0];

    #[test]
    fn an_unmoved_bar_is_centred_near_the_top() {
        let [x, y] = BarPrefs::default().resolve(ORIGIN, AREA, BAR);
        assert_eq!(x, ORIGIN[0] + (AREA[0] - BAR[0]) / 2.0);
        assert_eq!(y, ORIGIN[1] + DEFAULT_TOP_GAP);
    }

    #[test]
    fn a_moved_bar_comes_back_where_it_was_left() {
        let place = placement_from([300.0, 200.0], ORIGIN, AREA, BAR);
        let prefs = BarPrefs {
            placement: Some(place),
            hidden: false,
        };
        let [x, y] = prefs.resolve(ORIGIN, AREA, BAR);
        assert!((x - 300.0).abs() < 0.01 && (y - 200.0).abs() < 0.01);
    }

    #[test]
    fn a_bar_dragged_past_an_edge_stays_in_the_area() {
        let far = placement_from([5000.0, -300.0], ORIGIN, AREA, BAR);
        assert_eq!(far, BarPlacement { x: 1.0, y: 0.0 });
        let prefs = BarPrefs {
            placement: Some(far),
            hidden: false,
        };
        let [x, y] = prefs.resolve(ORIGIN, AREA, BAR);
        assert_eq!(x + BAR[0], ORIGIN[0] + AREA[0]);
        assert_eq!(y, ORIGIN[1]);
    }

    #[test]
    fn a_smaller_window_keeps_the_bar_reachable() {
        let prefs = BarPrefs {
            placement: Some(BarPlacement { x: 1.0, y: 1.0 }),
            hidden: false,
        };
        let small = [260.0, 120.0];
        let [x, y] = prefs.resolve(ORIGIN, small, BAR);
        assert!(x >= ORIGIN[0] && x + BAR[0] <= ORIGIN[0] + small[0] + 0.01);
        assert!(y >= ORIGIN[1] && y + BAR[1] <= ORIGIN[1] + small[1] + 0.01);
        // An area smaller than the bar pins the bar to its corner.
        assert_eq!(prefs.resolve(ORIGIN, [100.0, 20.0], BAR), ORIGIN);
    }

    #[test]
    fn a_stored_value_this_build_cannot_read_resets_only_its_own_field() {
        let prefs: BarPrefs =
            serde_json::from_str(r#"{"placement":"left","hidden":true}"#).unwrap();
        assert_eq!(
            prefs,
            BarPrefs {
                placement: None,
                hidden: true
            }
        );
        let prefs: BarPrefs = serde_json::from_str("{}").unwrap();
        assert_eq!(prefs, BarPrefs::default());
    }
}
