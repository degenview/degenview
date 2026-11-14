//! Read-only views of the Pine Studio's scripts, built on the worker thread, and what the Pine
//! tools (`tools`) hand the app thread ([`PineCommand`]).
//!
//! These read through `ScriptStore` (the owner of script files) and the real Pine compiler, so
//! the saved/draft distinction, content hashes and diagnostics are exactly the app's. Nothing is
//! saved, compiled into a record, run, or sent anywhere: a compile here is a pure function of
//! the source. Paths never leave this module.
//!
//! `scripts` lists, reads and compiles; `edit` writes drafts and saves; `inputs` reads a script's
//! inputs and checks overrides; `apply` resolves what goes on a chart. This file keeps the
//! command and the lookups they share.

pub mod tools;

mod apply;
mod edit;
mod inputs;
mod scripts;
#[cfg(test)]
mod tests;

pub use apply::{ScriptToApply, instance_inputs, script_to_apply};
pub use edit::{MAX_EDIT_CHARS, is_revision, save_script, update_draft};
pub use inputs::{InputArgs, script_inputs};
pub use scripts::{compile_script, diagnostic_info, get_script, list_scripts, script_status};

use crate::mcp::bridge::McpError;
use crate::mcp::confirm::{Confirm, Gate};
use crate::mcp::tools::{
    ADD_PINE_TO_CHART, REMOVE_PINE_INSTANCE, SAVE_PINE_SCRIPT, UPDATE_PINE_INSTANCE,
    UPDATE_PINE_SCRIPT,
};
use crate::mcp::types::SourceVersion;
use crate::model::pine::script::{LocalScript, ScriptDraft};
use crate::pine::compiler;
use crate::pine::compiler::program::CompiledProgram;
use crate::pine::syntax::token::{Diagnostic, DiagnosticSeverity, Limits};
use crate::pine_studio::library::registry::LibraryRegistry;
use crate::store::scripts::ScriptStore;

/// What the Pine tools hand the app thread, run in `host::mcp::pine`: what a chart's scripts
/// computed (private to the chart, so only the app thread can read it), and changes to the
/// scripts on a chart and to a script's text.
#[derive(Debug, Clone, PartialEq)]
pub enum PineCommand {
    ListInstances {
        workspace_id: u64,
        chart_id: u64,
    },
    GetInstance {
        workspace_id: u64,
        chart_id: u64,
        instance_id: u64,
    },
    GetStyles {
        workspace_id: u64,
        chart_id: u64,
        instance_id: u64,
    },
    GetReport {
        workspace_id: u64,
        chart_id: u64,
        instance_id: u64,
        offset: usize,
        limit: usize,
        include_equity: bool,
    },
    ListAlerts,
    /// Applies a script (its checked saved text and input overrides) to a chart.
    AddInstance {
        workspace_id: u64,
        chart_id: u64,
        script: Box<ScriptToApply>,
        visible: bool,
    },
    /// Changes an applied script's inputs (`None` clears that input) and/or visibility.
    UpdateInstance {
        workspace_id: u64,
        chart_id: u64,
        instance_id: u64,
        inputs: Option<InputArgs>,
        visible: Option<bool>,
    },
    RemoveInstance {
        workspace_id: u64,
        chart_id: u64,
        instance_id: u64,
    },
    /// Edit a script's text. `source: Some` writes it as the draft; `None` saves the current
    /// draft over the saved source. Either way `expected_revision` must match the working text.
    EditScript {
        script_id: String,
        expected_revision: String,
        source: Option<String>,
    },
}

impl Confirm for PineCommand {
    fn gate(&self) -> Gate {
        match self {
            // Cannot be undone from here.
            Self::RemoveInstance { .. } => Gate::Ask(REMOVE_PINE_INSTANCE),
            // Overwrites the saved source. Saving is the step that counts for a local credential.
            Self::EditScript { source: None, .. } => Gate::Ask(SAVE_PINE_SCRIPT),
            // Writing a draft changes what is in the editor, and is free for a local credential.
            Self::EditScript {
                source: Some(_), ..
            } => Gate::AskRemote(UPDATE_PINE_SCRIPT),
            // Changes, but easy to see and to undo in the app.
            Self::AddInstance { .. } => Gate::AskRemote(ADD_PINE_TO_CHART),
            Self::UpdateInstance { .. } => Gate::AskRemote(UPDATE_PINE_INSTANCE),
            // Reads.
            Self::ListInstances { .. }
            | Self::GetInstance { .. }
            | Self::GetStyles { .. }
            | Self::GetReport { .. }
            | Self::ListAlerts => Gate::Free,
        }
    }

    fn plain_summary(&self) -> Option<(String, String)> {
        Some(match self {
            Self::AddInstance { script, .. } => {
                ("Apply a script to a chart".to_owned(), script.name.clone())
            }
            Self::UpdateInstance { .. } => ("Change a script on a chart".to_owned(), String::new()),
            Self::EditScript {
                script_id,
                source: Some(_),
                ..
            } => ("Write a script draft".to_owned(), script_id.clone()),
            _ => return None,
        })
    }
}

/// Source returned or compiled is bounded: past this the script is not served.
pub const MAX_SOURCE_CHARS: usize = 1_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Version {
    Saved,
    Draft,
}

impl Version {
    pub fn wire(self) -> SourceVersion {
        match self {
            Self::Saved => SourceVersion::Saved,
            Self::Draft => SourceVersion::Draft,
        }
    }
}

/// What to compile: a script's saved or draft text, or source passed in (not persisted).
pub enum Target {
    Script(String, Version),
    Inline(String),
}

fn store_error<E>(_: E) -> McpError {
    // The reason can name file paths; the caller only learns that the library was unreadable.
    McpError::AppUnavailable("The script library could not be read".into())
}

/// An unsaved draft that actually differs from the saved text; one that equals it is no draft.
fn real_draft(store: &mut ScriptStore, script: &LocalScript) -> Option<ScriptDraft> {
    store
        .draft(&script.id)
        .ok()
        .flatten()
        .filter(|draft| draft.source != script.source)
}

fn find_script(store: &mut ScriptStore, id: &str) -> Result<LocalScript, McpError> {
    // Scanning the library also publishes its libraries to the compiler's catalog.
    store
        .script(id)
        .map_err(store_error)?
        .ok_or_else(|| McpError::NotFound(format!("No script {id}")))
}

/// The chosen text of a script, never the other one in its place.
fn text_of(
    store: &mut ScriptStore,
    id: &str,
    version: Version,
) -> Result<(LocalScript, String, Option<ScriptDraft>), McpError> {
    let script = find_script(store, id)?;
    let draft = real_draft(store, &script);
    let source = match version {
        Version::Saved => script.source.clone(),
        Version::Draft => draft
            .as_ref()
            .map(|draft| draft.source.clone())
            .ok_or_else(|| McpError::NotFound("This script has no unsaved draft".into()))?,
    };
    if source.chars().count() > MAX_SOURCE_CHARS {
        return Err(McpError::ResourceLimit(format!(
            "The source is over {MAX_SOURCE_CHARS} characters"
        )));
    }
    Ok((script, source, draft))
}

fn compile_source(source: &str) -> CompiledProgram {
    compiler::compile(source, Limits::default(), &LibraryRegistry)
}

fn count(diagnostics: &[Diagnostic], severity: DiagnosticSeverity) -> usize {
    diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.severity == severity)
        .count()
}
