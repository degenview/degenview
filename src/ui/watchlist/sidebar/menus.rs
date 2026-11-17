//! The sidebar's menus: their widths, the section and row menus, and the watchlist membership
//! submenu.

use super::header::flag_row;
use super::{PromptKind, WatchlistAction, WatchlistSidebarInput, WatchlistSidebarState};

use crate::i18n::tr;
use crate::model::watchlist::{
    EntryId, Watchlist, WatchlistColumn, WatchlistFlag, WatchlistInstrument, WatchlistSortKey,
};
use crate::ui::icons::Icon;
use crate::ui::kit;
use eframe::egui;

/// The header "…" menu and the row menus, and their submenus. Measured against their rows by
/// `menu_widths_fit_their_rows`, which fails if either drifts too narrow or wide. (The list menu
/// is sized from its list names when it opens.)
///
/// The system font sets the width: macOS's measures 179 and 173, the font a Linux machine with no
/// system fonts falls back to (egui's own) 190 and 179, so the other platforms get a few points
/// more rather than a cut label. Windows is not measured; its UI font is assumed no wider.
#[cfg(target_os = "macos")]
pub(super) const MENU_WIDTH: f32 = 186.0;
#[cfg(target_os = "macos")]
pub(super) const SUBMENU_WIDTH: f32 = 178.0;
#[cfg(not(target_os = "macos"))]
pub(super) const MENU_WIDTH: f32 = 194.0;
#[cfg(not(target_os = "macos"))]
pub(super) const SUBMENU_WIDTH: f32 = 184.0;
/// What the menu frame's margins take from its width.
pub(super) const MENU_FRAME: f32 = 10.0;

/// The width of the options and row menus: `MENU_WIDTH` (measured in English) or what their rows
/// need in the language in use, whichever is more. Inside a menu's body.
pub(super) fn menu_width_now(ui: &egui::Ui) -> f32 {
    let labels = [
        tr!("Add Section…"),
        tr!("Sort By"),
        tr!("Columns"),
        tr!("Filter by Flag"),
        tr!("Show Details"),
        tr!("Import Symbols…"),
        tr!("Export to File…"),
        tr!("Copy as Text"),
        tr!("Open in Focused Chart"),
        tr!("Add as New Chart"),
        tr!("Open in New Tab"),
        tr!("Move to Section"),
        tr!("Set Flag"),
        tr!("Add to Watchlist"),
        tr!("Create Price Alert…"),
        tr!("Copy Symbol"),
        tr!("Copy Qualified Symbol"),
        tr!("Remove from Watchlist"),
    ];
    fitted(ui, &labels, MENU_WIDTH)
}

/// The width of the submenus and the section menu, as `menu_width_now`.
pub(super) fn submenu_width_now(ui: &egui::Ui) -> f32 {
    let mut labels = vec![
        tr!("Expand Section"),
        tr!("Collapse Section"),
        tr!("Rename Section…"),
        tr!("Add Section…"),
        tr!("Delete Section"),
        tr!("Show Symbol Subtitle"),
        tr!("Show 1M Chart"),
        tr!("Descending"),
        tr!("No Section"),
        tr!("Clear Flag"),
        tr!("New Watchlist…"),
    ];
    labels.extend(WatchlistColumn::ALL.map(WatchlistColumn::accessibility_title));
    labels.extend(WatchlistSortKey::ALL.map(WatchlistSortKey::title));
    labels.extend(WatchlistFlag::ALL.map(WatchlistFlag::title));
    fitted(ui, &labels, SUBMENU_WIDTH)
}

/// `menu_width_now` for a popup's builder, which runs every frame: the rows are measured only
/// once `trigger` has opened the menu (or is about to).
pub(super) fn menu_width(ui: &egui::Ui, trigger: &egui::Response) -> f32 {
    if opens(ui, trigger) {
        menu_width_now(ui)
    } else {
        MENU_WIDTH
    }
}

/// `submenu_width_now` for a popup's builder; see `menu_width`.
pub(super) fn submenu_width(ui: &egui::Ui, trigger: &egui::Response) -> f32 {
    if opens(ui, trigger) {
        submenu_width_now(ui)
    } else {
        SUBMENU_WIDTH
    }
}

fn opens(ui: &egui::Ui, trigger: &egui::Response) -> bool {
    trigger.clicked()
        || trigger.secondary_clicked()
        || egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(trigger))
}

fn fitted(ui: &egui::Ui, labels: &[&str], floor: f32) -> f32 {
    let rows: Vec<(&str, Option<&str>)> = labels.iter().map(|label| (*label, None)).collect();
    floor.max(kit::menu_width_for_rows(ui, &rows, 0.0))
}

pub(super) fn section_menu(
    ui: &mut egui::Ui,
    state: &mut WatchlistSidebarState,
    list: &Watchlist,
    section: EntryId,
    title: &str,
    collapsed: bool,
    actions: &mut Vec<WatchlistAction>,
) {
    kit::menu_scope(ui, submenu_width_now(ui) - MENU_FRAME);
    let (icon, label) = if collapsed {
        (Icon::ChevronDown, tr!("Expand Section"))
    } else {
        (Icon::ChevronUp, tr!("Collapse Section"))
    };
    if kit::menu_item(ui, icon, label, true, false) {
        actions.push(WatchlistAction::SetSectionCollapsed {
            list: list.id,
            section,
            collapsed: !collapsed,
        });
        ui.close();
    }
    if kit::menu_item(ui, Icon::Pencil, tr!("Rename Section…"), true, false) {
        state.open_prompt(
            PromptKind::RenameSection {
                list: list.id,
                section,
            },
            title.to_owned(),
        );
        ui.close();
    }
    if kit::menu_item(ui, Icon::Plus, tr!("Add Section…"), true, false) {
        state.open_prompt(PromptKind::NewSection(list.id), String::new());
        ui.close();
    }
    kit::menu_separator(ui);
    if kit::menu_item(ui, Icon::Trash, tr!("Delete Section"), true, true) {
        actions.push(WatchlistAction::DeleteSection {
            list: list.id,
            section,
        });
        ui.close();
    }
}

/// The actions for one market, used by its context menu.
pub(super) fn instrument_menu(
    ui: &mut egui::Ui,
    state: &mut WatchlistSidebarState,
    input: &WatchlistSidebarInput,
    list: &Watchlist,
    item: &WatchlistInstrument,
    actions: &mut Vec<WatchlistAction>,
) {
    let menu = menu_width_now(ui);
    let submenu = submenu_width_now(ui);
    kit::menu_scope(ui, menu - MENU_FRAME);
    if kit::menu_item(
        ui,
        Icon::ChartLine,
        tr!("Open in Focused Chart"),
        true,
        false,
    ) {
        actions.push(WatchlistAction::Open(item.clone()));
        ui.close();
    }
    if kit::menu_item(ui, Icon::Plus, tr!("Add as New Chart"), true, false) {
        actions.push(WatchlistAction::AddChart(item.clone()));
        ui.close();
    }
    if kit::menu_item(
        ui,
        Icon::ArrowUpRightSquare,
        tr!("Open in New Tab"),
        true,
        false,
    ) {
        actions.push(WatchlistAction::OpenInNewTab(item.clone()));
        ui.close();
    }
    kit::menu_separator(ui);

    membership_menu(ui, input.lists, item, input.load_failed, actions, |item| {
        state.prompt_new_list_with(item);
    });

    if list.sections().next().is_some() {
        let owner = list.owning_section(item.id).map(|section| section.id);
        let row = kit::MenuRow {
            icon: Some(Icon::Layers),
            ..kit::MenuRow::new(tr!("Move to Section"))
        };
        kit::menu_submenu(ui, row, submenu, |ui| {
            let none = kit::menu_row(
                ui,
                kit::MenuRow {
                    checked: owner.is_none(),
                    ..kit::MenuRow::new(tr!("No Section"))
                },
            );
            if none.clicked() {
                actions.push(WatchlistAction::MoveToSection {
                    list: list.id,
                    entry: item.id,
                    section: None,
                });
                ui.close();
            }
            for section in list.sections() {
                let target = kit::menu_row(
                    ui,
                    kit::MenuRow {
                        checked: owner == Some(section.id),
                        ..kit::MenuRow::new(&section.title)
                    },
                );
                if target.clicked() {
                    actions.push(WatchlistAction::MoveToSection {
                        list: list.id,
                        entry: item.id,
                        section: Some(section.id),
                    });
                    ui.close();
                }
            }
        });
    }

    let current = input.flags.get(&item.instrument.key()).copied();
    let row = kit::MenuRow {
        icon: Some(Icon::Tag),
        ..kit::MenuRow::new(tr!("Set Flag"))
    };
    kit::menu_submenu(ui, row, submenu, |ui| {
        for flag in WatchlistFlag::ALL {
            if flag_row(ui, flag, current == Some(flag)).clicked() {
                actions.push(WatchlistAction::SetFlag {
                    instrument: item.instrument.clone(),
                    flag: (current != Some(flag)).then_some(flag),
                });
                ui.close();
            }
        }
        kit::menu_separator(ui);
        if kit::menu_item(ui, Icon::Xmark, tr!("Clear Flag"), current.is_some(), false) {
            actions.push(WatchlistAction::SetFlag {
                instrument: item.instrument.clone(),
                flag: None,
            });
            ui.close();
        }
    });

    if !item.instrument.source.is_prediction_market()
        && item.instrument.source != crate::model::DataSource::CoinMarketCap
        && kit::menu_item(ui, Icon::Bell, tr!("Create Price Alert…"), true, false)
    {
        actions.push(WatchlistAction::CreateAlert(item.clone()));
        ui.close();
    }

    kit::menu_separator(ui);
    if kit::menu_item(ui, Icon::CopyDoc, tr!("Copy Symbol"), true, false) {
        actions.push(WatchlistAction::CopyText(item.instrument.symbol.clone()));
        ui.close();
    }
    if kit::menu_item(
        ui,
        Icon::Clipboard,
        tr!("Copy Qualified Symbol"),
        true,
        false,
    ) {
        actions.push(WatchlistAction::CopyText(format!(
            "{}:{}",
            item.instrument.source.watchlist_slug(),
            item.instrument.symbol
        )));
        ui.close();
    }
    kit::menu_separator(ui);
    if kit::menu_item(ui, Icon::Trash, tr!("Remove from Watchlist"), true, true) {
        actions.push(WatchlistAction::RemoveEntry {
            list: list.id,
            entry: item.id,
        });
        ui.close();
    }
}

/// "Add to Watchlist": every list with a check where the market already is, then New
/// Watchlist…. Shared by the sidebar, chart cards and search results so membership
/// reads the same everywhere. `on_new_list` is called for "New Watchlist…"; the caller
/// decides where the name prompt lives. Ticking a list leaves the menu open, so a market can
/// join several in one visit.
pub fn membership_menu(
    ui: &mut egui::Ui,
    lists: &[Watchlist],
    item: &WatchlistInstrument,
    disabled: bool,
    actions: &mut Vec<WatchlistAction>,
    on_new_list: impl FnOnce(WatchlistInstrument),
) {
    let row = kit::MenuRow {
        icon: Some(Icon::Star),
        enabled: !disabled,
        ..kit::MenuRow::new(tr!("Add to Watchlist"))
    };
    let submenu = submenu_width_now(ui);
    let response = kit::menu_submenu(ui, row, submenu, |ui| {
        for list in lists {
            let member = list.contains(&item.instrument);
            let toggle = kit::menu_row(
                ui,
                kit::MenuRow {
                    checked: member,
                    ..kit::MenuRow::new(&list.name)
                },
            );
            if toggle.clicked() {
                actions.push(if member {
                    WatchlistAction::RemoveFromList {
                        list: list.id,
                        instrument: item.instrument.clone(),
                    }
                } else {
                    WatchlistAction::AddToList {
                        list: list.id,
                        item: item.clone(),
                    }
                });
            }
        }
        kit::menu_separator(ui);
        if kit::menu_item(ui, Icon::Plus, tr!("New Watchlist…"), true, false) {
            on_new_list(item.clone());
            ui.close();
        }
    });
    if disabled {
        response.on_hover_text(tr!("Watchlists are unavailable"));
    }
}
