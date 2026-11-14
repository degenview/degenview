//! The sidebar's rows, with drag reorder and its insertion line, and the empty state.

use super::menus::{MENU_WIDTH, SUBMENU_WIDTH, instrument_menu, section_menu};
use super::{
    Drag, WatchlistAction, WatchlistSidebarInput, WatchlistSidebarState, centered_message,
};

use crate::model::watchlist;
use crate::model::watchlist::layout::Row;
use crate::model::watchlist::{Watchlist, WatchlistSortKey};
use crate::ui::icons::{self, Icon};
use crate::ui::kit;
use crate::ui::theme;
use crate::ui::watchlist::rows::{self, RowStyle};
use crate::ui::watchlist::style::{LEADING_INSET, TRAILING_INSET};
use eframe::egui::{self, Id, Pos2, Rect, Sense, Stroke, Vec2};

pub(super) fn content(
    ui: &mut egui::Ui,
    state: &mut WatchlistSidebarState,
    input: &WatchlistSidebarInput,
    list: &Watchlist,
    rows: &[Row],
    actions: &mut Vec<WatchlistAction>,
) {
    if rows.is_empty() {
        empty_state(ui, state, list, actions);
        return;
    }

    if let Some(key) = rows::column_header(ui, &list.display) {
        let sort = list.display.sort.after_clicking(key);
        actions.push(WatchlistAction::SetSort {
            list: list.id,
            sort,
        });
    }

    // Order on screen is the stored order only in Manual with no filter.
    let derived_order =
        list.display.sort.key != WatchlistSortKey::Manual || state.filter.is_active();
    let mut rects: Vec<Rect> = Vec::with_capacity(rows.len());
    let mut drop_requested = false;
    let mut drag_started = None;

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for (index, row) in rows.iter().enumerate() {
                let dragged = state.drag.as_ref().is_some_and(|drag| drag.index == index);
                let response = match row {
                    Row::Section { section, count } => {
                        let response = rows::section_row(ui, section, *count, index == 0, dragged);
                        if response.clicked() {
                            actions.push(WatchlistAction::SetSectionCollapsed {
                                list: list.id,
                                section: section.id,
                                collapsed: !section.is_collapsed,
                            });
                        }
                        kit::context_popup(&response)
                            .width(SUBMENU_WIDTH)
                            .show(|ui| {
                                section_menu(
                                    ui,
                                    state,
                                    list,
                                    section.id,
                                    &section.title,
                                    section.is_collapsed,
                                    actions,
                                );
                            });
                        response
                    }
                    Row::Instrument { item, .. } => {
                        let style = RowStyle {
                            display: &list.display,
                            flags: input.flags,
                            quote: input.quote,
                            selected: state.selected_entry == Some(item.id),
                            dragged,
                        };
                        let response = rows::instrument_row(ui, item, &style);
                        let more_rect = Rect::from_center_size(
                            Pos2::new(
                                response.rect.right() - TRAILING_INSET - 11.0,
                                response.rect.center().y,
                            ),
                            Vec2::splat(22.0),
                        );
                        let more = ui.interact(
                            more_rect,
                            Id::new(("watchlist-row-more", item.id)),
                            Sense::click(),
                        );
                        let more = crate::ui::accessibility::label(
                            more,
                            format!("Actions for {}", item.name),
                        );
                        let popup = kit::menu_popup(&more).width(MENU_WIDTH);
                        let open = popup.is_open();
                        if response.hovered() || more.hovered() || open {
                            // Opaque, so the row's own trailing text does not show through.
                            ui.painter().rect_filled(more_rect, 6.0, theme::chrome());
                            if open || more.is_pointer_button_down_on() {
                                ui.painter().rect_filled(more_rect, 6.0, theme::ink(26));
                            } else if more.hovered() {
                                ui.painter().rect_filled(more_rect, 6.0, theme::ink(14));
                            }
                            icons::paint(
                                ui.painter(),
                                Rect::from_center_size(more_rect.center(), Vec2::splat(13.0)),
                                Icon::Ellipsis,
                                theme::text_secondary(),
                            );
                        }
                        if response.clicked() && !more.clicked() {
                            state.selected_entry = Some(item.id);
                            actions.push(WatchlistAction::Open(item.clone()));
                        }
                        popup.show(|ui| instrument_menu(ui, state, input, list, item, actions));
                        kit::context_popup(&response)
                            .width(MENU_WIDTH)
                            .show(|ui| instrument_menu(ui, state, input, list, item, actions));
                        response
                    }
                    Row::EmptySection { .. } => rows::empty_section_row(ui),
                };
                rects.push(response.rect);

                if row.entry_id().is_some() {
                    if response.drag_started() {
                        drag_started = Some(index);
                    }
                    if response.drag_stopped() {
                        drop_requested = true;
                    }
                }
            }
        });

    if let Some(index) = drag_started {
        state.drag = Some(Drag { index });
    }
    if let Some(drag) = &state.drag {
        let pointer = ui.input(|input| input.pointer.latest_pos());
        let destination = pointer.map(|pointer| insertion_index(&rects, pointer.y));
        if let Some(destination) = destination {
            paint_insertion_line(ui, &rects, destination);
        }
        if drop_requested {
            if let Some(destination) = destination {
                let source = drag.index;
                let dragged_is_symbol = matches!(rows.get(source), Some(Row::Instrument { .. }));
                if derived_order && dragged_is_symbol {
                    if let Some(mv) =
                        watchlist::layout::resolve_section_move(rows, &[source], destination)
                    {
                        actions.push(WatchlistAction::MoveToSectionByDrop { list: list.id, mv });
                    }
                } else if let Some(mv) =
                    watchlist::layout::resolve_move(rows, &[source], destination)
                {
                    actions.push(WatchlistAction::Move { list: list.id, mv });
                }
            }
            state.drag = None;
        }
    }
    // Only a row that is drawn can be opened or removed from the keyboard, and not on the
    // frame the filter box lets go of focus (its Enter confirms the filter, not a row).
    if !ui.ctx().egui_wants_keyboard_input()
        && !state.filter_blurred
        && let Some(item) = state.selected_entry.and_then(|id| {
            rows.iter().find_map(|row| match row {
                Row::Instrument { item, .. } if item.id == id => Some(item),
                _ => None,
            })
        })
    {
        if ui.input(|input| input.key_pressed(egui::Key::Enter)) {
            actions.push(WatchlistAction::Open(item.clone()));
        }
        if ui.input(|input| {
            input.key_pressed(egui::Key::Delete) || input.key_pressed(egui::Key::Backspace)
        }) {
            actions.push(WatchlistAction::RemoveEntry {
                list: list.id,
                entry: item.id,
            });
            state.selected_entry = None;
        }
    }
}

/// The row boundary the pointer is nearest to: before the first row whose middle is
/// below it, or the end.
pub(super) fn insertion_index(rects: &[Rect], pointer_y: f32) -> usize {
    rects
        .iter()
        .position(|rect| rect.center().y > pointer_y)
        .unwrap_or(rects.len())
}

fn paint_insertion_line(ui: &egui::Ui, rects: &[Rect], destination: usize) {
    let Some(first) = rects.first() else {
        return;
    };
    let y = match rects.get(destination) {
        Some(rect) => rect.top(),
        None => rects.last().map_or(first.top(), |rect| rect.bottom()),
    };
    let painter = ui.painter();
    let (left, right) = (first.left() + LEADING_INSET, first.right() - TRAILING_INSET);
    painter.line_segment(
        [Pos2::new(left, y), Pos2::new(right, y)],
        Stroke::new(2.0, theme::ACCENT),
    );
    painter.circle_filled(Pos2::new(left, y), 3.0, theme::ACCENT);
}

fn empty_state(
    ui: &mut egui::Ui,
    state: &mut WatchlistSidebarState,
    list: &Watchlist,
    actions: &mut Vec<WatchlistAction>,
) {
    if state.filter.is_active() {
        centered_message(
            ui,
            Icon::Magnifier,
            "No Matches",
            &format!("Nothing in {} matches this filter.", list.name),
        );
        ui.vertical_centered(|ui| {
            if ui.button("Clear Filter").clicked() {
                state.clear_filter();
            }
        });
    } else {
        centered_message(
            ui,
            Icon::Star,
            "No Symbols",
            "Add a stock, crypto, or prediction-market item to follow it here.",
        );
        ui.vertical_centered(|ui| {
            if ui.button("Add Symbol").clicked() {
                actions.push(WatchlistAction::AddSymbol {
                    list: Some(list.id),
                });
            }
        });
    }
}
