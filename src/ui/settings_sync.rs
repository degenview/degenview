//! Settings ▸ iCloud Sync: the switch, what is happening, what needs a decision, what syncs.
//!
//! A view over `SyncHost`: it reads a `SyncView` and sends `SyncAction`s, and never talks to
//! iCloud or the store itself. Anything that moves data between Apple Accounts or discards a
//! copy of this Mac's data is a button the user presses, with the consequence written next to it.

use crate::sync::account::AccountChoice;
use crate::sync::core::{PauseReason, Phase};
use crate::sync::initial::{Counts, JoinMode, JoinSummary};
use crate::sync::transport::ErrorKind;
use crate::sync_host::{SyncAction, SyncHost, SyncPrompt, SyncView};
use crate::ui::alert_center::{Tone, relative_time};
use crate::ui::icons::Icon;
use crate::ui::settings_window::{card, divider, row, section, status_badge};
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

const NOT_SYNCED: &str = "Never synced: market data caches, paper-trading accounts, portfolios, \
webhook endpoints and their secrets, API keys, MCP access, and which windows and tabs are open. \
Alerts you sync arrive switched off on your other Macs, so each one notifies from one Mac only.";

const PRIVACY: &str = "Your data is stored in your private iCloud storage for DegenView, tied to \
your Apple Account. Pine scripts are synced as written, so remove any secrets you pasted into them.";

pub fn body(ui: &mut egui::Ui, sync: &SyncHost, now_ms: i64) {
    let view = sync.view();
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
    prompts(ui, sync, &view);
    section(ui, "iCloud Sync", None, |ui| {
        settings_card::card(
            ui,
            Icon::Cycle,
            "Sync DegenView data with iCloud",
            Some(
                "Keeps your watchlists, layouts, drawings, templates and scripts the same on every Mac signed in to your Apple Account. Everything also stays on this Mac.",
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
    status(ui, sync, &view, now_ms);
    attention(ui, sync, &view);
    categories(ui, sync, &view);
    ui.add_space(14.0);
    small(ui, NOT_SYNCED);
    ui.add_space(8.0);
    small(ui, PRIVACY);
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
            let (label, tone) = phase_label(view.phase);
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
            small(ui, error);
        }
    });
}

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
        "Merge keeps everything from both. Using iCloud's data replaces this Mac's watchlists, layouts and templates after saving a copy you can restore; scripts and alerts are always merged.",
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
    fn the_page_states_what_never_syncs() {
        for word in ["paper-trading", "secrets", "API keys", "MCP"] {
            assert!(NOT_SYNCED.contains(word), "{word}");
        }
        assert!(PRIVACY.contains("private iCloud storage"));
        assert!(
            !PRIVACY.to_lowercase().contains("end-to-end"),
            "no unverified claim"
        );
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
