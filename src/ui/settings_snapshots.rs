//! Settings ▸ Snapshots: where chart snapshots are shared, what they show of the account and how
//! sharp they are. A pure view over the saved choices: it returns [`SnapshotsAction`]s and the
//! settings host applies them.
//!
//! DegenView hosts nothing itself. Copy link, Open in new tab and Tweet image upload the image to
//! the host set here, so the page says so plainly and the app asks once per host before the
//! first upload.

use crate::model::snapshot::{SnapshotHost, SnapshotPrefs, SnapshotScale};
use crate::service::snapshot_upload::token_is_valid;
use crate::ui::alert_center::Tone;
use crate::ui::icons::Icon;
use crate::ui::settings_api_keys::{StatusMessage, destructive_button, field, footer_status};
use crate::ui::settings_window::{card, footer_row, section, status_badge};
use crate::ui::{kit, settings_card};
use eframe::egui;

/// What the page shows.
pub struct SnapshotsInput<'a> {
    pub prefs: &'a SnapshotPrefs,
    /// A token is in the Keychain (or the environment gives one).
    pub token_saved: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum SnapshotsAction {
    /// Save the host's address and, if one was typed, the token.
    Save {
        host: String,
        token: Option<String>,
    },
    RemoveToken,
    IncludeTrading(bool),
    Scale(SnapshotScale),
    ForgetConsent,
}

#[derive(Default)]
pub struct SnapshotsState {
    host: String,
    token: String,
    loaded: bool,
    /// What the last save said; the settings host sets it.
    pub status: Option<StatusMessage>,
}

impl SnapshotsState {
    /// Puts the saved address in its field, once.
    fn load(&mut self, prefs: &SnapshotPrefs) {
        if !self.loaded {
            self.loaded = true;
            self.host = prefs.host.clone().unwrap_or_default();
        }
    }

    /// What is wrong with the address in the field, if anything. Empty is fine: it clears it.
    pub fn host_problem(&self) -> Option<&'static str> {
        let text = self.host.trim();
        (!text.is_empty())
            .then(|| SnapshotHost::parse(text).err().map(|error| error.message()))
            .flatten()
    }

    pub fn token_problem(&self) -> Option<&'static str> {
        let token = self.token.trim();
        (!token.is_empty() && !token_is_valid(token))
            .then_some("A token has no spaces and only plain letters, digits and symbols.")
    }

    pub fn can_save(&self, prefs: &SnapshotPrefs) -> bool {
        self.host_problem().is_none()
            && self.token_problem().is_none()
            && (self.host.trim() != prefs.host.as_deref().unwrap_or_default()
                || !self.token.trim().is_empty())
    }

    /// The action Save means.
    pub fn save_action(&self) -> SnapshotsAction {
        let token = self.token.trim();
        SnapshotsAction::Save {
            host: self.host.trim().to_owned(),
            token: (!token.is_empty()).then(|| token.to_owned()),
        }
    }

    /// A save went through: the token is the Keychain's now, not the field's.
    pub fn saved_token(&mut self) {
        self.token.clear();
    }
}

pub fn body(
    ui: &mut egui::Ui,
    state: &mut SnapshotsState,
    input: &SnapshotsInput,
    actions: &mut Vec<SnapshotsAction>,
) {
    state.load(input.prefs);
    match input.prefs.host() {
        Some(host) => status_badge(ui, &format!("Sharing to {}", host.authority()), Tone::Good),
        None => status_badge(ui, "Sharing not set up", Tone::Neutral),
    }
    ui.add_space(18.0);

    section(
        ui,
        "Sharing",
        Some("Where Copy link, Open in new tab and Tweet image upload a snapshot."),
        |ui| {
            card(ui, |ui| {
                ui.vertical(|ui| {
                    field(
                        ui,
                        "Snapshot host",
                        Some("An https:// address that takes the image and answers with a link."),
                        "https://snapshots.example.com/v1/snapshots",
                        &mut state.host,
                        false,
                    );
                });
                if let Some(problem) = state.host_problem() {
                    problem_line(ui, problem);
                }
                ui.vertical(|ui| {
                    let hint = if input.token_saved {
                        "A token is saved. Type a new one to replace it."
                    } else {
                        "Sent as a bearer token, if the host asks for one."
                    };
                    field(
                        ui,
                        "Access token (optional)",
                        Some(hint),
                        "",
                        &mut state.token,
                        true,
                    );
                });
                if let Some(problem) = state.token_problem() {
                    problem_line(ui, problem);
                }
            });
        },
    );
    kit::notice_card(
        ui,
        Icon::InfoCircle,
        kit::ACCENT,
        "DegenView does not host snapshots",
        Some(
            "Sharing uploads the image to the host above, which you run or trust. Saving or \
             copying an image never uploads anything.",
        ),
        None,
    );
    ui.add_space(18.0);

    section(ui, "Images", None, |ui| {
        let mut include = input.prefs.include_trading;
        settings_card::card(
            ui,
            Icon::Briefcase,
            "Include orders, executions and positions in snapshots",
            Some(
                "Draws your paper-trading orders and positions on the picture, with their sizes, \
                 entry prices and open profit. Off, a shared chart shows nothing of your account.",
            ),
            |ui| {
                if settings_card::switch(ui, &mut include) {
                    actions.push(SnapshotsAction::IncludeTrading(include));
                }
            },
        );
        ui.add_space(10.0);
        settings_card::card_reserving(
            ui,
            Icon::Camera,
            "Resolution",
            Some("Images are drawn again at the scale, not enlarged, so text stays sharp."),
            260.0,
            |ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                for scale in SnapshotScale::ALL.into_iter().rev() {
                    let current = scale == input.prefs.scale;
                    if kit::sheet_button(ui, scale.label(), current, true).clicked() && !current {
                        actions.push(SnapshotsAction::Scale(scale));
                    }
                }
            },
        );
    });

    section(ui, "Privacy", None, |ui| {
        let count = input.prefs.consented.len();
        settings_card::card(
            ui,
            Icon::LockFill,
            "Approved hosts",
            Some(&approved_hosts_hint(&input.prefs.consented)),
            |ui| {
                if kit::sheet_button(ui, "Forget", false, count > 0).clicked() {
                    actions.push(SnapshotsAction::ForgetConsent);
                }
            },
        );
    });
}

/// The privacy card's words: which hosts the user has agreed to upload to.
fn approved_hosts_hint(consented: &[String]) -> String {
    if consented.is_empty() {
        "DegenView asks before the first upload to a host, and never uploads on its own.".to_owned()
    } else {
        format!(
            "You agreed to upload to {}. DegenView asks again for any other host.",
            consented.join(", ")
        )
    }
}

fn problem_line(ui: &mut egui::Ui, problem: &str) {
    ui.label(kit::text(
        problem,
        kit::size::CAPTION + 0.5,
        kit::REGULAR,
        kit::RED,
    ));
}

pub fn footer(
    ui: &mut egui::Ui,
    state: &mut SnapshotsState,
    input: &SnapshotsInput,
    actions: &mut Vec<SnapshotsAction>,
) {
    let status = footer_status(&state.status);
    footer_row(ui, status, |ui| {
        if kit::sheet_button(ui, "Save", true, state.can_save(input.prefs)).clicked() {
            actions.push(state.save_action());
        }
        if input.token_saved && destructive_button(ui, "Remove Token").clicked() {
            actions.push(SnapshotsAction::RemoveToken);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(host: &str, token: &str) -> SnapshotsState {
        SnapshotsState {
            host: host.to_owned(),
            token: token.to_owned(),
            loaded: true,
            status: None,
        }
    }

    #[test]
    fn a_host_is_checked_as_it_is_typed() {
        assert_eq!(state("", "").host_problem(), None, "empty clears the host");
        assert_eq!(state("https://s.example/up", "").host_problem(), None);
        assert!(state("s.example", "").host_problem().is_some());
        assert!(
            state("http://s.example/up", "")
                .host_problem()
                .unwrap()
                .contains("https://")
        );
        assert!(
            state("https://u:p@s.example/up", "")
                .host_problem()
                .is_some()
        );
    }

    #[test]
    fn a_token_that_cannot_ride_in_a_header_is_refused() {
        assert_eq!(state("", "abc123").token_problem(), None);
        assert_eq!(state("", "").token_problem(), None);
        assert!(state("", "has space").token_problem().is_some());
        assert!(state("", "é").token_problem().is_some());
    }

    #[test]
    fn save_needs_a_change_and_nothing_wrong() {
        let prefs = SnapshotPrefs {
            host: Some("https://s.example/up".into()),
            ..SnapshotPrefs::default()
        };
        assert!(
            !state("https://s.example/up", "").can_save(&prefs),
            "no change"
        );
        assert!(state("https://other.example/up", "").can_save(&prefs));
        assert!(state("https://s.example/up", "new-token").can_save(&prefs));
        assert!(
            state("", "").can_save(&prefs),
            "clearing the host is a change"
        );
        assert!(!state("nonsense", "").can_save(&prefs));
        assert!(!state("https://s.example/up", "bad token").can_save(&prefs));
        assert!(
            !state("", "").can_save(&SnapshotPrefs::default()),
            "nothing to save"
        );
    }

    #[test]
    fn save_sends_the_trimmed_address_and_only_a_typed_token() {
        assert_eq!(
            state("  https://s.example/up ", "").save_action(),
            SnapshotsAction::Save {
                host: "https://s.example/up".into(),
                token: None
            }
        );
        assert_eq!(
            state("https://s.example/up", " tok ").save_action(),
            SnapshotsAction::Save {
                host: "https://s.example/up".into(),
                token: Some("tok".into())
            }
        );
    }

    #[test]
    fn the_privacy_card_names_the_hosts_agreed_to() {
        assert!(approved_hosts_hint(&[]).contains("asks before the first upload"));
        let hint = approved_hosts_hint(&["a.example".into(), "b.example:8443".into()]);
        assert!(hint.contains("a.example, b.example:8443"));
    }

    #[test]
    fn the_page_draws_and_reports_a_switch() {
        let ctx = egui::Context::default();
        let prefs = SnapshotPrefs::default();
        let mut page = SnapshotsState::default();
        let mut actions = Vec::new();
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(700.0, 900.0),
            )),
            ..Default::default()
        };
        let mut out = ctx.run_ui(raw, |ui| {
            body(
                ui,
                &mut page,
                &SnapshotsInput {
                    prefs: &prefs,
                    token_saved: false,
                },
                &mut actions,
            );
        });
        out.textures_delta.clear();
        assert!(actions.is_empty(), "drawing a page asks for nothing");
    }
}
