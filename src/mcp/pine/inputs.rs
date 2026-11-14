//! A script's inputs: the schema the compiler found, and the overrides a client sends, checked
//! against it.

use super::{Version, compile_source, text_of};
use crate::mcp::bridge::McpError;
use crate::mcp::types::{PineInputInfo, PineInputOption, PineInputsResult, parse_color};
use crate::pine::compiler::program::{InputDefinition, InputSchema, InputValue};
use crate::store::scripts::{ScriptStore, hash_source};
use schemars::JsonSchema;
use serde::Deserialize;
use std::collections::{BTreeMap, HashMap};

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
        version: version.wire(),
        source_hash: hash_source(&source),
        compiled: program.is_valid(),
        inputs: program.input_schema.inputs.iter().map(input_info).collect(),
    })
}

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
            parse_color(text)
                .map(|(rgba, _)| u32::from_be_bytes(rgba))
                .ok_or("expects a colour like #RRGGBB or #RRGGBBAA")?,
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
