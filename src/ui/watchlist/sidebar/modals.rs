//! The sidebar's modals: the name prompt, the delete confirmation and the import sheet.

use super::{PromptKind, WatchlistAction, WatchlistSidebarInput, WatchlistSidebarState};

use crate::model::watchlist::Watchlist;
use crate::ui::icons::Icon;
use crate::ui::kit::sheet;
use crate::ui::kit::{self, Confirm, ConfirmOutcome};
use crate::ui::theme;
use eframe::egui::{self, Align, Id, Layout, RichText};

pub(super) fn modals(
    ctx: &egui::Context,
    state: &mut WatchlistSidebarState,
    input: &WatchlistSidebarInput,
    list: Option<&Watchlist>,
    actions: &mut Vec<WatchlistAction>,
) {
    prompt_window(ctx, state, actions);
    delete_window(ctx, state, list, actions);
    import_window(ctx, state, input, list, actions);
    if let Some(notice) = input.error {
        let dismissed = kit::notice(
            ctx,
            Id::new("watchlist-error"),
            &kit::Notice {
                icon: Icon::Triangle,
                tint: theme::ORANGE,
                title: notice.title,
                message: &notice.message,
            },
        );
        if dismissed {
            actions.push(WatchlistAction::DismissError);
        }
    }
}

pub(super) const PROMPT_WIDTH: f32 = 400.0;

pub(super) fn prompt_window(
    ctx: &egui::Context,
    state: &mut WatchlistSidebarState,
    actions: &mut Vec<WatchlistAction>,
) {
    let Some(kind) = state.prompt.clone() else {
        return;
    };
    let mut commit = false;
    let mut cancel = false;
    let message = kind.message();
    let field_id = Id::new("watchlist-prompt-name");
    let valid = Watchlist::validated(&state.prompt_text).is_ok();
    let fresh = ctx.data_mut(|data| data.get_temp::<bool>(field_id).is_none());
    sheet::show(ctx, Id::new("watchlist-prompt"), PROMPT_WIDTH, |ui| {
        ui.add_space(24.0);
        sheet::inset(ui, |ui| {
            sheet::header(ui, Icon::List, kind.title(), &message, theme::ACCENT);
            ui.add_space(20.0);
            let focused = ui.memory(|memory| memory.has_focus(field_id));
            kit::field_frame(focused).show(ui, |ui| {
                ui.set_width(ui.available_width());
                let width = ui.available_width();
                let response = kit::plain_field(
                    ui,
                    field_id,
                    &mut state.prompt_text,
                    "Name",
                    kit::size::BODY,
                    kit::REGULAR,
                    width,
                );
                // A name is at most `MAX_NAME_CHARS`; a longer paste is cut, not refused.
                if let Some((end, _)) = state
                    .prompt_text
                    .char_indices()
                    .nth(crate::model::watchlist::MAX_NAME_CHARS)
                {
                    state.prompt_text.truncate(end);
                }
                if fresh {
                    response.request_focus();
                    // Select the current name so typing replaces it.
                    if let Some(mut edit) = egui::TextEdit::load_state(ui.ctx(), field_id) {
                        edit.cursor
                            .set_char_range(Some(egui::text::CCursorRange::two(
                                egui::text::CCursor::new(0),
                                egui::text::CCursor::new(state.prompt_text.chars().count()),
                            )));
                        edit.store(ui.ctx(), field_id);
                    }
                }
            });
            ui.add_space(22.0);
        });
        ui.separator();
        sheet::inset(ui, |ui| {
            ui.add_space(14.0);
            ui.horizontal(|ui| {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 12.0;
                    if kit::sheet_button(ui, kind.confirm(), true, valid).clicked() {
                        commit = true;
                    }
                    if kit::sheet_button(ui, "Cancel", false, true).clicked() {
                        cancel = true;
                    }
                });
            });
            ui.add_space(14.0);
        });
    });
    ctx.data_mut(|data| data.insert_temp(field_id, true));
    if ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
        cancel = true;
    } else if valid && ctx.input(|input| input.key_pressed(egui::Key::Enter)) {
        commit = true;
    }
    if commit || cancel {
        ctx.data_mut(|data| data.remove::<bool>(field_id));
    }
    if commit {
        let name = state.prompt_text.trim().to_owned();
        match kind {
            PromptKind::NewList => actions.push(WatchlistAction::CreateList { name, with: None }),
            PromptKind::NewListWith(item) => actions.push(WatchlistAction::CreateList {
                name,
                with: Some(*item),
            }),
            PromptKind::RenameList(id) => actions.push(WatchlistAction::RenameList { id, name }),
            PromptKind::CopyList(id) => actions.push(WatchlistAction::DuplicateList { id, name }),
            PromptKind::NewSection(list) => {
                actions.push(WatchlistAction::AddSection { list, title: name })
            }
            PromptKind::RenameSection { list, section } => {
                actions.push(WatchlistAction::RenameSection {
                    list,
                    section,
                    title: name,
                });
            }
        }
        state.prompt = None;
    } else if cancel {
        state.prompt = None;
    }
}

fn delete_window(
    ctx: &egui::Context,
    state: &mut WatchlistSidebarState,
    list: Option<&Watchlist>,
    actions: &mut Vec<WatchlistAction>,
) {
    if !state.confirm_delete {
        return;
    }
    let Some(list) = list else {
        state.confirm_delete = false;
        return;
    };
    let title = format!("Delete “{}”?", list.name);
    let result = kit::confirm(
        ctx,
        Id::new("watchlist-delete"),
        &Confirm {
            icon: Icon::Trash,
            title: &title,
            message: "Its symbols stay in your other watchlists. Charts, drawings and alerts are not affected.",
            destructive: "Delete",
        },
    );
    if result == Some(ConfirmOutcome::Confirmed) {
        actions.push(WatchlistAction::DeleteList(list.id));
    }
    if result.is_some() {
        state.confirm_delete = false;
    }
}

fn import_window(
    ctx: &egui::Context,
    state: &mut WatchlistSidebarState,
    input: &WatchlistSidebarInput,
    list: Option<&Watchlist>,
    actions: &mut Vec<WatchlistAction>,
) {
    let (Some(text), Some(list)) = (state.import_text.as_mut(), list) else {
        return;
    };
    let mut close = false;
    let mut submit = false;
    sheet::show(ctx, Id::new("watchlist-import"), 460.0, |ui| {
        ui.set_height(380.0);
        ui.add_space(20.0);
        sheet::inset(ui, |ui| {
            sheet::header(
                ui,
                Icon::Download,
                &format!("Import into {}", list.name),
                "Paste symbols or open a text file.",
                theme::ACCENT,
            );
            ui.add_space(14.0);
            if let Some(report) = input.import_report {
                ui.label(RichText::new(report.summary()).strong().color(
                    if report.skipped.is_empty() {
                        theme::UP
                    } else {
                        theme::ORANGE
                    },
                ));
                if !report.skipped.is_empty() {
                    ui.add_space(10.0);
                    ui.label(
                        RichText::new("SKIPPED")
                            .small()
                            .strong()
                            .color(ui.visuals().weak_text_color()),
                    );
                    egui::ScrollArea::vertical()
                        .max_height(190.0)
                        .show(ui, |ui| {
                            for skipped in &report.skipped {
                                ui.label(RichText::new(&skipped.text).monospace().small());
                                ui.label(
                                    RichText::new(format!(
                                        "Line {} · {}",
                                        skipped.line, skipped.reason
                                    ))
                                    .small()
                                    .color(ui.visuals().weak_text_color()),
                                );
                                ui.add_space(7.0);
                            }
                        });
                }
            } else {
                ui.label(RichText::new("One symbol per line as SOURCE:SYMBOL, for example BINANCE:BTCUSDT or COINBASE:ETH-USD. Lines starting with ### become sections. A single comma-separated line from TradingView is read too; markets DegenView has no provider for are skipped.").small().color(ui.visuals().weak_text_color()));
                ui.add_space(10.0);
                ui.add_sized(
                    [ui.available_width(), 155.0],
                    egui::TextEdit::multiline(text)
                        .char_limit(crate::model::watchlist::text::MAX_IMPORT_BYTES)
                        .font(egui::TextStyle::Monospace)
                        .hint_text("BINANCE:BTCUSDT\nCOINBASE:ETH-USD"),
                );
                if let Some(error) = &state.import_error {
                    ui.label(RichText::new(error).small().color(theme::DOWN));
                }
            }
            ui.add_space(14.0);
        });
        ui.separator();
        sheet::inset(ui, |ui| {
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                if input.import_report.is_none()
                    && ui
                        .add_enabled(
                            state.import_picker.is_none(),
                            egui::Button::new("Open File…"),
                        )
                        .clicked()
                {
                    actions.push(WatchlistAction::ChooseImportFile);
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if input.import_report.is_some() {
                        if ui.button("Done").clicked() {
                            close = true;
                        }
                    } else {
                        if ui
                            .add_enabled(!text.trim().is_empty(), egui::Button::new("Import"))
                            .clicked()
                        {
                            submit = true;
                        }
                        if ui.button("Cancel").clicked() {
                            close = true;
                        }
                    }
                });
            });
            ui.add_space(12.0);
        });
    });
    if submit {
        actions.push(WatchlistAction::Import {
            list: list.id,
            text: text.clone(),
        });
    }
    if close || ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
        state.import_text = None;
        state.import_picker = None;
        state.import_error = None;
    }
}
