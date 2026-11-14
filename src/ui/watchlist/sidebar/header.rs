//! The sidebar's header: the list selector, the add and options buttons with the options menu,
//! and the filter field.

use super::menus::{MENU_FRAME, MENU_WIDTH, SUBMENU_WIDTH};
use super::{PromptKind, WatchlistAction, WatchlistSidebarInput, WatchlistSidebarState};

use crate::model::watchlist::{
    Watchlist, WatchlistColumn, WatchlistFlag, WatchlistSort, WatchlistSortKey,
};
use crate::ui::icons::{self, Icon};
use crate::ui::kit;
use crate::ui::theme;
use crate::ui::watchlist::style::{self as style, LEADING_INSET, TRAILING_INSET};
use eframe::egui::{self, Color32, Id, Pos2, Rect, Sense, Vec2};

pub(super) fn header(
    ui: &mut egui::Ui,
    state: &mut WatchlistSidebarState,
    input: &WatchlistSidebarInput,
    list: Option<&Watchlist>,
    actions: &mut Vec<WatchlistAction>,
) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 44.0), Sense::hover());
    let controls_right = rect.right() - TRAILING_INSET;
    let options_rect = Rect::from_center_size(
        Pos2::new(controls_right - 12.0, rect.center().y),
        Vec2::splat(24.0),
    );
    let filter_rect = options_rect.translate(Vec2::new(-32.0, 0.0));
    let add_rect = filter_rect.translate(Vec2::new(-32.0, 0.0));
    let selector_rect = Rect::from_min_max(
        Pos2::new(rect.left() + LEADING_INSET, rect.center().y - 13.0),
        Pos2::new(
            (add_rect.left() - 8.0).max(rect.left() + LEADING_INSET + 64.0),
            rect.center().y + 13.0,
        ),
    );

    list_selector(ui, state, input, list, selector_rect);
    let enabled = !input.load_failed && list.is_some();
    let add = kit::hover_button(
        ui,
        add_rect,
        Id::new("watchlist-add"),
        Icon::Plus,
        "Add Symbol",
        true,
        enabled,
    );
    if add.clicked() {
        actions.push(WatchlistAction::AddSymbol {
            list: list.map(|list| list.id),
        });
    }
    let filtering = state.filter_visible || state.filter.is_active();
    let filter = ui.interact(
        filter_rect,
        Id::new("watchlist-filter-toggle"),
        Sense::click(),
    );
    let filter = crate::ui::accessibility::label(filter, "Filter this watchlist")
        .on_hover_text("Filter this watchlist");
    if filtering {
        ui.painter()
            .rect_filled(filter_rect, 6.0, theme::selected_tint(filter.hovered()));
    } else if filter.hovered() {
        ui.painter()
            .rect_filled(filter_rect, 6.0, Color32::from_white_alpha(17));
    }
    icons::paint(
        ui.painter(),
        Rect::from_center_size(filter_rect.center(), Vec2::splat(15.0)),
        Icon::Magnifier,
        theme::text_primary(),
    );
    if filter.clicked() {
        state.filter_visible = !state.filter_visible;
        if state.filter_visible {
            state.focus_filter = true;
        } else {
            state.clear_filter();
        }
    }
    options_menu(ui, state, list, actions, options_rect, enabled);
}

/// The header's list title with its chevron: a plate that hugs them, lit like the other header
/// buttons while hovered and while its menu is open.
pub(super) fn list_selector(
    ui: &mut egui::Ui,
    state: &mut WatchlistSidebarState,
    input: &WatchlistSidebarInput,
    list: Option<&Watchlist>,
    rect: Rect,
) {
    const PAD: f32 = 6.0;
    const CHEVRON: f32 = 10.0;
    let title = list.map_or("Watchlists", |list| list.name.as_str());
    let color = if input.load_failed {
        theme::text_tertiary()
    } else {
        theme::text_primary()
    };
    let galley = kit::galley(
        ui,
        title,
        14.0,
        kit::SEMIBOLD,
        color,
        (rect.width() - PAD * 3.0 - CHEVRON).max(10.0),
    );
    // The title keeps the panel's leading edge; the plate reaches out behind it.
    let plate = Rect::from_min_size(
        Pos2::new(rect.left() - PAD, rect.center().y - 14.0),
        Vec2::new(PAD * 3.0 + galley.size().x + CHEVRON, 28.0),
    );
    let response = ui.interact(
        plate,
        Id::new("watchlist-selector"),
        if input.load_failed {
            Sense::hover()
        } else {
            Sense::click()
        },
    );
    let response = crate::ui::accessibility::label(response, format!("Watchlist: {title}"));
    let open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&response));
    if !input.load_failed {
        if open || response.is_pointer_button_down_on() {
            ui.painter().rect_filled(plate, 6.0, theme::ink(26));
        } else if response.hovered() {
            ui.painter().rect_filled(plate, 6.0, theme::ink(14));
        }
    }
    ui.painter().galley(
        Pos2::new(rect.left(), rect.center().y - galley.size().y / 2.0),
        galley.clone(),
        color,
    );
    icons::paint(
        ui.painter(),
        Rect::from_center_size(
            Pos2::new(plate.right() - PAD - CHEVRON / 2.0, rect.center().y),
            Vec2::splat(CHEVRON),
        ),
        Icon::ChevronDown,
        theme::text_secondary(),
    );
    if !(open || response.clicked()) {
        return;
    }
    // As wide as the list names (with their counts) and the fixed rows need, no wider.
    let counts: Vec<String> = input
        .lists
        .iter()
        .map(|candidate| candidate.instruments().count().to_string())
        .collect();
    let mut rows: Vec<(&str, Option<&str>)> = input
        .lists
        .iter()
        .zip(&counts)
        .map(|(candidate, count)| (candidate.name.as_str(), Some(count.as_str())))
        .collect();
    rows.extend(
        [
            "Create New Watchlist\u{2026}",
            "Rename Watchlist\u{2026}",
            "Make a Copy\u{2026}",
            "Delete Watchlist\u{2026}",
        ]
        .map(|label| (label, None)),
    );
    let list_menu_width = kit::menu_width_for_rows(ui, &rows, 0.0);
    kit::menu_popup(&response)
        .width(list_menu_width)
        .show(|ui| {
            kit::menu_scope(ui, list_menu_width - MENU_FRAME);
            if !input.lists.is_empty() {
                kit::menu_heading(ui, "Watchlists");
                egui::ScrollArea::vertical()
                    .max_height(kit::MENU_MAX_HEIGHT)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        for candidate in input.lists {
                            let count = candidate.instruments().count().to_string();
                            let row = kit::menu_row(
                                ui,
                                kit::MenuRow {
                                    icon: Some(if candidate.is_favorites {
                                        Icon::StarFill
                                    } else {
                                        Icon::List
                                    }),
                                    shortcut: Some(&count),
                                    current: list.is_some_and(|list| list.id == candidate.id),
                                    ..kit::MenuRow::new(&candidate.name)
                                },
                            );
                            if row.clicked() {
                                state.select_list(candidate.id);
                                state.selected_entry = None;
                                state.synced_list = None;
                                ui.close();
                            }
                        }
                    });
                kit::menu_separator(ui);
            }
            if kit::menu_item(ui, Icon::Plus, "Create New Watchlist…", true, false) {
                state.open_prompt(PromptKind::NewList, String::new());
                ui.close();
            }
            let Some(list) = list else {
                return;
            };
            if kit::menu_item(ui, Icon::Pencil, "Rename Watchlist…", true, false) {
                state.open_prompt(PromptKind::RenameList(list.id), list.name.clone());
                ui.close();
            }
            if kit::menu_item(ui, Icon::CopyDoc, "Make a Copy…", true, false) {
                state.open_prompt(PromptKind::CopyList(list.id), format!("{} copy", list.name));
                ui.close();
            }
            kit::menu_separator(ui);
            let deletable = !list.is_favorites;
            let delete = kit::menu_row(
                ui,
                kit::MenuRow {
                    icon: Some(Icon::Trash),
                    enabled: deletable,
                    destructive: true,
                    ..kit::MenuRow::new("Delete Watchlist…")
                },
            );
            if !deletable {
                delete.on_hover_text("Favorites can be renamed but not deleted");
            } else if delete.clicked() {
                state.confirm_delete = true;
                ui.close();
            }
        });
}

/// A row for one of the flags, with its swatch in the icon's place; `current` marks the chosen one.
pub(super) fn flag_row(ui: &mut egui::Ui, flag: WatchlistFlag, current: bool) -> egui::Response {
    kit::menu_row(
        ui,
        kit::MenuRow {
            glyph: Some(&|painter: &egui::Painter, rect: Rect, _: Color32| {
                style::paint_swatch(painter, rect, flag);
            }),
            current,
            ..kit::MenuRow::new(flag.title())
        },
    )
}

fn options_menu(
    ui: &mut egui::Ui,
    state: &mut WatchlistSidebarState,
    list: Option<&Watchlist>,
    actions: &mut Vec<WatchlistAction>,
    rect: Rect,
    enabled: bool,
) {
    let response = kit::hover_button(
        ui,
        rect,
        Id::new("watchlist-options"),
        Icon::Ellipsis,
        "Watchlist options",
        true,
        enabled,
    );
    kit::menu_popup(&response).width(MENU_WIDTH).show(|ui| {
        kit::menu_scope(ui, MENU_WIDTH - MENU_FRAME);
        let Some(list) = list else {
            return;
        };
        if kit::menu_item(ui, Icon::Plus, "Add Section…", true, false) {
            state.open_prompt(PromptKind::NewSection(list.id), String::new());
            ui.close();
        }
        kit::menu_separator(ui);

        let row = |icon, label| kit::MenuRow {
            icon: Some(icon),
            ..kit::MenuRow::new(label)
        };
        kit::menu_submenu(
            ui,
            row(Icon::ChevronUpDown, "Sort By"),
            SUBMENU_WIDTH,
            |ui| {
                let sort = list.display.sort;
                for key in WatchlistSortKey::ALL {
                    let chosen = kit::menu_row(
                        ui,
                        kit::MenuRow {
                            checked: sort.key == key,
                            ..kit::MenuRow::new(key.title())
                        },
                    );
                    if chosen.clicked() {
                        let sort = if key == WatchlistSortKey::Manual {
                            WatchlistSort::MANUAL
                        } else {
                            WatchlistSort {
                                key,
                                ascending: key == WatchlistSortKey::Symbol,
                            }
                        };
                        actions.push(WatchlistAction::SetSort {
                            list: list.id,
                            sort,
                        });
                        ui.close();
                    }
                }
                kit::menu_separator(ui);
                let manual = sort.key == WatchlistSortKey::Manual;
                let descending = kit::menu_row(
                    ui,
                    kit::MenuRow {
                        checked: !sort.ascending && !manual,
                        enabled: !manual,
                        ..kit::MenuRow::new("Descending")
                    },
                );
                if descending.clicked() {
                    actions.push(WatchlistAction::SetSort {
                        list: list.id,
                        sort: WatchlistSort {
                            key: sort.key,
                            ascending: !sort.ascending,
                        },
                    });
                }
            },
        );

        kit::menu_submenu(
            ui,
            row(Icon::SplitColumns, "Columns"),
            SUBMENU_WIDTH,
            |ui| {
                for column in WatchlistColumn::ALL {
                    let visible = list.display.columns.contains(&column);
                    let toggle = kit::menu_row(
                        ui,
                        kit::MenuRow {
                            checked: visible,
                            ..kit::MenuRow::new(column.accessibility_title())
                        },
                    );
                    if toggle.clicked() {
                        actions.push(WatchlistAction::SetColumn {
                            list: list.id,
                            column,
                            visible: !visible,
                        });
                    }
                }
                kit::menu_separator(ui);
                let shows = list.display.shows_description;
                let subtitle = kit::menu_row(
                    ui,
                    kit::MenuRow {
                        checked: shows,
                        ..kit::MenuRow::new("Show Symbol Subtitle")
                    },
                );
                if subtitle.clicked() {
                    actions.push(WatchlistAction::SetShowsDescription {
                        list: list.id,
                        shows: !shows,
                    });
                }
            },
        );

        kit::menu_submenu(ui, row(Icon::Tag, "Filter by Flag"), SUBMENU_WIDTH, |ui| {
            for flag in WatchlistFlag::ALL {
                if flag_row(ui, flag, state.filter.flag == Some(flag)).clicked() {
                    state.filter.flag = Some(flag);
                    ui.close();
                }
            }
            kit::menu_separator(ui);
            if kit::menu_item(
                ui,
                Icon::Xmark,
                "Show All",
                state.filter.flag.is_some(),
                false,
            ) {
                state.filter.flag = None;
                ui.close();
            }
        });

        let details = kit::menu_row(
            ui,
            kit::MenuRow {
                icon: Some(Icon::Eye),
                current: state.shows_detail,
                ..kit::MenuRow::new("Show Details")
            },
        );
        if details.clicked() {
            state.shows_detail = !state.shows_detail;
        }

        kit::menu_separator(ui);
        if kit::menu_item(ui, Icon::Download, "Import Symbols…", true, false) {
            state.import_text = Some(String::new());
            state.import_error = None;
            actions.push(WatchlistAction::BeginImport);
            ui.close();
        }
        if kit::menu_item(ui, Icon::ShareUp, "Export to File…", true, false) {
            actions.push(WatchlistAction::Export { list: list.id });
            ui.close();
        }
        if kit::menu_item(ui, Icon::CopyDoc, "Copy as Text", true, false) {
            actions.push(WatchlistAction::CopyText(
                crate::model::watchlist::text::export(list),
            ));
            ui.close();
        }
    });
}

pub(super) fn filter_field(ui: &mut egui::Ui, state: &mut WatchlistSidebarState) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 32.0), Sense::hover());
    icons::paint(
        ui.painter(),
        Rect::from_center_size(
            Pos2::new(rect.left() + LEADING_INSET + 7.0, rect.top() + 11.0),
            Vec2::splat(14.0),
        ),
        Icon::Magnifier,
        theme::text_secondary(),
    );
    let clear_width = if state.filter.is_active() { 24.0 } else { 0.0 };
    let flag_width = if state.filter.flag.is_some() {
        14.0
    } else {
        0.0
    };
    let text_rect = Rect::from_min_max(
        Pos2::new(rect.left() + LEADING_INSET + 22.0, rect.top() + 1.0),
        Pos2::new(
            rect.right() - TRAILING_INSET - clear_width - flag_width,
            rect.top() + 25.0,
        ),
    );
    let response = ui.put(
        text_rect,
        egui::TextEdit::singleline(&mut state.filter.text)
            .hint_text("Filter symbols")
            .frame(egui::Frame::NONE),
    );
    if response.lost_focus() {
        state.filter_blurred = true;
    }
    if std::mem::take(&mut state.focus_filter) {
        response.request_focus();
    }
    if response.has_focus() && ui.input(|input| input.key_pressed(egui::Key::Escape)) {
        state.filter_visible = false;
        state.clear_filter();
    }
    if let Some(flag) = state.filter.flag {
        style::paint_flag(
            ui.painter(),
            rect.right() - TRAILING_INSET - clear_width - 8.0,
            rect.top() + 12.0,
            flag,
        );
    }
    if state.filter.is_active() {
        let clear_rect = Rect::from_center_size(
            Pos2::new(rect.right() - TRAILING_INSET - 10.0, rect.top() + 12.0),
            Vec2::splat(20.0),
        );
        if kit::hover_button(
            ui,
            clear_rect,
            Id::new("watchlist-filter-clear"),
            Icon::XmarkCircleFill,
            "Clear filter",
            true,
            true,
        )
        .clicked()
        {
            state.clear_filter();
            response.request_focus();
        }
    }
}
