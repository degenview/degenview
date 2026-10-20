//! Settings ▸ iCloud Sync: the switch, what is happening, what needs a decision, what syncs.
//!
//! A view over `SyncHost`: it reads a `SyncView` and sends `SyncAction`s, and never talks to
//! iCloud or the store itself. Anything that moves data between Apple Accounts or discards a
//! copy of this Mac's data is a button the user presses, with the consequence written next to it.

use crate::sync::account::AccountChoice;
use crate::sync::core::{PauseReason, Phase};
use crate::sync::initial::{Counts, JoinMode, JoinSummary};
use crate::sync::transport::ErrorKind;
use crate::sync_host::{SyncAction, SyncHost, SyncNotice, SyncPrompt, SyncView};
use crate::ui::alert_center::{Tone, relative_time};
use crate::ui::icons::Icon;
use crate::ui::settings_window::{card, divider, row, section, status_badge, status_message};
use crate::ui::{kit, settings_card, theme};
use eframe::egui;

/// The status line and its colour. Transient trouble is calm; only problems the user can act on
/// are red.
pub fn phase_label(phase: Phase) -> (String, Tone) {
    let (text, tone) = match phase {
        Phase::Off => ("Off", Tone::Neutral),
        Phase::CheckingAccount => ("Checking iCloud…", Tone::Neutral),
        Phase::NeedsSetup => ("Waiting for your OK", Tone::Warning),
        Phase::AwaitingJoin => ("Choose how to combine data", Tone::Warning),
        Phase::Ready => ("Ready", Tone::Neutral),
        Phase::Syncing => ("Syncing…", Tone::Neutral),
        Phase::UpToDate => ("Up to date", Tone::Good),
        Phase::WaitingForNetwork => ("Waiting for the network", Tone::Neutral),
        Phase::WaitingForAccount => ("Sign in to iCloud to sync", Tone::Warning),
        Phase::Paused(PauseReason::AccountChanged) => {
            ("Paused: a different Apple Account", Tone::Warning)
        }
        Phase::Paused(PauseReason::RemoteDataDeleted) => {
            ("Paused: iCloud data was removed", Tone::Warning)
        }
        Phase::Paused(PauseReason::NeedsUpdate) => {
            ("Update DegenView to keep syncing", Tone::Warning)
        }
        Phase::NeedsAttention => ("Needs your attention", Tone::Warning),
        Phase::Error(kind) => return (error_label(kind).to_owned(), Tone::Bad),
    };
    (text.to_owned(), tone)
}

fn error_label(kind: ErrorKind) -> &'static str {
    match kind {
        ErrorKind::Quota => "iCloud storage is full",
        ErrorKind::Permission => "DegenView is not allowed to use iCloud",
        ErrorKind::AccountUnavailable => "iCloud is unavailable",
        _ => "Something went wrong syncing",
    }
}

/// "3 watchlists, 2 layouts, 1 script", leaving out what is zero.
pub fn counts_text(counts: &Counts) -> String {
    let parts = [
        (counts.watchlists, "watchlist", "watchlists"),
        (counts.layouts, "layout", "layouts"),
        (counts.templates, "template", "templates"),
        (counts.scripts, "script", "scripts"),
        (counts.alerts, "alert", "alerts"),
        (counts.portfolios, "portfolio", "portfolios"),
        (counts.paper_accounts, "paper account", "paper accounts"),
    ];
    let listed: Vec<String> = parts
        .iter()
        .filter(|(n, _, _)| *n > 0)
        .map(|(n, one, many)| format!("{n} {}", if *n == 1 { one } else { many }))
        .collect();
    if listed.is_empty() {
        "nothing yet".to_owned()
    } else {
        listed.join(", ")
    }
}

pub fn conflict_text(kind: &str) -> String {
    let what = match kind {
        "layout" => "A layout",
        "script" => "A Pine script",
        _ => "An item",
    };
    format!(
        "{what} was changed on two Macs. Both versions were kept; look for \"conflict from\" in the name."
    )
}

const NOT_SYNCED: &str = "Never synced: market data caches, portfolio value history (rebuilt on each Mac), \
webhook endpoints and their secrets, API keys, MCP access, and which windows and tabs are open. \
Alerts you sync arrive switched off on your other Macs, so each one notifies from one Mac only.";

const PRIVACY: &str = "Your data is stored in your private iCloud storage for DegenView, tied to \
your Apple Account. Pine scripts are synced as written, so remove any secrets you pasted into them.";

pub fn body(ui: &mut egui::Ui, sync: &SyncHost, now_ms: i64) {
    let view = sync.view();
    show(ui, sync, &view, now_ms);
}

fn show(ui: &mut egui::Ui, sync: &SyncHost, view: &SyncView, now_ms: i64) {
    if let Some(reason) = &view.unavailable {
        kit::notice_card(
            ui,
            Icon::TriangleFill,
            kit::ORANGE,
            "iCloud Sync isn't available",
            Some(reason),
            None,
        );
        return;
    }
    notice(ui, sync, view);
    prompts(ui, sync, view);
    section(ui, "iCloud Sync", None, |ui| {
        settings_card::card(
            ui,
            Icon::Cycle,
            "Sync DegenView data with iCloud",
            Some(
                "Keeps your watchlists, layouts, drawings, templates, scripts, portfolios and paper-trading accounts the same on every Mac signed in to your Apple Account. Everything also stays on this Mac.",
            ),
            |ui| {
                let mut on = view.enabled;
                if settings_card::switch(ui, &mut on) {
                    sync.act(if on {
                        SyncAction::Enable
                    } else {
                        SyncAction::Disable
                    });
                }
            },
        );
    });
    if !view.enabled {
        ui.add_space(10.0);
        small(
            ui,
            "Turning sync off keeps all your data here and removes nothing from iCloud.",
        );
        return;
    }
    status(ui, sync, view, now_ms);
    attention(ui, sync, view);
    categories(ui, sync, view);
    ui.add_space(14.0);
    small(ui, NOT_SYNCED);
    ui.add_space(8.0);
    small(ui, PRIVACY);
    delete_section(ui, sync, view);
}

/// A finished thing worth a line, until the user dismisses it. Shown even when sync is off, which
/// is where the page lands after iCloud's data was deleted.
fn notice(ui: &mut egui::Ui, sync: &SyncHost, view: &SyncView) {
    let Some(SyncNotice::CloudDeleted) = view.notice else {
        return;
    };
    kit::notice_card(
        ui,
        Icon::CheckCircleFill,
        kit::GREEN,
        "DegenView's data was deleted from iCloud",
        Some(
            "Sync is off. Everything on this Mac is still here, and your other Macs will ask what to do with their copies.",
        ),
        None,
    );
    ui.add_space(8.0);
    if kit::push_button(ui, None, "OK", true).clicked() {
        sync.act(SyncAction::DismissNotice);
    }
    ui.add_space(14.0);
}

/// The word the user types to confirm deleting everything in iCloud.
pub const DELETE_WORD: &str = "delete";

/// Whether what was typed confirms the deletion.
pub fn delete_confirmed(typed: &str) -> bool {
    typed.trim().eq_ignore_ascii_case(DELETE_WORD)
}

/// Whether the delete action is offered at all, and whether it can run right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeleteOffer {
    /// Nothing to delete from here: sync is off, not set up yet, or paused for another reason.
    Hidden,
    /// Offered, but iCloud cannot be reached, so the button waits.
    Waiting(&'static str),
    Ready,
}

pub fn delete_offer(view: &SyncView) -> DeleteOffer {
    if !view.enabled || view.prompt.is_some() {
        return DeleteOffer::Hidden;
    }
    match view.phase {
        Phase::Ready
        | Phase::Syncing
        | Phase::UpToDate
        | Phase::NeedsAttention
        | Phase::Error(_) => DeleteOffer::Ready,
        Phase::WaitingForNetwork => DeleteOffer::Waiting("Connect to the internet to delete."),
        Phase::WaitingForAccount => DeleteOffer::Waiting("Sign in to iCloud to delete."),
        Phase::Off
        | Phase::CheckingAccount
        | Phase::NeedsSetup
        | Phase::AwaitingJoin
        | Phase::Paused(_) => DeleteOffer::Hidden,
    }
}

const DELETE_CARD_HINT: &str = "Removes everything DegenView stored in your iCloud and turns sync off. Your data on this Mac is kept.";

/// A separate, red-edged card at the foot of the page. Deleting iCloud data is a different thing
/// from turning sync off, and the card says so before anything is asked.
fn delete_section(ui: &mut egui::Ui, sync: &SyncHost, view: &SyncView) {
    let offer = delete_offer(view);
    if offer == DeleteOffer::Hidden && !view.deleting {
        return;
    }
    let dialog_id = egui::Id::new("icloud-delete-dialog");
    let mut open: bool = ui.data(|data| data.get_temp(dialog_id)).unwrap_or(false);
    ui.add_space(22.0);
    section(ui, "Delete iCloud data", None, |ui| {
        let tint = kit::RED;
        egui::Frame::new()
            .fill(theme::with_alpha(tint, 0.06))
            .stroke(egui::Stroke::new(1.0, theme::with_alpha(tint, 0.24)))
            .corner_radius(10.0)
            .inner_margin(egui::Margin::same(14))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 14.0;
                    let (badge, _) =
                        ui.allocate_exact_size(egui::Vec2::splat(30.0), egui::Sense::hover());
                    ui.painter()
                        .rect_filled(badge, 8.0, theme::with_alpha(tint, 0.14));
                    crate::ui::icons::paint(
                        ui.painter(),
                        egui::Rect::from_center_size(badge.center(), egui::Vec2::splat(14.0)),
                        Icon::Trash,
                        tint,
                    );
                    ui.vertical(|ui| {
                        ui.set_max_width((ui.available_width() - 150.0).max(140.0));
                        ui.spacing_mut().item_spacing.y = 3.0;
                        ui.label(kit::text(
                            "Delete iCloud Data",
                            12.0,
                            kit::SEMIBOLD,
                            theme::text_primary(),
                        ));
                        ui.add(
                            egui::Label::new(kit::text(
                                DELETE_CARD_HINT,
                                10.5,
                                kit::REGULAR,
                                theme::text_secondary(),
                            ))
                            .wrap(),
                        );
                        if let DeleteOffer::Waiting(why) = offer {
                            ui.label(kit::text(why, 10.5, kit::REGULAR, theme::text_tertiary()));
                        }
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if view.deleting {
                            ui.label(kit::text(
                                "Deleting…",
                                kit::size::BODY,
                                kit::REGULAR,
                                theme::text_secondary(),
                            ));
                            ui.add(egui::Spinner::new().size(14.0));
                        } else if offer == DeleteOffer::Ready {
                            let label = if view.delete_error.is_some() {
                                "Try Again…"
                            } else {
                                "Delete…"
                            };
                            if crate::ui::settings_api_keys::destructive_button(ui, label).clicked()
                            {
                                open = true;
                            }
                        } else {
                            ui.add_enabled_ui(false, |ui| {
                                crate::ui::settings_api_keys::destructive_button(ui, "Delete…");
                            });
                        }
                    });
                });
            });
        if let Some(error) = &view.delete_error
            && !view.deleting
        {
            ui.add_space(8.0);
            status_message(ui, &format!("{error} Nothing was deleted."), true);
        }
    });

    if open && !view.deleting {
        match delete_dialog(ui.ctx()) {
            Some(DialogOutcome::Delete) => {
                sync.act(SyncAction::DeleteCloudData);
                open = false;
            }
            Some(DialogOutcome::Cancel) => open = false,
            None => {}
        }
    } else {
        open = false;
    }
    ui.data_mut(|data| data.insert_temp(dialog_id, open));
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DialogOutcome {
    Delete,
    Cancel,
}

/// The confirmation: what goes, what stays, and the word to type. Escape or a click outside
/// cancels; Return confirms only once the word is typed.
fn delete_dialog(ctx: &egui::Context) -> Option<DialogOutcome> {
    let id = egui::Id::new("icloud-delete-dialog-text");
    let mut typed: String = ctx.data(|data| data.get_temp(id)).unwrap_or_default();
    let mut outcome = None;
    let modal = egui::Modal::new(egui::Id::new("icloud-delete-modal"))
        .backdrop_color(egui::Color32::from_black_alpha(90))
        .frame(
            egui::Frame::new()
                .fill(theme::pick(
                    egui::Color32::from_rgb(44, 52, 57),
                    egui::Color32::from_rgb(246, 246, 247),
                ))
                .corner_radius(12.0)
                .stroke(egui::Stroke::new(1.0, kit::primary(0.1)))
                .inner_margin(egui::Margin::same(22))
                .shadow(egui::Shadow {
                    offset: [0, 10],
                    blur: 30,
                    spread: 0,
                    color: egui::Color32::from_black_alpha(130),
                }),
        )
        .show(ctx, |ui| {
            ui.set_width(340.0);
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.vertical_centered(|ui| {
                let (badge, _) = ui.allocate_exact_size(egui::Vec2::splat(52.0), egui::Sense::hover());
                ui.painter().rect_filled(badge, 12.0, theme::with_alpha(kit::RED, 0.14));
                crate::ui::icons::paint(
                    ui.painter(),
                    egui::Rect::from_center_size(badge.center(), egui::Vec2::splat(24.0)),
                    Icon::Trash,
                    kit::RED,
                );
                ui.add_space(12.0);
                ui.label(kit::text(
                    "Delete all DegenView data from iCloud?",
                    kit::size::HEADLINE,
                    kit::BOLD,
                    theme::text_primary(),
                ));
                ui.add_space(6.0);
                ui.add(
                    egui::Label::new(kit::text(
                        "This permanently removes what DegenView synced to your private iCloud storage.",
                        kit::size::SUBHEADLINE,
                        kit::REGULAR,
                        theme::text_secondary(),
                    ))
                    .wrap()
                    .halign(egui::Align::Center),
                );
            });
            ui.add_space(16.0);
            egui::Frame::new()
                .fill(kit::primary(0.05))
                .corner_radius(10.0)
                .inner_margin(egui::Margin::symmetric(12, 10))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.spacing_mut().item_spacing.y = 10.0;
                    consequence(
                        ui,
                        Icon::XmarkCircleFill,
                        kit::RED,
                        "Removed from iCloud",
                        "Watchlists, layouts and drawings, templates, Pine scripts, alerts, portfolios, paper-trading accounts and preferences.",
                    );
                    consequence(
                        ui,
                        Icon::CheckCircleFill,
                        kit::GREEN,
                        "Kept on this Mac",
                        "All your data stays here. Sync turns off.",
                    );
                    consequence(
                        ui,
                        Icon::Layers,
                        theme::text_secondary(),
                        "Your other Macs",
                        "They keep their own copies and will offer to upload them again.",
                    );
                });
            ui.add_space(16.0);
            ui.label(kit::text(
                format!("Type “{DELETE_WORD}” to confirm"),
                kit::size::CAPTION + 0.5,
                kit::MEDIUM,
                theme::text_secondary(),
            ));
            ui.add_space(6.0);
            let field_id = egui::Id::new("icloud-delete-field");
            let focused = ui.memory(|memory| memory.has_focus(field_id));
            let field = kit::field_frame(focused)
                .show(ui, |ui| {
                    kit::plain_field(ui, field_id, &mut typed, DELETE_WORD, kit::size::BODY, kit::REGULAR, ui.available_width())
                })
                .inner;
            if !ui.memory(|memory| memory.has_focus(field_id)) && typed.is_empty() {
                field.request_focus();
            }
            let confirmed = delete_confirmed(&typed);
            let submitted = field.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
            ui.add_space(16.0);
            if dialog_button(ui, "Delete iCloud Data", Some(kit::RED), confirmed).clicked()
                || (submitted && confirmed)
            {
                outcome = Some(DialogOutcome::Delete);
            }
            ui.add_space(8.0);
            if dialog_button(ui, "Cancel", None, true).clicked() {
                outcome = Some(DialogOutcome::Cancel);
            }
        });
    if outcome.is_none() && modal.should_close() {
        outcome = Some(DialogOutcome::Cancel);
    }
    // Typed text never outlives the dialog, so it always opens empty.
    ctx.data_mut(|data| {
        if outcome.is_some() {
            data.remove::<String>(id);
        } else {
            data.insert_temp(id, typed);
        }
    });
    outcome
}

/// One line of the consequences list: an icon, a bold lead and a sentence.
fn consequence(ui: &mut egui::Ui, icon: Icon, tint: egui::Color32, lead: &str, detail: &str) {
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        ui.add_space(0.0);
        crate::ui::icons::inline(ui, icon, 14.0, tint);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.label(kit::text(
                lead,
                kit::size::SUBHEADLINE,
                kit::SEMIBOLD,
                theme::text_primary(),
            ));
            ui.add(
                egui::Label::new(kit::text(
                    detail,
                    kit::size::CAPTION + 0.5,
                    kit::REGULAR,
                    theme::text_secondary(),
                ))
                .wrap(),
            );
        });
    });
}

/// A full-width dialog button, in the style of the app's alerts. A disabled one is dimmed and
/// does nothing, so the destructive button reads as "not yet" until the word is typed.
fn dialog_button(
    ui: &mut egui::Ui,
    label: &str,
    fill: Option<egui::Color32>,
    enabled: bool,
) -> egui::Response {
    let sense = if enabled {
        egui::Sense::click()
    } else {
        egui::Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 30.0), sense);
    let response = crate::ui::accessibility::label(response, label);
    let pressed = enabled && response.is_pointer_button_down_on();
    let background = match fill {
        Some(color) if !enabled => color.gamma_multiply(0.3),
        Some(color) if pressed => color.gamma_multiply(0.8),
        Some(color) => color,
        None => kit::primary(if pressed {
            0.2
        } else if response.hovered() {
            0.15
        } else {
            0.1
        }),
    };
    ui.painter().rect_filled(rect, 7.0, background);
    let color = match (fill.is_some(), enabled) {
        (true, true) => egui::Color32::WHITE,
        (true, false) => egui::Color32::WHITE.gamma_multiply(0.55),
        (false, _) => theme::text_primary(),
    };
    let galley = kit::galley(
        ui,
        label,
        kit::size::BODY,
        if fill.is_some() {
            kit::MEDIUM
        } else {
            kit::REGULAR
        },
        color,
        rect.width(),
    );
    ui.painter()
        .galley(rect.center() - galley.size() / 2.0, galley, color);
    if enabled {
        response.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        response
    }
}

fn small(ui: &mut egui::Ui, text: &str) {
    ui.add(
        egui::Label::new(kit::text(
            text,
            kit::size::CAPTION + 0.5,
            kit::REGULAR,
            theme::text_secondary(),
        ))
        .wrap(),
    );
}

fn status(ui: &mut egui::Ui, sync: &SyncHost, view: &SyncView, now_ms: i64) {
    section(ui, "Status", None, |ui| {
        card(ui, |ui| {
            let (mut label, tone) = phase_label(view.phase);
            if matches!(view.phase, Phase::Error(_))
                && view.last_error.as_deref().is_some_and(schema_not_deployed)
            {
                label = "iCloud isn't set up for this release yet".to_owned();
            }
            row(ui, "Status", |ui| status_badge(ui, &label, tone));
            let last = view.last_send_ms.max(view.last_fetch_ms);
            if let Some(last) = last {
                divider(ui);
                row(ui, "Last synced", |ui| {
                    ui.label(kit::text(
                        relative_time(now_ms, last),
                        kit::size::BODY,
                        kit::REGULAR,
                        theme::text_secondary(),
                    ));
                });
            }
            if view.pending > 0 {
                divider(ui);
                row(ui, "Waiting to upload", |ui| {
                    ui.label(kit::text(
                        format!("{} changes", view.pending),
                        kit::size::BODY,
                        kit::REGULAR,
                        theme::text_secondary(),
                    ));
                });
            }
            if let Some(name) = &view.device_name {
                divider(ui);
                row(ui, "This Mac", |ui| {
                    ui.label(kit::text(
                        name,
                        kit::size::BODY,
                        kit::REGULAR,
                        theme::text_secondary(),
                    ));
                });
            }
            if view.environment.as_deref() == Some("Development") {
                divider(ui);
                row(ui, "iCloud environment", |ui| {
                    status_badge(ui, "Development (test data)", Tone::Warning);
                });
            }
        });
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            if kit::push_button(ui, None, "Sync Now", true).clicked() {
                sync.act(SyncAction::SyncNow);
            }
        });
        if let Some(error) = &view.last_error
            && matches!(view.phase, Phase::Error(_))
        {
            ui.add_space(8.0);
            if schema_not_deployed(error) {
                small(ui, SCHEMA_NOT_DEPLOYED_HELP);
            } else {
                small(ui, error);
            }
        }
    });
}

/// CloudKit's answer when a release build uploads before the record type has been deployed from
/// the Development environment to Production.
fn schema_not_deployed(message: &str) -> bool {
    message.contains("in production schema")
}

const SCHEMA_NOT_DEPLOYED_HELP: &str = "iCloud refused the upload because this release's iCloud database has not been deployed to production yet. Nothing is lost: your changes keep waiting here and upload on their own once it is.";

/// Decisions that must not be missed sit at the top of the page.
fn prompts(ui: &mut egui::Ui, sync: &SyncHost, view: &SyncView) {
    let Some(prompt) = &view.prompt else { return };
    match prompt {
        SyncPrompt::ConfirmAccount => {
            kit::notice_card(
                ui,
                Icon::PersonCircle,
                kit::ACCENT,
                "Use the Apple Account signed in on this Mac?",
                Some(
                    "Nothing is read from or sent to iCloud until you agree. You will choose how to combine data next.",
                ),
                None,
            );
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if kit::push_button(ui, None, "Use This Account", true).clicked() {
                    sync.act(SyncAction::ConfirmAccount);
                }
                if kit::push_button(ui, None, "Not Now", true).clicked() {
                    sync.act(SyncAction::Disable);
                }
            });
        }
        SyncPrompt::Join(summary) => join_prompt(ui, sync, summary),
        SyncPrompt::RemoteDataDeleted => {
            kit::notice_card(
                ui,
                Icon::TriangleFill,
                kit::ORANGE,
                "DegenView's data in iCloud was deleted",
                Some("Another Mac deleted it. Everything on this Mac is still here."),
                None,
            );
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if kit::push_button(ui, None, "Upload This Mac's Data Again", true).clicked() {
                    sync.act(SyncAction::ResolveRemoteDeletion { reupload: true });
                }
                if kit::push_button(ui, None, "Turn Sync Off", true).clicked() {
                    sync.act(SyncAction::ResolveRemoteDeletion { reupload: false });
                }
            });
        }
        SyncPrompt::AccountChanged => {
            kit::notice_card(
                ui,
                Icon::TriangleFill,
                kit::ORANGE,
                "A different Apple Account is signed in",
                Some(
                    "Nothing from this Mac has been sent to it. Choose what to do with your data here.",
                ),
                None,
            );
            ui.add_space(8.0);
            ui.horizontal_wrapped(|ui| {
                if kit::push_button(ui, None, "Keep Local Only", true).clicked() {
                    sync.act(SyncAction::ResolveAccount(AccountChoice::KeepLocalOnly));
                }
                if kit::push_button(ui, None, "Merge Into This Account", true).clicked() {
                    sync.act(SyncAction::ResolveAccount(
                        AccountChoice::MergeIntoThisAccount,
                    ));
                }
                if kit::push_button(ui, None, "Use This Account's Data", true).clicked() {
                    sync.act(SyncAction::ResolveAccount(
                        AccountChoice::UseThisAccountsData,
                    ));
                }
            });
            ui.add_space(4.0);
            small(
                ui,
                "Using the other account's data first saves a copy of this Mac's data that you can restore. Nothing is deleted from either account.",
            );
        }
    }
    ui.add_space(14.0);
}

fn join_prompt(ui: &mut egui::Ui, sync: &SyncHost, summary: &JoinSummary) {
    if summary.cloud_is_empty() {
        kit::notice_card(
            ui,
            Icon::Cycle,
            kit::ACCENT,
            "iCloud has no DegenView data yet",
            Some(&format!(
                "This Mac's data ({}) will be uploaded so your other Macs can have it.",
                counts_text(&summary.local)
            )),
            None,
        );
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if kit::push_button(ui, None, "Start Syncing", true).clicked() {
                sync.act(SyncAction::ChooseJoin(JoinMode::Merge));
            }
            if kit::push_button(ui, None, "Don't Sync", true).clicked() {
                sync.act(SyncAction::ChooseJoin(JoinMode::KeepLocalOnly));
            }
        });
        return;
    }
    let macs = summary.cloud_devices.len();
    kit::notice_card(
        ui,
        Icon::Cycle,
        kit::ACCENT,
        "iCloud already has DegenView data",
        Some(&format!(
            "iCloud has {}{}. This Mac has {}.",
            counts_text(&summary.cloud),
            if macs > 0 {
                format!(" from {macs} Mac{}", if macs == 1 { "" } else { "s" })
            } else {
                String::new()
            },
            counts_text(&summary.local)
        )),
        None,
    );
    ui.add_space(8.0);
    ui.horizontal_wrapped(|ui| {
        if kit::push_button(ui, None, "Merge (Recommended)", true).clicked() {
            sync.act(SyncAction::ChooseJoin(JoinMode::Merge));
        }
        if kit::push_button(ui, None, "Use iCloud's Data", true).clicked() {
            sync.act(SyncAction::ChooseJoin(JoinMode::UseCloud));
        }
        if kit::push_button(ui, None, "Don't Sync", true).clicked() {
            sync.act(SyncAction::ChooseJoin(JoinMode::KeepLocalOnly));
        }
    });
    ui.add_space(4.0);
    small(
        ui,
        "Merge keeps everything from both. Using iCloud's data replaces this Mac's watchlists, layouts and templates after saving a copy you can restore; scripts, alerts, portfolios and paper accounts are always merged.",
    );
}

fn attention(ui: &mut egui::Ui, sync: &SyncHost, view: &SyncView) {
    if view.held > 0 {
        ui.add_space(10.0);
        kit::notice_card(
            ui,
            Icon::TriangleFill,
            kit::ORANGE,
            &format!("{} items were deleted at once", view.held),
            Some("Delete them from iCloud and your other Macs too?"),
            None,
        );
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if kit::push_button(ui, None, "Delete Everywhere", true).clicked() {
                sync.act(SyncAction::ConfirmHeldDeletes);
            }
            if kit::push_button(ui, None, "Keep on Other Macs", true).clicked() {
                sync.act(SyncAction::KeepHeldDeletes);
            }
        });
    }
    if view.blocked > 0 {
        ui.add_space(10.0);
        kit::notice_card(
            ui,
            Icon::TriangleFill,
            kit::ORANGE,
            &format!("{} items are too large to sync", view.blocked),
            Some("They stay on this Mac. Shortening a very long Pine script usually fixes it."),
            None,
        );
    }
    for conflict in &view.conflicts {
        ui.add_space(10.0);
        kit::notice_card(
            ui,
            Icon::Layers,
            kit::ACCENT,
            "Two Macs changed the same thing",
            Some(&conflict_text(&conflict.kind)),
            None,
        );
        ui.add_space(6.0);
        if kit::push_button(ui, None, "OK", true).clicked() {
            sync.act(SyncAction::DismissConflict(conflict.id));
        }
    }
}

fn categories(ui: &mut egui::Ui, sync: &SyncHost, view: &SyncView) {
    section(
        ui,
        "What syncs",
        Some("Turn off anything you want to keep on this Mac only."),
        |ui| {
            card(ui, |ui| {
                for (index, (category, on)) in view.categories.iter().enumerate() {
                    if index > 0 {
                        divider(ui);
                    }
                    row(ui, category.title(), |ui| {
                        let mut value = *on;
                        if settings_card::switch(ui, &mut value) {
                            sync.act(SyncAction::SetCategory(*category, value));
                        }
                    });
                }
            });
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_phase_has_a_calm_label() {
        let phases = [
            Phase::Off,
            Phase::CheckingAccount,
            Phase::NeedsSetup,
            Phase::AwaitingJoin,
            Phase::Ready,
            Phase::Syncing,
            Phase::UpToDate,
            Phase::WaitingForNetwork,
            Phase::WaitingForAccount,
            Phase::Paused(PauseReason::AccountChanged),
            Phase::Paused(PauseReason::RemoteDataDeleted),
            Phase::Paused(PauseReason::NeedsUpdate),
            Phase::NeedsAttention,
            Phase::Error(ErrorKind::Quota),
        ];
        for phase in phases {
            let (label, _) = phase_label(phase);
            assert!(!label.is_empty());
        }
        assert_eq!(phase_label(Phase::UpToDate).1, Tone::Good);
        assert_eq!(
            phase_label(Phase::WaitingForNetwork).1,
            Tone::Neutral,
            "being offline is not an error"
        );
        assert_eq!(phase_label(Phase::Error(ErrorKind::Quota)).1, Tone::Bad);
        assert_eq!(
            phase_label(Phase::AwaitingJoin).1,
            Tone::Warning,
            "a decision is waiting"
        );
    }

    #[test]
    fn counts_read_naturally() {
        assert_eq!(counts_text(&Counts::default()), "nothing yet");
        let counts = Counts {
            watchlists: 3,
            layouts: 1,
            scripts: 2,
            ..Default::default()
        };
        assert_eq!(counts_text(&counts), "3 watchlists, 1 layout, 2 scripts");
    }

    #[test]
    fn conflicts_say_both_versions_were_kept() {
        assert!(conflict_text("layout").contains("Both versions were kept"));
        assert!(conflict_text("script").starts_with("A Pine script"));
    }

    #[test]
    fn deleting_icloud_data_needs_the_word_typed() {
        assert!(!delete_confirmed(""));
        assert!(!delete_confirmed("del"));
        assert!(!delete_confirmed("yes"));
        assert!(delete_confirmed("delete"));
        assert!(delete_confirmed("  Delete "));
    }

    #[test]
    fn the_production_schema_error_is_recognised() {
        assert!(schema_not_deployed(
            "Error saving record <CKRecordID: 0x1; recordName=watchlist.1> to server: Cannot create new type DegenViewSyncRecord in production schema"
        ));
        assert!(!schema_not_deployed("The network connection was lost"));
    }

    #[test]
    fn the_page_states_what_never_syncs() {
        for word in ["value history", "secrets", "API keys", "MCP"] {
            assert!(NOT_SYNCED.contains(word), "{word}");
        }
        assert!(PRIVACY.contains("private iCloud storage"));
        assert!(
            !PRIVACY.to_lowercase().contains("end-to-end"),
            "no unverified claim"
        );
    }

    fn ready_view() -> SyncView {
        let mut view = SyncView::initial(None);
        view.enabled = true;
        view.phase = Phase::UpToDate;
        view
    }

    #[test]
    fn deleting_is_offered_only_where_it_means_something() {
        assert_eq!(delete_offer(&ready_view()), DeleteOffer::Ready);
        for phase in [
            Phase::Ready,
            Phase::Syncing,
            Phase::NeedsAttention,
            Phase::Error(ErrorKind::Quota),
        ] {
            let mut view = ready_view();
            view.phase = phase;
            assert_eq!(delete_offer(&view), DeleteOffer::Ready, "{phase:?}");
        }
        for phase in [
            Phase::Off,
            Phase::CheckingAccount,
            Phase::NeedsSetup,
            Phase::AwaitingJoin,
            Phase::Paused(PauseReason::AccountChanged),
            Phase::Paused(PauseReason::RemoteDataDeleted),
        ] {
            let mut view = ready_view();
            view.phase = phase;
            assert_eq!(delete_offer(&view), DeleteOffer::Hidden, "{phase:?}");
        }
        let mut off = ready_view();
        off.enabled = false;
        assert_eq!(delete_offer(&off), DeleteOffer::Hidden);
        let mut asking = ready_view();
        asking.prompt = Some(SyncPrompt::ConfirmAccount);
        assert_eq!(
            delete_offer(&asking),
            DeleteOffer::Hidden,
            "a pending decision comes first"
        );
    }

    #[test]
    fn deleting_waits_with_a_reason_when_icloud_cannot_be_reached() {
        let mut view = ready_view();
        view.phase = Phase::WaitingForNetwork;
        assert!(
            matches!(delete_offer(&view), DeleteOffer::Waiting(why) if why.contains("internet"))
        );
        view.phase = Phase::WaitingForAccount;
        assert!(
            matches!(delete_offer(&view), DeleteOffer::Waiting(why) if why.contains("Sign in"))
        );
    }

    fn draw_view(view: &SyncView) {
        let store = std::sync::Arc::new(std::sync::Mutex::new(
            crate::store::Store::open_in_memory().unwrap(),
        ));
        let host = SyncHost::new(
            store,
            None,
            None,
            std::env::temp_dir().join("degenview-ui-sync-draw"),
            std::sync::Arc::new(|| {}),
        );
        let ctx = egui::Context::default();
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(700.0, 1400.0),
            )),
            ..Default::default()
        };
        let mut out = ctx.run_ui(raw, |ui| show(ui, &host, view, 0));
        out.textures_delta.clear();
    }

    #[test]
    fn every_state_of_the_delete_card_draws() {
        draw_view(&ready_view());
        let mut deleting = ready_view();
        deleting.deleting = true;
        draw_view(&deleting);
        let mut failed = ready_view();
        failed.delete_error = Some("DegenView couldn't reach iCloud.".into());
        draw_view(&failed);
        let mut offline = ready_view();
        offline.phase = Phase::WaitingForNetwork;
        draw_view(&offline);
        let mut done = ready_view();
        done.enabled = false;
        done.phase = Phase::Off;
        done.notice = Some(SyncNotice::CloudDeleted);
        draw_view(&done);
    }

    #[test]
    fn the_dialog_draws_and_its_typed_text_never_outlives_it() {
        let ctx = egui::Context::default();
        let raw = || egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(700.0, 900.0),
            )),
            ..Default::default()
        };
        let mut outcome = None;
        let mut out = ctx.run_ui(raw(), |ui| {
            outcome = delete_dialog(ui.ctx());
        });
        out.textures_delta.clear();
        assert_eq!(outcome, None, "opening it asks for nothing");
    }

    fn draw(host: &SyncHost) {
        let ctx = egui::Context::default();
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(700.0, 1200.0),
            )),
            ..Default::default()
        };
        let mut out = ctx.run_ui(raw, |ui| body(ui, host, 0));
        out.textures_delta.clear();
    }

    #[test]
    fn the_page_draws_when_icloud_is_unavailable_and_asks_for_nothing() {
        let store = std::sync::Arc::new(std::sync::Mutex::new(
            crate::store::Store::open_in_memory().unwrap(),
        ));
        let host = SyncHost::new(
            store.clone(),
            None,
            None,
            std::env::temp_dir().join("degenview-ui-sync"),
            std::sync::Arc::new(|| {}),
        );
        draw(&host);
        host.tick();
        assert!(!store.lock().unwrap().sync_enabled());
        assert!(host.view().unavailable.is_some());
    }
}
