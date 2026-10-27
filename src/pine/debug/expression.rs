//! Expressions typed into the debugger: watches, breakpoint conditions, logpoint templates, hover.
//!
//! They are parsed by the Pine parser and run by the Pine interpreter, so a name, an operator, `na`
//! and `close[1]` mean exactly what they mean in the script. What they may not do is change the
//! script. The interpreter keeps state in places an expression can reach without saying so
//! (`ta.*` keeps a window per call site, `x[n]` on an expression writes a history slot, a user
//! function rewrites the variable table), so instead of trying to undo effects afterwards, an
//! expression is accepted only if everything in it is known to be read-only. Anything else is
//! refused, with the reason, before it runs.

use std::collections::HashSet;

use crate::pine::ast::{Argument, Expression, Statement};
use crate::pine::{Lexer, Limits, Parser};

/// Call sites of debugger expressions are numbered from here, far above any script's.
const CALL_SITE_BASE: usize = 1 << 40;
const MAX_LENGTH: usize = 2_000;

/// An expression the debugger will run: parsed, and checked read-only.
#[derive(Clone, Debug, PartialEq)]
pub struct DebugExpression {
    source: String,
    expression: Expression,
}

impl DebugExpression {
    pub fn source(&self) -> &str {
        &self.source
    }

    pub(crate) fn expression(&self) -> &Expression {
        &self.expression
    }
}

/// Why text is not an expression the debugger can run.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExpressionError {
    pub message: String,
}

impl ExpressionError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl std::fmt::Display for ExpressionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ExpressionError {}

/// Parses `text` as one Pine expression and checks it is safe to run against a stopped script.
pub fn parse(text: &str) -> Result<DebugExpression, ExpressionError> {
    let text = text.trim();
    if text.is_empty() {
        return Err(ExpressionError::new("Enter an expression."));
    }
    if text.len() > MAX_LENGTH {
        return Err(ExpressionError::new("The expression is too long."));
    }
    if text.contains('\n') {
        return Err(ExpressionError::new("An expression must fit on one line."));
    }
    let limits = Limits::default();
    let lexed = Lexer::with_limits(text, limits).lex();
    if let Some(error) = lexed.diagnostics.first() {
        return Err(ExpressionError::new(error.message.clone()));
    }
    let mut parser =
        Parser::new(lexed.tokens, limits, HashSet::new()).with_call_site_base(CALL_SITE_BASE);
    let (mut statements, diagnostics) = parser.parse();
    if let Some(error) = diagnostics.first() {
        return Err(ExpressionError::new(error.message.clone()));
    }
    let expression = match (statements.pop(), statements.is_empty()) {
        (Some(Statement::Expression(expression)), true) => expression,
        _ => {
            return Err(ExpressionError::new(
                "Only an expression can be evaluated, not a declaration or assignment.",
            ));
        }
    };
    check(&expression)?;
    Ok(DebugExpression {
        source: text.to_owned(),
        expression,
    })
}

fn check(expression: &Expression) -> Result<(), ExpressionError> {
    match expression {
        Expression::Literal(..) | Expression::Identifier(..) => Ok(()),
        Expression::Unary { operand, .. } => check(operand),
        Expression::Binary { lhs, rhs, .. } => check(lhs).and_then(|()| check(rhs)),
        Expression::Ternary {
            condition,
            when_true,
            when_false,
            ..
        } => check(condition)
            .and_then(|()| check(when_true))
            .and_then(|()| check(when_false)),
        Expression::History { base, offset, .. } => {
            // A history of anything but a plain name is kept in a slot named after where the
            // expression starts in the script; ours would write over the script's own.
            if !matches!(base.as_ref(), Expression::Identifier(..)) {
                return Err(ExpressionError::new(
                    "Only a variable can be indexed with [ ] while debugging.",
                ));
            }
            check(offset)
        }
        Expression::Tuple(items, _) => items.iter().try_for_each(check),
        Expression::Member { base, .. } => check(base),
        Expression::Call {
            name, arguments, ..
        } => {
            check_call(name)?;
            check_arguments(arguments)
        }
        Expression::MethodCall {
            receiver,
            name,
            arguments,
            ..
        } => {
            check_method(name)?;
            check(receiver)?;
            check_arguments(arguments)
        }
        Expression::Statement(..) => Err(ExpressionError::new(
            "An if or switch block cannot be evaluated while debugging.",
        )),
    }
}

fn check_arguments(arguments: &[Argument]) -> Result<(), ExpressionError> {
    arguments
        .iter()
        .try_for_each(|argument| check(&argument.value))
}

/// Calls that read and compute and nothing else.
fn check_call(name: &str) -> Result<(), ExpressionError> {
    let Some((namespace, function)) = name.split_once('.') else {
        return if matches!(name, "na" | "nz" | "int" | "float" | "bool") {
            Ok(())
        } else if name == "fixnan" {
            Err(ExpressionError::new(
                "fixnan() keeps state at each call site in the script, so it is unavailable while debugging.",
            ))
        } else if is_effectful(name) {
            Err(effect(name))
        } else {
            Err(ExpressionError::new(format!(
                "'{name}' is a function of the script or has side effects: functions cannot be called while debugging."
            )))
        };
    };
    match namespace {
        "math" if function != "random" => Ok(()),
        "str" | "color" => Ok(()),
        "array" | "map" | "matrix" if READ_ONLY_METHODS.contains(&function) => Ok(()),
        "timeframe" if matches!(function, "in_seconds" | "from_seconds") => Ok(()),
        "ta" => Err(ExpressionError::new(format!(
            "{name}() keeps state at each call site in the script, so it is unavailable while debugging."
        ))),
        "math" | "array" | "map" | "matrix" | "timeframe" => Err(ExpressionError::new(format!(
            "{name}() can change the script or its state and cannot be called while debugging."
        ))),
        "strategy" | "alert" | "log" | "request" | "line" | "label" | "box" | "table"
        | "linefill" | "polyline" | "chart" | "input" => Err(effect(name)),
        // `variable.method(…)`: a method of a value, which must be one that only reads.
        _ => check_method(function),
    }
}

fn check_method(name: &str) -> Result<(), ExpressionError> {
    if READ_ONLY_METHODS.contains(&name) {
        Ok(())
    } else {
        Err(ExpressionError::new(format!(
            "'{name}' can change the script or its state and cannot be called while debugging."
        )))
    }
}

fn is_effectful(name: &str) -> bool {
    matches!(
        name,
        "plot"
            | "plotshape"
            | "plotchar"
            | "plotarrow"
            | "plotcandle"
            | "plotbar"
            | "bgcolor"
            | "barcolor"
            | "fill"
            | "hline"
            | "alert"
            | "alertcondition"
            | "indicator"
            | "strategy"
    )
}

fn effect(name: &str) -> ExpressionError {
    ExpressionError::new(format!(
        "{name}() draws, trades, alerts or logs, and cannot be called while debugging."
    ))
}

/// Methods and `array.*` / `map.*` / `matrix.*` functions that never write and never allocate a
/// new object (so `array.slice`, `array.copy` and `map.keys` are not here).
const READ_ONLY_METHODS: &[&str] = &[
    "get",
    "size",
    "first",
    "last",
    "includes",
    "every",
    "some",
    "contains",
    "indexof",
    "lastindexof",
    "avg",
    "min",
    "max",
    "median",
    "mode",
    "sum",
    "stdev",
    "variance",
    "range",
    "covariance",
    "percentile_linear_interpolation",
    "percentile_nearest_rank",
    "percentrank",
    "binary_search",
    "binary_search_leftmost",
    "binary_search_rightmost",
    "join",
    "rows",
    "columns",
    "elements_count",
    "is_empty",
];

#[cfg(test)]
mod tests {
    use super::*;

    fn accepted(text: &str) {
        parse(text).unwrap_or_else(|error| panic!("{text}: {error}"));
    }

    fn refused(text: &str) -> String {
        parse(text).expect_err(text).message
    }

    #[test]
    fn plain_expressions_are_accepted() {
        for text in [
            "close - open",
            "bar_index == 250",
            "close > open and volume > 100",
            "na(close[1]) ? 0 : nz(close[2])",
            "math.max(open, close) - math.abs(high - low)",
            "str.tostring(close, \"#.##\")",
            "color.new(color.red, 50)",
            "myArray.get(0) + myArray.size()",
            "array.get(prices, 1)",
            "map.get(counts, \"k\")",
            "matrix.get(grid, 0, 1)",
            "position.entryPrice",
            "[close, open]",
        ] {
            accepted(text);
        }
    }

    #[test]
    fn indexing_is_for_variables_only() {
        accepted("close[1]");
        accepted("slow[10]");
        assert!(refused("(close + open)[1]").contains("indexed"));
        assert!(refused("math.max(a, b)[2]").contains("indexed"));
    }

    #[test]
    fn stateful_and_effectful_calls_are_refused_with_the_reason() {
        assert!(refused("ta.sma(close, 20)").contains("keeps state"));
        assert!(refused("strategy.entry(\"L\", strategy.long)").contains("trades"));
        assert!(refused("plot(close)").contains("draws"));
        assert!(refused("alert(\"x\")").contains("alerts"));
        assert!(refused("log.info(\"x\")").contains("logs"));
        assert!(refused("request.security(syminfo.tickerid, \"D\", close)").contains("logs"));
        assert!(refused("line.new(1, 2, 3, 4)").contains("draws"));
        assert!(refused("myArray.push(1)").contains("change"));
        assert!(refused("array.push(a, 1)").contains("change"));
        assert!(refused("map.put(m, \"k\", 1)").contains("change"));
        assert!(refused("array.slice(a, 0, 2)").contains("change"));
        assert!(refused("math.random()").contains("change"));
        assert!(refused("myFunction(close)").contains("functions cannot be called"));
        assert!(refused("fixnan(close)").contains("keeps state"));
    }

    #[test]
    fn declarations_assignments_and_blocks_are_not_expressions() {
        assert!(refused("x = 1").contains("Only an expression"));
        assert!(refused("x := 1").contains("Only an expression"));
        assert!(refused("a\nb").contains("one line"));
        assert!(refused("").contains("Enter"));
        assert!(refused("   ").contains("Enter"));
    }

    #[test]
    fn syntax_errors_say_what_is_wrong() {
        assert!(!refused("close +").is_empty());
        assert!(!refused("(close").is_empty());
        assert!(!refused("close $ open").is_empty());
    }

    #[test]
    fn call_sites_are_numbered_apart_from_the_scripts() {
        let parsed = parse("math.max(1, 2) + math.min(3, 4)").unwrap();
        let mut sites = Vec::new();
        fn collect(expression: &Expression, sites: &mut Vec<usize>) {
            if let Expression::Call { site, .. } = expression {
                sites.push(*site);
            }
            for child in crate::pine::traversal::children(expression) {
                collect(child, sites);
            }
        }
        collect(parsed.expression(), &mut sites);
        assert_eq!(sites.len(), 2);
        assert!(sites.iter().all(|site| *site > CALL_SITE_BASE));
    }
}
