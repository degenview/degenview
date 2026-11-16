//! The rows of a menu as plain data, and how they are drawn. Each menu builds its entries from
//! the state of the studio (`file`, `edit`, `view`); drawing and sizing live here, once.

use crate::pine_studio::commands::Cmd;
use crate::ui::icons::Icon;
use crate::ui::kit::{self, MenuRow};
use crate::ui::shortcut::Shortcut;
use eframe::egui::Ui;

pub(super) enum Entry {
    Item(Item),
    Separator,
    Submenu(Submenu),
}

/// A row that asks for `command` when chosen.
pub(super) struct Item {
    pub label: String,
    pub icon: Option<Icon>,
    pub shortcut: Option<String>,
    pub enabled: bool,
    /// A check in the icon's place: the setting is on, or the choice is the current one.
    pub checked: bool,
    pub destructive: bool,
    pub command: Cmd,
}

/// A row that opens rows of its own to its side. Greyed, and not opening, when `enabled` is off.
pub(super) struct Submenu {
    pub label: String,
    pub icon: Option<Icon>,
    pub enabled: bool,
    pub entries: Vec<Entry>,
}

impl Item {
    pub fn new(label: impl Into<String>, command: Cmd) -> Self {
        Self {
            label: label.into(),
            icon: None,
            shortcut: None,
            enabled: true,
            checked: false,
            destructive: false,
            command,
        }
    }

    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }

    pub fn shortcut(mut self, shortcut: &Shortcut) -> Self {
        self.shortcut = Some(shortcut.label());
        self
    }

    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    pub fn checked(mut self, checked: bool) -> Self {
        self.checked = checked;
        self
    }

    pub fn destructive(mut self, destructive: bool) -> Self {
        self.destructive = destructive;
        self
    }
}

impl From<Item> for Entry {
    fn from(item: Item) -> Self {
        Self::Item(item)
    }
}

impl Submenu {
    pub fn new(label: impl Into<String>, entries: Vec<Entry>) -> Self {
        Self {
            label: label.into(),
            icon: None,
            enabled: true,
            entries,
        }
    }

    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }

    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }
}

impl From<Submenu> for Entry {
    fn from(submenu: Submenu) -> Self {
        Self::Submenu(submenu)
    }
}

/// `groups` as one list of entries, a separator between the groups that have rows.
pub(super) fn grouped(groups: Vec<Vec<Entry>>) -> Vec<Entry> {
    let mut entries = Vec::new();
    for section in groups.into_iter().filter(|group| !group.is_empty()) {
        if !entries.is_empty() {
            entries.push(Entry::Separator);
        }
        entries.extend(section);
    }
    entries
}

/// What the width of a menu of `entries` is measured on: each row's label and shortcut. A
/// submenu's own rows are measured by the submenu.
pub(super) fn measured(entries: &[Entry]) -> Vec<(String, Option<String>)> {
    entries
        .iter()
        .filter_map(|entry| match entry {
            Entry::Item(item) => Some((item.label.clone(), item.shortcut.clone())),
            Entry::Submenu(submenu) => Some((submenu.label.clone(), None)),
            Entry::Separator => None,
        })
        .collect()
}

/// Borrows `rows` the way the kit's sizing takes them.
pub(super) fn as_rows(rows: &[(String, Option<String>)]) -> Vec<(&str, Option<&str>)> {
    rows.iter()
        .map(|(label, shortcut)| (label.as_str(), shortcut.as_deref()))
        .collect()
}

/// Draws `entries` in the menu `ui` is the body of, and pushes the command of the row chosen. A
/// choice closes the whole menu.
pub(super) fn show(ui: &mut Ui, entries: Vec<Entry>, chosen: &mut Vec<Cmd>) {
    for entry in entries {
        match entry {
            Entry::Separator => kit::menu_separator(ui),
            Entry::Item(item) => {
                let row = kit::menu_row(
                    ui,
                    MenuRow {
                        icon: item.icon,
                        shortcut: item.shortcut.as_deref(),
                        enabled: item.enabled,
                        checked: item.checked,
                        destructive: item.destructive,
                        ..MenuRow::new(&item.label)
                    },
                );
                if row.clicked() {
                    chosen.push(item.command);
                    ui.close();
                }
            }
            Entry::Submenu(submenu) => {
                let row = MenuRow {
                    icon: submenu.icon,
                    enabled: submenu.enabled,
                    ..MenuRow::new(&submenu.label)
                };
                kit::menu_submenu(ui, row, 0.0, |ui| {
                    let rows = measured(&submenu.entries);
                    kit::menu_fit_rows(ui, &as_rows(&rows));
                    show(ui, submenu.entries, chosen);
                });
            }
        }
    }
}
