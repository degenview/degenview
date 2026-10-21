//! Read-only views of the Script Manager's scripts, built on the worker thread.
//!
//! These read through `ScriptStore` (the owner of script files) and the real Pine compiler, so
//! the saved/draft distinction, content hashes and diagnostics are exactly the app's. Nothing is
//! saved, compiled into a record, run, or sent anywhere: a compile here is a pure function of
//! the source. Paths never leave this module.

use crate::format::iso8601;
use crate::mcp::bridge::McpError;
use crate::mcp::types::{
    PineCompileResult, PineCompileSummary, PineDiagnosticInfo, PineEditResult, PineInputInfo,
    PineInputOption, PineInputsResult, PineScriptInfo, PineScriptSource, PineScriptsResult,
};
use crate::pine::compiler;
use crate::pine::program::{
    CompiledProgram, InputDefinition, InputSchema, InputValue, ScriptType as DeclaredType,
};
use crate::pine::token::{Diagnostic, DiagnosticCategory, DiagnosticSeverity, Limits};
use crate::script_manager::model::{CompileStatus, LocalScript, ScriptDraft, ScriptId, ScriptType};
use crate::script_manager::registry::LibraryRegistry;
use crate::script_manager::store::{COMPILER_VERSION, ScriptStore, hash_source};
use schemars::JsonSchema;
use serde::Deserialize;
use std::collections::{BTreeMap, HashMap};

/// Source returned or compiled is bounded: past this the script is not served.
pub const MAX_SOURCE_CHARS: usize = 1_000_000;
/// Diagnostics returned per compile.
const MAX_DIAGNOSTICS: usize = 200;
/// Scripts listed at most.
const MAX_SCRIPTS: usize = 500;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Version {
    Saved,
    Draft,
}

impl Version {
    pub fn label(self) -> &'static str {
        match self {
            Self::Saved => "saved",
            Self::Draft => "draft",
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

fn type_name(script_type: ScriptType) -> &'static str {
    match script_type {
        ScriptType::Indicator => "indicator",
        ScriptType::Strategy => "strategy",
        ScriptType::Library => "library",
    }
}

fn declared_name(script_type: DeclaredType) -> &'static str {
    match script_type {
        DeclaredType::Indicator => "indicator",
        DeclaredType::Strategy => "strategy",
        DeclaredType::Library => "library",
    }
}

fn status_name(status: CompileStatus) -> &'static str {
    match status {
        CompileStatus::NotCompiled => "not_compiled",
        CompileStatus::Valid => "valid",
        CompileStatus::Warning => "warning",
        CompileStatus::Error => "error",
    }
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
                script_type: type_name(script.script_type).to_owned(),
                has_draft,
                is_favorite: script.is_favorite,
                modified_at: iso8601(script.modified_at_ms),
                last_compile: script
                    .compile_record
                    .as_ref()
                    .map(|record| PineCompileSummary {
                        status: status_name(record.status).to_owned(),
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

pub fn get_script(
    store: &mut ScriptStore,
    id: &str,
    version: Version,
) -> Result<PineScriptSource, McpError> {
    let (script, source, draft) = text_of(store, id, version)?;
    Ok(PineScriptSource {
        script_id: script.id,
        name: script.name,
        script_type: type_name(script.script_type).to_owned(),
        version: version.label().to_owned(),
        size_chars: source.chars().count(),
        source_hash: hash_source(&source),
        source,
        revision_id: script.latest_revision_id,
        has_draft: draft.is_some(),
        draft_modified_at: draft.map(|draft| iso8601(draft.modified_at_ms)),
    })
}

fn severity_name(severity: DiagnosticSeverity) -> &'static str {
    match severity {
        DiagnosticSeverity::Error => "error",
        DiagnosticSeverity::Warning => "warning",
    }
}

fn category_name(category: DiagnosticCategory) -> &'static str {
    match category {
        DiagnosticCategory::Lexical => "lexical",
        DiagnosticCategory::Syntax => "syntax",
        DiagnosticCategory::Semantic => "semantic",
        DiagnosticCategory::Unsupported => "unsupported",
        DiagnosticCategory::Resource => "resource",
        DiagnosticCategory::Runtime => "runtime",
        DiagnosticCategory::Cancellation => "cancellation",
    }
}

pub fn diagnostic_info(diagnostic: &Diagnostic) -> PineDiagnosticInfo {
    PineDiagnosticInfo {
        severity: severity_name(diagnostic.severity).to_owned(),
        code: diagnostic.code.clone(),
        category: category_name(diagnostic.category).to_owned(),
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
) -> Result<String, McpError> {
    let Some(id) = script_id else {
        return Ok("unlinked".into());
    };
    Ok(match store.script(id).map_err(store_error)? {
        None => "deleted",
        Some(script) if hash_source(&script.source) == applied_hash => "unchanged",
        Some(_) => "changed",
    }
    .into())
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

/// Compiles without running: no trades, alerts, webhooks, saves or chart changes.
pub fn compile_script(
    store: &mut ScriptStore,
    target: Target,
) -> Result<PineCompileResult, McpError> {
    let (script_id, version, source): (Option<ScriptId>, &str, String) = match target {
        Target::Script(id, version) => {
            let (script, source, _) = text_of(store, &id, version)?;
            (Some(script.id), version.label(), source)
        }
        Target::Inline(source) => {
            if source.chars().count() > MAX_SOURCE_CHARS {
                return Err(McpError::ResourceLimit(format!(
                    "The source is over {MAX_SOURCE_CHARS} characters"
                )));
            }
            // Warm the library catalog so `import` lines resolve against the user's libraries.
            let _ = store.all_scripts();
            (None, "inline", source)
        }
    };
    let program = compile_source(&source);
    let errors = count(&program.diagnostics, DiagnosticSeverity::Error);
    let warnings = count(&program.diagnostics, DiagnosticSeverity::Warning);
    Ok(PineCompileResult {
        success: errors == 0,
        script_id,
        version: version.to_owned(),
        source_hash: hash_source(&source),
        script_type: (errors == 0)
            .then(|| declared_name(program.declaration.script_type).to_owned()),
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

// MARK: Editing

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
    action: &str,
    status: &str,
    previous: String,
    revision: String,
    saved_revision_id: Option<String>,
    has_draft: bool,
) -> PineEditResult {
    PineEditResult {
        script_id: script.id.clone(),
        action: action.to_owned(),
        status: status.to_owned(),
        previous_revision: previous,
        revision,
        saved_revision_id,
        applied_to_open_editor: false,
        has_draft,
        compile: None,
    }
}

/// Writes `source` as the script's draft. The saved source, chart-applied scripts and alert
/// subscriptions are untouched; nothing is compiled into a record or run. `live` is the text of
/// the script's open Script Manager editor, if it is open: the revision is checked against that,
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
            "update_draft",
            "unchanged",
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
            modified_at_ms: crate::script_manager::model::now_ms(),
            based_on_revision_id: script.latest_revision_id.clone(),
        })
        .map_err(store_error)?;
    Ok(edit_result(
        &script,
        "update_draft",
        "updated",
        previous,
        hash_source(source),
        None,
        source != script.source,
    ))
}

/// Saves the current draft over the saved source (which records a revision, as the editor's Save
/// does) — but only a draft that compiles without errors, and only the text the caller reviewed.
/// Charts keep their own copy of the source they were applied with.
pub fn save_script(
    store: &mut ScriptStore,
    id: &str,
    expected_revision: &str,
    live: Option<&str>,
) -> Result<PineEditResult, McpError> {
    let script = find_script(store, id)?;
    let draft = real_draft(store, &script);
    let working = working_text(&script, draft.as_ref(), live);
    let previous = check_revision(expected_revision, &working)?;
    if working.text == script.source {
        return Ok(edit_result(
            &script,
            "save",
            "unchanged",
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
    let saved = store.save(id, &working.text).map_err(store_error)?;
    Ok(edit_result(
        &saved,
        "save",
        "saved",
        previous,
        hash_source(&saved.source),
        saved.latest_revision_id.clone(),
        false,
    ))
}

fn value_json(value: &InputValue) -> serde_json::Value {
    match value {
        InputValue::Int(value) => serde_json::json!(value),
        InputValue::Float(value) => serde_json::Number::from_f64(*value)
            .map_or(serde_json::Value::Null, serde_json::Value::Number),
        InputValue::Bool(value) => serde_json::json!(value),
        InputValue::String(value) | InputValue::Source(value) => serde_json::json!(value),
        InputValue::Color(rgba) => serde_json::json!(format!("#{rgba:08X}")),
    }
}

fn value_type(value: &InputValue) -> &'static str {
    match value {
        InputValue::Int(_) => "integer",
        InputValue::Float(_) => "float",
        InputValue::Bool(_) => "bool",
        InputValue::String(_) => "string",
        InputValue::Color(_) => "color",
        InputValue::Source(_) => "source",
    }
}

fn input_info(definition: &InputDefinition) -> PineInputInfo {
    let titles = definition.option_titles.as_deref().unwrap_or(&[]);
    PineInputInfo {
        id: definition.id.clone(),
        title: definition.title.clone(),
        value_type: value_type(&definition.default_value).to_owned(),
        default: value_json(&definition.default_value),
        min: definition.min_value,
        max: definition.max_value,
        step: definition.step,
        options: definition.options.as_ref().map(|options| {
            options
                .iter()
                .enumerate()
                .map(|(index, value)| PineInputOption {
                    value: value_json(value),
                    title: titles.get(index).cloned(),
                })
                .collect()
        }),
        group: definition.group.clone(),
        inline: definition.inline.clone(),
        tooltip: definition.tooltip.clone(),
        confirm: definition.confirm,
    }
}

/// The inputs the compiler found, straight from its own schema (no re-parsing of the text).
pub fn script_inputs(
    store: &mut ScriptStore,
    id: &str,
    version: Version,
) -> Result<PineInputsResult, McpError> {
    let (script, source, _) = text_of(store, id, version)?;
    let program = compile_source(&source);
    Ok(PineInputsResult {
        script_id: script.id,
        version: version.label().to_owned(),
        source_hash: hash_source(&source),
        compiled: program.is_valid(),
        inputs: program.input_schema.inputs.iter().map(input_info).collect(),
    })
}

// MARK: Applying scripts to charts

/// A value for a script input as a client sends it. What it may be depends on the input.
#[derive(Debug, Clone, PartialEq, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum InputArg {
    Bool(bool),
    Number(serde_json::Number),
    /// Text, an option's value, a source name, or a colour as `#RRGGBB` / `#RRGGBBAA`.
    Text(String),
}

/// Input overrides by input id; `null` puts that input back to its default.
pub type InputArgs = BTreeMap<String, Option<InputArg>>;

const MAX_INPUT_ARGS: usize = 100;
const MAX_INPUT_TEXT_CHARS: usize = 500;
/// The sources an `input.source` offers, as the Inputs card lists them.
const SOURCE_NAMES: [&str; 5] = ["open", "high", "low", "close", "volume"];

/// `#RRGGBB` (opaque) or `#RRGGBBAA`, the form `degenview_get_pine_inputs` returns.
fn parse_color(text: &str) -> Option<u32> {
    let hex = text.trim().strip_prefix('#').unwrap_or(text.trim());
    if !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    match hex.len() {
        6 => u32::from_str_radix(hex, 16).ok().map(|rgb| rgb << 8 | 0xFF),
        8 => u32::from_str_radix(hex, 16).ok(),
        _ => None,
    }
}

fn input_value(definition: &InputDefinition, arg: &InputArg) -> Result<InputValue, String> {
    let wrong_type = || format!("expects a {} value", value_type(&definition.default_value));
    let value = match (&definition.default_value, arg) {
        (InputValue::Int(_), InputArg::Number(number)) => {
            let whole = number.as_i64().or_else(|| {
                number
                    .as_f64()
                    .filter(|f| f.fract() == 0.0 && f.abs() < 9e15)
                    .map(|f| f as i64)
            });
            InputValue::Int(whole.ok_or("expects a whole number")?)
        }
        (InputValue::Float(_), InputArg::Number(number)) => {
            InputValue::Float(number.as_f64().ok_or_else(wrong_type)?)
        }
        (InputValue::Bool(_), InputArg::Bool(flag)) => InputValue::Bool(*flag),
        (InputValue::String(_), InputArg::Text(text)) => {
            if text.chars().count() > MAX_INPUT_TEXT_CHARS {
                return Err(format!("is limited to {MAX_INPUT_TEXT_CHARS} characters"));
            }
            InputValue::String(text.clone())
        }
        (InputValue::Color(_), InputArg::Text(text)) => InputValue::Color(
            parse_color(text).ok_or("expects a colour like #RRGGBB or #RRGGBBAA")?,
        ),
        (InputValue::Source(_), InputArg::Text(text)) => {
            let name = text.trim().to_ascii_lowercase();
            if !SOURCE_NAMES.contains(&name.as_str()) {
                return Err(format!("expects one of {SOURCE_NAMES:?}"));
            }
            InputValue::Source(name)
        }
        _ => return Err(wrong_type()),
    };
    let number = match &value {
        InputValue::Int(v) => Some(*v as f64),
        InputValue::Float(v) => Some(*v),
        _ => None,
    };
    if let Some(number) = number {
        if definition.min_value.is_some_and(|min| number < min) {
            return Err(format!(
                "must be at least {}",
                definition.min_value.unwrap_or_default()
            ));
        }
        if definition.max_value.is_some_and(|max| number > max) {
            return Err(format!(
                "must be at most {}",
                definition.max_value.unwrap_or_default()
            ));
        }
    }
    if let Some(options) = &definition.options
        && !options.contains(&value)
    {
        let choices: Vec<String> = options.iter().map(|o| value_json(o).to_string()).collect();
        return Err(format!("must be one of {}", choices.join(", ")));
    }
    Ok(value)
}

/// Checks `args` against the script's declared inputs: each must exist and hold a value of the
/// right kind inside the input's range and options. `None` clears an override.
pub fn parse_input_args(
    schema: &InputSchema,
    args: &InputArgs,
) -> Result<Vec<(String, Option<InputValue>)>, McpError> {
    if args.len() > MAX_INPUT_ARGS {
        return Err(McpError::InvalidArgument(format!(
            "At most {MAX_INPUT_ARGS} inputs can be set at once"
        )));
    }
    args.iter()
        .map(|(id, arg)| {
            let Some(definition) = schema.inputs.iter().find(|input| input.id == *id) else {
                let known: Vec<&str> = schema
                    .inputs
                    .iter()
                    .take(20)
                    .map(|i| i.id.as_str())
                    .collect();
                return Err(McpError::InvalidArgument(format!(
                    "This script has no input '{id}' (inputs: {})",
                    if known.is_empty() {
                        "none".to_owned()
                    } else {
                        known.join(", ")
                    }
                )));
            };
            let value = arg
                .as_ref()
                .map(|arg| input_value(definition, arg))
                .transpose()
                .map_err(|reason| McpError::InvalidArgument(format!("Input '{id}' {reason}")))?;
            Ok((id.clone(), value))
        })
        .collect()
}

/// Applies checked changes to the overrides in `current`. As in the Inputs card, a value equal
/// to the input's default is no override at all.
pub fn merge_input_changes(
    schema: &InputSchema,
    current: &mut HashMap<String, InputValue>,
    changes: Vec<(String, Option<InputValue>)>,
) {
    for (id, value) in changes {
        let default = schema
            .inputs
            .iter()
            .find(|input| input.id == id)
            .map(|input| &input.default_value);
        match value {
            Some(value) if default != Some(&value) => {
                current.insert(id, value);
            }
            _ => {
                current.remove(&id);
            }
        }
    }
}

fn compile_failure(program: &CompiledProgram, what: &str) -> McpError {
    let errors = count(&program.diagnostics, DiagnosticSeverity::Error);
    let first = program
        .diagnostics
        .iter()
        .find(|d| d.severity == DiagnosticSeverity::Error)
        .map(|d| format!("{} at line {}: {}", d.code, d.range.start.line, d.message))
        .unwrap_or_default();
    McpError::CompilationFailed(format!(
        "{what} has {errors} error(s) (first: {first}). Compile it for the full list"
    ))
}

/// A saved script ready to be applied to a chart, with its checked input overrides.
#[derive(Debug, Clone, PartialEq)]
pub struct ScriptToApply {
    pub script_id: String,
    pub name: String,
    pub source: String,
    pub inputs: HashMap<String, InputValue>,
}

/// Resolves the script to apply: the saved text (never the draft, as "Add to Chart" does) of an
/// indicator or strategy that compiles, with `args` checked against its declared inputs.
pub fn script_to_apply(
    store: &mut ScriptStore,
    id: &str,
    args: &InputArgs,
) -> Result<ScriptToApply, McpError> {
    let script = find_script(store, id)?;
    let not_applicable = || {
        McpError::Unsupported(format!(
            "'{}' is a library: libraries are imported by other scripts, not applied to a chart",
            script.name
        ))
    };
    if script.script_type == ScriptType::Library {
        return Err(not_applicable());
    }
    if script.source.chars().count() > MAX_SOURCE_CHARS {
        return Err(McpError::ResourceLimit(format!(
            "The source is over {MAX_SOURCE_CHARS} characters"
        )));
    }
    let program = compile_source(&script.source);
    if !program.is_valid() {
        return Err(compile_failure(&program, &format!("'{}'", script.name)));
    }
    if program.declaration.script_type == DeclaredType::Library {
        return Err(not_applicable());
    }
    let changes = parse_input_args(&program.input_schema, args)?;
    let mut inputs = HashMap::new();
    merge_input_changes(&program.input_schema, &mut inputs, changes);
    Ok(ScriptToApply {
        script_id: script.id,
        name: script.name,
        source: script.source,
        inputs,
    })
}

/// An applied instance's overrides after `args`, checked against the schema of its own source
/// (an instance keeps the text it was applied with).
pub fn instance_inputs(
    source: &str,
    mut current: HashMap<String, InputValue>,
    args: &InputArgs,
) -> Result<HashMap<String, InputValue>, McpError> {
    let program = compile_source(source);
    if !program.is_valid() {
        return Err(compile_failure(&program, "This applied script"));
    }
    let changes = parse_input_args(&program.input_schema, args)?;
    merge_input_changes(&program.input_schema, &mut current, changes);
    Ok(current)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::watchlist::{new_id, now_ms};
    use std::path::PathBuf;

    fn temp_store(tag: &str) -> (ScriptStore, PathBuf) {
        let root = std::env::temp_dir().join(format!("degenview-mcp-pine-{tag}-{}", new_id()));
        let store = ScriptStore::new(root.join("Scripts"), root.join("ScriptMetadata"));
        (store, root)
    }

    const SHAPED_INPUTS: &str = "//@version=6\nindicator(\"Shaped\", overlay=true)\n\
len = input.int(20, \"Length\", minval=1, maxval=500)\n\
mult = input.float(2.5, \"Multiplier\")\n\
show = input.bool(true, \"Show\")\n\
col = input.color(color.red, \"Colour\")\n\
src = input.source(close, \"Source\")\n\
mode = input.string(\"fast\", \"Mode\", options=[\"fast\", \"slow\"])\n\
plot(ta.sma(src, len) * mult)\n";

    fn args(json: serde_json::Value) -> InputArgs {
        serde_json::from_value(json).unwrap()
    }

    fn parsed(json: serde_json::Value) -> Result<HashMap<String, InputValue>, McpError> {
        let program = compile_source(SHAPED_INPUTS);
        assert!(program.is_valid(), "{:?}", program.diagnostics);
        let changes = parse_input_args(&program.input_schema, &args(json))?;
        let mut inputs = HashMap::new();
        merge_input_changes(&program.input_schema, &mut inputs, changes);
        Ok(inputs)
    }

    #[test]
    fn input_arguments_are_checked_against_what_the_script_declares() {
        let ok = parsed(serde_json::json!({
            "len": 50, "mult": 3, "show": false, "col": "#00FF00",
            "src": "High", "mode": "slow"
        }))
        .unwrap();
        assert_eq!(ok["len"], InputValue::Int(50));
        assert_eq!(ok["mult"], InputValue::Float(3.0));
        assert_eq!(ok["show"], InputValue::Bool(false));
        assert_eq!(ok["col"], InputValue::Color(0x00FF_00FF));
        assert_eq!(ok["src"], InputValue::Source("high".into()));
        assert_eq!(ok["mode"], InputValue::String("slow".into()));
        assert_eq!(
            parsed(serde_json::json!({"col": "#11223344"})).unwrap()["col"],
            InputValue::Color(0x1122_3344)
        );

        for bad in [
            serde_json::json!({"nope": 1}),
            serde_json::json!({"len": 0}),
            serde_json::json!({"len": 501}),
            serde_json::json!({"len": 1.5}),
            serde_json::json!({"len": "20"}),
            serde_json::json!({"show": 1}),
            serde_json::json!({"col": "red"}),
            serde_json::json!({"col": "#12345"}),
            serde_json::json!({"src": "vwap"}),
            serde_json::json!({"mode": "medium"}),
            serde_json::json!({"mode": 1}),
        ] {
            assert!(
                matches!(parsed(bad.clone()), Err(McpError::InvalidArgument(_))),
                "{bad}"
            );
        }
        let unknown = parsed(serde_json::json!({"nope": 1})).unwrap_err();
        assert!(unknown.message().contains("len"), "{}", unknown.message());
    }

    #[test]
    fn a_default_or_null_is_no_override() {
        let program = compile_source(SHAPED_INPUTS);
        let mut current = HashMap::from([
            ("len".to_owned(), InputValue::Int(50)),
            ("show".to_owned(), InputValue::Bool(false)),
        ]);
        let changes = parse_input_args(
            &program.input_schema,
            &args(serde_json::json!({"len": 20, "show": null, "mult": 4.5})),
        )
        .unwrap();
        merge_input_changes(&program.input_schema, &mut current, changes);
        assert_eq!(
            current,
            HashMap::from([("mult".to_owned(), InputValue::Float(4.5))])
        );
    }

    #[test]
    fn only_a_compiling_indicator_or_strategy_can_be_applied() {
        let (mut store, root) = temp_store("apply");
        let good = create(&mut store, "Good", ScriptType::Indicator, SHAPED_INPUTS);
        let strategy = create(
            &mut store,
            "Strat",
            ScriptType::Strategy,
            "//@version=6\nstrategy(\"S\")\n",
        );
        let broken = create(
            &mut store,
            "Broken",
            ScriptType::Indicator,
            "//@version=6\nindicator(\"B\")\nplot(close\n",
        );
        let library = create(
            &mut store,
            "Lib",
            ScriptType::Library,
            "//@version=6\nlibrary(\"Lib\")\n",
        );
        let none = InputArgs::new();

        let applied =
            script_to_apply(&mut store, &good.id, &args(serde_json::json!({"len": 9}))).unwrap();
        assert_eq!(applied.source, SHAPED_INPUTS);
        assert_eq!(applied.inputs["len"], InputValue::Int(9));
        assert!(script_to_apply(&mut store, &strategy.id, &none).is_ok());
        assert!(matches!(
            script_to_apply(&mut store, &broken.id, &none),
            Err(McpError::CompilationFailed(_))
        ));
        assert!(matches!(
            script_to_apply(&mut store, &library.id, &none),
            Err(McpError::Unsupported(_))
        ));
        assert!(matches!(
            script_to_apply(&mut store, "missing", &none),
            Err(McpError::NotFound(_))
        ));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn an_instance_is_checked_against_the_source_it_was_applied_with() {
        let current = HashMap::from([("len".to_owned(), InputValue::Int(50))]);
        let updated = instance_inputs(
            SHAPED_INPUTS,
            current.clone(),
            &args(serde_json::json!({"mult": 4.0})),
        )
        .unwrap();
        assert_eq!(updated["len"], InputValue::Int(50));
        assert_eq!(updated["mult"], InputValue::Float(4.0));
        assert!(matches!(
            instance_inputs("plot(", current, &args(serde_json::json!({"len": 3}))),
            Err(McpError::CompilationFailed(_))
        ));
    }

    const WITH_INPUTS: &str = "//@version=6\nindicator(\"Inputs\", overlay=true)\n\
len = input.int(20, \"Length\", minval=1, maxval=500, step=1, group=\"Main\", tooltip=\"Bars\")\n\
mult = input.float(2.5, \"Multiplier\")\n\
show = input.bool(true, \"Show\")\n\
col = input.color(color.red, \"Colour\")\n\
plot(ta.sma(close, len) * mult)\n";

    fn create(store: &mut ScriptStore, name: &str, kind: ScriptType, source: &str) -> LocalScript {
        store.create(name, kind, Some(source), true).unwrap()
    }

    #[test]
    fn lists_scripts_with_type_draft_and_compile_state_and_no_paths() {
        let (mut store, root) = temp_store("list");
        let plain = create(
            &mut store,
            "Plain",
            ScriptType::Indicator,
            "//@version=6\nindicator(\"P\")\nplot(close)\n",
        );
        let strat = create(
            &mut store,
            "Strat",
            ScriptType::Strategy,
            "//@version=6\nstrategy(\"S\")\n",
        );
        store
            .save_draft(&ScriptDraft {
                script_id: strat.id.clone(),
                source: "//@version=6\nstrategy(\"S2\")\n".into(),
                modified_at_ms: now_ms(),
                based_on_revision_id: None,
            })
            .unwrap();
        // A draft equal to the saved text is not a draft.
        store
            .save_draft(&ScriptDraft {
                script_id: plain.id.clone(),
                source: plain.source.clone(),
                modified_at_ms: now_ms(),
                based_on_revision_id: None,
            })
            .unwrap();

        let listed = list_scripts(&mut store).unwrap();
        assert_eq!(listed.total, 2);
        let by_name = |name: &str| listed.scripts.iter().find(|s| s.name == name).unwrap();
        assert_eq!(by_name("Strat").script_type, "strategy");
        assert!(by_name("Strat").has_draft);
        assert!(!by_name("Plain").has_draft);
        assert_eq!(by_name("Plain").source_hash, hash_source(&plain.source));
        let json = serde_json::to_string(&listed).unwrap();
        assert!(
            !json.contains(root.to_str().unwrap()),
            "paths must not leak: {json}"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn saved_and_draft_are_never_substituted_for_each_other() {
        let (mut store, root) = temp_store("versions");
        let script = create(
            &mut store,
            "V",
            ScriptType::Indicator,
            "//@version=6\nindicator(\"V\")\nplot(close)\n",
        );

        let saved = get_script(&mut store, &script.id, Version::Saved).unwrap();
        assert_eq!(saved.version, "saved");
        assert_eq!(saved.source, script.source);
        assert_eq!(saved.source_hash, hash_source(&script.source));
        assert!(!saved.has_draft);
        // No draft yet: asking for it is an error, not the saved text.
        assert!(matches!(
            get_script(&mut store, &script.id, Version::Draft),
            Err(McpError::NotFound(_))
        ));

        let draft_text = "//@version=6\nindicator(\"V\")\nplot(open)\n";
        store
            .save_draft(&ScriptDraft {
                script_id: script.id.clone(),
                source: draft_text.into(),
                modified_at_ms: now_ms(),
                based_on_revision_id: None,
            })
            .unwrap();
        let draft = get_script(&mut store, &script.id, Version::Draft).unwrap();
        assert_eq!(
            (draft.version.as_str(), draft.source.as_str()),
            ("draft", draft_text)
        );
        assert_eq!(draft.source_hash, hash_source(draft_text));
        assert!(draft.has_draft && draft.draft_modified_at.is_some());
        assert_eq!(
            get_script(&mut store, &script.id, Version::Saved)
                .unwrap()
                .source,
            script.source
        );
        assert!(matches!(
            get_script(&mut store, "nope", Version::Saved),
            Err(McpError::NotFound(_))
        ));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn compiling_reports_real_diagnostics_with_positions() {
        let (mut store, root) = temp_store("compile");
        let ok = create(
            &mut store,
            "Ok",
            ScriptType::Indicator,
            "//@version=6\nindicator(\"Ok\")\nplot(close)\n",
        );
        let result =
            compile_script(&mut store, Target::Script(ok.id.clone(), Version::Saved)).unwrap();
        assert!(result.success);
        assert_eq!(result.script_type.as_deref(), Some("indicator"));
        assert_eq!(result.title.as_deref(), Some("Ok"));
        assert_eq!(result.pine_version, Some(6));
        assert_eq!((result.error_count, result.version.as_str()), (0, "saved"));

        let bad = compile_script(
            &mut store,
            Target::Inline("//@version=6\nindicator(\"B\")\nplot(\n".into()),
        )
        .unwrap();
        assert!(!bad.success);
        assert_eq!(bad.version, "inline");
        assert!(bad.script_id.is_none());
        assert!(bad.error_count > 0);
        let first = &bad.diagnostics[0];
        assert_eq!(first.severity, "error");
        assert!(first.code.starts_with("PINE"), "{}", first.code);
        assert!(first.line >= 1 && first.column >= 1);
        assert!(!first.message.is_empty());

        // The wrong version header is its own coded error.
        let old = compile_script(
            &mut store,
            Target::Inline("//@version=4\nindicator(\"X\")\n".into()),
        )
        .unwrap();
        assert!(!old.success);
        assert_eq!(old.diagnostics[0].code, "PINE0002");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn compiling_changes_nothing_on_disk() {
        let (mut store, root) = temp_store("pure");
        let script = create(
            &mut store,
            "Pure",
            ScriptType::Indicator,
            "//@version=6\nindicator(\"P\")\nplot(close)\n",
        );
        let before =
            std::fs::read_to_string(store.file_path(&script.id).unwrap().unwrap()).unwrap();
        let record_before = store.script(&script.id).unwrap().unwrap().compile_record;
        compile_script(
            &mut store,
            Target::Script(script.id.clone(), Version::Saved),
        )
        .unwrap();
        compile_script(&mut store, Target::Inline("plot(close)".into())).unwrap();
        let after = std::fs::read_to_string(store.file_path(&script.id).unwrap().unwrap()).unwrap();
        assert_eq!(before, after);
        assert_eq!(
            store.script(&script.id).unwrap().unwrap().compile_record,
            record_before
        );
        assert!(store.draft(&script.id).unwrap().is_none());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn oversized_source_is_refused() {
        let (mut store, root) = temp_store("big");
        let huge = "x".repeat(MAX_SOURCE_CHARS + 1);
        assert!(matches!(
            compile_script(&mut store, Target::Inline(huge)),
            Err(McpError::ResourceLimit(_))
        ));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn inputs_come_from_the_compilers_own_schema() {
        let (mut store, root) = temp_store("inputs");
        let script = create(&mut store, "Inputs", ScriptType::Indicator, WITH_INPUTS);
        let result = script_inputs(&mut store, &script.id, Version::Saved).unwrap();
        assert!(result.compiled);
        let ids: Vec<_> = result.inputs.iter().map(|i| i.id.as_str()).collect();
        assert_eq!(ids, ["len", "mult", "show", "col"]);

        let len = &result.inputs[0];
        assert_eq!(len.value_type, "integer");
        assert_eq!(len.default, serde_json::json!(20));
        assert_eq!(
            (len.min, len.max, len.step),
            (Some(1.0), Some(500.0), Some(1.0))
        );
        assert_eq!(len.title.as_deref(), Some("Length"));
        assert_eq!(len.group.as_deref(), Some("Main"));
        assert_eq!(len.tooltip.as_deref(), Some("Bars"));
        assert_eq!(result.inputs[1].value_type, "float");
        assert_eq!(result.inputs[1].default, serde_json::json!(2.5));
        assert_eq!(result.inputs[2].default, serde_json::json!(true));
        assert_eq!(result.inputs[3].value_type, "color");
        assert!(
            result.inputs[3]
                .default
                .as_str()
                .is_some_and(|c| c.starts_with('#') && c.len() == 9)
        );

        // A draft's inputs are the draft's, not the saved script's.
        store
            .save_draft(&ScriptDraft {
                script_id: script.id.clone(),
                source: "//@version=6\nindicator(\"D\")\nn = input.int(5, \"N\")\nplot(close)\n"
                    .into(),
                modified_at_ms: now_ms(),
                based_on_revision_id: None,
            })
            .unwrap();
        let draft = script_inputs(&mut store, &script.id, Version::Draft).unwrap();
        assert_eq!(draft.inputs.len(), 1);
        assert_eq!(draft.inputs[0].id, "n");
        let _ = std::fs::remove_dir_all(root);
    }

    // MARK: Editing

    const V1: &str = "//@version=6\nindicator(\"E\")\nplot(close)\n";
    const V2: &str = "//@version=6\nindicator(\"E\")\nplot(open)\n";
    const V3: &str = "//@version=6\nindicator(\"E\")\nplot(high)\n";
    const BROKEN: &str = "//@version=6\nindicator(\"E\")\nplot(\n";

    fn edit_store(tag: &str) -> (ScriptStore, LocalScript, PathBuf) {
        let (mut store, root) = temp_store(tag);
        let script = create(&mut store, "E", ScriptType::Indicator, V1);
        (store, script, root)
    }

    #[test]
    fn a_draft_edit_changes_only_the_draft() {
        let (mut store, script, root) = edit_store("draft");
        let revision = hash_source(V1);
        let result = update_draft(&mut store, &script.id, &revision, V2, None).unwrap();
        assert_eq!(
            (result.action.as_str(), result.status.as_str()),
            ("update_draft", "updated")
        );
        assert_eq!(result.previous_revision, revision);
        assert_eq!(result.revision, hash_source(V2));
        assert!(result.has_draft && result.saved_revision_id.is_none());

        // Saved text, its history and its compile state are exactly as they were.
        let after = store.script(&script.id).unwrap().unwrap();
        assert_eq!(after.source, V1);
        assert_eq!(after.latest_revision_id, script.latest_revision_id);
        assert_eq!(
            get_script(&mut store, &script.id, Version::Draft)
                .unwrap()
                .source,
            V2
        );
        assert_eq!(
            get_script(&mut store, &script.id, Version::Saved)
                .unwrap()
                .source,
            V1
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_stale_or_malformed_revision_is_a_conflict_and_nothing_is_written() {
        let (mut store, script, root) = edit_store("conflict");
        update_draft(&mut store, &script.id, &hash_source(V1), V2, None).unwrap();

        // The client still holds the saved revision, but a draft now exists: its edit loses.
        let stale = update_draft(&mut store, &script.id, &hash_source(V1), V3, None);
        let Err(McpError::Conflict(message)) = stale else {
            panic!("expected a conflict");
        };
        assert!(
            message.contains(&hash_source(V2)),
            "tells the caller the current revision"
        );
        assert_eq!(
            get_script(&mut store, &script.id, Version::Draft)
                .unwrap()
                .source,
            V2
        );

        for bad in ["", "abc", &"G".repeat(64), &hash_source(V2).to_uppercase()] {
            assert!(
                matches!(
                    update_draft(&mut store, &script.id, bad, V3, None),
                    Err(McpError::InvalidArgument(_))
                ),
                "{bad:?}"
            );
        }
        assert_eq!(
            get_script(&mut store, &script.id, Version::Draft)
                .unwrap()
                .source,
            V2
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn two_clients_editing_the_same_revision_cannot_both_win() {
        let (mut store, script, root) = edit_store("race");
        let revision = hash_source(V1);
        assert!(update_draft(&mut store, &script.id, &revision, V2, None).is_ok());
        assert!(matches!(
            update_draft(&mut store, &script.id, &revision, V3, None),
            Err(McpError::Conflict(_))
        ));
        assert_eq!(
            get_script(&mut store, &script.id, Version::Draft)
                .unwrap()
                .source,
            V2
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn an_open_editors_text_outranks_what_is_on_disk() {
        let (mut store, script, root) = edit_store("live");
        // The user has typed V3 into the open editor; nothing has reached disk yet.
        let live = Some(V3);
        // A client that read the saved text is told it is behind, and nothing is written.
        assert!(matches!(
            update_draft(&mut store, &script.id, &hash_source(V1), V2, live),
            Err(McpError::Conflict(_))
        ));
        assert!(store.draft(&script.id).unwrap().is_none());
        // One that read the editor's text may proceed.
        let result = update_draft(&mut store, &script.id, &hash_source(V3), V2, live).unwrap();
        assert_eq!(result.previous_revision, hash_source(V3));
        assert_eq!(store.draft(&script.id).unwrap().unwrap().source, V2);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn repeating_an_edit_is_a_no_op_and_sources_are_checked() {
        let (mut store, script, root) = edit_store("noop");
        let same = update_draft(&mut store, &script.id, &hash_source(V1), V1, None).unwrap();
        assert_eq!(same.status, "unchanged");
        assert!(!same.has_draft);
        assert!(
            store.draft(&script.id).unwrap().is_none(),
            "no file for a no-op"
        );

        assert!(matches!(
            update_draft(&mut store, &script.id, &hash_source(V1), "a\u{0}b", None),
            Err(McpError::InvalidArgument(_))
        ));
        assert!(matches!(
            update_draft(
                &mut store,
                &script.id,
                &hash_source(V1),
                &"x".repeat(MAX_EDIT_CHARS + 1),
                None
            ),
            Err(McpError::ResourceLimit(_))
        ));
        assert!(matches!(
            update_draft(&mut store, "missing", &hash_source(V1), V2, None),
            Err(McpError::NotFound(_))
        ));
        // Tabs, newlines and carriage returns are ordinary source text.
        assert!(
            update_draft(
                &mut store,
                &script.id,
                &hash_source(V1),
                "a\tb\r\nc\n",
                None
            )
            .is_ok()
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn saving_promotes_the_reviewed_draft_and_records_a_revision() {
        let (mut store, script, root) = edit_store("save");
        update_draft(&mut store, &script.id, &hash_source(V1), V2, None).unwrap();

        // A wrong revision saves nothing.
        assert!(matches!(
            save_script(&mut store, &script.id, &hash_source(V1), None),
            Err(McpError::Conflict(_))
        ));
        assert_eq!(store.script(&script.id).unwrap().unwrap().source, V1);

        let saved = save_script(&mut store, &script.id, &hash_source(V2), None).unwrap();
        assert_eq!(
            (saved.action.as_str(), saved.status.as_str()),
            ("save", "saved")
        );
        assert_eq!(saved.revision, hash_source(V2));
        assert!(saved.saved_revision_id.is_some() && !saved.has_draft);
        let after = store.script(&script.id).unwrap().unwrap();
        assert_eq!(after.source, V2);
        assert_ne!(after.latest_revision_id, script.latest_revision_id);
        assert!(
            store.draft(&script.id).unwrap().is_none(),
            "the draft is consumed"
        );
        // A script has no history until its first save; this save is that first revision.
        assert_eq!(store.revisions(&script.id).unwrap().len(), 1);
        assert_eq!(store.revisions(&script.id).unwrap()[0].source, V2);

        // Nothing left to save.
        let again = save_script(&mut store, &script.id, &hash_source(V2), None).unwrap();
        assert_eq!(again.status, "unchanged");
        assert_eq!(store.revisions(&script.id).unwrap().len(), 1);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_draft_with_errors_is_never_saved_over_a_working_script() {
        let (mut store, script, root) = edit_store("broken");
        update_draft(&mut store, &script.id, &hash_source(V1), BROKEN, None).unwrap();
        let result = save_script(&mut store, &script.id, &hash_source(BROKEN), None);
        let Err(McpError::CompilationFailed(message)) = result else {
            panic!("expected COMPILATION_FAILED");
        };
        assert!(message.contains("PINE"), "{message}");
        let after = store.script(&script.id).unwrap().unwrap();
        assert_eq!(after.source, V1, "the working script is untouched");
        assert_eq!(
            store.draft(&script.id).unwrap().unwrap().source,
            BROKEN,
            "the draft is kept for fixing"
        );
        assert!(
            store.revisions(&script.id).unwrap().is_empty(),
            "no revision was recorded"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn saving_what_an_open_editor_shows_works_without_a_draft_file() {
        let (mut store, script, root) = edit_store("save-live");
        let saved = save_script(&mut store, &script.id, &hash_source(V2), Some(V2)).unwrap();
        assert_eq!(saved.status, "saved");
        assert_eq!(store.script(&script.id).unwrap().unwrap().source, V2);
        let _ = std::fs::remove_dir_all(root);
    }
}
