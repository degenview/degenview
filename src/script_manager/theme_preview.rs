//! The editor shown in Settings with a sample script, so a theme is judged on the real thing.
//!
//! It is [`editor::show`](super::editor::show) over a throwaway [`EditorState`]: the same layouter,
//! gutter, current line, brackets, search and suggestion popups, wearing whatever appearance and
//! font the app's editors wear. Nothing here draws code of its own. The sample is editable (type in
//! it, select, press ⌘F to see the search colours); it is never saved and never compiled. Its two
//! underlined lines carry fixed diagnostics, so error and warning colours have something to colour.

use super::editor::{EditorState, show};
use super::model::CompileStatus;
use crate::pine::token::{
    Diagnostic, DiagnosticCategory, DiagnosticSeverity, SourcePosition, SourceRange,
};
use eframe::egui::{self, Ui, Vec2};

/// A Pine v6 script with a comment, strings, numbers, keywords, user and builtin names, a
/// function with parameters, namespaces, operators, constants and punctuation, then, under a comment
/// that says so, a line the compiler rejects and one that stands for a warning.
pub const SAMPLE: &str = "//@version=6
indicator(\"Theme Preview\", overlay=true)

// User-defined settings
length = input.int(14, \"Period\")
source = close

calculateAverage(src, period) =>
    ta.sma(src, period)

average = calculateAverage(source, length)
isBullish = close > average

var float peak = na
if close > nz(peak)
    peak := close

lineColor = isBullish ? color.green : color.red

plot(average, color=lineColor, linewidth=2)
plot(peak, color=#FF8800)

// Problems are underlined like this:
plot(missing)
alert(\"Bullish\\n\", alert.freq_once_per_bar)
";

const ID: &str = "theme-preview";

pub struct ThemePreview {
    state: EditorState,
    caret_placed: bool,
}

/// A diagnostic under the first `needle` in `SAMPLE`.
fn diagnostic(severity: DiagnosticSeverity, needle: &str, message: &str) -> Diagnostic {
    let offset = SAMPLE.find(needle).expect("the sample has the text");
    let line = SAMPLE[..offset].matches('\n').count() + 1;
    let column = offset - SAMPLE[..offset].rfind('\n').map_or(0, |at| at + 1) + 1;
    Diagnostic {
        code: "PINE0000".into(),
        severity,
        category: DiagnosticCategory::Semantic,
        message: message.into(),
        range: SourceRange::new(
            SourcePosition::new(line, column, offset),
            SourcePosition::new(line, column + needle.len(), offset + needle.len()),
        ),
    }
}

impl Default for ThemePreview {
    fn default() -> Self {
        Self::new()
    }
}

impl ThemePreview {
    pub fn new() -> Self {
        let mut state = EditorState::new(ID, SAMPLE, SAMPLE).sampled();
        state.diagnostics = vec![
            diagnostic(
                DiagnosticSeverity::Error,
                "missing",
                "Example error: `missing` is not declared in this script.",
            ),
            diagnostic(
                DiagnosticSeverity::Warning,
                "alert(",
                "Example warning: this is how a warning is underlined.",
            ),
        ];
        state.status = CompileStatus::Error;
        Self {
            state,
            caret_placed: false,
        }
    }

    /// Draws the editor in `size`. The caret starts in a name that is used twice, so the current
    /// line and the same-name highlights show; the first frame only places it.
    pub fn show(&mut self, ui: &mut Ui, size: Vec2) {
        let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
        if !self.caret_placed {
            self.caret_placed = true;
            let at = SAMPLE.rfind("average").expect("the sample uses average");
            let at = SAMPLE[..at].chars().count() + 3;
            self.state.place_caret(ui.ctx(), at);
        }
        let mut editor = ui.new_child(egui::UiBuilder::new().max_rect(rect));
        editor.set_clip_rect(rect.intersect(ui.clip_rect()));
        show(&mut editor, &mut self.state);
    }

    /// The text in the preview, for tests.
    #[cfg(test)]
    pub fn text(&self) -> &str {
        &self.state.source
    }
}

#[cfg(test)]
mod tests;
