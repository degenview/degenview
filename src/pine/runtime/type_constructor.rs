//! `Type.new(…)` — instantiates a script-defined (or library) type into a `containers::Object`,
//! ported from `PineRuntimeSession+Objects.swift`. The session wires [`Session::constructor_call`]
//! into `call`, ahead of the user-function/builtin dispatch.

use std::collections::HashMap;

use super::containers::Object;
use super::session::Session;
use super::types::RuntimeContext;
use super::value::{RefKind, RuntimeValue};
use crate::pine::outputs::drawings;
use crate::pine::syntax::ast::{Argument, Expression};
use crate::pine::syntax::token::{Diagnostic, DiagnosticCategory, SourceRange};

impl Session {
    /// `Type.new(…)`. None when `name` does not name a declared type's constructor.
    pub(crate) fn constructor_call(
        &mut self,
        name: &str,
        arguments: &[Argument],
        range: SourceRange,
        context: &mut RuntimeContext,
    ) -> Result<Option<RuntimeValue>, Diagnostic> {
        let Some(written) = name.strip_suffix(".new") else {
            return Ok(None);
        };
        let Some(type_name) = self.type_key(written, range, context)? else {
            return Ok(None);
        };
        let Some(fields) = self.types.get(&type_name).cloned() else {
            return Ok(None);
        };

        let names: Vec<&str> = fields.iter().map(|field| field.name.as_str()).collect();
        let mut args = Vec::with_capacity(arguments.len());
        for argument in arguments {
            args.push(drawings::Arg {
                name: argument.name.clone(),
                value: self.eval(&argument.value, context)?,
            });
        }
        let supplied = drawings::bind(&args, &names);
        if let Some(unknown) = supplied.keys().find(|name| !names.contains(&name.as_str())) {
            return Err(Diagnostic::error(
                "PINE4019",
                DiagnosticCategory::Runtime,
                format!("Type '{written}' has no field '{unknown}'."),
                range,
            ));
        }

        // A field's default value belongs to the type's definition scope: a library's, or the script's.
        let library = type_name.split_once("::").map(|(path, _)| path.to_owned());
        let mut values = HashMap::new();
        for field in &fields {
            if let Some(value) = supplied.get(&field.name) {
                values.insert(field.name.clone(), value.clone());
            } else if let Some(initial) = &field.default_value {
                values.insert(
                    field.name.clone(),
                    self.eval_in_library(initial, library.as_deref(), context)?,
                );
            } else {
                values.insert(field.name.clone(), RuntimeValue::Na);
            }
        }

        self.working.next_reference += 1;
        let id = self.working.next_reference;
        self.working.instances.insert(
            id,
            Object {
                type_name,
                fields: values,
            },
        );
        Ok(Some(RuntimeValue::Ref(RefKind::Object, id)))
    }

    /// Evaluates `expression` in `library`'s scope (a type's field default), or the script's when nil.
    fn eval_in_library(
        &mut self,
        expression: &Expression,
        library: Option<&str>,
        context: &mut RuntimeContext,
    ) -> Result<RuntimeValue, Diagnostic> {
        let saved_scope = context.scope.clone();
        let saved_locals = context.locals.clone();
        context.scope = library.map(str::to_owned);
        context.locals = library.map(|_| Default::default());
        let result = self.eval(expression, context);
        context.scope = saved_scope;
        context.locals = saved_locals;
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Kline;
    use crate::pine::compiler::program::SymbolInfo;
    use crate::pine::compiler::{self, NoLibraries};
    use crate::pine::runtime::types::BarFlags;
    use crate::pine::syntax::ast::{Argument, Expression, Literal};
    use crate::pine::syntax::token::{Limits, SourceRange};

    fn bar() -> Kline {
        Kline {
            open_time: 0,
            close_time: 60_000,
            open: 1.0,
            high: 2.0,
            low: 0.5,
            close: 1.5,
            volume: 1.0,
        }
    }

    fn literal(number: f64) -> Expression {
        Expression::Literal(Literal::Float(number), SourceRange::ZERO)
    }

    fn argument(value: Expression) -> Argument {
        Argument {
            name: None,
            value,
            name_range: None,
        }
    }

    #[test]
    fn a_type_constructor_instantiates_an_object() {
        let program = compiler::compile(
            "//@version=6\nindicator(\"T\")\ntype Point\n    float x\n    float y",
            Limits::default(),
            &NoLibraries,
        );
        let mut session = Session::new(
            program,
            HashMap::new(),
            Limits::default(),
            None,
            SymbolInfo::default(),
            None,
            None,
        );
        let mut context = RuntimeContext::new(bar(), BarFlags::default());
        let result = session
            .constructor_call(
                "Point.new",
                &[argument(literal(1.0)), argument(literal(2.0))],
                SourceRange::ZERO,
                &mut context,
            )
            .unwrap()
            .unwrap();
        let RuntimeValue::Ref(RefKind::Object, id) = result else {
            panic!("{result:?}");
        };
        let object = &session.working.instances[&id];
        assert_eq!(object.type_name, "Point");
        assert_eq!(object.fields["x"], RuntimeValue::Float(1.0));
        assert_eq!(object.fields["y"], RuntimeValue::Float(2.0));
    }

    #[test]
    fn a_missing_field_is_pine4019() {
        let program = compiler::compile(
            "//@version=6\nindicator(\"T\")\ntype Point\n    float x",
            Limits::default(),
            &NoLibraries,
        );
        let mut session = Session::new(
            program,
            HashMap::new(),
            Limits::default(),
            None,
            SymbolInfo::default(),
            None,
            None,
        );
        let mut context = RuntimeContext::new(bar(), BarFlags::default());
        let error = session
            .constructor_call(
                "Point.new",
                &[Argument {
                    name: Some("z".to_owned()),
                    value: literal(1.0),
                    name_range: None,
                }],
                SourceRange::ZERO,
                &mut context,
            )
            .unwrap_err();
        assert_eq!(error.code, "PINE4019");
    }

    #[test]
    fn a_non_type_name_is_none() {
        let program = compiler::compile(
            "//@version=6\nindicator(\"T\")",
            Limits::default(),
            &NoLibraries,
        );
        let mut session = Session::new(
            program,
            HashMap::new(),
            Limits::default(),
            None,
            SymbolInfo::default(),
            None,
            None,
        );
        let mut context = RuntimeContext::new(bar(), BarFlags::default());
        let result = session
            .constructor_call("plot", &[], SourceRange::ZERO, &mut context)
            .unwrap();
        assert!(result.is_none());
    }
}
