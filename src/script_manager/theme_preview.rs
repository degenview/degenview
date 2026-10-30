//! The editor shown in Settings with a sample script, so a theme is judged on the real thing.
//!
//! It is [`editor::show`](super::editor::show) over a throwaway [`EditorState`]: the same layouter,
//! gutter, current line, brackets, search and suggestion popups, wearing whatever appearance and
//! font the app's editors wear. Nothing here draws code of its own. The sample is editable (type in
//! it, select, press ⌘F to see the search colours); it is never saved. It is compiled, as a script
//! in the editor is, whenever the text changes, so the problems it underlines are the compiler's and
//! follow the code as lines are added or removed: the sample ends with a line the compiler rejects
//! and one it warns about, which gives the error and warning colours something to colour.

use super::editor::{EditorState, show};
use super::model::CompileStatus;
use crate::pine::compiler::{NoLibraries, compile};
use crate::pine::token::Limits;
use eframe::egui::{self, Ui, Vec2};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

/// A Pine v6 script with a comment, strings, numbers, keywords, user and builtin names, a
/// function with parameters, namespaces, operators, constants and punctuation, then, under a comment
/// that says so, a line the compiler rejects (an undeclared name) and one it warns about (an input's
/// `display`, which is ignored here).
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
alert(\"Bullish\\n\", alert.freq_once_per_bar)

// Problems are underlined like this:
plot(missing)
hidden = input.int(1, \"Hidden\", display=display.none)
";

const ID: &str = "theme-preview";

pub struct ThemePreview {
    state: EditorState,
    caret_placed: bool,
    /// The hash of the text the diagnostics are for.
    compiled: Option<u64>,
}

impl Default for ThemePreview {
    fn default() -> Self {
        Self::new()
    }
}

impl ThemePreview {
    pub fn new() -> Self {
        let mut preview = Self {
            state: EditorState::new(ID, SAMPLE, SAMPLE).sampled(),
            caret_placed: false,
            compiled: None,
        };
        preview.recompile();
        preview
    }

    /// Compiles the text if it changed since it was last compiled, and hands the editor what the
    /// compiler says, so a problem is underlined where the code that causes it is now.
    fn recompile(&mut self) {
        let mut hasher = DefaultHasher::new();
        self.state.source.hash(&mut hasher);
        let key = hasher.finish();
        if self.compiled == Some(key) {
            return;
        }
        self.compiled = Some(key);
        let program = compile(&self.state.source, Limits::default(), &NoLibraries);
        self.state.status = CompileStatus::of(&program.diagnostics);
        self.state.diagnostics = program.diagnostics;
    }

    /// Draws the editor in `size`. The caret starts in a name that is used twice, so the current
    /// line and the same-name highlights show; the first frame only places it.
    pub fn show(&mut self, ui: &mut Ui, size: Vec2) {
        let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
        self.recompile();
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
