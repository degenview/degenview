//! Calls: dispatch by route and name, argument binding, collection and method calls, field
//! access, and invoking the script's own functions.

use super::{INTERNAL_PREFIX, Session, Step};
use crate::pine::builtins::array::Bound;
use crate::pine::compiler::program::InputValue;
use crate::pine::runtime::fast_hash::FastMap;
use crate::pine::runtime::operators;
use crate::pine::runtime::types::{RuntimeContext, RuntimeFunction};
use crate::pine::runtime::user_methods::Supplied;
use crate::pine::runtime::value::{RefKind, RuntimeValue};
use crate::pine::syntax::ast::{AssignmentOperator, Expression};
use crate::pine::{Diagnostic, DiagnosticCategory, SourceRange};
use crate::pine::{
    builtins::alert, builtins::array, builtins::map, builtins::math, builtins::matrix,
    builtins::strings, builtins::ta, outputs::drawings, outputs::visuals, strategy::builtins,
};

impl Session {
    pub(crate) fn call(
        &mut self,
        name: &str,
        arguments: &[crate::pine::syntax::ast::Argument],
        route: Option<crate::pine::syntax::ast::Route>,
        site: usize,
        range: SourceRange,
        context: &mut RuntimeContext,
    ) -> Result<RuntimeValue, Diagnostic> {
        // A call nothing the script declares can capture goes straight to the family that evaluates
        // it (`call_binding`). A family that does not take it hands it back, to the chain below.
        #[cfg(any(test, feature = "corpus"))]
        let route = if self.ignore_routes { None } else { route };
        if let Some(route) = route
            && let Some(value) = self.call_routed(route, name, arguments, site, range, context)?
        {
            return Ok(value);
        }
        // A script-defined (or library) type constructor: `Type.new(...)` / `alias.Type.new(...)`.
        if let Some(object) = self.constructor_call(name, arguments, range, context)? {
            return Ok(object);
        }
        // `area(rect)`: a method called as a function runs the overload that fits its first argument.
        if self.methods.contains_key(name)
            && let Some(first) = arguments.first().filter(|argument| argument.name.is_none())
        {
            let receiver = self.eval(&first.value, context)?;
            if let Some(result) =
                self.call_user_method(name, receiver, &arguments[1..], site, range, context)?
            {
                return Ok(result);
            }
        }
        // A function declared several times with different parameter types picks one per call.
        if let Some(key) = self.scoped_function_key(name, range, context)?
            && let Some(candidates) = self.overloads.get(&key).cloned()
        {
            return self.call_overloaded(name, &candidates, arguments, site, range, context);
        }
        // User function (own, or a library's via an import alias).
        if let Some(function) = self.scoped_function(name, range, context)?
            && self.methods.get(name).is_none_or(|m| m.len() <= 1)
        {
            return self.invoke(function, name, arguments, site, range, context);
        }
        // The declaration and input builtins are no-ops here (their values are read at compile time).
        if crate::pine::runtime::call_binding::is_input_call(name) {
            return self.input_call(name, site, range, context);
        }
        if let Some(value) = self.scalar_call(name, arguments, range, context)? {
            return Ok(value);
        }
        if let Some(value) = self.collection_call(name, arguments, range, context)? {
            return Ok(value);
        }
        if name == "str.split" {
            let values = self.eval_all(arguments, context)?;
            let subject = values
                .first()
                .and_then(RuntimeValue::text_value)
                .unwrap_or_default();
            let separator = values
                .get(1)
                .and_then(RuntimeValue::text_value)
                .unwrap_or_default();
            let pieces = strings::split(subject, separator)
                .into_iter()
                .map(RuntimeValue::String)
                .collect();
            return Ok(array::new_array(&mut self.working, pieces));
        }
        if math::SUPPORTED.contains(&name) && name != "math.sum" {
            return self.math_call(name, arguments, context);
        }
        if strings::SUPPORTED.contains(&name) && name != "str.tostring" {
            return self.strings_call(name, arguments, range, context);
        }
        if crate::pine::runtime::call_binding::INLINE.contains(&name) {
            return self.inline_call(name, arguments, site, range, context);
        }
        if ta::is_supported(name) {
            return self.ta_table_call(name, arguments, site, context);
        }
        if let Some(value) = self.extra_ta_call(name, arguments, site, range, context)? {
            return Ok(value);
        }
        if let Some(value) = self.misc_call(name, arguments, range, context)? {
            return Ok(value);
        }
        if drawings::handles(name) {
            return self.drawings_call(name, arguments, range, context);
        }
        if visuals::handles(name) {
            return self.visuals_call(name, arguments, site, range, context);
        }
        if name.starts_with("strategy.closedtrades.") || name.starts_with("strategy.opentrades.") {
            return self.strategy_trade_call(name, arguments, range, context);
        }
        if builtins::handles(name) {
            return self.strategy_call(name, arguments, range, context);
        }
        if alert::handles(name) {
            return self.alert_call(name, arguments, site, range, context);
        }
        if name == "request.security" {
            return self.security_call(arguments, site, range, context);
        }
        if name == "request.security_lower_tf" {
            return self.security_lower_tf_call(arguments, site, range, context);
        }
        if name == "request.footprint" {
            return self.footprint_request(arguments, range, context);
        }
        if let Some((namespace, member)) = name.split_once('.')
            && crate::pine::footprint::call::handles(namespace, member)
        {
            return self.footprint_function(namespace, member, arguments, range, context);
        }
        // `levels.push(x)`: the lexer reads `name.member` as one dotted name, so a variable (or
        // parameter) holding an array, map or matrix reaches here as a call to `levels.push`.
        if let Some((receiver, member)) = name.rsplit_once('.') {
            let expression = Expression::Identifier(receiver.to_owned(), range);
            // `l.delete()` and `l.set_x1(…)` on a drawing variable that is still `na` do nothing, as
            // the same calls on a missing id do in Pine; a `get_*` gives `na`.
            if let Ok(RuntimeValue::Na) = self.eval(&expression, context)
                && (member == "delete" || member.starts_with("set_") || member.starts_with("get_"))
                && !self.methods.contains_key(member)
            {
                return Ok(if member.starts_with("get_") {
                    RuntimeValue::Na
                } else {
                    RuntimeValue::Void
                });
            }
            if let Ok(value) = self.eval(&expression, context)
                && (matches!(value, RuntimeValue::Ref(..)) || self.methods.contains_key(member))
            {
                return self.method_call_on_value(
                    &expression,
                    member,
                    arguments,
                    site,
                    range,
                    context,
                );
            }
        }
        // Any name still unhandled is genuinely unknown.
        Err(Diagnostic::error(
            "PINE4007",
            DiagnosticCategory::Runtime,
            format!("Unknown or unsupported function '{name}'."),
            range,
        ))
    }

    /// The value an `input.*` call stands for: the user's override, or the declared default, of
    /// the variable it is assigned to. A call that isn't a plain declaration reads `na`.
    pub(crate) fn input_value(
        &mut self,
        site: usize,
        range: SourceRange,
        context: &mut RuntimeContext,
    ) -> Result<RuntimeValue, Diagnostic> {
        let Some(variable) = self.input_variables.get(&site) else {
            return Ok(RuntimeValue::Na);
        };
        Ok(match self.inputs.get(variable).cloned() {
            Some(InputValue::Int(value)) => RuntimeValue::Int(value),
            Some(InputValue::Float(value)) => RuntimeValue::Float(value),
            Some(InputValue::Bool(value)) => RuntimeValue::Bool(value),
            Some(InputValue::String(value)) => RuntimeValue::String(value),
            Some(InputValue::Color(value)) => RuntimeValue::Color(value),
            Some(InputValue::Source(name)) => self.resolve_identifier(&name, range, context)?,
            None => RuntimeValue::Na,
        })
    }

    pub(crate) fn arg_value(
        &mut self,
        arguments: &[crate::pine::syntax::ast::Argument],
        index: usize,
        key: Option<&str>,
        context: &mut RuntimeContext,
    ) -> Result<RuntimeValue, Diagnostic> {
        if let Some(key) = key
            && let Some(found) = arguments.iter().find(|a| a.name.as_deref() == Some(key))
        {
            return self.eval(&found.value, context);
        }
        if index < arguments.len() {
            return self.eval(&arguments[index].value, context);
        }
        Ok(RuntimeValue::Na)
    }

    pub(crate) fn eval_all(
        &mut self,
        arguments: &[crate::pine::syntax::ast::Argument],
        context: &mut RuntimeContext,
    ) -> Result<Vec<RuntimeValue>, Diagnostic> {
        arguments
            .iter()
            .map(|argument| self.eval(&argument.value, context))
            .collect()
    }

    /// Evaluates arguments as source-ordered `drawings::Arg`s (name + value), for the handlers
    /// that bind overloads themselves.
    pub(crate) fn eval_args(
        &mut self,
        arguments: &[crate::pine::syntax::ast::Argument],
        context: &mut RuntimeContext,
    ) -> Result<Vec<drawings::Arg>, Diagnostic> {
        arguments
            .iter()
            .map(|argument| {
                Ok(drawings::Arg {
                    name: argument.name.clone(),
                    value: self.eval(&argument.value, context)?,
                })
            })
            .collect()
    }

    /// Binds arguments by parameter name list (positional first, then named, then `na`), the
    /// builtin-handler analogue of `bind_parameters`.
    pub(crate) fn bind_named(
        &mut self,
        arguments: &[crate::pine::syntax::ast::Argument],
        parameters: &[&'static str],
        context: &mut RuntimeContext,
    ) -> Result<Bound, Diagnostic> {
        let mut bound = Bound::default();
        for (index, name) in parameters.iter().enumerate() {
            if let Some(named) = arguments
                .iter()
                .find(|argument| argument.name.as_deref() == Some(*name))
            {
                bound.insert(name, self.eval(&named.value, context)?);
            } else if let Some(positional) = arguments
                .iter()
                .filter(|argument| argument.name.is_none())
                .nth(index)
            {
                bound.insert(name, self.eval(&positional.value, context)?);
            } else {
                bound.insert(name, RuntimeValue::Na);
            }
        }
        Ok(bound)
    }

    /// The first `count` operands of a builtin whose arguments are all positional, evaluated in
    /// source order into stack slots (an omitted one is `na`), with `receiver` as the first when the
    /// call is a method. `None`, with nothing evaluated, when any argument is named: that call goes
    /// through `bind_named`. Arguments past `count` are not evaluated, as `bind_named` skips them.
    fn positional_operands(
        &mut self,
        receiver: Option<&RuntimeValue>,
        arguments: &[crate::pine::syntax::ast::Argument],
        count: usize,
        context: &mut RuntimeContext,
    ) -> Result<Option<[RuntimeValue; 3]>, Diagnostic> {
        if count > 3 || arguments.iter().any(|argument| argument.name.is_some()) {
            return Ok(None);
        }
        let mut slots = [RuntimeValue::Na, RuntimeValue::Na, RuntimeValue::Na];
        let first = usize::from(receiver.is_some());
        if let Some(receiver) = receiver {
            slots[0] = receiver.clone();
        }
        for (slot, argument) in slots[first..count].iter_mut().zip(arguments) {
            *slot = self.eval(&argument.value, context)?;
        }
        Ok(Some(slots))
    }

    /// A `map.*` function call with only positional arguments, run without binding them by name.
    /// Its own function so the operand slots stay out of `collection_call`'s frame, which every
    /// `array.*` call crosses.
    #[inline(never)]
    fn map_call_positional(
        &mut self,
        name: &str,
        arguments: &[crate::pine::syntax::ast::Argument],
        range: SourceRange,
        context: &mut RuntimeContext,
    ) -> Result<Option<RuntimeValue>, Diagnostic> {
        let count = map::parameters(name).len();
        let Some(slots) = self.positional_operands(None, arguments, count, context)? else {
            return Ok(None);
        };
        map::call_positional(name, &slots[..count], &mut self.working, range).map(Some)
    }

    /// Runs one `array.*`/`map.*`/`matrix.*` call, or `None` when the name is none of them.
    pub(crate) fn collection_call(
        &mut self,
        name: &str,
        arguments: &[crate::pine::syntax::ast::Argument],
        range: SourceRange,
        context: &mut RuntimeContext,
    ) -> Result<Option<RuntimeValue>, Diagnostic> {
        // `array.from` is variadic, so it cannot use the named-parameter binder. Evaluate its
        // arguments exactly once, in source order, and construct the array directly.
        if name == "array.from" {
            let values = self.eval_all(arguments, context)?;
            let value = array::call(
                name,
                &Bound::default(),
                &values,
                &mut self.working,
                self.mintick,
                range,
            )?;
            return Ok(Some(value));
        }
        let parameters = if array::handles(name) {
            array::parameters(name)
        } else if map::handles(name) {
            let parameters = map::parameters(name);
            if let Some(value) = self.map_call_positional(name, arguments, range, context)? {
                return Ok(Some(value));
            }
            parameters
        } else if matrix::handles(name) {
            matrix::parameters(name)
        } else {
            return Ok(None);
        };
        let bound = self.bind_named(arguments, parameters, context)?;
        let value = if array::handles(name) {
            array::call(name, &bound, &[], &mut self.working, self.mintick, range)?
        } else if map::handles(name) {
            map::call(name, &bound, &mut self.working, range)?
        } else {
            matrix::call(name, &bound, &mut self.working, self.mintick, range)?
        };
        Ok(Some(value))
    }

    pub(super) fn method_call_on_value(
        &mut self,
        receiver: &Expression,
        member: &str,
        arguments: &[crate::pine::syntax::ast::Argument],
        site: usize,
        range: SourceRange,
        context: &mut RuntimeContext,
    ) -> Result<RuntimeValue, Diagnostic> {
        let value = self.eval(receiver, context)?;
        if self.methods.contains_key(member)
            && let Some(result) =
                self.call_user_method(member, value.clone(), arguments, site, range, context)?
        {
            return Ok(result);
        }
        // `point.copy()`: a new object with the same field values (nested objects are shared).
        if member == "copy"
            && let RuntimeValue::Ref(RefKind::Object, id) = &value
            && let Some(object) = self.working.instances.get(id).cloned()
        {
            self.working.next_reference += 1;
            let copy = self.working.next_reference;
            self.working.instances.insert(copy, object);
            return Ok(RuntimeValue::Ref(RefKind::Object, copy));
        }
        // `ln.set_extend(...)` on a drawing handle is `line.set_extend(ln, ...)`: the receiver
        // becomes the first argument of the namespaced builtin.
        let drawing = match &value {
            RuntimeValue::Ref(RefKind::Line, _) => Some("line"),
            RuntimeValue::Ref(RefKind::Label, _) => Some("label"),
            RuntimeValue::Ref(RefKind::Box, _) => Some("box"),
            RuntimeValue::Ref(RefKind::Table, _) => Some("table"),
            RuntimeValue::Ref(RefKind::Linefill, _) => Some("linefill"),
            RuntimeValue::Ref(RefKind::Polyline, _) => Some("polyline"),
            _ => None,
        };
        if let Some(namespace) = drawing {
            let full = format!("{namespace}.{member}");
            if drawings::handles(&full) {
                let mut args = vec![drawings::Arg {
                    name: None,
                    value: value.clone(),
                }];
                args.extend(self.eval_args(arguments, context)?);
                return drawings::call(
                    &full,
                    &args,
                    &mut self.working,
                    &context.bar,
                    self.bar_seconds,
                    &self.drawing_limits,
                    range,
                );
            }
        }
        if value == RuntimeValue::Na {
            return Err(Diagnostic::error(
                "PINE4018",
                DiagnosticCategory::Runtime,
                format!("Cannot call '{member}' on a value that is na."),
                range,
            ));
        }
        let footprint_type = match &value {
            RuntimeValue::Ref(RefKind::Footprint, _) => Some("footprint"),
            RuntimeValue::Ref(RefKind::VolumeRow, _) => Some("volume_row"),
            _ => None,
        };
        if let Some(namespace) = footprint_type {
            return self.footprint_member(namespace, member, value, arguments, range, context);
        }
        let prefix = match &value {
            RuntimeValue::Ref(RefKind::Array, _) => "array",
            RuntimeValue::Ref(RefKind::Map, _) => "map",
            RuntimeValue::Ref(RefKind::Matrix, _) => "matrix",
            _ => {
                return Err(Diagnostic::error(
                    "PINE4007",
                    DiagnosticCategory::Runtime,
                    format!("Unknown or unsupported method '{member}'."),
                    range,
                ));
            }
        };
        if prefix == "map"
            && let Some(full) = map::qualified(member)
        {
            let count = map::parameters(full).len();
            if let Some(slots) =
                self.positional_operands(Some(&value), arguments, count, context)?
            {
                return map::call_positional(full, &slots[..count], &mut self.working, range);
            }
        }
        let full = format!("{prefix}.{member}");
        let parameters = match prefix {
            "array" => array::parameters(&full),
            "map" => map::parameters(&full),
            _ => matrix::parameters(&full),
        };
        // The receiver is the function's first parameter: `id`, or `id1` for a two-matrix function.
        let receiver = parameters.first().copied().unwrap_or("id");
        let mut bound = Bound::default();
        bound.insert(receiver, value.clone());
        let mut position = 0;
        for parameter in parameters {
            if *parameter == receiver {
                continue;
            }
            if let Some(named) = arguments
                .iter()
                .find(|argument| argument.name.as_deref() == Some(*parameter))
            {
                bound.insert(parameter, self.eval(&named.value, context)?);
            } else if let Some(positional) = arguments
                .iter()
                .filter(|argument| argument.name.is_none())
                .nth(position)
            {
                bound.insert(parameter, self.eval(&positional.value, context)?);
                position += 1;
            } else {
                bound.insert(parameter, RuntimeValue::Na);
            }
        }
        let result = match prefix {
            "array" => array::call(&full, &bound, &[], &mut self.working, self.mintick, range)?,
            "map" => map::call(&full, &bound, &mut self.working, range)?,
            _ => matrix::call(&full, &bound, &mut self.working, self.mintick, range)?,
        };
        Ok(result)
    }

    pub(super) fn read_field(
        &self,
        name: &str,
        base: RuntimeValue,
        range: SourceRange,
    ) -> Result<RuntimeValue, Diagnostic> {
        if let RuntimeValue::Ref(RefKind::Object, id) = base
            && let Some(object) = self.working.instances.get(&id)
            && let Some(value) = object.fields.get(name)
        {
            return Ok(value.clone());
        }
        Err(Diagnostic::error(
            "PINE4010",
            DiagnosticCategory::Runtime,
            format!("Field '{name}' not found on this value."),
            range,
        ))
    }

    pub(super) fn run_field_assignment(
        &mut self,
        target: &Expression,
        op: AssignmentOperator,
        value: &Expression,
        range: SourceRange,
        context: &mut RuntimeContext,
    ) -> Result<Step, Diagnostic> {
        // `a.b := x` reaches here as the dotted name `a.b`; `f().b := x` as a member of a call.
        let (base, name) = match target {
            Expression::Member { base, name, .. } => (self.eval(base, context)?, name.clone()),
            Expression::Identifier(path, _) if path.contains('.') => {
                let (owner, field) = path.rsplit_once('.').unwrap_or_default();
                let owner = self.resolve_identifier(owner, range, context)?;
                (owner, field.to_owned())
            }
            _ => {
                return Err(Diagnostic::error(
                    "PINE4007",
                    DiagnosticCategory::Runtime,
                    "Assignment to this target is not yet supported.",
                    range,
                ));
            }
        };
        let RuntimeValue::Ref(RefKind::Object, id) = base else {
            return Err(Diagnostic::error(
                "PINE4007",
                DiagnosticCategory::Runtime,
                "Field assignment requires an object.",
                range,
            ));
        };
        let name = &name;
        let rhs = self.eval(value, context)?;
        let assigned = match op {
            AssignmentOperator::Reassign => rhs,
            AssignmentOperator::Compound(_) => {
                let old = self.read_field(name, RuntimeValue::Ref(RefKind::Object, id), range)?;
                operators::apply_assignment(op, &old, &rhs)
            }
        };
        if let Some(object) = self.working.instances.get_mut(&id) {
            object.fields.insert(name.clone(), assigned);
        }
        Ok(Step::normal(RuntimeValue::Void))
    }

    pub(crate) fn site_key(&self, site: usize, context: &RuntimeContext) -> usize {
        if context.site_prefix == 0 {
            site
        } else {
            context
                .site_prefix
                .wrapping_mul(1_000_003)
                .wrapping_add(site)
        }
    }

    pub(crate) fn invoke(
        &mut self,
        function: RuntimeFunction,
        call_name: &str,
        arguments: &[crate::pine::syntax::ast::Argument],
        site: usize,
        range: SourceRange,
        context: &mut RuntimeContext,
    ) -> Result<RuntimeValue, Diagnostic> {
        let supplied = Supplied::Expressions {
            receiver: None,
            arguments,
        };
        self.invoke_with(function, call_name, supplied, site, range, context)
    }

    pub(crate) fn invoke_with(
        &mut self,
        function: RuntimeFunction,
        call_name: &str,
        supplied: Supplied<'_>,
        site: usize,
        range: SourceRange,
        context: &mut RuntimeContext,
    ) -> Result<RuntimeValue, Diagnostic> {
        if context.depth >= self.limits.call_depth {
            return Err(Diagnostic::error(
                "PINE8005",
                DiagnosticCategory::Resource,
                format!("Call depth limit exceeded in '{call_name}'."),
                range,
            ));
        }
        let bound = self.bind_parameters(&function, supplied, context)?;
        let prefix = self.site_key(site, context);
        let stash_key = |local: &str| format!("{INTERNAL_PREFIX}fn:{prefix}:{local}");

        let saved: Vec<(String, Option<RuntimeValue>)> = function
            .locals
            .iter()
            .map(|local| (local.clone(), self.working.variables.get(local).cloned()))
            .collect();
        for local in function.locals.iter() {
            self.working.variables.remove(local);
        }
        // A `var` local that has run before resumes with its value; on the first call it is left
        // unset so its declaration initialises it.
        for local in function.persistent_locals.iter() {
            if let Some(value) = self.working.variables.get(&stash_key(local)).cloned() {
                self.working.variables.insert(local.clone(), value);
            }
        }
        for (parameter, value) in bound {
            self.working.variables.insert(parameter, value);
        }

        let caller_prefix = context.site_prefix;
        let caller_scope = context.scope.clone();
        let caller_locals = context.locals.clone();
        let caller_history_locals = context
            .history_locals
            .replace(function.history_locals.clone());
        context.site_prefix = prefix;
        context.depth += 1;
        context.scope = function.scope.clone();
        context.locals = if function.scope.is_none() {
            None
        } else {
            Some(function.locals.clone())
        };
        if self.hooks.is_some() {
            let first_statement = function
                .body
                .first()
                .map(|first| first.range().start.offset);
            self.hook_enter_function(
                call_name,
                range,
                function.scope.as_deref(),
                first_statement,
                &saved,
            );
        }
        let result = self.run(&function.body, context);
        if self.hooks.is_some() {
            self.hook_leave_function();
        }
        context.site_prefix = caller_prefix;
        context.depth -= 1;
        context.scope = caller_scope;
        context.locals = caller_locals;
        context.history_locals = caller_history_locals;

        // What the call leaves in a local that is read as `local[n]` becomes that call site's next
        // value of the series, one per bar.
        if result.is_ok() {
            let bar = self.working.bar_index;
            for local in function.history_locals.iter() {
                let value = self
                    .working
                    .variables
                    .get(local)
                    .cloned()
                    .unwrap_or(RuntimeValue::Na);
                let key = format!("fn:{prefix}:{local}");
                let series = self.working.histories.entry(key.clone()).or_default();
                if self.working.local_history_bar.get(&key) == Some(&bar) {
                    if let Some(last) = series.last_mut() {
                        *last = value;
                    }
                } else {
                    series.push(value);
                    self.working.local_history_bar.insert(key, bar);
                }
            }
        }
        let result = result.inspect_err(|_| {
            for (local, previous) in &saved {
                match previous {
                    Some(previous) => {
                        self.working
                            .variables
                            .insert(local.clone(), previous.clone());
                    }
                    None => {
                        self.working.variables.remove(local);
                    }
                }
            }
        });
        let (_, value) = result?;
        for local in function.persistent_locals.iter() {
            let value = self
                .working
                .variables
                .get(local)
                .cloned()
                .unwrap_or(RuntimeValue::Na);
            let key = stash_key(local);
            // A `varip` local keeps the value the call left for the next tick of this bar as well.
            if !self.intrabar.is_empty()
                && let Some(slot) = self.intrabar.get_mut(&key)
            {
                *slot = value.clone();
            }
            self.working.variables.insert(key, value);
        }
        for (local, previous) in saved {
            if let Some(previous) = previous {
                self.working.variables.insert(local, previous);
            } else {
                self.working.variables.remove(&local);
            }
        }
        Ok(value)
    }

    fn bind_parameters(
        &mut self,
        function: &RuntimeFunction,
        supplied: Supplied<'_>,
        context: &mut RuntimeContext,
    ) -> Result<FastMap<String, RuntimeValue>, Diagnostic> {
        let mut bound = FastMap::default();
        match supplied {
            Supplied::Expressions {
                mut receiver,
                arguments,
            } => {
                // The receiver stands for the first parameter, so the written arguments start at the second.
                let offset = usize::from(receiver.is_some());
                let positional: Vec<&crate::pine::syntax::ast::Argument> =
                    arguments.iter().filter(|a| a.name.is_none()).collect();
                for (i, parameter) in function.parameters.iter().enumerate() {
                    if i == 0
                        && let Some(value) = receiver.take()
                    {
                        bound.insert(parameter.name.clone(), value);
                    } else if let Some(named) = arguments
                        .iter()
                        .find(|a| a.name.as_deref() == Some(&parameter.name))
                    {
                        bound.insert(parameter.name.clone(), self.eval(&named.value, context)?);
                    } else if let Some(argument) =
                        i.checked_sub(offset).and_then(|n| positional.get(n))
                    {
                        bound.insert(parameter.name.clone(), self.eval(&argument.value, context)?);
                    } else if let Some(fallback) = &parameter.default_value {
                        bound.insert(parameter.name.clone(), self.eval(fallback, context)?);
                    } else {
                        bound.insert(parameter.name.clone(), RuntimeValue::Na);
                    }
                }
            }
            Supplied::Values(values) => {
                let mut labelled = Vec::new();
                let mut positional = Vec::new();
                for (label, value) in values {
                    match label {
                        Some(label) => labelled.push((label, value)),
                        None => positional.push(Some(value)),
                    }
                }
                for (i, parameter) in function.parameters.iter().enumerate() {
                    let given = match labelled
                        .iter()
                        .position(|(label, _)| label == &parameter.name)
                    {
                        Some(at) => Some(labelled.swap_remove(at).1),
                        None => positional.get_mut(i).and_then(Option::take),
                    };
                    if let Some(value) = given {
                        bound.insert(parameter.name.clone(), value);
                    } else if let Some(fallback) = &parameter.default_value {
                        bound.insert(parameter.name.clone(), self.eval(fallback, context)?);
                    } else {
                        bound.insert(parameter.name.clone(), RuntimeValue::Na);
                    }
                }
            }
        }
        Ok(bound)
    }
}
