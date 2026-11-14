//! The status strip under the text: the problems chip and the Save button.

use super::EditorState;
use super::diff::DiffSnapshot;
use crate::model::pine::script::CompileStatus;
use crate::pine::syntax::token::{Diagnostic, DiagnosticSeverity};
use crate::ui::icons::{self, Icon};
use crate::ui::{shortcut, theme};
use eframe::egui::{self, Align, Color32, Rect, RichText, Ui, UiBuilder, Vec2};

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

/// The chip and the Save button. Returns whether Save was clicked.
pub(super) fn status_bar(
    ui: &mut Ui,
    bar: Rect,
    state: &EditorState,
    changes: &DiffSnapshot,
) -> bool {
    crate::pine_studio::drawer::bottom_bar::paint(ui, bar);
    let mut save = false;
    let mut inner = ui.new_child(
        UiBuilder::new()
            .max_rect(bar.shrink2(Vec2::new(12.0, 0.0)))
            .layout(egui::Layout::left_to_right(Align::Center)),
    );
    let (icon, color, text) =
        status_chip_with_runtime(state.status, &state.diagnostics, &state.runtime_problems);
    inner.spacing_mut().item_spacing.x = 5.0;
    icons::inline(&mut inner, icon, 12.0, color);
    let hover = match state.runtime_problems.first() {
        Some(failure) => format!("The script failed when it ran: {}", failure.message),
        None => "Compile status of the code in the editor".to_owned(),
    };
    inner
        .label(egui::RichText::new(text).size(10.0).color(color))
        .on_hover_text(hover);
    if !changes.is_clean() {
        let (added, removed) = (changes.lines_added(), changes.lines_removed());
        inner.add_space(6.0);
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
    if !state.sample {
        inner.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
            // egui never draws a button under `interact_size.y`, so the bar lowers it.
            ui.spacing_mut().interact_size.y = 20.0;
            let button = egui::Button::new(egui::RichText::new("Save").size(12.0));
            if ui
                .add(button)
                .on_hover_text(shortcut::SAVE.hint("Save the script"))
                .clicked()
            {
                save = true;
            }
        });
    }
    save
}
