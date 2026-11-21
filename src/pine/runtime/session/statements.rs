//! Running statements: the statement loop, declarations and assignments, `varip`, and the
//! control flow (`if`, `once`, `for`, `for ... in`, `while`, `switch`).

use super::{INTERNAL_PREFIX, Session, Step};
use crate::pine::runtime::operators;
use crate::pine::runtime::types::{Flow, RuntimeContext};
use crate::pine::runtime::value::{RefKind, RuntimeValue};
use crate::pine::syntax::ast::{
    AssignmentOperator, BinaryOperator, DeclarationMode, Expression, Statement, SwitchArm,
};
use crate::pine::{Diagnostic, DiagnosticCategory, SourceRange};

/// What a `for` loop counts through, its bounds already evaluated.
#[derive(Clone, Copy)]
struct Counting<'s> {
    variable: &'s str,
    start: f64,
    stop: f64,
    stride: f64,
    integral: bool,
}

impl Session {
    /// Runs a list of statements: the script's top level, a function body or a block.
    pub(crate) fn run(
        &mut self,
        statements: &[Statement],
        context: &mut RuntimeContext,
    ) -> Result<(Flow, RuntimeValue), Diagnostic> {
        let mut last = RuntimeValue::Void;
        for statement in statements {
            self.budget()
                .map_err(|error| self.hook_failed(statement, context, error, false))?;
            let hooked = self.hooks.is_some();
            if hooked {
                self.hook_before(statement, context)?;
            }
            let step = self
                .run_statement(statement, context)
                .map_err(|error| self.hook_failed(statement, context, error, true))?;
            if hooked {
                self.hook_after(statement, context);
            }
            if step.value != RuntimeValue::Void {
                last = step.value;
            }
            if step.flow != Flow::Normal {
                return Ok((step.flow, last));
            }
        }
        Ok((Flow::Normal, last))
    }

    fn run_statement(
        &mut self,
        statement: &Statement,
        context: &mut RuntimeContext,
    ) -> Result<Step, Diagnostic> {
        match statement {
            Statement::Declaration {
                name, mode, value, ..
            } => self.run_declaration(name, *mode, value, context),
            Statement::Assignment {
                name, op, value, ..
            } => self.run_assignment(name, *op, value, context),
            Statement::Expression(expression) => Ok(Step::normal(self.eval(expression, context)?)),
            Statement::Conditional {
                condition,
                when_true,
                when_false,
                ..
            } => self.run_conditional(condition, when_true, when_false, context),
            Statement::ForRange {
                variable,
                from,
                to,
                step,
                body,
                range,
            } => self.run_for_range(variable, from, to, step.as_ref(), body, *range, context),
            Statement::ForIn {
                index,
                value,
                collection,
                body,
                range,
            } => self.run_for_in(index.as_deref(), value, collection, body, *range, context),
            Statement::WhileLoop {
                condition, body, ..
            } => self.run_while(condition, body, context),
            Statement::Once {
                condition,
                body,
                range,
            } => self.run_once(condition.as_ref(), body, *range, context),
            Statement::Switch {
                subject,
                arms,
                range,
            } => self.run_switch(subject.as_ref(), arms, *range, context),
            Statement::TupleDeclaration { names, value, .. } => {
                self.run_tuple_declaration(names, value, context)
            }
            Statement::LoopControl(control, _) => Ok(Step {
                flow: if *control == crate::pine::syntax::ast::LoopControl::Break {
                    Flow::Break
                } else {
                    Flow::Continue
                },
                value: RuntimeValue::Void,
            }),
            Statement::FieldAssignment {
                target,
                op,
                value,
                range,
            } => self.run_field_assignment(target, *op, value, *range, context),
            Statement::Function { .. }
            | Statement::TypeDeclaration { .. }
            | Statement::EnumDeclaration { .. } => Ok(Step {
                flow: Flow::Normal,
                value: RuntimeValue::Void,
            }),
        }
    }

    fn run_declaration(
        &mut self,
        name: &str,
        mode: DeclarationMode,
        expression: &Expression,
        context: &mut RuntimeContext,
    ) -> Result<Step, Diagnostic> {
        if mode == DeclarationMode::Variable
            && let Some(existing) = self.working.variables.get(name)
        {
            return Ok(Step::normal(existing.clone()));
        }
        if mode == DeclarationMode::Intrabar {
            self.fill_intrabar_key(name, context);
            let kept = self
                .intrabar
                .get(self.intrabar_key.as_str())
                .or_else(|| self.working.variables.get(name))
                .cloned();
            let value = match kept {
                Some(value) => value,
                None => self.eval(expression, context)?,
            };
            // `eval` may run a function that reuses the scratch key.
            self.fill_intrabar_key(name, context);
            self.set_variable(name, value.clone());
            self.keep_intrabar(value.clone());
            context.record(mode, &self.intrabar_key);
            return Ok(Step::normal(value));
        }
        let value = self.eval(expression, context)?;
        self.set_variable(name, value.clone());
        context.record(mode, name);
        Ok(Step::normal(value))
    }

    /// Writes into `intrabar_key` where a `varip` name keeps its value between ticks. A global is its
    /// own name. A local of a user function is one per call site, in the place `invoke_with` keeps a
    /// persistent local between calls, so two calls (and a global of the same name) never share it.
    fn fill_intrabar_key(&mut self, name: &str, context: &RuntimeContext) {
        use std::fmt::Write;
        self.intrabar_key.clear();
        if context.depth == 0 {
            self.intrabar_key.push_str(name);
        } else {
            let _ = write!(
                self.intrabar_key,
                "{INTERNAL_PREFIX}fn:{}:{name}",
                context.site_prefix
            );
        }
    }

    /// Stores `value` under `intrabar_key`, allocating the key only the first time.
    fn keep_intrabar(&mut self, value: RuntimeValue) {
        match self.intrabar.get_mut(self.intrabar_key.as_str()) {
            Some(slot) => *slot = value,
            None => {
                self.intrabar.insert(self.intrabar_key.clone(), value);
            }
        }
    }

    /// Sets a variable, allocating its name only the first time it is seen.
    fn set_variable(&mut self, name: &str, value: RuntimeValue) {
        match self.working.variables.get_mut(name) {
            Some(slot) => *slot = value,
            None => {
                self.working.variables.insert(name.to_owned(), value);
            }
        }
    }

    fn run_assignment(
        &mut self,
        name: &str,
        op: AssignmentOperator,
        expression: &Expression,
        context: &mut RuntimeContext,
    ) -> Result<Step, Diagnostic> {
        let rhs = self.eval(expression, context)?;
        let old = self
            .working
            .variables
            .get(name)
            .cloned()
            .unwrap_or(RuntimeValue::Na);
        let value = operators::apply_assignment(op, &old, &rhs);
        self.set_variable(name, value.clone());
        if !context.intrabar_names.is_empty() || !self.intrabar.is_empty() {
            self.fill_intrabar_key(name, context);
            if context.intrabar_names.contains(self.intrabar_key.as_str())
                || self.intrabar.contains_key(self.intrabar_key.as_str())
            {
                self.keep_intrabar(value.clone());
            }
        }
        Ok(Step::normal(value))
    }

    fn run_tuple_declaration(
        &mut self,
        names: &[String],
        expression: &Expression,
        context: &mut RuntimeContext,
    ) -> Result<Step, Diagnostic> {
        let value = self.eval(expression, context)?;
        let items = match &value {
            RuntimeValue::Tuple(items) => items.clone(),
            _ => Vec::new(),
        };
        for (i, name) in names.iter().enumerate() {
            if name != "_" {
                self.set_variable(name, items.get(i).cloned().unwrap_or(RuntimeValue::Na));
            }
        }
        Ok(Step::normal(value))
    }

    fn run_conditional(
        &mut self,
        condition: &Expression,
        when_true: &[Statement],
        when_false: &[Statement],
        context: &mut RuntimeContext,
    ) -> Result<Step, Diagnostic> {
        let value = self.eval(condition, context)?;
        let test = self.require_bool(
            value,
            "if condition must be bool; numeric-to-bool coercion is not allowed in v6.",
            condition.range(),
        )?;
        let (flow, value) = self.run(if test { when_true } else { when_false }, context)?;
        Ok(Step { flow, value })
    }

    /// `once [condition]`. The condition is evaluated on every bar, like an `if`'s, so the `ta.*`
    /// calls in it keep their history; the block runs only until it has run once. The finished
    /// flag lives in the working state, so an unconfirmed realtime tick that ran the block leaves
    /// the structure active, as TradingView's rollback does.
    fn run_once(
        &mut self,
        condition: Option<&Expression>,
        body: &[Statement],
        range: SourceRange,
        context: &mut RuntimeContext,
    ) -> Result<Step, Diagnostic> {
        if let Some(condition) = condition {
            let value = self.eval(condition, context)?;
            let test = self.require_bool(
                value,
                "once condition must be bool; numeric-to-bool coercion is not allowed in v6.",
                condition.range(),
            )?;
            if !test {
                return Ok(Step::normal(RuntimeValue::Void));
            }
        }
        let key = self.site_key(range.start.offset, context);
        if self.working.once_finished.contains(&key) {
            return Ok(Step::normal(RuntimeValue::Void));
        }
        let (flow, _) = self.run(body, context)?;
        self.working.once_finished.push(key);
        Ok(Step {
            flow,
            value: RuntimeValue::Void,
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn run_for_range(
        &mut self,
        variable: &str,
        from: &Expression,
        to: &Expression,
        step_expression: Option<&Expression>,
        body: &[Statement],
        range: SourceRange,
        context: &mut RuntimeContext,
    ) -> Result<Step, Diagnostic> {
        let start_value = self.eval(from, context)?;
        let Some(start) = start_value.number() else {
            return Ok(Step {
                flow: Flow::Normal,
                value: RuntimeValue::Void,
            });
        };
        let Some(stop) = self.eval(to, context)?.number() else {
            return Ok(Step {
                flow: Flow::Normal,
                value: RuntimeValue::Void,
            });
        };
        let mut stride = 1.0;
        let mut integral = start_value.is_int();
        if let Some(step_expression) = step_expression {
            let step_value = self.eval(step_expression, context)?;
            let Some(size) = step_value.number() else {
                return Err(Diagnostic::error(
                    "PINE4014",
                    DiagnosticCategory::Runtime,
                    "for loop step must be a non-zero number.",
                    range,
                ));
            };
            if size == 0.0 || !size.is_finite() {
                return Err(Diagnostic::error(
                    "PINE4014",
                    DiagnosticCategory::Runtime,
                    "for loop step must be a non-zero number.",
                    range,
                ));
            }
            stride = size.abs();
            if !step_value.is_int() {
                integral = false;
            }
        }
        let counting = Counting {
            variable,
            start,
            stop,
            stride,
            integral,
        };
        let result = self.count(&counting, body, context);
        Ok(Step {
            flow: Flow::Normal,
            value: result?,
        })
    }

    /// The iterations of a `for` loop, up to its last value.
    fn count(
        &mut self,
        counting: &Counting<'_>,
        body: &[Statement],
        context: &mut RuntimeContext,
    ) -> Result<RuntimeValue, Diagnostic> {
        let Counting {
            variable,
            start,
            stop,
            stride,
            integral,
        } = *counting;
        let direction = if stop >= start { 1.0 } else { -1.0 };
        let mut last = RuntimeValue::Void;
        let mut i = start;
        while if direction > 0.0 {
            i <= stop
        } else {
            i >= stop
        } {
            self.budget()?;
            self.set_variable(
                variable,
                if integral {
                    RuntimeValue::integral(i)
                } else {
                    RuntimeValue::Float(i)
                },
            );
            let (flow, value) = self.run(body, context)?;
            last = value;
            if flow == Flow::Break {
                break;
            }
            i += direction * stride;
        }
        Ok(last)
    }

    #[allow(clippy::too_many_arguments)]
    fn run_for_in(
        &mut self,
        index_name: Option<&str>,
        value_name: &str,
        collection: &Expression,
        body: &[Statement],
        range: SourceRange,
        context: &mut RuntimeContext,
    ) -> Result<Step, Diagnostic> {
        let target = self.eval(collection, context)?;
        if target == RuntimeValue::Na {
            return Ok(Step {
                flow: Flow::Normal,
                value: RuntimeValue::Void,
            });
        }
        let value = match target {
            RuntimeValue::Ref(RefKind::Array, id) => {
                let items = self.working.arrays.get(&id).cloned().unwrap_or_default();
                let mut indexed = items.into_iter().enumerate();
                self.iterate(
                    index_name,
                    value_name,
                    body,
                    context,
                    |_, stopped| {
                        Ok(if stopped {
                            None
                        } else {
                            indexed
                                .next()
                                .map(|(index, item)| (RuntimeValue::Int(index as i64), item))
                        })
                    },
                )
            }
            RuntimeValue::Ref(RefKind::Map, id) => {
                let Some(index_name) = index_name else {
                    return Err(Diagnostic::error(
                        "PINE4011",
                        DiagnosticCategory::Runtime,
                        "Iterating a map requires `for [key, value] in map`.",
                        range,
                    ));
                };
                let revision = self
                    .working
                    .maps
                    .get(&id)
                    .ok_or_else(|| {
                        Diagnostic::error(
                            "PINE4011",
                            DiagnosticCategory::Runtime,
                            "for...in cannot iterate a missing map.",
                            range,
                        )
                    })?
                    .key_revision();
                let mut offset = 0;
                self.iterate(
                    Some(index_name),
                    value_name,
                    body,
                    context,
                    |session, stopped| {
                        let map = session
                            .working
                            .maps
                            .get(&id)
                            .filter(|map| map.key_revision() == revision)
                            .ok_or_else(|| {
                                Diagnostic::error(
                                    "PINE4039",
                                    DiagnosticCategory::Runtime,
                                    "Cannot add or remove map entries while iterating it.",
                                    range,
                                )
                            })?;
                        if stopped {
                            return Ok(None);
                        }
                        let pair = map.pair_at(offset);
                        offset += 1;
                        Ok(pair)
                    },
                )
            }
            _ => {
                return Err(Diagnostic::error(
                    "PINE4011",
                    DiagnosticCategory::Runtime,
                    "for...in requires an array or a map.",
                    range,
                ));
            }
        }?;
        Ok(Step::normal(value))
    }

    /// After a `break`, the pair source still validates the final iteration but returns no item.
    fn iterate(
        &mut self,
        first_name: Option<&str>,
        value_name: &str,
        body: &[Statement],
        context: &mut RuntimeContext,
        mut next_pair: impl FnMut(
            &Self,
            bool,
        ) -> Result<Option<(RuntimeValue, RuntimeValue)>, Diagnostic>,
    ) -> Result<RuntimeValue, Diagnostic> {
        let mut last = RuntimeValue::Void;
        let mut stopped = false;
        while let Some((first, item)) = next_pair(self, stopped)? {
            self.budget()?;
            if let Some(first_name) = first_name {
                self.set_variable(first_name, first);
            }
            self.set_variable(value_name, item);
            let (flow, value) = self.run(body, context)?;
            last = value;
            stopped = flow == Flow::Break;
        }
        Ok(last)
    }

    fn run_while(
        &mut self,
        condition: &Expression,
        body: &[Statement],
        context: &mut RuntimeContext,
    ) -> Result<Step, Diagnostic> {
        let mut last = RuntimeValue::Void;
        loop {
            self.budget()?;
            let value = self.eval(condition, context)?;
            let test =
                self.require_bool(value, "while condition must be bool.", condition.range())?;
            if !test {
                break;
            }
            let (flow, value) = self.run(body, context)?;
            last = value;
            if flow == Flow::Break {
                break;
            }
        }
        Ok(Step {
            flow: Flow::Normal,
            value: last,
        })
    }

    fn run_switch(
        &mut self,
        subject: Option<&Expression>,
        arms: &[SwitchArm],
        range: SourceRange,
        context: &mut RuntimeContext,
    ) -> Result<Step, Diagnostic> {
        let target = match subject {
            Some(subject) => Some(self.eval(subject, context)?),
            None => None,
        };
        let Some(chosen) = self.choose_arm(arms, target.as_ref(), range, context)? else {
            return Ok(Step {
                flow: Flow::Normal,
                value: RuntimeValue::Na,
            });
        };
        let (flow, value) = self.run(&arms[chosen].body, context)?;
        Ok(Step { flow, value })
    }

    fn choose_arm(
        &mut self,
        arms: &[SwitchArm],
        target: Option<&RuntimeValue>,
        range: SourceRange,
        context: &mut RuntimeContext,
    ) -> Result<Option<usize>, Diagnostic> {
        let mut fallback = None;
        for (index, arm) in arms.iter().enumerate() {
            let Some(condition) = &arm.condition else {
                fallback = fallback.or(Some(index));
                continue;
            };
            let value = self.eval(condition, context)?;
            let matches = if let Some(target) = target {
                operators::apply(BinaryOperator::Equal, target, &value) == RuntimeValue::Bool(true)
            } else {
                self.require_bool(value, "switch arm condition must be bool.", range)?
            };
            if matches {
                return Ok(Some(index));
            }
        }
        Ok(fallback)
    }

    fn require_bool(
        &self,
        value: RuntimeValue,
        message: &str,
        range: SourceRange,
    ) -> Result<bool, Diagnostic> {
        Self::truth(value).ok_or_else(|| {
            Diagnostic::error("PINE4001", DiagnosticCategory::Runtime, message, range)
        })
    }

    pub fn truth(value: RuntimeValue) -> Option<bool> {
        match value {
            RuntimeValue::Bool(b) => Some(b),
            RuntimeValue::Na => Some(false),
            _ => None,
        }
    }
}
