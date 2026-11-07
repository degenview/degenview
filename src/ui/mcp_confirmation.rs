//! The question an MCP client's high-impact request raises: "Allow “Claude” to delete a price
//! alert?" with what it would change, a countdown, and Allow Once / Deny.
//!
//! A pure view over the open questions: it returns the user's answer and never touches the
//! server or the book itself. It is a modal alert, drawn by the focused window, so it cannot be
//! missed and cannot be answered by accident from another window. Escape or a click outside is
//! a No: the safe answer is the one that needs no aim. A button must be chosen on purpose
//! (Tab to it, or click): Return alone does nothing.

use crate::mcp::confirm::PendingView;
use crate::ui::icons::Icon;
use crate::ui::kit;
use eframe::egui::{self, Id};
use std::time::Duration;

/// What the user chose for the question on screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    Allow(u64),
    Deny(u64),
}

/// "1:42" for 102 seconds.
fn countdown(left: Duration) -> String {
    let secs = left.as_secs() + u64::from(left.subsec_nanos() > 0);
    format!("{}:{:02}", secs / 60, secs % 60)
}

/// "Delete a price alert" as "delete a price alert", for use inside a sentence.
fn sentence(action: &str) -> String {
    let mut chars = action.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_lowercase().chain(chars).collect()
    })
}

fn title(view: &PendingView) -> String {
    format!(
        "Allow “{}” to {}?",
        view.client_name,
        sentence(&view.summary.action)
    )
}

/// What the dialog says under the title: the target, a consequence, and how long it waits.
fn message(view: &PendingView, waiting: usize) -> String {
    let mut lines = Vec::new();
    if !view.summary.target.is_empty() {
        lines.push(view.summary.target.clone());
    }
    if let Some(note) = &view.summary.note {
        lines.push(note.clone());
    }
    lines.push(String::new());
    if waiting > 1 {
        lines.push(format!(
            "1 of {waiting} waiting. Expires in {}.",
            countdown(view.expires_in)
        ));
    } else {
        lines.push(format!("Expires in {}.", countdown(view.expires_in)));
    }
    lines.join("\n")
}

/// Shows the oldest open question, if there is one. `None` until the user answers it.
pub fn show(ctx: &egui::Context, pending: &[PendingView]) -> Option<Answer> {
    let view = pending.first()?;
    // The countdown moves, so the dialog repaints itself once a second while it is up.
    ctx.request_repaint_after(Duration::from_secs(1));
    let outcome = kit::alert(
        ctx,
        Id::new("mcp-confirmation"),
        &kit::Alert {
            icon: Icon::LockFill,
            tint: kit::ORANGE,
            title: &title(view),
            message: &message(view, pending.len()),
            accept: "Allow Once",
            dismiss: "Deny",
        },
    )?;
    Some(match outcome {
        kit::ConfirmOutcome::Confirmed => Answer::Allow(view.id),
        kit::ConfirmOutcome::Cancelled => Answer::Deny(view.id),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::confirm::Summary;
    use eframe::egui::{Pos2, Rect, vec2};

    fn view(id: u64, action: &str, target: &str) -> PendingView {
        PendingView {
            id,
            client_name: "Claude Desktop".into(),
            summary: Summary::new(action, target).with_note("Its trigger history is kept."),
            expires_in: Duration::from_secs(102),
        }
    }

    #[test]
    fn the_title_and_message_say_who_what_and_how_long() {
        let v = view(
            1,
            "Delete a price alert",
            "BTC/USDT crosses above 70000 USD",
        );
        assert_eq!(title(&v), "Allow “Claude Desktop” to delete a price alert?");
        assert_eq!(
            message(&v, 1),
            "BTC/USDT crosses above 70000 USD\nIts trigger history is kept.\n\nExpires in 1:42."
        );
        assert!(message(&v, 3).ends_with("1 of 3 waiting. Expires in 1:42."));
        assert_eq!(
            countdown(Duration::from_millis(100)),
            "0:01",
            "never shows 0:00 early"
        );
        assert_eq!(countdown(Duration::ZERO), "0:00");
        assert_eq!(sentence(""), "");
    }

    /// Runs the dialog over frames, pressing `keys` in turn; returns the answer it gave.
    fn answer_after(pending: &[PendingView], keys: &[egui::Key]) -> Option<Answer> {
        let ctx = egui::Context::default();
        let mut answer = None;
        let mut run = |events: Vec<egui::Event>, time: f64| {
            let raw = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(700.0, 700.0))),
                time: Some(time),
                events,
                ..Default::default()
            };
            let mut output = ctx.run_ui(raw, |ui| {
                let _ = ui;
                if let Some(given) = show(ui.ctx(), pending) {
                    answer = Some(given);
                }
            });
            output.textures_delta.clear();
        };
        run(Vec::new(), 0.0);
        run(Vec::new(), 0.1);
        for (index, key) in keys.iter().enumerate() {
            let press = |pressed| egui::Event::Key {
                key: *key,
                physical_key: Some(*key),
                pressed,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            };
            let time = 0.2 + index as f64 * 0.2;
            run(vec![press(true)], time);
            run(vec![press(false)], time + 0.1);
        }
        answer
    }

    #[test]
    fn nothing_is_shown_or_answered_without_a_question() {
        assert_eq!(answer_after(&[], &[]), None);
        assert_eq!(
            answer_after(&[], &[egui::Key::Escape, egui::Key::Enter]),
            None
        );
    }

    #[test]
    fn an_open_question_stays_open_until_it_is_answered_on_purpose() {
        let open = [view(7, "Delete a price alert", "BTC/USDT")];
        assert_eq!(answer_after(&open, &[]), None);
        // Return alone must not approve anything: no button has focus yet.
        assert_eq!(answer_after(&open, &[egui::Key::Enter]), None);
    }

    #[test]
    fn escape_is_a_no_and_a_chosen_button_is_a_yes() {
        let open = [
            view(7, "Delete a price alert", "BTC/USDT"),
            view(8, "Remove a chart", "ETH/USDT"),
        ];
        assert_eq!(
            answer_after(&open, &[egui::Key::Escape]),
            Some(Answer::Deny(7))
        );
        // Tab moves to the first button, Allow Once; Return presses it. Only the oldest question
        // is on screen, so the answer is for that one.
        assert_eq!(
            answer_after(&open, &[egui::Key::Tab, egui::Key::Enter]),
            Some(Answer::Allow(7))
        );
    }
}
