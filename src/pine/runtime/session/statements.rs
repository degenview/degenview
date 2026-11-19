//! Running statements: the statement loop, declarations and assignments, `varip`, and the
//! control flow (`if`, `once`, `for`, `for ... in`, `while`, `switch`).

use super::{INTERNAL_PREFIX, Session, Step};
use crate::pine::runtime::fast_hash::FastMap;
use crate::pine::runtime::operators;
use crate::pine::runtime::types::{Flow, RuntimeContext};
use crate::pine::runtime::value::{RefKind, RuntimeValue};
use crate::pine::syntax::ast::{
    AssignmentOperator, BinaryOperator, DeclarationMode, Expression, ShadowId, Statement, SwitchArm,
};
use crate::pine::{Diagnostic, DiagnosticCategory, SourceRange};

/// The values a loop's counters held outside it, to put back when it ends: an index and a value at
/// most. The names borrow from the statement, so saving one builds no `String`.
#[derive(Default)]
struct Counters<'s>([Option<(&'s str, Option<RuntimeValue>)>; 2]);

impl<'s> Counters<'s> {
    /// `value` is what `name` holds now; None for a builtin's name, which has no variable.
    fn push(&mut self, name: &'s str, value: Option<RuntimeValue>) {
        if let Some(slot) = self.0.iter_mut().find(|slot| slot.is_none()) {
            *slot = Some((name, value));
        }
    }

    fn restore(self, variables: &mut FastMap<String, RuntimeValue>) {
        for (name, value) in self.0.into_iter().flatten() {
            put_back(variables, name, value);
        }
    }
}

/// Gives `name` the value it held outside a block, or removes it (a builtin's name has no variable).
fn put_back(
    variables: &mut FastMap<String, RuntimeValue>,
    name: &str,
    value: Option<RuntimeValue>,
) {
    match (value, variables.get_mut(name)) {
        (Some(value), Some(slot)) => *slot = value,
        (Some(value), None) => {
            variables.insert(name.to_owned(), value);
        }
        (None, _) => {
            variables.remove(name);
        }
    }
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

    /// Runs one execution of a block (a branch, an arm, a loop body, a `once` body). A declaration
    /// in it that hides an outer name (`Declaration::shadows`) saves the outer value as it stands
    /// when the declaration is about to give the name a value of its own, and the block puts the
    /// saved values back, innermost first, when it ends, however it ends. The variable table is
    /// flat, so this is what makes the name the outer one again outside the block; a `:=` above the
    /// declaration, in the declaration's own value or in another branch is the outer variable's
    /// and stays. Declarations that hide nothing save nothing.
    fn run_block(
        &mut self,
        statements: &[Statement],
        context: &mut RuntimeContext,
    ) -> Result<(Flow, RuntimeValue), Diagnostic> {
        let mark = self.shadow_stack.len();
        let result = self.run(statements, context);
        if self.shadow_stack.len() > mark {
            self.unshadow(mark);
        }
        result
    }

    /// Saves what the name `id` holds now, for the declaration about to hide it. A name with no
    /// variable yet saves nothing, except a builtin's (`low`), which is removed after.
    fn shadow(&mut self, id: ShadowId) {
        let name = &self.shadow_names[id as usize];
        let value = match self.working.variables.get(name.as_str()) {
            Some(value) => Some(value.clone()),
            None if self.shadowing_builtins.contains(name) => None,
            None => return,
        };
        self.shadow_stack.push((id, value));
    }

    /// Puts back what the blocks since `mark` saved.
    fn unshadow(&mut self, mark: usize) {
        while self.shadow_stack.len() > mark {
            if let Some((id, value)) = self.shadow_stack.pop() {
                let name = &self.shadow_names[id as usize];
                put_back(&mut self.working.variables, name, value);
            }
        }
    }

    fn run_statement(
        &mut self,
        statement: &Statement,
        context: &mut RuntimeContext,
    ) -> Result<Step, Diagnostic> {
        // A loop's counters are the loop's own: set before its body runs, put back after it.
        let mut counters = Counters::default();
        match statement {
            Statement::ForRange { variable, .. } => self.save_counter(variable, &mut counters),
            Statement::ForIn { index, value, .. } => {
                if let Some(index) = index.as_deref() {
                    self.save_counter(index, &mut counters);
                }
                self.save_counter(value, &mut counters);
            }
            _ => return self.run_statement_unscoped(statement, context),
        }
        let result = self.run_statement_unscoped(statement, context);
        counters.restore(&mut self.working.variables);
        result
    }

    /// Saves what a loop counter holds outside the loop, if it holds anything; a builtin's name has
    /// no variable, so it is removed after.
    fn save_counter<'s>(&self, name: &'s str, counters: &mut Counters<'s>) {
        match self.working.variables.get(name) {
            Some(value) => counters.push(name, Some(value.clone())),
            None if self.shadowing_builtins.iter().any(|shadow| shadow == name) => {
                counters.push(name, None);
            }
            None => {}
        }
    }

    fn run_statement_unscoped(
        &mut self,
        statement: &Statement,
        context: &mut RuntimeContext,
    ) -> Result<Step, Diagnostic> {
        match statement {
            Statement::Declaration {
                name,
                mode,
                value,
                shadows,
                ..
            } => self.run_declaration(name, *mode, value, *shadows, context),
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
            Statement::TupleDeclaration {
                names,
                value,
                shadows,
                ..
            } => self.run_tuple_declaration(names, shadows, value, context),
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
        shadows: Option<ShadowId>,
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
        // The value is done: what the name holds now is the outer variable, as the initializer left
        // it.
        if let Some(id) = shadows {
            self.shadow(id);
        }
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
        shadows: &[Option<ShadowId>],
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
                if let Some(Some(id)) = shadows.get(i) {
                    self.shadow(*id);
                }
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
        let (flow, value) = self.run_block(if test { when_true } else { when_false }, context)?;
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
        let (flow, _) = self.run_block(body, context)?;
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
            let (flow, value) = self.run_block(body, context)?;
            last = value;
            if flow == Flow::Break {
                break;
            }
            i += direction * stride;
        }
        Ok(Step {
            flow: Flow::Normal,
            value: last,
        })
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
        let RuntimeValue::Ref(RefKind::Array, id) = target else {
            return Err(Diagnostic::error(
                "PINE4011",
                DiagnosticCategory::Runtime,
                "for...in requires an array or a map.",
                range,
            ));
        };
        let items = self.working.arrays.get(&id).cloned().unwrap_or_default();
        let mut last = RuntimeValue::Void;
        for (offset, item) in items.into_iter().enumerate() {
            self.budget()?;
            if let Some(index_name) = index_name {
                self.set_variable(index_name, RuntimeValue::Int(offset as i64));
            }
            self.set_variable(value_name, item);
            let (flow, value) = self.run_block(body, context)?;
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
            let (flow, value) = self.run_block(body, context)?;
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
        let (flow, value) = self.run_block(&arms[chosen].body, context)?;
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
