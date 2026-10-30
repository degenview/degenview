//! Running the script's functions and methods. A call the type checker resolved carries its
//! `target` and runs that function directly (`call_target`, `method_call_target`). Any other call
//! (`area(rect)`, `rect.area()` on a receiver of unknown type, a library's overloads) finds its
//! function here by name: a method is the overload whose first parameter accepts the receiver and
//! whose other parameters take the arguments written; a function declared several times with
//! different parameter types is resolved from all of its arguments. Builtin namespaces keep
//! priority only when no overload matches.

use super::ast::{Argument, Expression, Parameter};
use super::runtime_types::{RuntimeContext, RuntimeFunction};
use super::runtime_value::{RefKind, RuntimeValue};
use super::session::Session;
use super::token::{Diagnostic, DiagnosticCategory, SourceRange, ValueType};

/// What a call gives a function's parameters.
pub(crate) enum Supplied<'a> {
    /// Argument expressions, evaluated as the parameters are bound. A `receiver` that was already
    /// evaluated stands for the first parameter, so the arguments written start at the second.
    Expressions {
        receiver: Option<RuntimeValue>,
        arguments: &'a [Argument],
    },
    /// Values already evaluated, each with the name it was passed by, if any.
    Values(Vec<(Option<String>, RuntimeValue)>),
}

impl Session {
    /// Runs the script's function `id` for a call the checker resolved. A dotted name is a method
    /// called on a variable or field path, which is the receiver.
    pub(crate) fn call_target(
        &mut self,
        id: usize,
        name: &str,
        arguments: &[Argument],
        site: usize,
        range: SourceRange,
        context: &mut RuntimeContext,
    ) -> Result<RuntimeValue, Diagnostic> {
        let function = self.callables[id].clone();
        let receiver = match name.rsplit_once('.') {
            Some((path, _)) => Some(self.resolve_identifier(path, range, context)?),
            None => None,
        };
        let supplied = Supplied::Expressions {
            receiver,
            arguments,
        };
        self.invoke_with(function, name, supplied, site, range, context)
    }

    /// Runs the script's function `id` on a receiver that is an expression (`f(x).area()`).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn method_call_target(
        &mut self,
        id: usize,
        receiver: &Expression,
        name: &str,
        arguments: &[Argument],
        site: usize,
        range: SourceRange,
        context: &mut RuntimeContext,
    ) -> Result<RuntimeValue, Diagnostic> {
        let function = self.callables[id].clone();
        let receiver = Some(self.eval(receiver, context)?);
        let supplied = Supplied::Expressions {
            receiver,
            arguments,
        };
        self.invoke_with(function, name, supplied, site, range, context)
    }

    /// Runs the user method `name` on `receiver`; None when no overload takes the receiver and the
    /// arguments written.
    pub(crate) fn call_user_method(
        &mut self,
        name: &str,
        receiver: RuntimeValue,
        arguments: &[Argument],
        site: usize,
        range: SourceRange,
        context: &mut RuntimeContext,
    ) -> Result<Option<RuntimeValue>, Diagnostic> {
        let mut candidates: Vec<RuntimeFunction> = self
            .methods
            .get(name)
            .into_iter()
            .flatten()
            .filter(|function| {
                function
                    .parameters
                    .first()
                    .is_some_and(|parameter| self.accepts(parameter, &receiver))
                    && takes(function, arguments)
            })
            .cloned()
            .collect();
        let result = match candidates.len() {
            0 => return Ok(None),
            1 => {
                let supplied = Supplied::Expressions {
                    receiver: Some(receiver),
                    arguments,
                };
                self.invoke_with(candidates.remove(0), name, supplied, site, range, context)
            }
            // Several take the call as written, so the values decide.
            _ => {
                let mut values = vec![(None, receiver)];
                for argument in arguments {
                    values.push((argument.name.clone(), self.eval(&argument.value, context)?));
                }
                let Some(function) = candidates
                    .into_iter()
                    .find(|function| self.fits(function, &values))
                else {
                    return Err(no_overload(name, range));
                };
                self.invoke_with(
                    function,
                    name,
                    Supplied::Values(values),
                    site,
                    range,
                    context,
                )
            }
        };
        result.map(Some)
    }

    /// Runs the overload of `name` whose parameters accept every argument.
    pub(crate) fn call_overloaded(
        &mut self,
        name: &str,
        candidates: &[RuntimeFunction],
        arguments: &[Argument],
        site: usize,
        range: SourceRange,
        context: &mut RuntimeContext,
    ) -> Result<RuntimeValue, Diagnostic> {
        let mut values = Vec::with_capacity(arguments.len());
        for argument in arguments {
            values.push((argument.name.clone(), self.eval(&argument.value, context)?));
        }
        let Some(function) = candidates
            .iter()
            .find(|function| self.fits(function, &values))
            .cloned()
        else {
            return Err(no_overload(name, range));
        };
        self.invoke_with(
            function,
            name,
            Supplied::Values(values),
            site,
            range,
            context,
        )
    }

    /// Whether `function` can be called with these evaluated arguments: every one lands on a
    /// parameter that accepts it, and every parameter without a default gets one.
    fn fits(&self, function: &RuntimeFunction, values: &[(Option<String>, RuntimeValue)]) -> bool {
        let mut supplied = vec![false; function.parameters.len()];
        let mut position = 0;
        for (label, value) in values {
            let index = match label {
                Some(label) => function.parameters.iter().position(|p| &p.name == label),
                None => {
                    position += 1;
                    Some(position - 1)
                }
            };
            let Some(parameter) = index.and_then(|i| function.parameters.get(i)) else {
                return false;
            };
            if !self.accepts(parameter, value) {
                return false;
            }
            supplied[index.unwrap_or_default()] = true;
        }
        function
            .parameters
            .iter()
            .zip(supplied)
            .all(|(parameter, given)| given || parameter.default_value.is_some())
    }

    /// Whether a `this` parameter of this declared type takes `value`. An untyped parameter takes
    /// anything, and `na` fits every type.
    fn accepts(&self, parameter: &Parameter, value: &RuntimeValue) -> bool {
        if matches!(value, RuntimeValue::Na) {
            return true;
        }
        if let Some(declared) = parameter.type_name() {
            return match value {
                RuntimeValue::Ref(RefKind::Object, id) => self
                    .working
                    .instances
                    .get(id)
                    .is_some_and(|object| same_type(declared, &object.type_name)),
                RuntimeValue::Ref(RefKind::Footprint, _) => same_type(declared, "footprint"),
                RuntimeValue::Ref(RefKind::VolumeRow, _) => same_type(declared, "volume_row"),
                // An enum member reads as the text `Enum.member`.
                RuntimeValue::String(text) => text
                    .split_once('.')
                    .is_some_and(|(enum_name, _)| same_type(declared, enum_name)),
                _ => false,
            };
        }
        let Some(kind) = parameter.kind() else {
            return true;
        };
        match (kind, value) {
            (ValueType::Int | ValueType::Time, RuntimeValue::Int(_))
            | (ValueType::Float, RuntimeValue::Int(_) | RuntimeValue::Float(_))
            | (ValueType::Bool, RuntimeValue::Bool(_))
            | (ValueType::String, RuntimeValue::String(_))
            | (ValueType::Color, RuntimeValue::Color(_)) => true,
            (kind, RuntimeValue::Ref(handle, _)) => matches!(
                (kind, handle),
                (ValueType::Line, RefKind::Line)
                    | (ValueType::Label, RefKind::Label)
                    | (ValueType::Box, RefKind::Box)
                    | (ValueType::Table, RefKind::Table)
                    | (ValueType::Linefill, RefKind::Linefill)
                    | (ValueType::Polyline, RefKind::Polyline)
                    | (ValueType::Array, RefKind::Array)
                    | (ValueType::Map, RefKind::Map)
                    | (ValueType::Matrix, RefKind::Matrix)
                    | (ValueType::Object, RefKind::Object)
            ),
            _ => false,
        }
    }
}

/// A library type is registered as `path::Name`; an annotation may name it with or without the path.
fn same_type(declared: &str, actual: &str) -> bool {
    let short = |name: &str| name.rsplit(['.', ':']).next().unwrap_or(name).to_owned();
    declared == actual || short(declared) == short(actual)
}

fn no_overload(name: &str, range: SourceRange) -> Diagnostic {
    Diagnostic::error(
        "PINE4007",
        DiagnosticCategory::Runtime,
        format!("No overload of '{name}' accepts these arguments."),
        range,
    )
}

/// Whether the arguments written (after the receiver) can be passed to `function`: none too many,
/// every name one of its parameters, and every parameter without a default given.
fn takes(function: &RuntimeFunction, arguments: &[Argument]) -> bool {
    let rest = function.parameters.get(1..).unwrap_or_default();
    let positional = arguments.iter().filter(|a| a.name.is_none()).count();
    if positional > rest.len() {
        return false;
    }
    let named_ok = arguments
        .iter()
        .filter_map(|a| a.name.as_deref())
        .all(|label| rest.iter().any(|parameter| parameter.name == label));
    named_ok
        && rest.iter().enumerate().all(|(i, parameter)| {
            i < positional
                || parameter.default_value.is_some()
                || arguments
                    .iter()
                    .any(|a| a.name.as_deref() == Some(parameter.name.as_str()))
        })
}
