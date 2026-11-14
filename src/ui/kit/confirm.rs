//! The confirmation, alert and notice dialogs, drawn by [`crate::ui::kit::dialog`].

use super::RED;
use super::dialog::{self, Dialog};
use crate::ui::icons::Icon;
use eframe::egui::{self, Color32, Id};

/// What a confirmation alert's buttons say.
pub struct Confirm<'a> {
    pub icon: Icon,
    pub title: &'a str,
    pub message: &'a str,
    pub destructive: &'a str,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConfirmOutcome {
    Confirmed,
    Cancelled,
}

/// A confirmation of something destructive, drawn by [`crate::ui::kit::dialog`]: a red badge, the
/// title and message, then Cancel and the red action side by side. Escape, the close button or a
/// click outside cancels; the dialog starts on Cancel, so a stray Enter destroys nothing.
pub fn confirm(ctx: &egui::Context, id: Id, confirm: &Confirm) -> Option<ConfirmOutcome> {
    let mut spec = Dialog::new(
        confirm.icon,
        RED,
        confirm.title,
        confirm.message,
        confirm.destructive,
    );
    spec.destructive = true;
    outcome_of(dialog::show(ctx, id, &spec, |_| {}))
}

/// What an `alert`'s badge and buttons say.
pub struct Alert<'a> {
    pub icon: Icon,
    /// The badge.
    pub tint: Color32,
    pub title: &'a str,
    pub message: &'a str,
    pub accept: &'a str,
    pub dismiss: &'a str,
    /// The dialog starts on the dismiss button, so a stray Enter does not approve (a request to
    /// let something through, an upload).
    pub cautious: bool,
}

/// The same dialog as `confirm` for a question that is not a warning (a restart to finish an
/// update): `tint` colours the badge, the action is neutral, and the dismiss button says what it
/// does.
pub fn alert(ctx: &egui::Context, id: Id, spec: &Alert) -> Option<ConfirmOutcome> {
    let mut dialog = Dialog::new(spec.icon, spec.tint, spec.title, spec.message, spec.accept);
    dialog.dismiss = spec.dismiss;
    dialog.cautious = spec.cautious;
    outcome_of(dialog::show(ctx, id, &dialog, |_| {}))
}

fn outcome_of(outcome: Option<dialog::Outcome>) -> Option<ConfirmOutcome> {
    outcome.map(|outcome| match outcome {
        dialog::Outcome::Accepted => ConfirmOutcome::Confirmed,
        dialog::Outcome::Dismissed | dialog::Outcome::Leading => ConfirmOutcome::Cancelled,
    })
}

/// What `notice` says: an icon badge in `tint`, a title and the message.
pub struct Notice<'a> {
    pub icon: Icon,
    pub tint: Color32,
    pub title: &'a str,
    pub message: &'a str,
}

/// A one-button dialog that only informs ("Already in Watchlist"). Enter, Escape, the button, the
/// close button or a click outside all dismiss it. Returns true on the frame it is dismissed.
pub fn notice(ctx: &egui::Context, id: Id, spec: &Notice) -> bool {
    let mut dialog = Dialog::new(spec.icon, spec.tint, spec.title, spec.message, "OK");
    dialog.dismiss = "";
    dialog::show(ctx, id, &dialog, |_| {}).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::theme;
    use eframe::egui::{Rect, pos2, vec2};

    #[test]
    fn a_notice_keeps_its_size_from_frame_to_frame() {
        let ctx = egui::Context::default();
        let id = Id::new("notice-size");
        let spec = Notice {
            icon: Icon::Triangle,
            tint: theme::ORANGE,
            title: "Already in Watchlist",
            message: "\u{201c}BTC\u{201d} is already in this watchlist.",
        };
        let mut heights = Vec::new();
        let mut dismissed = false;
        for frame in 0..8 {
            let raw = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(900.0, 700.0))),
                time: Some(f64::from(frame) * 0.1),
                ..Default::default()
            };
            let mut output = ctx.run_ui(raw, |ui| {
                dismissed |= notice(ui.ctx(), id, &spec);
            });
            output.textures_delta.clear();
            let rect = ctx
                .memory(|memory| memory.area_rect(Id::new(id).with("Modal")))
                .or_else(|| ctx.memory(|memory| memory.area_rect(id)));
            if let Some(rect) = rect {
                heights.push(rect.height());
            }
        }
        assert!(!dismissed, "nothing dismissed it");
        assert!(!heights.is_empty(), "the notice was laid out");
        let last = *heights.last().unwrap();
        assert!(last < 300.0, "a one-line notice is small, not {last}");
        assert!(
            heights
                .windows(2)
                .skip(2)
                .all(|w| (w[0] - w[1]).abs() < 0.5),
            "the height settles: {heights:?}"
        );
    }
}
