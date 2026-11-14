//! Editing a script's text: a draft write, or a save of the reviewed draft over the saved source.
//! Both are checked against the revision the caller read, and a save never overwrites an outside
//! edit.

use super::{compile_source, count, find_script, real_draft, store_error};
use crate::mcp::bridge::McpError;
use crate::mcp::types::{EditAction, EditStatus, PineEditResult};
use crate::model::pine::script::{LocalScript, ScriptDraft};
use crate::pine::syntax::token::DiagnosticSeverity;
use crate::store::scripts::{ScriptStore, ScriptStoreError, hash_source};

fn changed_on_disk() -> McpError {
    McpError::Conflict(
        "The script's file was changed outside DegenView after it was opened, so nothing was saved. Resolve it in Pine Studio (keep the file's text or your editor's), then read the script again and retry".into(),
    )
}

/// The longest source an edit may carry (the request body is capped near this anyway).
pub const MAX_EDIT_CHARS: usize = 200_000;

/// Revisions are lowercase hex SHA-256 digests, as `source_hash` reports them.
pub fn is_revision(text: &str) -> bool {
    text.len() == 64 && text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

fn check_edit_source(source: &str) -> Result<(), McpError> {
    if source.chars().count() > MAX_EDIT_CHARS {
        return Err(McpError::ResourceLimit(format!(
            "A script edit is limited to {MAX_EDIT_CHARS} characters"
        )));
    }
    if source
        .chars()
        .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
    {
        return Err(McpError::InvalidArgument(
            "source must be text: it contains control characters".into(),
        ));
    }
    Ok(())
}

/// The text a person sees for the script right now, and where it comes from. An open editor's
/// text is the truth (it can be ahead of anything written to disk); then an unsaved draft; then
/// the saved source.
struct Working {
    text: String,
    origin: &'static str,
}

fn working_text(script: &LocalScript, draft: Option<&ScriptDraft>, live: Option<&str>) -> Working {
    match (live, draft) {
        (Some(text), _) => Working {
            text: text.to_owned(),
            origin: "open editor",
        },
        (None, Some(draft)) => Working {
            text: draft.source.clone(),
            origin: "draft",
        },
        (None, None) => Working {
            text: script.source.clone(),
            origin: "saved source",
        },
    }
}

/// Optimistic concurrency: the caller must have read the text that is current now.
fn check_revision(expected: &str, working: &Working) -> Result<String, McpError> {
    if !is_revision(expected) {
        return Err(McpError::InvalidArgument(
            "expected_revision must be the 64-character source_hash from degenview_get_pine_script"
                .into(),
        ));
    }
    let current = hash_source(&working.text);
    if current == expected {
        Ok(current)
    } else {
        Err(McpError::Conflict(format!(
            "The script changed since it was read: its current revision is {current} ({}). Read it again and re-apply the change; nothing was overwritten",
            working.origin
        )))
    }
}

fn edit_result(
    script: &LocalScript,
    action: EditAction,
    status: EditStatus,
    previous: String,
    revision: String,
    saved_revision_id: Option<String>,
    has_draft: bool,
) -> PineEditResult {
    PineEditResult {
        script_id: script.id.clone(),
        action,
        status,
        previous_revision: previous,
        revision,
        saved_revision_id,
        applied_to_open_editor: false,
        has_draft,
        compile: None,
        compile_warning: None,
    }
}

/// Writes `source` as the script's draft. The saved source, chart-applied scripts and alert
/// subscriptions are untouched; nothing is compiled into a record or run. `live` is the text of
/// the script's open Pine Studio editor, if it is open: the revision is checked against that,
/// never against a stale file.
pub fn update_draft(
    store: &mut ScriptStore,
    id: &str,
    expected_revision: &str,
    source: &str,
    live: Option<&str>,
) -> Result<PineEditResult, McpError> {
    check_edit_source(source)?;
    let script = find_script(store, id)?;
    let draft = real_draft(store, &script);
    let working = working_text(&script, draft.as_ref(), live);
    let previous = check_revision(expected_revision, &working)?;
    if source == working.text {
        return Ok(edit_result(
            &script,
            EditAction::UpdateDraft,
            EditStatus::Unchanged,
            previous.clone(),
            previous,
            None,
            source != script.source,
        ));
    }
    store
        .save_draft(&ScriptDraft {
            script_id: script.id.clone(),
            source: source.to_owned(),
            modified_at_ms: crate::model::pine::script::now_ms(),
            based_on_revision_id: script.latest_revision_id.clone(),
        })
        .map_err(store_error)?;
    Ok(edit_result(
        &script,
        EditAction::UpdateDraft,
        EditStatus::Updated,
        previous,
        hash_source(source),
        None,
        source != script.source,
    ))
}

/// Saves the current draft over the saved source (which records a revision, as the editor's Save
/// does) — but only a draft that compiles without errors, and only the text the caller reviewed.
/// Charts keep their own copy of the source they were applied with.
///
/// The write is compared against what the file held when its author last saw it, exactly like the
/// editor's Save (`ScriptStore::save_if_unchanged`): `open` is the open editor's disk baseline
/// (`None` when the script is not open, then the file as read at the start of this call). An
/// outside edit the editor has not reconciled yet is therefore never overwritten; the call is a
/// `CONFLICT` and the editor, draft and file are left as they were.
pub fn save_script(
    store: &mut ScriptStore,
    id: &str,
    expected_revision: &str,
    live: Option<&str>,
    open: Option<&crate::pine_studio::OpenDiskState>,
) -> Result<PineEditResult, McpError> {
    let script = find_script(store, id)?;
    let draft = real_draft(store, &script);
    let working = working_text(&script, draft.as_ref(), live);
    let previous = check_revision(expected_revision, &working)?;
    if working.text == script.source {
        return Ok(edit_result(
            &script,
            EditAction::Save,
            EditStatus::Unchanged,
            previous.clone(),
            previous,
            script.latest_revision_id.clone(),
            false,
        ));
    }
    if working.text.chars().count() > MAX_EDIT_CHARS {
        return Err(McpError::ResourceLimit(format!(
            "A script save is limited to {MAX_EDIT_CHARS} characters"
        )));
    }
    let program = compile_source(&working.text);
    let errors = count(&program.diagnostics, DiagnosticSeverity::Error);
    if errors > 0 {
        let first = program
            .diagnostics
            .iter()
            .find(|d| d.severity == DiagnosticSeverity::Error)
            .map(|d| format!("{} at line {}: {}", d.code, d.range.start.line, d.message))
            .unwrap_or_default();
        return Err(McpError::CompilationFailed(format!(
            "The draft has {errors} error(s), so it was not saved over the saved source (first: {first}). Fix it, or compile it for the full list"
        )));
    }
    if open.is_some_and(|open| open.conflict) {
        return Err(changed_on_disk());
    }
    let baseline = open.map_or(script.source.as_str(), |open| open.saved_source.as_str());
    let saved = store
        .save_if_unchanged(id, &working.text, baseline)
        .map_err(|error| match error {
            ScriptStoreError::ChangedOnDisk => changed_on_disk(),
            other => store_error(other),
        })?;
    Ok(edit_result(
        &saved,
        EditAction::Save,
        EditStatus::Saved,
        previous,
        hash_source(&saved.source),
        saved.latest_revision_id.clone(),
        false,
    ))
}
