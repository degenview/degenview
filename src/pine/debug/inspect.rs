//! Reading a stopped script's values.
//!
//! Nothing here keeps a pointer into the interpreter. A value the person can expand is named by a
//! [`ValueRef`]: the stop it belongs to, a frame, and a path of steps from a variable (element 3 of
//! the array in `levels`, field `entry` of the object at key 2, …). Expanding re-walks the path, so
//! a reference whose script has since moved on fails as stale instead of reading something else,
//! and a large array is only ever read a page at a time.

use crate::pine::containers::Object;
use crate::pine::fast_hash::FastMap;
use crate::pine::number_format;
use crate::pine::runtime_types::RuntimeContext;
use crate::pine::runtime_value::{RefKind, RuntimeValue};
use crate::pine::session::Session;

use super::evaluate;
use super::expression::DebugExpression;
use super::frames::Frame;
use super::source_map::{DeclarationKind, SourceMap};

/// Longest value text shown in a row; the rest is in [`Variable::full`].
const SUMMARY_LIMIT: usize = 120;
/// Longest value text copied.
const FULL_LIMIT: usize = 4_096;
/// Most rows one expansion returns.
pub const PAGE_LIMIT: usize = 200;
/// Deepest path an expansion will follow.
const DEPTH_LIMIT: usize = 16;

const BUILTINS: &[&str] = &[
    "open",
    "high",
    "low",
    "close",
    "volume",
    "hl2",
    "hlc3",
    "ohlc4",
    "time",
    "bar_index",
];

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
enum Step {
    /// A variable or builtin by name, as the frame sees it.
    Variable(String),
    /// A watch expression, evaluated again when expanded.
    Expression(String),
    /// The values a series took on earlier bars.
    Series,
    /// `[n]` bars back, within a series.
    Offset(usize),
    /// An element of an array, a tuple, or a matrix row.
    Element(usize),
    /// A map entry's value, by position in insertion order.
    Entry(usize),
    Field(String),
    /// A matrix row.
    Row(usize),
}

/// A handle to something that can be expanded. Cheap to hold and to compare; useless once the
/// script resumes.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ValueRef {
    generation: u64,
    frame: usize,
    path: Vec<Step>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Group {
    Local,
    Global,
    Persistent,
    BuiltIn,
}

impl Group {
    pub fn title(self) -> &'static str {
        match self {
            Self::Local => "Local",
            Self::Global => "Global",
            Self::Persistent => "Persistent",
            Self::BuiltIn => "Built-in",
        }
    }
}

/// One row of the variables panel, a watch result, or an expanded value.
#[derive(Clone, Debug, PartialEq)]
pub struct Variable {
    pub name: String,
    /// Shortened for a row.
    pub value: String,
    /// Everything, up to a limit, for copying and tooltips.
    pub full: String,
    pub type_name: String,
    pub is_na: bool,
    /// False for a name in scope whose declaration has not run yet on this bar.
    pub assigned: bool,
    /// Different from the last stop that was inspected.
    pub changed: bool,
    pub declaration: Option<DeclarationKind>,
    /// Press to see the parts or the earlier bars.
    pub expand: Option<ValueRef>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct VariableGroup {
    pub group: Group,
    pub variables: Vec<Variable>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Page {
    pub total: usize,
    pub start: usize,
    pub items: Vec<Variable>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InspectError {
    /// The script has resumed since this reference was made.
    Stale,
    /// The value is no longer there (the variable left scope, the array shrank).
    Gone,
    Failed(String),
}

impl std::fmt::Display for InspectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Stale => f.write_str("The script has moved on."),
            Self::Gone => f.write_str("The value is no longer available."),
            Self::Failed(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for InspectError {}

/// What inspecting needs of the stopped script.
pub(crate) struct Env<'a> {
    pub session: &'a mut Session,
    pub frames: &'a [Frame],
    pub context: &'a RuntimeContext,
    pub map: &'a SourceMap,
    pub generation: u64,
}

impl Env<'_> {
    /// The innermost `request.security()` frame: below it the chart's variables, above it the
    /// expression's own.
    fn boundary(&self) -> Option<usize> {
        self.frames.iter().rposition(Frame::is_security)
    }

    fn frame_value(&self, frame: usize, name: &str) -> Option<RuntimeValue> {
        // A callee took the names it declares off the table and parked the caller's values with its
        // own frame; the nearest callee that did so holds what this frame had.
        let callees_end = match self.boundary() {
            Some(boundary) if frame == boundary => return None,
            Some(boundary) if frame < boundary => {
                // The expression runs in a state of its own and the interpreter set the chart's
                // variables aside; only the nearest enclosing context can be read back. One further
                // out is on the interpreter's stack, out of reach.
                if self.frames[frame + 1..boundary]
                    .iter()
                    .any(Frame::is_security)
                {
                    return None;
                }
                for callee in &self.frames[frame + 1..boundary] {
                    if let Some((_, value)) = callee.shadowed.iter().find(|(n, _)| n == name) {
                        return value.clone();
                    }
                }
                return self
                    .session
                    .security_globals
                    .as_ref()
                    .and_then(|globals| globals.get(name).cloned());
            }
            _ => self.frames.len(),
        };
        for callee in &self.frames[(frame + 1).min(callees_end)..callees_end] {
            if let Some((_, value)) = callee.shadowed.iter().find(|(n, _)| n == name) {
                return value.clone();
            }
        }
        self.session.working.variables.get(name).cloned()
    }

    fn builtin(&self, frame: usize, name: &str) -> Option<RuntimeValue> {
        // Outside a running request the bar in hand is the requested one, not the chart's.
        if self.boundary().is_some_and(|boundary| frame <= boundary) {
            return None;
        }
        let bar = &self.context.bar;
        Some(match name {
            "open" => RuntimeValue::Float(bar.open),
            "high" => RuntimeValue::Float(bar.high),
            "low" => RuntimeValue::Float(bar.low),
            "close" => RuntimeValue::Float(bar.close),
            "volume" => RuntimeValue::Float(bar.volume),
            "hl2" => RuntimeValue::Float((bar.high + bar.low) / 2.0),
            "hlc3" => RuntimeValue::Float((bar.high + bar.low + bar.close) / 3.0),
            "ohlc4" => RuntimeValue::Float((bar.open + bar.high + bar.low + bar.close) / 4.0),
            "time" => RuntimeValue::Int(bar.open_time),
            "bar_index" => RuntimeValue::Int(self.session.working.bar_index),
            _ => return None,
        })
    }

    fn has_history(&self, name: &str) -> bool {
        self.session
            .working
            .histories
            .get(name)
            .is_some_and(|history| !history.is_empty())
    }

    fn mintick(&self) -> f64 {
        self.session.mintick
    }
}

// MARK: Listing

/// The variables visible in `frame`, in the groups of the variables panel. `previous` maps a name
/// to the text it had at the last inspected stop.
pub(crate) fn variables(
    env: &mut Env<'_>,
    frame: usize,
    previous: &FastMap<String, String>,
) -> Vec<VariableGroup> {
    // A request boundary has no names of its own: the chart's are in the frame below it, the
    // expression's in the frames above.
    if env.frames.get(frame).is_none_or(Frame::is_security) {
        return Vec::new();
    }
    let Some(location) = env.frames.get(frame).and_then(|frame| frame.location) else {
        return Vec::new();
    };
    let entries = env.map.visible(location.offset);
    let mut groups: [Vec<Variable>; 4] = Default::default();
    // Declaration order, oldest first, reads like the source.
    for entry in entries.into_iter().rev() {
        let value = env.frame_value(frame, &entry.name);
        let assigned = match entry.kind {
            DeclarationKind::Parameter | DeclarationKind::Loop => true,
            DeclarationKind::Var | DeclarationKind::Varip => value.is_some(),
            DeclarationKind::Ordinary | DeclarationKind::Tuple => {
                value.is_some()
                    && env.map.index_of(entry.offset).is_some_and(|index| {
                        // A global was declared by the script's own frame, wherever the
                        // function that is looking at it was called from.
                        env.frames[if entry.global { 0 } else { frame }].has_executed(index)
                    })
            }
        };
        let group = if entry.kind.is_persistent() {
            Group::Persistent
        } else if entry.global {
            Group::Global
        } else {
            Group::Local
        };
        let mut variable = match (&value, assigned) {
            (Some(value), true) => describe(env, frame, &entry.name, value),
            _ => unassigned(&entry.name),
        };
        variable.declaration = Some(entry.kind);
        variable.changed = assigned
            && previous
                .get(&entry.name)
                .is_some_and(|p| *p != variable.full);
        groups[group_index(group)].push(variable);
    }
    let shadowed: std::collections::HashSet<&str> = groups
        .iter()
        .flatten()
        .map(|variable| variable.name.as_str())
        .collect();
    let builtins: Vec<Variable> = BUILTINS
        .iter()
        .filter(|name| !shadowed.contains(**name))
        .filter_map(|name| env.builtin(frame, name).map(|value| (*name, value)))
        .map(|(name, value)| {
            let mut variable = describe(env, frame, name, &value);
            variable.changed = previous.get(name).is_some_and(|p| *p != variable.full);
            variable
        })
        .collect();
    groups[group_index(Group::BuiltIn)] = builtins;
    let [local, global, persistent, built_in] = groups;
    [
        (Group::Local, local),
        (Group::Global, global),
        (Group::Persistent, persistent),
        (Group::BuiltIn, built_in),
    ]
    .into_iter()
    .filter(|(_, variables)| !variables.is_empty())
    .map(|(group, variables)| VariableGroup { group, variables })
    .collect()
}

fn group_index(group: Group) -> usize {
    match group {
        Group::Local => 0,
        Group::Global => 1,
        Group::Persistent => 2,
        Group::BuiltIn => 3,
    }
}

fn unassigned(name: &str) -> Variable {
    Variable {
        name: name.to_owned(),
        value: "not yet assigned on this bar".to_owned(),
        full: "not yet assigned on this bar".to_owned(),
        type_name: String::new(),
        is_na: false,
        assigned: false,
        changed: false,
        declaration: None,
        expand: None,
    }
}

/// A row for `value`, which a variable of this name holds in `frame`.
fn describe(env: &Env<'_>, frame: usize, name: &str, value: &RuntimeValue) -> Variable {
    let (full, type_name) = summarize(env, value);
    let structural = is_compound(value);
    let series = !structural && env.has_history(name);
    let expand = (structural || series).then(|| ValueRef {
        generation: env.generation,
        frame,
        path: if structural {
            vec![Step::Variable(name.to_owned())]
        } else {
            vec![Step::Variable(name.to_owned()), Step::Series]
        },
    });
    row(name.to_owned(), full, type_name, value, expand)
}

fn row(
    name: String,
    full: String,
    type_name: String,
    value: &RuntimeValue,
    expand: Option<ValueRef>,
) -> Variable {
    Variable {
        name,
        value: shortened(&full),
        full: truncate(&full, FULL_LIMIT),
        type_name,
        is_na: matches!(value, RuntimeValue::Na),
        assigned: true,
        changed: false,
        declaration: None,
        expand,
    }
}

fn truncate(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_owned();
    }
    let mut cut: String = text.chars().take(limit).collect();
    cut.push('…');
    cut
}

fn shortened(text: &str) -> String {
    let single_line = text.replace('\n', "⏎");
    truncate(&single_line, SUMMARY_LIMIT)
}

fn is_compound(value: &RuntimeValue) -> bool {
    matches!(
        value,
        RuntimeValue::Tuple(_)
            | RuntimeValue::Ref(
                RefKind::Array | RefKind::Map | RefKind::Matrix | RefKind::Object,
                _
            )
    )
}

// MARK: Formatting

/// The text and the Pine type name of a value. Never recurses: a container is summarised by its
/// size and the type of its first element.
fn summarize(env: &Env<'_>, value: &RuntimeValue) -> (String, String) {
    let state = &env.session.working;
    match value {
        RuntimeValue::Na => ("na".to_owned(), "na".to_owned()),
        RuntimeValue::Void => ("void".to_owned(), "void".to_owned()),
        RuntimeValue::Int(_) => (
            number_format::format(value, None, env.mintick()),
            "int".to_owned(),
        ),
        RuntimeValue::Float(x) if !x.is_finite() => ("NaN".to_owned(), "float".to_owned()),
        RuntimeValue::Float(_) => (
            number_format::format(value, None, env.mintick()),
            "float".to_owned(),
        ),
        RuntimeValue::Bool(flag) => (flag.to_string(), "bool".to_owned()),
        RuntimeValue::String(text) => (format!("\"{text}\""), "string".to_owned()),
        RuntimeValue::Color(rgba) => (color_text(*rgba), "color".to_owned()),
        RuntimeValue::Tuple(items) => (
            format!("[{}]", join_scalars(env, items, 4)),
            format!("tuple ({})", items.len()),
        ),
        RuntimeValue::Ref(kind, id) => match kind {
            RefKind::Array => match state.arrays.get(id) {
                Some(items) => (
                    format!("size {}", items.len()),
                    generic("array", elements_type(items)),
                ),
                None => ("(removed)".to_owned(), "array".to_owned()),
            },
            RefKind::Map => match state.maps.get(id) {
                Some(map) => {
                    let pairs = map.pairs();
                    let types = pairs.first().map_or("?, ?".to_owned(), |(key, value)| {
                        format!("{}, {}", element_type(key), element_type(value))
                    });
                    (format!("size {}", map.count()), format!("map<{types}>"))
                }
                None => ("(removed)".to_owned(), "map".to_owned()),
            },
            RefKind::Matrix => match state.matrices.get(id) {
                Some(matrix) => (
                    format!("{} × {}", matrix.rows(), matrix.columns()),
                    generic("matrix", elements_type(matrix.all_elements())),
                ),
                None => ("(removed)".to_owned(), "matrix".to_owned()),
            },
            RefKind::Object => match state.instances.get(id) {
                Some(object) => (object_text(env, object), object.type_name.clone()),
                None => ("(removed)".to_owned(), "object".to_owned()),
            },
            other => (format!("#{id}"), format!("{other:?}").to_lowercase()),
        },
    }
}

/// `array<float>`, or plain `array` when nothing in it says what it holds.
fn generic(name: &str, element: Option<String>) -> String {
    match element {
        Some(element) => format!("{name}<{element}>"),
        None => name.to_owned(),
    }
}

/// The type of a collection's elements, read off the first that has one: `na` says nothing.
fn elements_type<'a>(items: impl IntoIterator<Item = &'a RuntimeValue>) -> Option<String> {
    items
        .into_iter()
        .take(16)
        .find(|item| !matches!(item, RuntimeValue::Na))
        .map(element_type)
}

fn element_type(value: &RuntimeValue) -> String {
    match value {
        RuntimeValue::Int(_) => "int",
        RuntimeValue::Float(_) => "float",
        RuntimeValue::Bool(_) => "bool",
        RuntimeValue::String(_) => "string",
        RuntimeValue::Color(_) => "color",
        RuntimeValue::Na => "na",
        RuntimeValue::Tuple(_) => "tuple",
        RuntimeValue::Void => "void",
        RuntimeValue::Ref(kind, _) => {
            return format!("{kind:?}").to_lowercase();
        }
    }
    .to_owned()
}

fn color_text(rgba: u32) -> String {
    if rgba & 0xFF == 0xFF {
        format!("#{:06X}", rgba >> 8)
    } else {
        format!("#{rgba:08X}")
    }
}

fn scalar_text(env: &Env<'_>, value: &RuntimeValue) -> String {
    match value {
        RuntimeValue::Ref(..) | RuntimeValue::Tuple(_) => summarize(env, value).0,
        other => summarize(env, other).0,
    }
}

fn join_scalars(env: &Env<'_>, items: &[RuntimeValue], limit: usize) -> String {
    let mut parts: Vec<String> = items
        .iter()
        .take(limit)
        .map(|item| scalar_text(env, item))
        .collect();
    if items.len() > limit {
        parts.push("…".to_owned());
    }
    parts.join(", ")
}

fn object_text(env: &Env<'_>, object: &Object) -> String {
    let mut names: Vec<&String> = object.fields.keys().collect();
    names.sort();
    let mut parts: Vec<String> = names
        .iter()
        .take(3)
        .map(|name| format!("{name}: {}", scalar_text(env, &object.fields[*name])))
        .collect();
    if names.len() > 3 {
        parts.push("…".to_owned());
    }
    format!("{{ {} }}", parts.join(", "))
}

// MARK: Expanding

/// A value reached by following a path.
enum Node {
    Value(RuntimeValue),
    /// The earlier bars of the series called this.
    Series(String),
    MatrixRow {
        matrix: usize,
        row: usize,
    },
}

fn resolve(env: &mut Env<'_>, reference: &ValueRef) -> Result<Node, InspectError> {
    if reference.generation != env.generation {
        return Err(InspectError::Stale);
    }
    if reference.frame >= env.frames.len() || reference.path.len() > DEPTH_LIMIT {
        return Err(InspectError::Gone);
    }
    let mut steps = reference.path.iter();
    let mut node = match steps.next() {
        Some(Step::Variable(name)) => Node::Value(
            env.frame_value(reference.frame, name)
                .or_else(|| env.builtin(reference.frame, name))
                .ok_or(InspectError::Gone)?,
        ),
        Some(Step::Expression(text)) => {
            let expression = super::expression::parse(text)
                .map_err(|error| InspectError::Failed(error.message))?;
            Node::Value(
                evaluate::evaluate(
                    env.session,
                    env.frames,
                    reference.frame,
                    env.context,
                    &expression,
                )
                .map_err(InspectError::Failed)?,
            )
        }
        _ => return Err(InspectError::Gone),
    };
    let state_has = |env: &Env<'_>, kind: RefKind, id: usize| -> bool {
        let state = &env.session.working;
        match kind {
            RefKind::Array => state.arrays.contains_key(&id),
            RefKind::Map => state.maps.contains_key(&id),
            RefKind::Matrix => state.matrices.contains_key(&id),
            RefKind::Object => state.instances.contains_key(&id),
            _ => false,
        }
    };
    for step in steps {
        node = match (node, step) {
            (Node::Value(value), Step::Series) => match &reference.path[0] {
                Step::Variable(name) if !is_compound(&value) => Node::Series(name.clone()),
                _ => return Err(InspectError::Gone),
            },
            (Node::Series(name), Step::Offset(back)) => Node::Value(
                series_value(env, reference.frame, &name, *back).ok_or(InspectError::Gone)?,
            ),
            (Node::Value(RuntimeValue::Tuple(items)), Step::Element(index)) => {
                Node::Value(items.get(*index).cloned().ok_or(InspectError::Gone)?)
            }
            (Node::Value(RuntimeValue::Ref(RefKind::Array, id)), Step::Element(index)) => {
                Node::Value(
                    env.session
                        .working
                        .arrays
                        .get(&id)
                        .and_then(|items| items.get(*index))
                        .cloned()
                        .ok_or(InspectError::Gone)?,
                )
            }
            (Node::Value(RuntimeValue::Ref(RefKind::Map, id)), Step::Entry(index)) => Node::Value(
                env.session
                    .working
                    .maps
                    .get(&id)
                    .and_then(|map| map.pairs().into_iter().nth(*index))
                    .map(|(_, value)| value)
                    .ok_or(InspectError::Gone)?,
            ),
            (Node::Value(RuntimeValue::Ref(RefKind::Object, id)), Step::Field(name)) => {
                Node::Value(
                    env.session
                        .working
                        .instances
                        .get(&id)
                        .and_then(|object| object.fields.get(name))
                        .cloned()
                        .ok_or(InspectError::Gone)?,
                )
            }
            (Node::Value(RuntimeValue::Ref(RefKind::Matrix, id)), Step::Row(row)) => {
                if !state_has(env, RefKind::Matrix, id)
                    || *row >= env.session.working.matrices[&id].rows()
                {
                    return Err(InspectError::Gone);
                }
                Node::MatrixRow {
                    matrix: id,
                    row: *row,
                }
            }
            (Node::MatrixRow { matrix, row }, Step::Element(column)) => Node::Value(
                env.session
                    .working
                    .matrices
                    .get(&matrix)
                    .filter(|m| row < m.rows() && *column < m.columns())
                    .map(|m| m.get(row, *column).clone())
                    .ok_or(InspectError::Gone)?,
            ),
            _ => return Err(InspectError::Gone),
        };
    }
    Ok(node)
}

fn series_value(env: &Env<'_>, frame: usize, name: &str, back: usize) -> Option<RuntimeValue> {
    if back == 0 {
        return env
            .frame_value(frame, name)
            .or_else(|| env.builtin(frame, name));
    }
    let history = env.session.working.histories.get(name)?;
    history
        .len()
        .checked_sub(back)
        .map(|index| history[index].clone())
}

/// `limit` rows of what `reference` holds, from row `start`.
pub(crate) fn expand(
    env: &mut Env<'_>,
    reference: &ValueRef,
    start: usize,
    limit: usize,
) -> Result<Page, InspectError> {
    let limit = limit.min(PAGE_LIMIT);
    let node = resolve(env, reference)?;
    let child = |step: Step| {
        let mut path = reference.path.clone();
        path.push(step);
        ValueRef {
            generation: reference.generation,
            frame: reference.frame,
            path,
        }
    };
    let mut items = Vec::new();
    let total;
    match node {
        Node::Series(name) => {
            let depth = env
                .session
                .working
                .histories
                .get(&name)
                .map_or(0, |history| history.len());
            total = depth + 1;
            for back in (start..total).take(limit) {
                let Some(value) = series_value(env, reference.frame, &name, back) else {
                    break;
                };
                let (full, type_name) = summarize(env, &value);
                items.push(row(format!("[{back}]"), full, type_name, &value, None));
            }
        }
        Node::MatrixRow { matrix, row: index } => {
            let columns = env
                .session
                .working
                .matrices
                .get(&matrix)
                .map(|m| m.row(index))
                .ok_or(InspectError::Gone)?;
            total = columns.len();
            for (column, value) in columns.iter().enumerate().skip(start).take(limit) {
                items.push(element_row(
                    env,
                    &child(Step::Element(column)),
                    format!("[{column}]"),
                    value,
                ));
            }
        }
        Node::Value(value) => match &value {
            RuntimeValue::Tuple(values) => {
                total = values.len();
                for (index, value) in values.iter().enumerate().skip(start).take(limit) {
                    items.push(element_row(
                        env,
                        &child(Step::Element(index)),
                        format!("[{index}]"),
                        value,
                    ));
                }
            }
            RuntimeValue::Ref(RefKind::Array, id) => {
                let values = env
                    .session
                    .working
                    .arrays
                    .get(id)
                    .ok_or(InspectError::Gone)?;
                total = values.len();
                let page: Vec<(usize, RuntimeValue)> = values
                    .iter()
                    .enumerate()
                    .skip(start)
                    .take(limit)
                    .map(|(index, value)| (index, value.clone()))
                    .collect();
                for (index, value) in &page {
                    items.push(element_row(
                        env,
                        &child(Step::Element(*index)),
                        format!("[{index}]"),
                        value,
                    ));
                }
            }
            RuntimeValue::Ref(RefKind::Map, id) => {
                let pairs = env
                    .session
                    .working
                    .maps
                    .get(id)
                    .ok_or(InspectError::Gone)?
                    .pairs();
                total = pairs.len();
                for (index, (key, value)) in pairs.iter().enumerate().skip(start).take(limit) {
                    let name = summarize(env, key).0;
                    items.push(element_row(env, &child(Step::Entry(index)), name, value));
                }
            }
            RuntimeValue::Ref(RefKind::Matrix, id) => {
                let matrix = env
                    .session
                    .working
                    .matrices
                    .get(id)
                    .ok_or(InspectError::Gone)?;
                total = matrix.rows();
                let columns = matrix.columns();
                for index in (start..total).take(limit) {
                    items.push(Variable {
                        name: format!("[{index}]"),
                        value: format!("{columns} columns"),
                        full: format!("{columns} columns"),
                        type_name: "row".to_owned(),
                        is_na: false,
                        assigned: true,
                        changed: false,
                        declaration: None,
                        expand: Some(child(Step::Row(index))),
                    });
                }
            }
            RuntimeValue::Ref(RefKind::Object, id) => {
                let object = env
                    .session
                    .working
                    .instances
                    .get(id)
                    .ok_or(InspectError::Gone)?;
                let mut fields: Vec<(String, RuntimeValue)> = object
                    .fields
                    .iter()
                    .map(|(name, value)| (name.clone(), value.clone()))
                    .collect();
                fields.sort_by(|a, b| a.0.cmp(&b.0));
                total = fields.len();
                for (name, value) in fields.iter().skip(start).take(limit) {
                    items.push(element_row(
                        env,
                        &child(Step::Field(name.clone())),
                        name.clone(),
                        value,
                    ));
                }
            }
            _ => total = 0,
        },
    }
    Ok(Page {
        total,
        start,
        items,
    })
}

fn element_row(
    env: &Env<'_>,
    reference: &ValueRef,
    name: String,
    value: &RuntimeValue,
) -> Variable {
    let (full, type_name) = summarize(env, value);
    let expand = is_compound(value).then(|| reference.clone());
    row(name, full, type_name, value, expand)
}

// MARK: Watches

/// The outcome of a watch or console expression.
#[derive(Clone, Debug, PartialEq)]
pub enum Watched {
    Value(Variable),
    Error(String),
}

/// Evaluates `expression` in `frame` without changing the script.
pub(crate) fn watch(env: &mut Env<'_>, frame: usize, expression: &DebugExpression) -> Watched {
    match evaluate::evaluate(env.session, env.frames, frame, env.context, expression) {
        Ok(value) => {
            let (full, type_name) = summarize(env, &value);
            let expand = is_compound(&value).then(|| ValueRef {
                generation: env.generation,
                frame,
                path: vec![Step::Expression(expression.source().to_owned())],
            });
            Watched::Value(row(
                expression.source().to_owned(),
                full,
                type_name,
                &value,
                expand,
            ))
        }
        Err(message) => Watched::Error(message),
    }
}

/// The text a value gets in a logpoint message or a condition's report.
pub(crate) fn plain_text(env: &Env<'_>, value: &RuntimeValue) -> String {
    match value {
        RuntimeValue::String(text) => text.clone(),
        RuntimeValue::Na => "na".to_owned(),
        other => summarize(env, other).0,
    }
}
