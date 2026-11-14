//! Evaluating expressions: literals, identifiers and the builtin variables they name, market
//! and symbol values, operators and `[]` history.

use super::{INTERNAL_PREFIX, Session, VARIABLE_NAMESPACES};
use crate::pine::runtime::fast_hash::FastMap;
use crate::pine::runtime::operators;
use crate::pine::runtime::types::RuntimeContext;
use crate::pine::runtime::value::{RefKind, RuntimeValue};
use crate::pine::syntax::ast::{BinaryOperator, Expression, Literal, UnaryOperator};
use crate::pine::{Diagnostic, DiagnosticCategory, SourceRange};
use crate::pine::{builtins::array, builtins::market_values, strategy::builtins};

/// `(minmove, pricescale)` with `minmove / pricescale = mintick`: the scale is the smallest power of
/// ten that makes the tick a whole number of units. None while the tick is not a positive number,
/// or is finer than twelve decimals.
pub(super) fn price_scale(mintick: f64) -> Option<(i64, i64)> {
    if !(mintick.is_finite() && mintick > 0.0) {
        return None;
    }
    let mut scale = 1_i64;
    for _ in 0..=12 {
        let units = mintick * scale as f64;
        if units >= 0.5 && (units - units.round()).abs() <= 1e-9 * units {
            return Some((units.round() as i64, scale));
        }
        scale *= 10;
    }
    None
}

impl Session {
    pub(crate) fn eval(
        &mut self,
        expression: &Expression,
        context: &mut RuntimeContext,
    ) -> Result<RuntimeValue, Diagnostic> {
        self.budget()?;
        match expression {
            Expression::Literal(value, _) => Ok(match value {
                Literal::Int(n) => RuntimeValue::Int(*n),
                Literal::Float(n) => RuntimeValue::Float(*n),
                Literal::Bool(b) => RuntimeValue::Bool(*b),
                Literal::String(s) => RuntimeValue::String(s.clone()),
                Literal::Color(c) => RuntimeValue::Color(*c),
                Literal::Na => RuntimeValue::Na,
            }),
            Expression::Identifier(name, range) => self.resolve_identifier(name, *range, context),
            Expression::Unary { op, operand, range } => {
                self.eval_unary(*op, operand, *range, context)
            }
            Expression::Binary {
                lhs,
                op,
                rhs,
                range,
            } => self.eval_binary(lhs, *op, rhs, *range, context),
            Expression::Ternary {
                condition,
                when_true,
                when_false,
                range,
            } => {
                let test = Self::truth(self.eval(condition, context)?).ok_or_else(|| {
                    Diagnostic::error(
                        "PINE4005",
                        DiagnosticCategory::Runtime,
                        "Ternary condition must be bool.",
                        *range,
                    )
                })?;
                self.eval(if test { when_true } else { when_false }, context)
            }
            Expression::History {
                base,
                offset,
                range,
            } => self.eval_history(base, offset, *range, context),
            Expression::Tuple(expressions, _) => {
                let mut values = Vec::with_capacity(expressions.len());
                for e in expressions {
                    values.push(self.eval(e, context)?);
                }
                Ok(RuntimeValue::Tuple(values))
            }
            Expression::MethodCall {
                receiver,
                name,
                arguments,
                site,
                range,
                target,
            } => {
                #[cfg(any(test, feature = "corpus"))]
                let target = if self.ignore_routes { &None } else { target };
                if let Some(id) = target {
                    return self.method_call_target(
                        *id, receiver, name, arguments, *site, *range, context,
                    );
                }
                self.method_call_on_value(receiver, name, arguments, *site, *range, context)
            }
            Expression::Member { base, name, range } => {
                let value = self.eval(base, context)?;
                self.read_field(name, value, *range)
            }
            Expression::Statement(statement, _) => {
                let (_, value) = self.run(std::slice::from_ref(statement.as_ref()), context)?;
                Ok(if value == RuntimeValue::Void {
                    RuntimeValue::Na
                } else {
                    value
                })
            }
            Expression::Call {
                name,
                arguments,
                site,
                range,
                route,
                target,
            } => {
                #[cfg(any(test, feature = "corpus"))]
                let target = if self.ignore_routes { &None } else { target };
                if let Some(id) = target {
                    return self.call_target(*id, name, arguments, *site, *range, context);
                }
                self.call(name, arguments, *route, *site, *range, context)
            }
        }
    }

    pub(crate) fn resolve_identifier(
        &mut self,
        name: &str,
        range: SourceRange,
        context: &mut RuntimeContext,
    ) -> Result<RuntimeValue, Diagnostic> {
        if let Some(value) = self.working.variables.get(name) {
            return Ok(value.clone());
        }
        // The names below are all dotted; a plain `close` skips straight to the market values.
        if name.contains('.') {
            if let Some(value) = self.field_path(name, range)? {
                return Ok(value);
            }
            if let Some(value) = self.cumulative_variable(name, context) {
                return Ok(value);
            }
            if let Some(value) = self.all_drawings(name) {
                return Ok(value);
            }
        }
        if let Some(value) = self.security_globals.as_ref().and_then(|g| g.get(name)) {
            return Ok(value.clone());
        }
        if let Some(value) = self.market(name, context) {
            return Ok(value);
        }
        if let Some(flag) = context.flags.value(name) {
            return Ok(RuntimeValue::Bool(flag));
        }
        if let Some(color) = crate::pine::builtins::color(name) {
            return Ok(RuntimeValue::Color(color));
        }
        if let Some(constant) = crate::pine::builtins::constant(name) {
            return Ok(constant);
        }
        // A library constant or enum member (via an import alias).
        if let Some(value) = self.library_identifier(name, range, context)? {
            return Ok(value);
        }
        // A dotted name that matches nothing is a named constant (`size.small`) and stands for itself.
        if name.contains('.') && crate::pine::compiler::is_named_constant(name) {
            return Ok(RuntimeValue::String(name.to_owned()));
        }
        if let Some((enum_name, member)) = name.split_once('.')
            && let Some(members) = self.enums.get(enum_name)
        {
            if !members.iter().any(|m| m.name == member) {
                return Err(Diagnostic::error(
                    "PINE4025",
                    DiagnosticCategory::Runtime,
                    format!("Enum '{enum_name}' has no member '{member}'."),
                    range,
                ));
            }
            return Ok(RuntimeValue::String(name.to_owned()));
        }
        // A dotted name no table knows is a named constant (`label.style_label_up`) unless its
        // namespace holds variables: there an unknown member is a feature this engine lacks, and
        // handing the script a string would hide that.
        if let Some((namespace, _)) = name.split_once('.')
            && !VARIABLE_NAMESPACES.contains(&namespace)
        {
            return Ok(RuntimeValue::String(name.to_owned()));
        }
        Err(Diagnostic::error(
            "PINE4008",
            DiagnosticCategory::Runtime,
            format!("Undefined variable '{name}'."),
            range,
        ))
    }

    /// `line.all`, `label.all`, `box.all`, `table.all`, `linefill.all`, `polyline.all`: an array of the
    /// handles of every drawing of that kind the script has alive, oldest first.
    fn all_drawings(&mut self, name: &str) -> Option<RuntimeValue> {
        fn handles<T>(kind: RefKind, objects: &FastMap<usize, T>) -> Vec<RuntimeValue> {
            let mut ids: Vec<usize> = objects.keys().copied().collect();
            ids.sort_unstable();
            ids.into_iter()
                .map(|id| RuntimeValue::Ref(kind, id))
                .collect()
        }
        let items = match name {
            "line.all" => handles(RefKind::Line, &self.working.lines),
            "label.all" => handles(RefKind::Label, &self.working.labels),
            "box.all" => handles(RefKind::Box, &self.working.boxes),
            "table.all" => handles(RefKind::Table, &self.working.tables),
            "linefill.all" => handles(RefKind::Linefill, &self.working.linefills),
            "polyline.all" => handles(RefKind::Polyline, &self.working.polylines),
            _ => return None,
        };
        Some(array::new_array(&mut self.working, items))
    }

    /// `object.field.field`: the parser folds a dotted chain on a plain name into one identifier, so
    /// a chain whose head is a variable is read here, field by field. None when the head is no variable.
    fn field_path(
        &self,
        name: &str,
        range: SourceRange,
    ) -> Result<Option<RuntimeValue>, Diagnostic> {
        let Some((head, rest)) = name.split_once('.') else {
            return Ok(None);
        };
        // A variable that is not an object (`font = ...` shadowing the `font.*` constants) leaves the
        // name to the namespace lookup.
        let Some(mut value) = self
            .working
            .variables
            .get(head)
            .filter(|value| {
                matches!(
                    value,
                    RuntimeValue::Ref(RefKind::Object, _) | RuntimeValue::Na
                )
            })
            .cloned()
        else {
            return Ok(None);
        };
        for field in rest.split('.') {
            value = self.read_field(field, value, range)?;
        }
        Ok(Some(value))
    }

    pub(crate) fn market(&self, name: &str, context: &RuntimeContext) -> Option<RuntimeValue> {
        let bar = &context.bar;
        match name {
            "open" => Some(RuntimeValue::Float(bar.open)),
            "high" => Some(RuntimeValue::Float(bar.high)),
            "low" => Some(RuntimeValue::Float(bar.low)),
            "close" => Some(RuntimeValue::Float(bar.close)),
            "volume" => Some(RuntimeValue::Float(bar.volume)),
            "hl2" => Some(RuntimeValue::Float((bar.high + bar.low) / 2.0)),
            "hlc3" => Some(RuntimeValue::Float((bar.high + bar.low + bar.close) / 3.0)),
            "ohlc4" => Some(RuntimeValue::Float(
                (bar.open + bar.high + bar.low + bar.close) / 4.0,
            )),
            "hlcc4" => Some(RuntimeValue::Float(
                (bar.high + bar.low + 2.0 * bar.close) / 4.0,
            )),
            "time" => Some(RuntimeValue::Int(bar.open_time)),
            "time_close" => Some(RuntimeValue::Int(
                bar.open_time + (self.bar_seconds * 1000.0) as i64,
            )),
            "bar_index" => Some(RuntimeValue::Int(self.working.bar_index)),
            "last_bar_index" => Some(RuntimeValue::Int(self.last_bar_index)),
            "last_bar_time" => self
                .last_bar_time
                .map(RuntimeValue::Int)
                .or(Some(RuntimeValue::Na)),
            "year" | "month" | "dayofmonth" | "hour" | "minute" | "second" | "dayofweek"
            | "weekofyear" => Some(crate::pine::time::part(name, bar.open_time, 0)),
            _ => {
                if let Some(value) = builtins::strategy_value(name, &self.working.broker, bar.close)
                {
                    return Some(value);
                }
                if let Some(value) = market_values::read(
                    name,
                    &market_values::MarketContext {
                        state: &self.working,
                        bar: &context.bar,
                        symbol: &self.symbol,
                        bar_seconds: self.bar_seconds,
                        visible_range: self.visible_range,
                        first_bar_time: self.first_bar_time,
                        last_bar_time: self.last_bar_time,
                        theme: self.theme,
                        chart_bar_seconds: self.chart_bar_seconds.unwrap_or(self.bar_seconds),
                        alert_mode: self.alert_mode,
                    },
                ) {
                    return Some(value);
                }
                self.symbol_value(name)
            }
        }
    }

    fn symbol_value(&self, name: &str) -> Option<RuntimeValue> {
        match name {
            "syminfo.mintick" => Some(RuntimeValue::Float(self.mintick)),
            // The currency the results are in: the strategy's own, else the symbol's.
            "strategy.account_currency" => Some(RuntimeValue::String(
                self.working
                    .broker
                    .settings
                    .currency
                    .as_deref()
                    .map(|code| code.strip_prefix("currency.").unwrap_or(code))
                    .filter(|code| !code.is_empty() && *code != "NONE")
                    .map_or_else(|| self.symbol.currency.clone(), str::to_owned),
            )),
            "syminfo.minmove" | "syminfo.pricescale" => Some(
                price_scale(self.mintick)
                    .map(|(minmove, pricescale)| {
                        RuntimeValue::Int(if name == "syminfo.minmove" {
                            minmove
                        } else {
                            pricescale
                        })
                    })
                    .unwrap_or(RuntimeValue::Na),
            ),
            "syminfo.ticker" => Some(RuntimeValue::String(self.symbol.ticker.clone())),
            "syminfo.tickerid" => Some(RuntimeValue::String(self.symbol.ticker_id.clone())),
            "syminfo.currency" => Some(RuntimeValue::String(self.symbol.currency.clone())),
            "syminfo.root" => Some(RuntimeValue::String(self.symbol.ticker.clone())),
            "syminfo.pointvalue" => Some(RuntimeValue::Float(1.0)),
            "syminfo.type" => Some(RuntimeValue::String(self.symbol.kind.clone())),
            "syminfo.basecurrency" => self
                .symbol
                .base_currency
                .clone()
                .map(RuntimeValue::String)
                .or(Some(RuntimeValue::Na)),
            _ => None,
        }
    }

    fn eval_unary(
        &mut self,
        op: UnaryOperator,
        operand: &Expression,
        range: SourceRange,
        context: &mut RuntimeContext,
    ) -> Result<RuntimeValue, Diagnostic> {
        let value = self.eval(operand, context)?;
        match op {
            UnaryOperator::Negate => Ok(operators::negate(&value)),
            UnaryOperator::Plus => Ok(value),
            UnaryOperator::Not => {
                let b = Self::truth(value).ok_or_else(|| {
                    Diagnostic::error(
                        "PINE4002",
                        DiagnosticCategory::Runtime,
                        "not requires bool.",
                        range,
                    )
                })?;
                Ok(RuntimeValue::Bool(!b))
            }
        }
    }

    fn eval_binary(
        &mut self,
        left: &Expression,
        op: BinaryOperator,
        right: &Expression,
        range: SourceRange,
        context: &mut RuntimeContext,
    ) -> Result<RuntimeValue, Diagnostic> {
        let lhs = self.eval(left, context)?;
        match op {
            BinaryOperator::And => self.eval_logical(true, lhs, right, range, context),
            BinaryOperator::Or => self.eval_logical(false, lhs, right, range, context),
            _ => Ok(operators::apply(op, &lhs, &self.eval(right, context)?)),
        }
    }

    fn eval_logical(
        &mut self,
        is_and: bool,
        lhs: RuntimeValue,
        right: &Expression,
        range: SourceRange,
        context: &mut RuntimeContext,
    ) -> Result<RuntimeValue, Diagnostic> {
        let word = if is_and { "and" } else { "or" };
        let code = if is_and { "PINE4003" } else { "PINE4004" };
        let error = |r: SourceRange| {
            Diagnostic::error(
                code,
                DiagnosticCategory::Runtime,
                format!("{word} requires bool operands."),
                r,
            )
        };
        let left = Self::truth(lhs).ok_or_else(|| error(range))?;
        if left != is_and {
            return Ok(RuntimeValue::Bool(left));
        }
        let rhs = Self::truth(self.eval(right, context)?).ok_or_else(|| error(range))?;
        Ok(RuntimeValue::Bool(rhs))
    }

    fn eval_history(
        &mut self,
        base: &Expression,
        offset: &Expression,
        range: SourceRange,
        context: &mut RuntimeContext,
    ) -> Result<RuntimeValue, Diagnostic> {
        // An offset that is `na` (or not a finite number) reaches no bar: the result is `na`.
        let Some(n) = self
            .eval(offset, context)?
            .number()
            .filter(|n| n.is_finite())
        else {
            return Ok(RuntimeValue::Na);
        };
        let Some(i) = RuntimeValue::integral(n).int_value() else {
            if n < 0.0 {
                return Err(Diagnostic::error(
                    "PINE4006",
                    DiagnosticCategory::Runtime,
                    "History offset cannot be negative.",
                    range,
                ));
            }
            return Ok(RuntimeValue::Na);
        };
        if i < 0 {
            return Err(Diagnostic::error(
                "PINE4006",
                DiagnosticCategory::Runtime,
                "History offset cannot be negative.",
                range,
            ));
        }
        if i == 0 {
            return self.eval(base, context);
        }
        let key = match base {
            // A local of the running function that the body reads a history of keeps one series per
            // call site, because the local itself is gone between calls.
            Expression::Identifier(name, _)
                if context
                    .history_locals
                    .as_ref()
                    .is_some_and(|locals| locals.contains(name)) =>
            {
                format!("fn:{}:{name}", context.site_prefix)
            }
            Expression::Identifier(name, _) => name.clone(),
            _ => {
                let key = format!(
                    "{INTERNAL_PREFIX}expr:{}",
                    self.site_key(range.start.offset, context)
                );
                let value = self.eval(base, context)?;
                self.working.expression_values.insert(key.clone(), value);
                key
            }
        };
        let i = i as usize;
        Ok(match self.working.histories.get(&key) {
            Some(history) if i <= history.len() => history[history.len() - i].clone(),
            _ => RuntimeValue::Na,
        })
    }
}
