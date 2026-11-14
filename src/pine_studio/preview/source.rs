//! The script's text and inputs reaching the chart: the debounced apply, the inputs store, and
//! what the run left (its runtime error, logs and output).

use super::{APPLY_DELAY, PreviewState};
use crate::model::pine::script::ScriptType;
use crate::pine::compiler;
use crate::pine::compiler::program::VisualOutput;
use crate::pine::syntax::token::{DiagnosticSeverity, Limits};
use crate::pine_studio::inputs::{self, InputEdit, InputValues};
use crate::pine_studio::library::registry::LibraryRegistry;
use crate::pine_studio::workspace::PreviewInput;
use crate::ui::PREVIEW_INSTANCE;
use std::collections::HashMap;
use std::time::Instant;

impl PreviewState {
    pub(super) fn source_changed(&mut self, input: &PreviewInput) {
        self.script_type = Some(input.script_type);
        self.script_title = Some(input.name.clone());
        if self.latest_source.as_deref() == Some(input.source.as_str()) {
            return;
        }
        let first = self.latest_source.is_none();
        self.latest_source = Some(input.source.clone());
        self.changed_at = Some(if first {
            // The first source after opening a script applies at once.
            Instant::now() - APPLY_DELAY
        } else {
            Instant::now()
        });
    }

    /// Applies the latest source once typing pauses. Source that doesn't compile leaves the last
    /// working output on screen and shows the stale banner.
    pub(super) fn maybe_apply(&mut self, input: &PreviewInput) {
        let Some(at) = self.changed_at else {
            return;
        };
        // The diagnostics below decide whether the source may run; wait until they describe it.
        if at.elapsed() < APPLY_DELAY || input.compile_pending {
            return;
        }
        self.changed_at = None;
        if input.script_type == ScriptType::Library {
            self.card.set_pine_source(None);
            self.applied_source = None;
            return;
        }
        let has_errors = input
            .diagnostics
            .iter()
            .any(|d| d.severity == DiagnosticSeverity::Error);
        if has_errors {
            self.compile_failed = true;
            self.stale = self.applied_source.is_some();
            return;
        }
        self.compile_failed = false;
        self.stale = false;
        if self.applied_source.as_deref() != Some(input.source.as_str()) {
            // The script's declared inputs, with the user's overrides kept where they still fit.
            let program = compiler::compile(&input.source, Limits::default(), &LibraryRegistry);
            self.input_values = inputs::prune_values(&self.input_values, &program.input_schema);
            self.input_schema = program.input_schema;
            self.card.set_pine_inputs(self.input_values.clone());
            self.card.set_pine_source(Some(input.source.clone()));
            self.applied_source = Some(input.source.clone());
        }
    }

    /// Applies a card's change: a value equal to the default is no override at all.
    pub(in crate::pine_studio) fn apply_input_edit(&mut self, edit: InputEdit) {
        inputs::apply_edit(&mut self.input_values, &self.input_schema, edit);
        self.card.set_pine_inputs(self.input_values.clone());
        self.inputs_dirty_at = Some(Instant::now());
    }

    /// Brings the Swift app's saved preview inputs across (script id → values); a script that
    /// already has values here keeps them.
    pub(in crate::pine_studio) fn import_inputs(&mut self, imported: HashMap<String, InputValues>) {
        self.inputs_store.import_missing(imported);
    }

    pub(super) fn flush_inputs(&mut self) {
        if self.inputs_dirty_at.take().is_some()
            && let Some(id) = self.script_id.clone()
        {
            self.inputs_store.set_inputs(&id, &self.input_values);
        }
    }

    /// The runtime failure of the script on the chart, if its last run stopped early.
    pub(in crate::pine_studio) fn runtime_error(
        &self,
    ) -> Option<&crate::pine::syntax::token::Diagnostic> {
        self.card.pine_error()
    }

    /// The chart's runtime failure, if its run was of `source`. A run of older text has offsets
    /// that mean nothing in the text now in the editor, so it is not reported against it.
    pub(in crate::pine_studio) fn runtime_error_for(
        &self,
        source: &str,
    ) -> Option<&crate::pine::syntax::token::Diagnostic> {
        if self.applied_source.as_deref() == Some(source) {
            self.runtime_error()
        } else {
            None
        }
    }

    /// The messages the preview's script wrote with `log.*()` in its current run.
    pub(in crate::pine_studio) fn logs(&self) -> Option<crate::host::pine::chart::PineLogs<'_>> {
        self.card.pine_logs(PREVIEW_INSTANCE)
    }

    pub(in crate::pine_studio) fn clear_logs(&mut self) {
        self.card.clear_pine_logs(PREVIEW_INSTANCE);
    }

    pub(in crate::pine_studio) fn logging(&self) -> bool {
        self.card.pine_logging(PREVIEW_INSTANCE)
    }

    pub(in crate::pine_studio) fn set_logging(&mut self, enabled: bool) {
        self.card.set_pine_logging(PREVIEW_INSTANCE, enabled);
    }

    /// Scrolls the preview chart to the bar holding `time_ms`.
    pub(in crate::pine_studio) fn scroll_to_time(&mut self, time_ms: i64) -> crate::ui::Scrolled {
        self.card.scroll_to_time(time_ms, &self.candles)
    }

    /// The last run's output, for the Report tab.
    pub(in crate::pine_studio) fn visual(&self) -> Option<&VisualOutput> {
        self.card.pine_visual()
    }
}
