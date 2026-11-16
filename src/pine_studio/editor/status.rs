//! The problems chip (drawn at the right end of the menu strip over the editor) and the status
//! strip under the text: the changed-line counts and notices. Saving is ⌘S or File ▸ Save.

use super::EditorState;
use super::diff::DiffSnapshot;
use crate::model::pine::script::CompileStatus;
use crate::pine::syntax::token::{Diagnostic, DiagnosticSeverity};
use crate::ui::icons::{self, Icon};
use crate::ui::kit::{self, size};
use crate::ui::theme;
use eframe::egui::{
    self, Align, Color32, Rect, Response, RichText, Sense, Ui, UiBuilder, Vec2, pos2, vec2,
};

/// "No problems", "1 warning", "2 errors" — the chip's text and look.
pub fn status_chip(status: CompileStatus, diagnostics: &[Diagnostic]) -> (Icon, Color32, String) {
    let errors = diagnostics
        .iter()
        .filter(|d| d.severity == DiagnosticSeverity::Error)
        .count();
    let warnings = diagnostics.len() - errors;
    let plural = |n: usize, noun: &str| format!("{n} {noun}{}", if n == 1 { "" } else { "s" });
    match status {
        CompileStatus::NotCompiled => (
            Icon::CircleDashed,
            theme::text_secondary(),
            "Not compiled".to_owned(),
        ),
        CompileStatus::Valid => (
            Icon::CheckCircleFill,
            Color32::from_rgb(0x30, 0xD1, 0x58),
            "No problems".to_owned(),
        ),
        CompileStatus::Warning => (
            Icon::TriangleFill,
            theme::YELLOW,
            plural(warnings.max(1), "warning"),
        ),
        CompileStatus::Error => (
            Icon::XmarkOctagonFill,
            theme::syntax::string(),
            plural(errors.max(1), "error"),
        ),
    }
}

/// `status_chip`, counting the script's runtime failures too: text that compiles but fails when it
/// runs is not "No problems".
pub fn status_chip_with_runtime(
    status: CompileStatus,
    diagnostics: &[Diagnostic],
    runtime: &[Diagnostic],
) -> (Icon, Color32, String) {
    if runtime
        .iter()
        .any(|d| d.severity == DiagnosticSeverity::Error)
    {
        let all: Vec<Diagnostic> = diagnostics.iter().chain(runtime).cloned().collect();
        return status_chip(CompileStatus::Error, &all);
    }
    status_chip(status, diagnostics)
}

/// "No problems", "1 warning", "2 errors" for the editor's text, with what hovering it says.
pub struct ProblemsChip {
    icon: Icon,
    color: Color32,
    text: String,
    hover: String,
}

/// Room between the chip's icon and its text.
const CHIP_GAP: f32 = 5.0;
const CHIP_ICON: f32 = 12.0;

impl ProblemsChip {
    pub fn of(state: &EditorState) -> Self {
        let (icon, color, text) =
            status_chip_with_runtime(state.status, &state.diagnostics, &state.runtime_problems);
        let hover = match state.runtime_problems.first() {
            Some(failure) => format!("The script failed when it ran: {}", failure.message),
            None => "Compile status of the code in the editor".to_owned(),
        };
        Self {
            icon,
            color,
            text,
            hover,
        }
    }

    /// Draws the chip ending at `right`, centred on `centre_y`.
    pub fn show(&self, ui: &mut Ui, right: f32, centre_y: f32) -> Response {
        let text = kit::galley(
            ui,
            &self.text,
            size::CALLOUT,
            kit::REGULAR,
            self.color,
            f32::INFINITY,
        );
        let width = CHIP_ICON + CHIP_GAP + text.size().x;
        let rect = Rect::from_min_size(
            pos2(right - width, centre_y - text.size().y.max(CHIP_ICON) / 2.0),
            vec2(width, text.size().y.max(CHIP_ICON)),
        );
        let icon = Rect::from_center_size(
            pos2(rect.left() + CHIP_ICON / 2.0, centre_y),
            Vec2::splat(CHIP_ICON),
        );
        icons::paint(ui.painter(), icon, self.icon, self.color);
        let at = pos2(icon.right() + CHIP_GAP, centre_y - text.size().y / 2.0);
        ui.painter().galley(at, text, self.color);
        ui.interact(rect, ui.id().with("problems-chip"), Sense::hover())
            .on_hover_text(self.hover.as_str())
    }
}

/// The changed-line counts and the latest notice.
pub(super) fn status_bar(ui: &mut Ui, bar: Rect, state: &EditorState, changes: &DiffSnapshot) {
    crate::pine_studio::drawer::bottom_bar::paint(ui, bar);
    let mut inner = ui.new_child(
        UiBuilder::new()
            .max_rect(bar.shrink2(Vec2::new(12.0, 0.0)))
            .layout(egui::Layout::left_to_right(Align::Center)),
    );
    inner.spacing_mut().item_spacing.x = 5.0;
    if !changes.is_clean() {
        let (added, removed) = (changes.lines_added(), changes.lines_removed());
        inner
            .label(
                RichText::new(format!("+{added}"))
                    .size(10.0)
                    .color(theme::editor_diff_added()),
            )
            .on_hover_text(format!("{added} lines added since the last save"));
        inner
            .label(
                RichText::new(format!("−{removed}"))
                    .size(10.0)
                    .color(theme::editor_diff_deleted()),
            )
            .on_hover_text(format!("{removed} lines removed since the last save"));
    }
    if let Some((text, at)) = state.notice.as_ref()
        && let Some(left) =
            crate::pine_studio::library::external::NOTICE_FOR.checked_sub(at.elapsed())
    {
        inner.add_space(8.0);
        inner.add(
            egui::Label::new(
                RichText::new(text.as_str())
                    .size(10.0)
                    .color(theme::text_secondary()),
            )
            .truncate(),
        );
        inner.ctx().request_repaint_after(left);
    }
}
