//! Applying a saved script to a chart, and changing an applied script's inputs.

use super::inputs::{InputArgs, merge_input_changes, parse_input_args};
use super::{MAX_SOURCE_CHARS, compile_source, count, find_script};
use crate::mcp::bridge::McpError;
use crate::model::pine::script::ScriptType;
use crate::pine::compiler::program::{CompiledProgram, InputValue, ScriptType as DeclaredType};
use crate::pine::syntax::token::DiagnosticSeverity;
use crate::store::scripts::ScriptStore;
use std::collections::HashMap;

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
