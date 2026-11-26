//! What the expression of a `request.security` may read of the strategy.
//!
//! The strategy's state (`strategy.equity`, `strategy.position_size`, `strategy.closedtrades`, ...)
//! belongs to the chart's broker, and a request runs on another series, so TradingView refuses to
//! compile a request that reads it: `"strategy.equity" cannot be used with any parameter of the
//! "request.*()" functions.` (CE10059). Measured on a 5-minute BTCUSDT chart, Pine v6:
//!
//! - What is refused: every `strategy.*` variable, the `strategy.closedtrades.*()` and
//!   `strategy.opentrades.*()` functions, `strategy.convert_to_account()`,
//!   `strategy.convert_to_symbol()` and `strategy.default_entry_qty()`; the constants
//!   (`strategy.long`, `strategy.short`) are not.
//! - Where: anywhere in the `expression` argument (`strategy.equity * 2`, `strategy.equity[1]`,
//!   either arm of a ternary, named or positional); in a function the expression calls, however
//!   deep (`f()` whose body, or the body of a function it calls, reads one); and in the value of a
//!   variable that is a whole result of the expression, the expression itself or one value of its
//!   tuple (`a` or `[a, close]` after `a = strategy.equity * 2`; a `var`, a local, a tuple element
//!   of `[a, b] = f()`), through the functions that value calls.
//! - Where not: further than that. `b = a * 2`, `b = a`, `a + 1` and `a[1]` after
//!   `a = strategy.equity` compile, as does a function that only reads a global holding it, a
//!   variable reassigned from it after the request, and a parameter (even one named like such a
//!   variable). Only `request.security` is checked: `request.security_lower_tf` takes it, and so
//!   does the symbol argument.
//!
//! The error is reported on the expression, as TradingView does, and names the first read it finds.
//! A function name that several overloads share counts only when every overload reads it (which
//! one a call picks is the type checker's); a method called on a value is not followed.

use std::collections::HashMap;

use crate::pine::compiler::symbol_catalog;
use crate::pine::runtime::security_slice::expression_argument;
use crate::pine::syntax::ast::{Expression, FunctionKind, Statement};
use crate::pine::syntax::scopes::{self, Binding, Bindings};
use crate::pine::syntax::traversal;
use crate::pine::{Diagnostic, DiagnosticCategory};

const REQUEST: &str = "request.security";

/// One error for each `request.security` whose expression reads the strategy's state.
pub fn validate(statements: &[Statement]) -> Vec<Diagnostic> {
    let mut expressions = Vec::new();
    traversal::for_each_call_with_args(statements, &mut |name, arguments, _| {
        if name == REQUEST
            && let Some(expression) = expression_argument(name, arguments)
        {
            expressions.push(expression);
        }
    });
    if expressions.is_empty() {
        return Vec::new();
    }
    let script = Script::new(statements);
    let mut functions = Followed::new();
    expressions
        .into_iter()
        .filter_map(|expression| {
            let read = script.read_by_request(expression, &mut functions)?;
            Some(Diagnostic::error(
                "PINE3074",
                DiagnosticCategory::Semantic,
                format!(
                    "\"{read}\" cannot be used with any parameter of the \"request.*()\" functions."
                ),
                expression.range(),
            ))
        })
        .collect()
}

/// The name TradingView prints for a read of the strategy's state, if `name` is one. `call` is
/// whether it is called.
fn state_name(name: &str, call: bool) -> Option<String> {
    let member = name.strip_prefix("strategy.")?;
    if !call {
        return symbol_catalog::is_variable(name).then(|| name.to_owned());
    }
    if member.starts_with("closedtrades.") {
        Some("strategy.closedtrades.*()".to_owned())
    } else if member.starts_with("opentrades.") {
        Some("strategy.opentrades.*()".to_owned())
    } else if member.starts_with("convert_to_") {
        Some("strategy.convert.*()".to_owned())
    } else {
        (member == "default_entry_qty").then(|| name.to_owned())
    }
}

/// What the script declares: its functions and the value each variable is declared with.
struct Script<'a> {
    bindings: Bindings<'a>,
    /// The bodies of each function name (overloads share one), methods left out.
    functions: HashMap<&'a str, Vec<&'a [Statement]>>,
    values: HashMap<Binding<'a>, &'a Expression>,
}

impl<'a> Script<'a> {
    fn new(statements: &'a [Statement]) -> Self {
        let mut script = Self {
            bindings: scopes::resolve(statements),
            functions: HashMap::new(),
            values: HashMap::new(),
        };
        script.declared_in(statements);
        script
    }

    fn declared_in(&mut self, statements: &'a [Statement]) {
        for statement in statements {
            match statement {
                Statement::Declaration { value, .. } => {
                    if let Some(binding) = self.bindings.declaration(statement) {
                        self.values.insert(binding, value);
                    }
                }
                Statement::TupleDeclaration { value, .. } => {
                    for binding in self.bindings.introduced(statement) {
                        self.values.insert(binding, value);
                    }
                }
                Statement::Function {
                    name,
                    kind: FunctionKind::Function,
                    body,
                    ..
                } => self.functions.entry(name).or_default().push(body),
                _ => {}
            }
            for expression in traversal::expressions(statement) {
                self.declared_in_expression(expression);
            }
            for block in traversal::nested_blocks(statement) {
                self.declared_in(block);
            }
        }
    }

    /// The statements an expression holds (`x = if …`) declare too.
    fn declared_in_expression(&mut self, expression: &'a Expression) {
        if let Expression::Statement(statement, _) = expression {
            self.declared_in(std::slice::from_ref(statement.as_ref()));
        }
        for child in traversal::children(expression) {
            self.declared_in_expression(child);
        }
    }

    /// The first read of the strategy's state a request's `expression` makes, in the sense of the
    /// module documentation.
    fn read_by_request(
        &self,
        expression: &'a Expression,
        functions: &mut Followed<'a>,
    ) -> Option<String> {
        if let Some(read) = self.read_in(expression, functions) {
            return Some(read);
        }
        let results = match expression {
            Expression::Tuple(values, _) => values.as_slice(),
            _ => std::slice::from_ref(expression),
        };
        results
            .iter()
            .find_map(|result| self.read_by_variable(result, functions))
    }

    /// A variable that is a whole result of the request stands for the value it was declared with.
    fn read_by_variable(
        &self,
        result: &'a Expression,
        functions: &mut Followed<'a>,
    ) -> Option<String> {
        let Expression::Identifier(..) = result else {
            return None;
        };
        let value = self.values.get(&self.bindings.occurrence(result)?)?;
        self.read_in(value, functions)
    }

    /// A read of the strategy's state in `expression` itself or in a function it calls.
    fn read_in(&self, expression: &'a Expression, functions: &mut Followed<'a>) -> Option<String> {
        match expression {
            Expression::Identifier(name, _) => {
                if let Some(read) = state_name(name, false) {
                    return Some(read);
                }
            }
            Expression::Call { name, .. } => {
                if let Some(read) = state_name(name, true) {
                    return Some(read);
                }
                if let Some(read) = self.read_by_function(name, functions) {
                    return Some(read);
                }
            }
            Expression::Statement(statement, _) => {
                return self
                    .read_in_statements(std::slice::from_ref(statement.as_ref()), functions);
            }
            _ => {}
        }
        traversal::children(expression)
            .into_iter()
            .find_map(|child| self.read_in(child, functions))
    }

    fn read_in_statements(
        &self,
        statements: &'a [Statement],
        functions: &mut Followed<'a>,
    ) -> Option<String> {
        statements.iter().find_map(|statement| {
            traversal::expressions(statement)
                .into_iter()
                .find_map(|expression| self.read_in(expression, functions))
                .or_else(|| {
                    traversal::nested_blocks(statement)
                        .into_iter()
                        .find_map(|block| self.read_in_statements(block, functions))
                })
        })
    }

    /// What the function called `name` reads, when every function of that name reads it.
    fn read_by_function(&self, name: &'a str, functions: &mut Followed<'a>) -> Option<String> {
        let bodies = self.functions.get(name)?;
        if let Some(known) = functions.get(name) {
            return known.clone();
        }
        // Held as reading nothing while it is followed, which ends a recursive one.
        functions.insert(name, None);
        let reads: Vec<Option<String>> = bodies
            .iter()
            .map(|body| self.read_in_statements(body, functions))
            .collect();
        let read = if reads.iter().all(Option::is_some) {
            reads.into_iter().next().flatten()
        } else {
            None
        };
        functions.insert(name, read.clone());
        read
    }
}

/// What each function followed so far reads, so that each is walked once.
type Followed<'a> = HashMap<&'a str, Option<String>>;

#[cfg(test)]
mod tests {
    use crate::pine::compiler::{self, NoLibraries};
    use crate::pine::syntax::token::Limits;

    const REQUEST: &str = "request.security(syminfo.tickerid, \"5\", ";

    /// The messages of the `PINE3074` errors of a strategy that ends in `body`.
    fn errors(body: &str) -> Vec<String> {
        let source = format!("//@version=6\nstrategy(\"S\", overlay = true)\n{body}\nplot(1)\n");
        let program = compiler::compile(&source, Limits::default(), &NoLibraries);
        program
            .diagnostics
            .into_iter()
            .filter(|diagnostic| diagnostic.code == "PINE3074")
            .map(|diagnostic| diagnostic.message)
            .collect()
    }

    fn refused(body: &str) -> String {
        let found = errors(body);
        assert_eq!(found.len(), 1, "{body}: {found:?}");
        found.into_iter().next().unwrap_or_default()
    }

    fn message(name: &str) -> String {
        format!("\"{name}\" cannot be used with any parameter of the \"request.*()\" functions.")
    }

    /// Every case below was compiled on TradingView (see the module documentation).
    #[test]
    fn the_strategy_state_in_the_expression_is_refused() {
        for name in [
            "equity",
            "position_size",
            "closedtrades",
            "opentrades",
            "netprofit",
            "initial_capital",
            "account_currency",
        ] {
            let body = format!("x = {REQUEST}strategy.{name})");
            assert_eq!(
                refused(&body),
                message(&format!("strategy.{name}")),
                "{name}"
            );
        }
        for body in [
            format!("x = {REQUEST}strategy.equity * 2)"),
            format!("x = {REQUEST}strategy.equity[1])"),
            format!("[x, y] = {REQUEST}[strategy.equity, close])"),
            format!("x = {REQUEST}close > 0 ? strategy.equity : 0.0)"),
            "x = request.security(syminfo.tickerid, \"5\", expression = strategy.equity)".into(),
        ] {
            assert_eq!(refused(&body), message("strategy.equity"), "{body}");
        }
    }

    #[test]
    fn the_strategy_functions_that_read_its_state_are_refused() {
        for (call, printed) in [
            (
                "strategy.closedtrades.profit(0)",
                "strategy.closedtrades.*()",
            ),
            ("strategy.opentrades.size(0)", "strategy.opentrades.*()"),
            ("strategy.convert_to_account(close)", "strategy.convert.*()"),
            ("strategy.convert_to_symbol(close)", "strategy.convert.*()"),
            (
                "strategy.default_entry_qty(close)",
                "strategy.default_entry_qty",
            ),
        ] {
            let body = format!("x = {REQUEST}{call})");
            assert_eq!(refused(&body), message(printed), "{call}");
        }
    }

    #[test]
    fn a_variable_that_is_a_whole_result_stands_for_its_value() {
        for body in [
            format!("a = strategy.equity\nx = {REQUEST}a)"),
            format!("a = strategy.equity * 2\nx = {REQUEST}a)"),
            format!("var a = strategy.equity\nx = {REQUEST}a)"),
            format!("if barstate.isconfirmed\n    a = strategy.equity\n    x = {REQUEST}a)"),
            format!("f() => strategy.equity\na = f()\nx = {REQUEST}a)"),
            format!("f() => [strategy.equity, close]\n[a, b] = f()\nx = {REQUEST}a)"),
            format!("a = strategy.equity\nx = {REQUEST}(a))"),
            format!("a = strategy.equity\n[x, y] = {REQUEST}[a, close])"),
            format!("a = strategy.equity\n[x, y] = {REQUEST}[close, a])"),
        ] {
            assert_eq!(refused(&body), message("strategy.equity"), "{body}");
        }
    }

    #[test]
    fn a_function_the_expression_calls_is_followed_all_the_way() {
        for body in [
            format!("f() => strategy.equity\nx = {REQUEST}f())"),
            format!("f() => strategy.equity * 2\nx = {REQUEST}f())"),
            format!("f() =>\n    a = strategy.equity\n    a\nx = {REQUEST}f())"),
            format!("f() => strategy.equity\ng() => f()\nx = {REQUEST}g())"),
        ] {
            assert_eq!(refused(&body), message("strategy.equity"), "{body}");
        }
    }

    #[test]
    fn what_tradingview_compiles_is_left_alone() {
        for body in [
            // A constant is not state.
            format!("x = {REQUEST}strategy.long == strategy.short ? close : open)"),
            // A variable is followed once, and only when it is the whole expression.
            format!("a = strategy.equity\nb = a * 2\nx = {REQUEST}b)"),
            format!("a = strategy.equity\nb = a\nx = {REQUEST}b)"),
            format!("a = strategy.equity\nx = {REQUEST}a + 1)"),
            format!("a = strategy.equity\n[x, y] = {REQUEST}[a + 1, close])"),
            format!("a = strategy.equity\nx = {REQUEST}a[1])"),
            format!("a = strategy.equity\nx = {REQUEST}close)\nplot(a)"),
            // A function that only reads a global, and a variable reassigned afterwards.
            format!("a = strategy.equity\nf() => a\nx = {REQUEST}f())"),
            format!("float a = 0.0\nx = {REQUEST}a)\na := strategy.equity"),
            // A parameter is not the global it is named like, and what a call passes in is not read here.
            format!("a = strategy.equity\nf(a) => {REQUEST}a)\nx = f(close)"),
            format!("f(s) => {REQUEST}s)\nx = f(strategy.equity)"),
            // Other requests and other arguments.
            "x = request.security_lower_tf(syminfo.tickerid, \"1\", strategy.equity)".into(),
            "x = request.security(strategy.equity > 0 ? syminfo.tickerid : \"BINANCE:ETHUSDT\", \"5\", close)".into(),
        ] {
            let found = errors(&body);
            assert!(found.is_empty(), "{body}: {found:?}");
        }
    }

    /// Not measured on TradingView: where the checker cannot tell which function a call picks,
    /// nothing is refused rather than a script that may be fine.
    #[test]
    fn what_cannot_be_told_apart_is_left_alone() {
        for body in [
            // Overloads that do not all read it.
            format!("f(int n) => strategy.equity\nf(float n) => close\nx = {REQUEST}f(1.0))"),
            // A method called on a value is not followed.
            format!("method m(float this) => strategy.equity\nx = {REQUEST}close.m())"),
        ] {
            let found = errors(&body);
            assert!(found.is_empty(), "{body}: {found:?}");
        }
    }

    /// Many helpers that call the next twice are walked once each, not once per path.
    #[test]
    fn a_long_chain_of_helpers_is_walked_once() {
        let mut body = String::from("f0() => close\n");
        for step in 1..40 {
            let previous = step - 1;
            body.push_str(&format!("f{step}() => f{previous}() + f{previous}()\n"));
        }
        body.push_str(&format!("x = {REQUEST}f39())"));
        assert!(errors(&body).is_empty());
    }

    /// TradingView puts the error where the expression starts, not at the read inside it.
    #[test]
    fn the_error_starts_where_the_expression_does() {
        for expression in ["strategy.equity * 2", "close > 0 ? strategy.equity : 0.0"] {
            let source = format!(
                "//@version=6\nstrategy(\"S\")\nx = request.security(syminfo.tickerid, \"5\", {expression})\nplot(1)\n"
            );
            let program = compiler::compile(&source, Limits::default(), &NoLibraries);
            let at = program
                .diagnostics
                .iter()
                .find(|diagnostic| diagnostic.code == "PINE3074")
                .map(|diagnostic| diagnostic.range.start.offset)
                .expect("an error");
            assert_eq!(Some(at), source.find(expression), "{expression}");
        }
    }
}
