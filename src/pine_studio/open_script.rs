//! The script open in the editor: opening and closing it, its draft, revealing a logged call,
//! and edits made outside the app (another editor, the MCP server) reaching it.

use super::editor::EditorState;
use super::library::registry::LibraryRegistry;
use super::library::sidebar::RowKey;
use super::{OpenDiskState, OpenScript, PineStudio, compile_into, editor, group_of, library};
use crate::model::pine::script::{ScriptDraft, ScriptType, now_ms};
use crate::pine::compiler;
use crate::pine::syntax::token::Limits;
use crate::store::scripts::{COMPILER_VERSION, hash_source};
use eframe::egui;
use std::time::Instant;

impl PineStudio {
    /// Whether the open script's file has moved (or is still settling), so that the folder is worth
    /// scanning before the next full scan. Only the file's metadata is read here.
    pub(super) fn external_scan_due(&mut self, now: Instant) -> bool {
        let Some(open) = self.open.as_mut() else {
            return false;
        };
        if !open.watch.poll_due(now) {
            return false;
        }
        open.watch.note_poll(now);
        let id = open.id.clone();
        let stamp = self
            .script_store
            .as_mut()
            .and_then(|store| store.file_stamp(&id));
        self.open
            .as_mut()
            .is_some_and(|open| open.watch.wants_scan(stamp))
    }

    /// Compares what the last scan read of the open script's file with what the editor holds, and
    /// acts on a file that was edited outside the app (see `external`).
    pub(super) fn reconcile_open(&mut self, ctx: &egui::Context, now: Instant) {
        let Some(open) = self.open.as_mut() else {
            return;
        };
        let Some(script) = self.scripts.iter().find(|script| script.id == open.id) else {
            return;
        };
        let verdict = open.watch.observe(
            now,
            &script.source,
            open.editor.saved_source(),
            &open.editor.source,
        );
        match verdict {
            library::external::Verdict::Nothing | library::external::Verdict::Conflict => {}
            library::external::Verdict::Baseline => open.editor.mark_saved(),
            library::external::Verdict::Reload(text) => self.adopt_disk_text(ctx, &text),
        }
    }

    /// Shows the file's text in the open editor as its saved text. One undo step, so Undo brings
    /// back what was there (typing included).
    pub(super) fn adopt_disk_text(&mut self, ctx: &egui::Context, text: &str) {
        let Some(open) = self.open.as_mut() else {
            return;
        };
        editor::apply_script(ctx, &mut open.editor, text);
        open.editor.mark_saved();
        open.watch.reloaded();
        open.editor
            .set_notice("Reloaded: the file changed outside the app.");
        self.editor_changed();
    }

    /// Opens the script (or library) a Pine logs message was written by and puts the caret on the
    /// call. A script that was edited since the message was written is opened all the same, with a
    /// note that the line may have moved; one that is gone is told so and nothing else changes.
    pub(crate) fn reveal_source(&mut self, target: &crate::model::pine::logs_view::SourceTarget) {
        let Some(scripts) = self.with_store(|store| store.all_scripts()) else {
            return;
        };
        let found = match (&target.script_id, &target.library) {
            (Some(id), _) => scripts.iter().find(|script| &script.id == id),
            (None, Some(name)) => scripts.iter().find(|script| {
                script.script_type == ScriptType::Library
                    && (&script.name == name
                        || library::registry::title_of(&script.source).as_ref() == Some(name))
            }),
            (None, None) => None,
        };
        let Some(script) = found else {
            self.error = Some(match &target.library {
                Some(name) => format!("The library “{name}” is not in the Pine Studio."),
                None => "That script is no longer in the Pine Studio.".to_owned(),
            });
            return;
        };
        let changed = target
            .ran_source
            .as_ref()
            .is_some_and(|ran| *ran != script.source);
        let id = script.id.clone();
        self.renaming = None;
        self.selected_row = Some(RowKey {
            group: group_of(script),
            id: id.clone(),
        });
        // A script already in the editor stays as it is, with its caret and undo history.
        if self.open.as_ref().map(|open| open.id.as_str()) != Some(id.as_str()) {
            self.open_script(&id);
        }
        if let Some(open) = self.open.as_mut().filter(|open| open.id == id) {
            open.editor.reveal_offset(target.offset);
        }
        if changed {
            self.error = Some(
                "This script was edited after that message was written, so the line may have moved."
                    .to_owned(),
            );
        }
    }

    pub(crate) fn open_script(&mut self, id: &str) {
        self.flush_draft();
        let Some((script, draft)) = self.with_store(|store| {
            let script = store.script(id)?;
            let draft = store.draft(id)?;
            Ok((script, draft))
        }) else {
            return;
        };
        let Some(script) = script else {
            return;
        };
        let source = draft
            .filter(|d| d.source != script.source)
            .map(|d| d.source)
            .unwrap_or_else(|| script.source.clone());
        let mut editor = EditorState::new(&script.id, &source, &script.source);
        // A cached compile counts only when the compiler hasn't changed and the text matches.
        let cached = script.compile_record.as_ref().filter(|r| {
            r.compiler_version == COMPILER_VERSION && r.source_hash == hash_source(&source)
        });
        match cached {
            Some(record) => {
                editor.status = record.status;
                editor.diagnostics = record.diagnostics.clone();
            }
            None => compile_into(&mut editor),
        }
        self.preview.script_changed(&script.id);
        self.debug.script_changed(&script.id);
        self.profiler.script_changed();
        self.open = Some(OpenScript {
            id: script.id.clone(),
            name: script.name.clone(),
            script_type: script.script_type,
            editor,
            has_alerts: false,
            watch: library::external::Watch::default(),
        });
        self.prefs.selected_script_id = Some(script.id);
        self.refresh_debug_map();
    }

    pub(super) fn close_script(&mut self) {
        self.flush_draft();
        self.open = None;
        self.selected_row = None;
        self.renaming = None;
        self.prefs.selected_script_id = None;
    }

    /// Writes the pending draft now (switching scripts or closing).
    pub(super) fn flush_draft(&mut self) {
        let Some(open) = self.open.as_mut() else {
            return;
        };
        if open.editor.has_pending_draft() && open.editor.is_dirty() {
            let draft = ScriptDraft {
                script_id: open.id.clone(),
                source: open.editor.source.clone(),
                modified_at_ms: now_ms(),
                based_on_revision_id: None,
            };
            self.with_store(|store| store.save_draft(&draft));
        }
    }

    /// The text in the editor if `id` is the open script: what the person sees, which can be ahead
    /// of anything on disk.
    pub(crate) fn open_text(&self, id: &str) -> Option<String> {
        self.open
            .as_ref()
            .filter(|open| open.id == id)
            .map(|open| open.editor.source.clone())
    }

    /// What the open editor last read from disk, and whether the file has since changed under
    /// unsaved typing with no decision yet. `None` unless `id` is the open script.
    pub(crate) fn open_disk_state(&self, id: &str) -> Option<OpenDiskState> {
        self.open
            .as_ref()
            .filter(|open| open.id == id)
            .map(|open| OpenDiskState {
                saved_source: open.editor.saved_source().to_owned(),
                conflict: open.watch.conflict.is_some(),
            })
    }

    /// Looks at the open script's file at once instead of waiting for it to settle: an outside
    /// write was just refused, so a conflict question should appear over the typing.
    pub(crate) fn look_at_disk_now(&mut self, id: &str) {
        if let Some(open) = self.open.as_mut().filter(|open| open.id == id) {
            open.watch.force = true;
        }
    }

    /// Shows an edit made from outside in the open editor (`saved`: it is also the saved source
    /// now). Returns whether `id` was open. Recompiles, so the status chip and Problems follow.
    pub(crate) fn adopt_external_text(&mut self, id: &str, text: &str, saved: bool) -> bool {
        let Some(open) = self.open.as_mut().filter(|open| open.id == id) else {
            return false;
        };
        open.editor.replace_source(text, saved);
        self.editor_changed();
        if saved {
            self.refresh();
        }
        true
    }

    /// Recompiles the open script after an edit, feeding the diagnostics back to the editor.
    pub(crate) fn editor_changed(&mut self) {
        if let Some(open) = self.open.as_mut() {
            compile_into(&mut open.editor);
            let program =
                compiler::compile(&open.editor.source, Limits::default(), &LibraryRegistry);
            open.has_alerts = program.is_valid()
                && !crate::pine::builtins::alert::call::call_sites(&program.statements).is_empty();
            self.debug.set_map(
                program
                    .is_valid()
                    .then(|| crate::pine::debug::SourceMap::build(&program.statements)),
            );
        }
    }
}
