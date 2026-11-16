//! The View menu: the panels, where the preview chart sits and whether it shows, and the drawer.

use super::entry::{Entry, Item, Submenu, grouped};
use crate::pine_studio::commands::Cmd;
use crate::pine_studio::library::prefs::{ChartPosition, DrawerTab};
use crate::pine_studio::profiler::Status;
use crate::pine_studio::rail::RailState;
use crate::ui::shortcut;

/// What the View menu reads of the layout: what the rail shows, and the drawer.
pub(in crate::pine_studio) struct ViewState {
    pub panels: RailState,
    pub drawer_open: bool,
    pub drawer_tab: DrawerTab,
}

pub(super) fn entries(state: &ViewState) -> Vec<Entry> {
    let panels = &state.panels;
    let toggle = |label: &str, on: bool, command: Cmd| Item::new(label, command).checked(on);
    let (chart_label, drawer_label) = (
        if panels.chart_visible {
            "Hide Preview Chart"
        } else {
            "Show Preview Chart"
        },
        if state.drawer_open {
            "Hide Drawer"
        } else {
            "Show Drawer"
        },
    );
    grouped(vec![
        vec![
            toggle("Assistant", panels.assistant_open, Cmd::ToggleAiPanel).into(),
            toggle(
                "Profiler",
                panels.profiler != Status::Off,
                Cmd::ProfilerToggle,
            )
            .shortcut(&shortcut::PROFILER_TOGGLE)
            .into(),
            toggle("Debugger", panels.debugger_open, Cmd::ToggleDebugPanel).into(),
        ],
        vec![
            Submenu::new(
                "Preview Chart",
                ChartPosition::ALL
                    .into_iter()
                    .map(|position| {
                        toggle(
                            position.title(),
                            position == panels.chart_position,
                            Cmd::SetChartPosition(position),
                        )
                        .into()
                    })
                    .collect(),
            )
            .into(),
            Item::new(chart_label, Cmd::ToggleChart)
                .shortcut(&shortcut::TOGGLE_PREVIEW_CHART)
                .into(),
        ],
        vec![
            Item::new(drawer_label, Cmd::ToggleDrawer).into(),
            Submenu::new(
                "Drawer Tab",
                DrawerTab::ALL
                    .into_iter()
                    .map(|tab| {
                        toggle(tab.title(), tab == state.drawer_tab, Cmd::SetDrawerTab(tab)).into()
                    })
                    .collect(),
            )
            .into(),
        ],
    ])
}
