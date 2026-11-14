//! Listing, reading and compiling scripts: the library as the Pine Studio shows it, one
//! script's saved or draft text, and a compile's diagnostics.

use super::{
    MAX_SOURCE_CHARS, Target, Version, compile_source, count, real_draft, store_error, text_of,
};
use crate::format::iso8601;
use crate::mcp::bridge::McpError;
use crate::mcp::types::{
    CompileState, DiagnosticKind, PineCompileResult, PineCompileSummary, PineDiagnosticInfo,
    PineScriptInfo, PineScriptSource, PineScriptsResult, ScriptKind, ScriptStatus, Severity,
    SourceVersion,
};
use crate::model::pine::script::{CompileStatus, ScriptId, ScriptType};
use crate::pine::compiler::program::ScriptType as DeclaredType;
use crate::pine::syntax::token::{Diagnostic, DiagnosticCategory, DiagnosticSeverity};
use crate::store::scripts::{COMPILER_VERSION, ScriptStore, hash_source};

/// Diagnostics returned per compile.
const MAX_DIAGNOSTICS: usize = 200;
/// Scripts listed at most.
const MAX_SCRIPTS: usize = 500;

fn script_kind(script_type: ScriptType) -> ScriptKind {
    match script_type {
        ScriptType::Indicator => ScriptKind::Indicator,
        ScriptType::Strategy => ScriptKind::Strategy,
        ScriptType::Library => ScriptKind::Library,
    }
}

fn declared_kind(script_type: DeclaredType) -> ScriptKind {
    match script_type {
        DeclaredType::Indicator => ScriptKind::Indicator,
        DeclaredType::Strategy => ScriptKind::Strategy,
        DeclaredType::Library => ScriptKind::Library,
    }
}

fn compile_state(status: CompileStatus) -> CompileState {
    match status {
        CompileStatus::NotCompiled => CompileState::NotCompiled,
        CompileStatus::Valid => CompileState::Valid,
        CompileStatus::Warning => CompileState::Warning,
        CompileStatus::Error => CompileState::Error,
    }
}

pub fn list_scripts(store: &mut ScriptStore) -> Result<PineScriptsResult, McpError> {
    let scripts = store.all_scripts().map_err(store_error)?;
    let total = scripts.len();
    let infos = scripts
        .iter()
        .take(MAX_SCRIPTS)
        .map(|script| {
            let hash = hash_source(&script.source);
            let has_draft = real_draft(store, script).is_some();
            PineScriptInfo {
                script_id: script.id.clone(),
                name: script.name.clone(),
                script_type: script_kind(script.script_type),
                has_draft,
                is_favorite: script.is_favorite,
                modified_at: iso8601(script.modified_at_ms),
                last_compile: script
                    .compile_record
                    .as_ref()
                    .map(|record| PineCompileSummary {
                        status: compile_state(record.status),
                        current: record.source_hash == hash
                            && record.compiler_version == COMPILER_VERSION,
                        compiled_at: iso8601(record.compiled_at_ms),
                    }),
                source_hash: hash,
            }
        })
        .collect();
    Ok(PineScriptsResult {
        scripts: infos,
        total,
    })
}

pub fn get_script(
    store: &mut ScriptStore,
    id: &str,
    version: Version,
) -> Result<PineScriptSource, McpError> {
    let (script, source, draft) = text_of(store, id, version)?;
    Ok(PineScriptSource {
        script_id: script.id,
        name: script.name,
        script_type: script_kind(script.script_type),
        version: version.wire(),
        size_chars: source.chars().count(),
        source_hash: hash_source(&source),
        source,
        revision_id: script.latest_revision_id,
        has_draft: draft.is_some(),
        draft_modified_at: draft.map(|draft| iso8601(draft.modified_at_ms)),
    })
}

fn severity(severity: DiagnosticSeverity) -> Severity {
    match severity {
        DiagnosticSeverity::Error => Severity::Error,
        DiagnosticSeverity::Warning => Severity::Warning,
    }
}

fn diagnostic_kind(category: DiagnosticCategory) -> DiagnosticKind {
    match category {
        DiagnosticCategory::Lexical => DiagnosticKind::Lexical,
        DiagnosticCategory::Syntax => DiagnosticKind::Syntax,
        DiagnosticCategory::Semantic => DiagnosticKind::Semantic,
        DiagnosticCategory::Unsupported => DiagnosticKind::Unsupported,
        DiagnosticCategory::Resource => DiagnosticKind::Resource,
        DiagnosticCategory::Runtime => DiagnosticKind::Runtime,
        DiagnosticCategory::Cancellation => DiagnosticKind::Cancellation,
    }
}

pub fn diagnostic_info(diagnostic: &Diagnostic) -> PineDiagnosticInfo {
    PineDiagnosticInfo {
        severity: severity(diagnostic.severity),
        code: diagnostic.code.clone(),
        category: diagnostic_kind(diagnostic.category),
        message: diagnostic.message.clone(),
        line: diagnostic.range.start.line,
        column: diagnostic.range.start.column,
        end_line: diagnostic.range.end.line,
        end_column: diagnostic.range.end.column,
    }
}

/// How an applied script's source compares with the saved script it came from now.
pub fn script_status(
    store: &mut ScriptStore,
    script_id: Option<&str>,
    applied_hash: &str,
) -> Result<ScriptStatus, McpError> {
    let Some(id) = script_id else {
        return Ok(ScriptStatus::Unlinked);
    };
    Ok(match store.script(id).map_err(store_error)? {
        None => ScriptStatus::Deleted,
        Some(script) if hash_source(&script.source) == applied_hash => ScriptStatus::Unchanged,
        Some(_) => ScriptStatus::Changed,
    })
}

/// Compiles without running: no trades, alerts, webhooks, saves or chart changes.
pub fn compile_script(
    store: &mut ScriptStore,
    target: Target,
) -> Result<PineCompileResult, McpError> {
    let (script_id, version, source): (Option<ScriptId>, SourceVersion, String) = match target {
        Target::Script(id, version) => {
            let (script, source, _) = text_of(store, &id, version)?;
            (Some(script.id), version.wire(), source)
        }
        Target::Inline(source) => {
            if source.chars().count() > MAX_SOURCE_CHARS {
                return Err(McpError::ResourceLimit(format!(
                    "The source is over {MAX_SOURCE_CHARS} characters"
                )));
            }
            // Warm the library catalog so `import` lines resolve against the user's libraries.
            let _ = store.all_scripts();
            (None, SourceVersion::Inline, source)
        }
    };
    let program = compile_source(&source);
    let errors = count(&program.diagnostics, DiagnosticSeverity::Error);
    let warnings = count(&program.diagnostics, DiagnosticSeverity::Warning);
    Ok(PineCompileResult {
        success: errors == 0,
        script_id,
        version,
        source_hash: hash_source(&source),
        script_type: (errors == 0).then(|| declared_kind(program.declaration.script_type)),
        pine_version: program.declaration.pine_version,
        title: (errors == 0).then(|| program.declaration.title.clone()),
        error_count: errors,
        warning_count: warnings,
        diagnostics_truncated: program.diagnostics.len() > MAX_DIAGNOSTICS,
        diagnostics: program
            .diagnostics
            .iter()
            .take(MAX_DIAGNOSTICS)
            .map(diagnostic_info)
            .collect(),
    })
}
