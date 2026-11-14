//! Saving as the user types, so a page needs no Save button.

use eframe::egui;

/// Saves what was typed once the typing stops, so a page needs no Save button. A text edit is due
/// when its field lost focus (Enter included) or the typing paused for [`Autosave::PAUSE`]. A
/// secret waits for the field to lose focus only: a pause in the middle of a key must not save half
/// of it.
#[derive(Default)]
pub struct Autosave {
    since: Option<std::time::Instant>,
    on_blur: bool,
}

impl Autosave {
    pub const PAUSE: std::time::Duration = std::time::Duration::from_millis(700);

    /// Notes the field's response: an edit starts (or restarts) the wait.
    pub fn watch(&mut self, response: &egui::Response, secret: bool) {
        if response.changed() {
            self.since = Some(std::time::Instant::now());
            self.on_blur = secret;
        }
        if response.lost_focus() && self.since.is_some() {
            self.touch();
        }
    }

    /// Makes the change due now: a picked provider, a left field.
    pub fn touch(&mut self) {
        self.since = Some(std::time::Instant::now() - Self::PAUSE);
        self.on_blur = false;
    }

    pub fn is_pending(&self) -> bool {
        self.since.is_some()
    }

    /// Whether the change is due to be saved now. True once; asks for a repaint while waiting.
    pub fn due(&mut self, ctx: &egui::Context) -> bool {
        let Some(since) = self.since else {
            return false;
        };
        if self.on_blur {
            return false;
        }
        let waited = since.elapsed();
        if waited >= Self::PAUSE {
            self.since = None;
            return true;
        }
        ctx.request_repaint_after(Self::PAUSE - waited);
        false
    }

    /// Forgets a pending change (the save was refused, or there is nothing to save).
    pub fn cancel(&mut self) {
        self.since = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typing_is_saved_after_a_pause_and_a_secret_only_when_its_field_is_left() {
        let ctx = egui::Context::default();
        let mut text = Autosave::default();
        assert!(!text.due(&ctx), "nothing was typed");
        text.since = Some(std::time::Instant::now());
        assert!(!text.due(&ctx), "still typing");
        text.since = Some(std::time::Instant::now() - Autosave::PAUSE);
        assert!(text.due(&ctx), "the typing paused");
        assert!(!text.due(&ctx), "due once");

        let mut secret = Autosave {
            since: Some(std::time::Instant::now() - Autosave::PAUSE * 10),
            on_blur: true,
        };
        assert!(!secret.due(&ctx), "half a key is never saved on a pause");
        assert!(secret.is_pending());
        secret.touch();
        assert!(secret.due(&ctx), "leaving the field saves it");
    }
}
