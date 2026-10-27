//! User-defined methods (`method area(Rect this) => …`): `rect.area()` and `area(rect)` run the
//! overload whose first parameter accepts the receiver's type. Builtin namespaces keep priority only
//! when no overload matches. A function declared several times with different parameter types
//! (a library's overloads) is resolved the same way, from all of its arguments.

use super::ast::{Argument, Expression, Parameter};
use super::runtime_types::{RuntimeContext, RuntimeFunction};
use super::runtime_value::{RefKind, RuntimeValue};
use super::session::{INTERNAL_PREFIX, Session};
use super::token::{Diagnostic, DiagnosticCategory, SourceRange, ValueType};

impl Session {
    /// Runs the user method `name` on `receiver`; None when no overload accepts the receiver.
    pub(crate) fn call_user_method(
        &mut self,
        name: &str,
        receiver: RuntimeValue,
        arguments: &[Argument],
        site: usize,
        range: SourceRange,
        context: &mut RuntimeContext,
    ) -> Result<Option<RuntimeValue>, Diagnostic> {
        let Some(function) = self.method_overload(name, &receiver) else {
            return Ok(None);
        };
        // The receiver is already evaluated, so it travels to the parameter binder as a hidden variable.
        let hidden = format!("{INTERNAL_PREFIX}receiver");
        self.working.variables.insert(hidden.clone(), receiver);
        let mut all = vec![Argument {
            name: None,
            value: Expression::Identifier(hidden.clone(), range),
            name_range: None,
        }];
        all.extend_from_slice(arguments);
        let result = self.invoke(function, name, &all, site, range, context);
        self.working.variables.remove(&hidden);
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
            return Err(Diagnostic::error(
                "PINE4007",
                DiagnosticCategory::Runtime,
                format!("No overload of '{name}' accepts these arguments."),
                range,
            ));
        };
        // The values are already evaluated, so they reach the binder as hidden variables.
        let hidden = |index: usize| format!("{INTERNAL_PREFIX}argument{index}");
        let mut bound = Vec::with_capacity(values.len());
        for (index, (label, value)) in values.into_iter().enumerate() {
            self.working.variables.insert(hidden(index), value);
            bound.push(Argument {
                name: label,
                value: Expression::Identifier(hidden(index), range),
                name_range: None,
            });
        }
        let result = self.invoke(function, name, &bound, site, range, context);
        for index in 0..bound.len() {
            self.working.variables.remove(&hidden(index));
        }
        result
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

    fn method_overload(&self, name: &str, receiver: &RuntimeValue) -> Option<RuntimeFunction> {
        self.methods
            .get(name)?
            .iter()
            .find(|function| {
                function
                    .parameters
                    .first()
                    .is_some_and(|parameter| self.accepts(parameter, receiver))
            })
            .cloned()
    }

    /// Whether a `this` parameter of this declared type takes `value`. An untyped parameter takes
    /// anything, and `na` fits every type.
    fn accepts(&self, parameter: &Parameter, value: &RuntimeValue) -> bool {
        if matches!(value, RuntimeValue::Na) {
            return true;
        }
        if let Some(declared) = &parameter.type_name {
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
        let Some(kind) = parameter.kind else {
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
