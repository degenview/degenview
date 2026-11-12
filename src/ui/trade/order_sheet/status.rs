//! What the sheet tells the user about the order: messages between the fields, the venue's check
//! and why the button is not pressable.

use std::time::{Duration, Instant};

use eframe::egui;

use super::state::SheetState;
use crate::model::broker::ticket::PreviewGate;
use crate::trading::draft::{DraftCheck, DraftField};
use crate::trading::model::VenueInfo;
use crate::ui::icons::{self, Icon};
use crate::ui::{kit, theme};

/// How long a message stays under the button.
const NOTICE_FOR: Duration = Duration::from_secs(10);

/// A message between the sheet's fields: a glyph and wrapped text on a faint tint with a hairline
/// edge. Red for what went wrong, orange for a warning, quiet for what is only worth knowing.
pub(super) fn note(ui: &mut egui::Ui, text: &str, error: bool) {
    let tint = if error {
        kit::RED
    } else {
        theme::text_secondary()
    };
    egui::Frame::new()
        .fill(theme::with_alpha(tint, 0.08))
        .stroke(egui::Stroke::new(1.0, theme::with_alpha(tint, 0.22)))
        .corner_radius(8.0)
        .inner_margin(egui::Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                icons::inline(ui, Icon::InfoCircle, 13.0, tint);
                ui.add(
                    egui::Label::new(kit::text(
                        text,
                        kit::size::CAPTION + 1.0,
                        kit::REGULAR,
                        if error {
                            theme::text_primary()
                        } else {
                            theme::text_secondary()
                        },
                    ))
                    .wrap(),
                );
            });
        });
}

pub(super) fn line(
    ui: &mut egui::Ui,
    state: &mut SheetState,
    check: &DraftCheck,
    gate: &PreviewGate,
    info: &VenueInfo,
    now: Instant,
) {
    if let Some(order_issue) = check.issue_for(DraftField::Order) {
        note(ui, order_issue, true);
        return;
    }
    if !check.is_complete() {
        return;
    }
    match gate {
        PreviewGate::Waiting | PreviewGate::Stale => {
            ui.label(kit::text(
                format!("Checking with {}…", info.name),
                kit::size::CAPTION + 1.0,
                kit::REGULAR,
                theme::text_tertiary(),
            ));
        }
        PreviewGate::Failed(text) => {
            note(
                ui,
                &format!("{} could not check this order: {text}", info.name),
                true,
            );
            ui.add_space(6.0);
            if kit::sheet_button(ui, "Try again", false, true).clicked() {
                state.preview.retry(now);
            }
        }
        PreviewGate::Blocked => {
            if let Some(preview) = state.preview.result() {
                for problem in &preview.problems {
                    note(ui, problem, true);
                    ui.add_space(4.0);
                }
            }
        }
        PreviewGate::Ready => {
            if let Some(preview) = state.preview.result() {
                for warning in &preview.warnings {
                    note(ui, warning, false);
                    ui.add_space(4.0);
                }
            }
        }
    }
}

/// Why the button is not pressable, in a sentence.
pub(super) fn why_not(
    check: &DraftCheck,
    gate: &PreviewGate,
    paused: Option<&str>,
    replay: bool,
) -> Option<String> {
    if replay {
        return Some("Not available during replay.".to_owned());
    }
    if let Some(text) = paused {
        return Some(text.to_owned());
    }
    if let Some(issue) = check.issues.first() {
        return Some(issue.text.clone());
    }
    if let Some(field) = check.missing.first() {
        return Some(
            match field {
                DraftField::Size => "Enter an amount.",
                DraftField::Price => "Enter a price.",
                DraftField::StopPrice => "Enter a stop price.",
                _ => "The order is not complete.",
            }
            .to_owned(),
        );
    }
    match gate {
        PreviewGate::Ready => None,
        PreviewGate::Waiting | PreviewGate::Stale => Some("Checking the order first.".to_owned()),
        PreviewGate::Failed(_) => Some("The order could not be checked.".to_owned()),
        PreviewGate::Blocked => Some("The venue would refuse this order.".to_owned()),
    }
}

pub(super) fn notice(ui: &mut egui::Ui, state: &mut SheetState) {
    let Some((text, error, at)) = &state.notice else {
        return;
    };
    if at.elapsed() > NOTICE_FOR {
        state.notice = None;
        return;
    }
    ui.add_space(8.0);
    note(ui, text, *error);
}
