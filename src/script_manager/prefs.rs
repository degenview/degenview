//! The Script Manager's persisted window layout: the `scriptManager.*` UserDefaults of the Swift
//! app, kept as one JSON setting.

use crate::store::SharedStore;
use serde::{Deserialize, Serialize};

const KEY: &str = "scriptManager.layout";

/// Where the preview chart sits next to the code (`ChartPosition`).
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ChartPosition {
    #[default]
    Left,
    Top,
    Bottom,
}

impl ChartPosition {
    pub const ALL: [Self; 3] = [Self::Left, Self::Top, Self::Bottom];

    pub fn title(self) -> &'static str {
        match self {
            Self::Left => "Left of Code",
            Self::Top => "Above Code",
            Self::Bottom => "Below Code",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DrawerTab {
    #[default]
    Inputs,
    Report,
    Problems,
    Logs,
    Profile,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct LayoutPrefs {
    pub sidebar_visible: bool,
    /// 0 means the default width.
    pub sidebar_width: f32,
    pub chart_visible: bool,
    pub chart_position: ChartPosition,
    pub chart_width: f32,
    pub chart_height: f32,
    pub drawer_open: bool,
    pub drawer_tab: DrawerTab,
    pub drawer_height: f32,
    pub selected_script_id: Option<String>,
    /// The debug panel under the editor is shown.
    pub debug_open: bool,
    /// 0 means the default height.
    pub debug_height: f32,
    /// The width of the debug panel's second column; 0 means the default.
    pub debug_side_width: f32,
    /// Which sections of the debug panel's second column are open (see `debug::Section`).
    pub debug_sections: u8,
    /// The assistant's panel under the editor is shown.
    pub ai_open: bool,
    /// 0 means the default height.
    pub ai_height: f32,
}

impl Default for LayoutPrefs {
    fn default() -> Self {
        Self {
            sidebar_visible: true,
            sidebar_width: 0.0,
            chart_visible: true,
            chart_position: ChartPosition::Left,
            chart_width: 0.0,
            chart_height: 0.0,
            drawer_open: true,
            drawer_tab: DrawerTab::Inputs,
            drawer_height: 0.0,
            selected_script_id: None,
            debug_open: false,
            debug_height: 0.0,
            debug_side_width: 0.0,
            debug_sections: 0b0111,
            ai_open: false,
            ai_height: 0.0,
        }
    }
}

impl LayoutPrefs {
    pub fn load(store: &SharedStore) -> Self {
        store
            .lock()
            .ok()
            .and_then(|store| store.setting(KEY))
            .unwrap_or_default()
    }

    pub fn save(&self, store: &SharedStore) {
        if let Ok(store) = store.lock() {
            let _ = store.set_setting(KEY, self);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Store;
    use std::sync::{Arc, Mutex};

    #[test]
    fn prefs_round_trip_and_default_when_absent() {
        let store: SharedStore = Arc::new(Mutex::new(Store::open_in_memory().unwrap()));
        assert_eq!(LayoutPrefs::load(&store), LayoutPrefs::default());
        let prefs = LayoutPrefs {
            sidebar_visible: false,
            chart_position: ChartPosition::Bottom,
            drawer_tab: DrawerTab::Problems,
            selected_script_id: Some("abc".into()),
            ..LayoutPrefs::default()
        };
        prefs.save(&store);
        assert_eq!(LayoutPrefs::load(&store), prefs);
    }

    #[test]
    fn older_rows_missing_fields_fall_back_to_defaults() {
        let store: SharedStore = Arc::new(Mutex::new(Store::open_in_memory().unwrap()));
        store
            .lock()
            .unwrap()
            .set_setting(KEY, &serde_json::json!({ "sidebarVisible": false }))
            .unwrap();
        let prefs = LayoutPrefs::load(&store);
        assert!(!prefs.sidebar_visible);
        assert!(prefs.drawer_open);
    }
}
