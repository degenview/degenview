//! A tab's context menu and the rows it can show.

use super::{TabBar, TabBarEvent, TabView};
use crate::model::tab::title::TabTitleStyle;
use crate::ui::icons::Icon;
use crate::ui::kit;
use crate::ui::window::tab_link_menu::{self, TabLinkEdit};
use eframe::egui::{Rect, Ui};

impl TabBar<'_> {
    pub(super) fn tab_menu(
        &self,
        ui: &mut Ui,
        rect: Rect,
        (index, tab): (usize, &TabView),
        events: &mut Vec<TabBarEvent>,
    ) {
        if kit::menu_item(ui, Icon::Plus, "New Tab", true, false) {
            events.push(TabBarEvent::New);
            ui.close();
        }
        kit::menu_separator(ui);
        if kit::menu_item(ui, Icon::Xmark, "Close Tab", true, false) {
            events.push(TabBarEvent::Close(tab.id));
            ui.close();
        }
        if kit::menu_item(
            ui,
            Icon::Xmark,
            "Close Other Tabs",
            self.tabs.len() > 1,
            false,
        ) {
            events.push(TabBarEvent::CloseOthers(tab.id));
            ui.close();
        }
        if kit::menu_item(
            ui,
            Icon::Xmark,
            "Close Tabs to the Right",
            index + 1 < self.tabs.len(),
            false,
        ) {
            events.push(TabBarEvent::CloseToRight(tab.id));
            ui.close();
        }
        if let Some(chart) = &tab.chart {
            kit::menu_separator(ui);
            if kit::menu_item(ui, Icon::Pencil, "Rename Tab\u{2026}", true, false) {
                self.begin_rename(ui.ctx(), rect, tab.id, chart);
                ui.close();
            }
            if kit::menu_item(
                ui,
                Icon::Pencil,
                "Reset Tab Title",
                chart.custom_title.is_some(),
                false,
            ) {
                events.push(TabBarEvent::SetCustomTitle {
                    tab: tab.id,
                    title: None,
                });
                ui.close();
            }
            kit::menu_submenu(ui, kit::MenuRow::new("Tab Title"), 0.0, |ui| {
                let rows = TabTitleStyle::ALL.map(|style| (style.label(), None));
                kit::menu_fit_rows(ui, &rows);
                for style in TabTitleStyle::ALL {
                    let row = kit::menu_row(
                        ui,
                        kit::MenuRow {
                            checked: style == self.style,
                            ..kit::MenuRow::new(style.label())
                        },
                    );
                    if row.clicked() {
                        events.push(TabBarEvent::SetTitleStyle(style));
                        ui.close();
                    }
                }
            });
        }
        if self.tabs.len() > 1 || !self.other_windows.is_empty() {
            kit::menu_separator(ui);
        }
        if self.tabs.len() > 1 && kit::menu_item(ui, Icon::Plus, MOVE_NEW, true, false) {
            events.push(TabBarEvent::MoveToNewWindow(tab.id));
            ui.close();
        }
        if !self.other_windows.is_empty() {
            kit::menu_submenu(ui, kit::MenuRow::new(MOVE_TO), 0.0, |ui| {
                let titles: Vec<(&str, Option<&str>)> = self
                    .other_windows
                    .iter()
                    .map(|window| (window.title.as_str(), None))
                    .collect();
                kit::menu_fit_rows(ui, &titles);
                for window in self.other_windows {
                    if kit::menu_row(ui, kit::MenuRow::new(&window.title)).clicked() {
                        events.push(TabBarEvent::MoveToWindow {
                            tab: tab.id,
                            window: window.id,
                        });
                        ui.close();
                    }
                }
            });
        }
        if tab.linkable {
            kit::menu_separator(ui);
            match tab_link_menu::show(ui, tab.link, tab.link_toggles) {
                Some(TabLinkEdit::Color(color)) => {
                    events.push(TabBarEvent::SetLink { tab: tab.id, color });
                }
                Some(TabLinkEdit::Toggles(toggles)) => {
                    if let Some(color) = tab.link {
                        events.push(TabBarEvent::SetLinkToggles { color, toggles });
                    }
                }
                None => {}
            }
        }
    }
}

const MOVE_NEW: &str = "Move Tab to New Window";
const MOVE_TO: &str = "Move Tab to Window";

/// Every row the tab menu can show, for sizing it.
pub(super) const MENU_ROWS: [&str; 9] = [
    "New Tab",
    "Close Tab",
    "Close Other Tabs",
    "Close Tabs to the Right",
    "Rename Tab\u{2026}",
    "Reset Tab Title",
    "Tab Title",
    MOVE_NEW,
    MOVE_TO,
];
