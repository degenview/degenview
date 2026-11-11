//! Settings ▸ Storage: what the app has cached on this computer — logos and saved lookups — where
//! it lives, and a button to clear it.
//!
//! Only regenerable data is listed here. Charts, watchlists, portfolios, alerts and scripts are
//! user data and are never touched by Clear Cache. The numbers come from walking the cache folder,
//! so the host fetches them off the UI thread and hands them to `StorageState`; this view only
//! draws and returns what was asked for.

use crate::platform;
use crate::service::disk_cache::{CacheStats, ClearOutcome, Usage};
use crate::ui::icons::Icon;
use crate::ui::settings::window::{card, divider, row, section, status_message};
use crate::ui::{kit, settings::card, theme};
use eframe::egui;

const CLEAR_HINT: &str = "Deletes downloaded logos and saved lookups. Your charts, watchlists, portfolios, alerts and scripts are not affected. Logos and lookups download again as needed.";

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Notice {
    pub text: String,
    pub is_error: bool,
}

#[derive(Default)]
pub struct StorageState {
    /// The last sizes read; `None` until the first read finishes.
    pub stats: Option<CacheStats>,
    /// Sizes were asked for on this visit to the page.
    requested: bool,
    confirming: bool,
    pub clearing: bool,
    pub notice: Option<Notice>,
}

impl StorageState {
    /// The page is no longer showing: the next visit reads the sizes again.
    pub fn leave(&mut self) {
        self.requested = false;
        self.confirming = false;
        self.notice = None;
    }

    /// Records the result of a clear.
    pub fn finish_clear(&mut self, outcome: ClearOutcome) {
        self.clearing = false;
        self.notice = Some(cleared_notice(outcome));
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StorageAction {
    /// Read the sizes (off the UI thread) and hand them back.
    Refresh,
    /// Delete the cache, then read the sizes again.
    Clear,
}

// MARK: Pure labels

/// "512 bytes", "12 KB", "1.4 MB": 1024-based steps, one decimal under ten.
pub fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["KB", "MB", "GB", "TB"];
    if bytes < 1024 {
        return if bytes == 1 {
            "1 byte".to_owned()
        } else {
            format!("{bytes} bytes")
        };
    }
    let mut value = bytes as f64 / 1024.0;
    let mut unit = 0;
    // 1023.7 KB would print as "1024 KB": call it a megabyte.
    while value >= 1023.5 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if value < 9.95 {
        format!("{value:.1} {}", UNITS[unit])
    } else {
        format!("{value:.0} {}", UNITS[unit])
    }
}

/// "128 files · 1.4 MB".
pub fn usage_label(usage: Usage) -> String {
    if usage.files == 0 {
        return "Empty".to_owned();
    }
    format!(
        "{} {} · {}",
        usage.files,
        if usage.files == 1 { "file" } else { "files" },
        format_bytes(usage.bytes)
    )
}

/// What to tell the user after a clear.
pub fn cleared_notice(outcome: ClearOutcome) -> Notice {
    let text = match (outcome.freed_bytes, outcome.failures) {
        (0, 0) => "The cache was already empty.".to_owned(),
        (freed, 0) => format!("Cleared {}.", format_bytes(freed)),
        (_, failures) => format!(
            "Cleared {}, but {failures} {} could not be removed.",
            format_bytes(outcome.freed_bytes),
            if failures == 1 { "item" } else { "items" }
        ),
    };
    Notice {
        text,
        is_error: outcome.failures > 0,
    }
}

// MARK: View

pub fn body(ui: &mut egui::Ui, state: &mut StorageState, actions: &mut Vec<StorageAction>) {
    if !state.requested {
        state.requested = true;
        actions.push(StorageAction::Refresh);
    }
    // A cache with no folder (the OS gave no location) cannot be listed or cleared.
    let location = state
        .stats
        .as_ref()
        .and_then(|stats| stats.location.clone());
    let available = state
        .stats
        .as_ref()
        .is_none_or(|stats| stats.location.is_some());

    section(
        ui,
        "Saved on this computer",
        Some("Logos and lookups kept so they appear instantly and need fewer requests."),
        |ui| {
            card(ui, |ui| {
                let (images, lookups, total) = match &state.stats {
                    Some(stats) => (
                        usage_label(stats.images),
                        usage_label(stats.lookups),
                        format_bytes(stats.total_bytes()),
                    ),
                    None => {
                        let waiting = "Calculating…".to_owned();
                        (waiting.clone(), waiting.clone(), waiting)
                    }
                };
                value_row(ui, "Logos", &images);
                divider(ui);
                value_row(ui, "Lookups", &lookups);
                divider(ui);
                value_row(ui, "Total", &total);
                if let Some(location) = &location {
                    divider(ui);
                    row(ui, "Location", |ui| {
                        let label = format!("Show in {}", platform::file_manager());
                        if kit::push_button(ui, None, &label, true).clicked() {
                            // Nothing may be cached yet; reveal an empty folder rather than fail.
                            let _ = std::fs::create_dir_all(location);
                            platform::reveal_in_file_manager(location);
                        }
                    });
                    ui.add(
                        egui::Label::new(kit::text(
                            location.display().to_string(),
                            kit::size::CAPTION + 0.5,
                            kit::REGULAR,
                            theme::text_secondary(),
                        ))
                        .wrap()
                        .selectable(true),
                    );
                }
            });
        },
    );

    section(ui, "Clear cache", None, |ui| {
        card::card(ui, Icon::Trash, "Clear Cache", Some(CLEAR_HINT), |ui| {
            let (label, enabled) = if state.clearing {
                ("Clearing…", false)
            } else {
                ("Clear Cache…", available)
            };
            if kit::push_button(ui, None, label, enabled).clicked() {
                state.confirming = true;
            }
        });
        if let Some(notice) = &state.notice {
            ui.add_space(10.0);
            status_message(ui, &notice.text, notice.is_error);
        }
        if !available {
            ui.add_space(10.0);
            status_message(
                ui,
                "DegenView could not find a folder to cache in, so nothing is saved between launches.",
                true,
            );
        }
    });

    confirm_clear(ui.ctx(), state, actions);
}

fn value_row(ui: &mut egui::Ui, title: &str, value: &str) {
    row(ui, title, |ui| {
        ui.label(kit::text(
            value,
            kit::size::BODY,
            kit::REGULAR,
            theme::text_secondary(),
        ));
    });
}

fn confirm_clear(ctx: &egui::Context, state: &mut StorageState, actions: &mut Vec<StorageAction>) {
    if !state.confirming {
        return;
    }
    match kit::confirm(
        ctx,
        egui::Id::new("settings-clear-cache"),
        &kit::Confirm {
            icon: Icon::Trash,
            title: "Clear the cache?",
            message: "Downloaded logos and saved lookups are deleted and fetched again as needed. Your charts, watchlists, portfolios, alerts and scripts are not affected.",
            destructive: "Clear Cache",
        },
    ) {
        Some(kit::ConfirmOutcome::Confirmed) => {
            state.confirming = false;
            state.clearing = true;
            state.notice = None;
            actions.push(StorageAction::Clear);
        }
        Some(kit::ConfirmOutcome::Cancelled) => state.confirming = false,
        None => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_read_in_the_nearest_unit() {
        for (bytes, label) in [
            (0, "0 bytes"),
            (1, "1 byte"),
            (512, "512 bytes"),
            (1023, "1023 bytes"),
            (1024, "1.0 KB"),
            (1536, "1.5 KB"),
            (12 * 1024, "12 KB"),
            (1024 * 1024, "1.0 MB"),
            (1_468_006, "1.4 MB"),
            (123 * 1024 * 1024, "123 MB"),
            (5 * 1024 * 1024 * 1024, "5.0 GB"),
            (u64::MAX, "16777216 TB"),
        ] {
            assert_eq!(format_bytes(bytes), label, "{bytes} bytes");
        }
    }

    #[test]
    fn a_size_that_rounds_up_to_the_next_unit_says_so() {
        assert_eq!(format_bytes(1024 * 1024 - 1), "1.0 MB");
        assert_eq!(format_bytes(1024 * 1024 * 1024 - 1), "1.0 GB");
        assert_eq!(format_bytes(10 * 1024 - 1), "10 KB");
    }

    #[test]
    fn usage_counts_files_and_bytes_and_says_when_empty() {
        assert_eq!(usage_label(Usage::default()), "Empty");
        assert_eq!(
            usage_label(Usage {
                files: 1,
                bytes: 2048
            }),
            "1 file · 2.0 KB"
        );
        assert_eq!(
            usage_label(Usage {
                files: 128,
                bytes: 1_468_006
            }),
            "128 files · 1.4 MB"
        );
    }

    #[test]
    fn the_notice_after_a_clear_reports_what_happened() {
        assert_eq!(
            cleared_notice(ClearOutcome::default()),
            Notice {
                text: "The cache was already empty.".into(),
                is_error: false
            }
        );
        let cleared = cleared_notice(ClearOutcome {
            freed_bytes: 2 * 1024 * 1024,
            failures: 0,
        });
        assert_eq!(cleared.text, "Cleared 2.0 MB.");
        assert!(!cleared.is_error);
        let partial = cleared_notice(ClearOutcome {
            freed_bytes: 1024,
            failures: 2,
        });
        assert_eq!(
            partial.text,
            "Cleared 1.0 KB, but 2 items could not be removed."
        );
        assert!(partial.is_error);
        assert!(
            cleared_notice(ClearOutcome {
                freed_bytes: 0,
                failures: 1
            })
            .text
            .ends_with("1 item could not be removed.")
        );
    }

    #[test]
    fn leaving_the_page_asks_for_fresh_sizes_next_time() {
        let mut state = StorageState::default();
        let mut actions = Vec::new();
        let ctx = egui::Context::default();
        let run = |state: &mut StorageState, actions: &mut Vec<StorageAction>| {
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| body(ui, state, actions));
            output.textures_delta.clear();
        };
        run(&mut state, &mut actions);
        run(&mut state, &mut actions);
        assert_eq!(actions, [StorageAction::Refresh], "asked once per visit");
        state.leave();
        run(&mut state, &mut actions);
        assert_eq!(actions, [StorageAction::Refresh, StorageAction::Refresh]);
    }

    #[test]
    fn finishing_a_clear_ends_the_busy_state() {
        let mut state = StorageState {
            clearing: true,
            ..StorageState::default()
        };
        state.finish_clear(ClearOutcome {
            freed_bytes: 10,
            failures: 0,
        });
        assert!(!state.clearing);
        assert_eq!(state.notice.unwrap().text, "Cleared 10 bytes.");
    }
}
