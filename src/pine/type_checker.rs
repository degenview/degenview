//! Static type checker, run after parsing and before the script ever executes. Ported
//! from `PineTypeChecker.swift` + `PineTypeChecker+Expressions.swift`.

use std::collections::{HashMap, HashSet};

use super::ast::{
    AssignmentOperator, BinaryOperator, DeclarationMode, Expression, Literal, Qualifier, Statement,
    TypeAnnotation, UnaryOperator,
};
use super::builtin_types::{self, value_type_name};
use crate::pine::{Diagnostic, DiagnosticCategory, SourceRange, ValueType};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Inferred {
    /// A qualifier of None means the type is certain but the qualifier is not.
    Known(ValueType, Option<Qualifier>),
    Na,
    Unknown,
}

impl Inferred {
    pub fn kind(self) -> Option<ValueType> {
        match self {
            Self::Known(kind, _) => Some(kind),
            _ => None,
        }
    }

    pub fn qualifier(self) -> Option<Qualifier> {
        match self {
            Self::Known(_, qualifier) => qualifier,
            Self::Na => Some(Qualifier::Constant),
            Self::Unknown => None,
        }
    }
}

type Scope = HashMap<String, Inferred>;

pub struct TypeChecker {
    diagnostics: Vec<Diagnostic>,
    reassigned: HashSet<String>,
    functions: HashSet<String>,
}

impl TypeChecker {
    pub fn check(statements: &[Statement]) -> Vec<Diagnostic> {
        let mut checker = Self {
            diagnostics: Vec::new(),
            reassigned: assigned_names(statements),
            functions: function_names(statements),
        };
        let mut scope = Scope::new();
        checker.check_statements(statements, &mut scope);
        checker.diagnostics
    }

    // MARK: Statements

    fn check_statements(&mut self, statements: &[Statement], scope: &mut Scope) {
        for statement in statements {
            match statement {
                Statement::Declaration {
                    name,
                    annotation,
                    mode,
                    value,
                    ..
                } => self.declare(name, *annotation, *mode, value, scope),
                Statement::Assignment {
                    name, op, value, ..
                } => {
                    self.assign(name, *op, value, scope);
                }
                Statement::Expression(expression) => {
                    self.infer(expression, scope);
                }
                Statement::Conditional {
                    condition,
                    when_true,
                    when_false,
                    ..
                } => {
                    let value = self.infer(condition, scope);
                    self.require_bool(value, "if condition", condition.range());
                    self.block(when_true, scope);
                    self.block(when_false, scope);
                }
                Statement::ForRange {
                    variable,
                    from,
                    to,
                    step,
                    body,
                    ..
                } => {
                    let mut operands = vec![self.infer(from, scope), self.infer(to, scope)];
                    if let Some(step) = step {
                        operands.push(self.infer(step, scope));
                    }
                    let mut inner = scope.clone();
                    let all_int = operands.iter().all(|o| o.kind() == Some(ValueType::Int));
                    let any_float = operands.iter().any(|o| o.kind() == Some(ValueType::Float));
                    inner.insert(
                        variable.clone(),
                        if all_int {
                            Inferred::Known(ValueType::Int, Some(Qualifier::Series))
                        } else if any_float {
                            Inferred::Known(ValueType::Float, Some(Qualifier::Series))
                        } else {
                            Inferred::Unknown
                        },
                    );
                    self.check_statements(body, &mut inner);
                }
                Statement::ForIn {
                    index,
                    value,
                    collection,
                    body,
                    ..
                } => {
                    self.infer(collection, scope);
                    let mut inner = scope.clone();
                    if let Some(index) = index {
                        inner.insert(
                            index.clone(),
                            Inferred::Known(ValueType::Int, Some(Qualifier::Series)),
                        );
                    }
                    inner.insert(value.clone(), Inferred::Unknown);
                    self.check_statements(body, &mut inner);
                }
                Statement::WhileLoop {
                    condition, body, ..
                } => {
                    let value = self.infer(condition, scope);
                    self.require_bool(value, "while condition", condition.range());
                    self.block(body, scope);
                }
                Statement::Once {
                    condition, body, ..
                } => {
                    if let Some(condition) = condition {
                        let value = self.infer(condition, scope);
                        self.require_bool(value, "once condition", condition.range());
                    }
                    self.block(body, scope);
                }
                Statement::Switch { subject, arms, .. } => {
                    if let Some(subject) = subject {
                        self.infer(subject, scope);
                    }
                    for arm in arms {
                        if let Some(condition) = &arm.condition {
                            let value = self.infer(condition, scope);
                            if subject.is_none() {
                                self.require_bool(value, "switch arm condition", condition.range());
                            }
                        }
                        self.block(&arm.body, scope);
                    }
                }
                Statement::TupleDeclaration { names, value, .. } => {
                    self.infer(value, scope);
                    for name in names {
                        scope.insert(name.clone(), Inferred::Unknown);
                    }
                }
                Statement::Function {
                    name,
                    parameters,
                    body,
                    ..
                } => {
                    let mut inner = scope.clone();
                    for parameter in parameters {
                        if let Some(default) = &parameter.default_value {
                            self.infer(default, scope);
                        }
                        inner.insert(
                            parameter.name.clone(),
                            parameter
                                .kind()
                                .and_then(|kind| {
                                    if matches!(
                                        kind,
                                        ValueType::Object | ValueType::Map | ValueType::Matrix
                                    ) {
                                        None
                                    } else {
                                        Some(Inferred::Known(kind, None))
                                    }
                                })
                                .unwrap_or(Inferred::Unknown),
                        );
                    }
                    self.check_statements(body, &mut inner);
                    scope.insert(name.clone(), Inferred::Unknown);
                }
                Statement::FieldAssignment { target, value, .. } => {
                    self.infer(target, scope);
                    self.infer(value, scope);
                }
                Statement::LoopControl(_, _)
                | Statement::TypeDeclaration { .. }
                | Statement::EnumDeclaration { .. } => {}
            }
        }
    }

    fn block(&mut self, statements: &[Statement], scope: &Scope) {
        let mut inner = scope.clone();
        self.check_statements(statements, &mut inner);
    }

    fn declare(
        &mut self,
        name: &str,
        annotation: TypeAnnotation,
        mode: DeclarationMode,
        expression: &Expression,
        scope: &mut Scope,
    ) {
        let value = self.infer(expression, scope);
        let series = mode != DeclarationMode::Ordinary || self.reassigned.contains(name);

        if let Some(wanted) = annotation.qualifier
            && let Some(actual) = value.qualifier()
            && actual > wanted
        {
            self.error(
                "PINE3031",
                format!(
                    "Cannot assign a {} value to a {} variable '{name}'.",
                    describe_qualifier(actual),
                    describe_qualifier(wanted)
                ),
                expression.range(),
            );
        }
        if annotation.kind == Some(ValueType::Object)
            || annotation.kind == Some(ValueType::Map)
            || annotation.kind == Some(ValueType::Matrix)
        {
            // Which type an object is, and whether a value fits it, is not tracked.
            scope.insert(name.to_owned(), Inferred::Unknown);
        } else if let Some(declared) = annotation.kind {
            if !fits(value, declared) {
                self.error(
                    "PINE3030",
                    format!(
                        "Cannot assign {} to {} variable '{name}'.",
                        describe(value),
                        value_type_name(declared)
                    ),
                    expression.range(),
                );
            }
            scope.insert(
                name.to_owned(),
                Inferred::Known(
                    declared,
                    annotation.qualifier.or(if series {
                        Some(Qualifier::Series)
                    } else {
                        value.qualifier()
                    }),
                ),
            );
        } else if let Inferred::Known(kind, qualifier) = value {
            scope.insert(
                name.to_owned(),
                Inferred::Known(
                    kind,
                    if series {
                        Some(Qualifier::Series)
                    } else {
                        annotation.qualifier.or(qualifier)
                    },
                ),
            );
        } else {
            scope.insert(name.to_owned(), Inferred::Unknown);
        }
    }

    fn assign(
        &mut self,
        name: &str,
        op: AssignmentOperator,
        expression: &Expression,
        scope: &Scope,
    ) {
        let value = self.infer(expression, scope);
        let Some(target) = scope.get(name) else {
            return;
        };
        let Inferred::Known(kind, _) = *target else {
            return;
        };
        let mut result = value;
        if let AssignmentOperator::Compound(underlying) = op {
            result = self.binary(
                underlying,
                *target,
                value,
                expression.range(),
                expression.range(),
                expression.range(),
            );
        }
        if !fits(result, kind) {
            self.error(
                "PINE3032",
                format!(
                    "Cannot assign {} to {} variable '{name}'.",
                    describe(result),
                    value_type_name(kind)
                ),
                expression.range(),
            );
        }
    }

    fn error(&mut self, code: &str, message: String, range: SourceRange) {
        self.diagnostics.push(Diagnostic::error(
            code,
            DiagnosticCategory::Semantic,
            message,
            range,
        ));
    }

    // MARK: Expressions

    fn infer(&mut self, expression: &Expression, scope: &Scope) -> Inferred {
        match expression {
            Expression::Literal(value, _) => match value {
                Literal::Int(_) => Inferred::Known(ValueType::Int, Some(Qualifier::Constant)),
                Literal::Float(_) => Inferred::Known(ValueType::Float, Some(Qualifier::Constant)),
                Literal::Bool(_) => Inferred::Known(ValueType::Bool, Some(Qualifier::Constant)),
                Literal::String(_) => Inferred::Known(ValueType::String, Some(Qualifier::Constant)),
                Literal::Color(_) => Inferred::Known(ValueType::Color, Some(Qualifier::Constant)),
                Literal::Na => Inferred::Na,
            },
            Expression::Identifier(name, _) => scope
                .get(name)
                .copied()
                .unwrap_or_else(|| builtin_types::identifier(name)),
            Expression::Unary { op, operand, range } => {
                let operand_inferred = self.infer(operand, scope);
                self.unary(*op, operand_inferred, *range)
            }
            Expression::Binary {
                lhs,
                op,
                rhs,
                range,
                ..
            } => {
                let a = self.infer(lhs, scope);
                let b = self.infer(rhs, scope);
                self.binary(*op, a, b, lhs.range(), rhs.range(), *range)
            }
            Expression::Ternary {
                condition,
                when_true,
                when_false,
                ..
            } => {
                let test = self.infer(condition, scope);
                let a = self.infer(when_true, scope);
                let b = self.infer(when_false, scope);
                self.require_bool(test, "Ternary condition", condition.range());
                if test == Inferred::Unknown {
                    return Inferred::Unknown;
                }
                unify(a, b, test.qualifier())
            }
            Expression::Call {
                name,
                arguments,
                range,
                ..
            } => self.call(name, arguments, scope, *range),
            Expression::History { base, offset, .. } => {
                let value = self.infer(base, scope);
                self.infer(offset, scope);
                if let Inferred::Known(kind, _) = value {
                    return Inferred::Known(kind, Some(Qualifier::Series));
                }
                Inferred::Unknown
            }
            Expression::Tuple(items, _) => {
                for item in items {
                    self.infer(item, scope);
                }
                Inferred::Unknown
            }
            Expression::Member { base, .. } => {
                self.infer(base, scope);
                Inferred::Unknown
            }
            Expression::MethodCall {
                receiver,
                arguments,
                ..
            } => {
                self.infer(receiver, scope);
                for argument in arguments {
                    self.infer(&argument.value, scope);
                }
                Inferred::Unknown
            }
            Expression::Statement(statement, _) => {
                self.block(std::slice::from_ref(statement.as_ref()), scope);
                Inferred::Unknown
            }
        }
    }

    // MARK: Operators

    fn require_bool(&mut self, value: Inferred, what: &str, range: SourceRange) {
        if let Inferred::Known(kind, _) = value
            && kind != ValueType::Bool
        {
            self.error(
                "PINE3033",
                format!("{what} must be bool, got {}.", value_type_name(kind)),
                range,
            );
        }
    }

    fn unary(&mut self, op: UnaryOperator, operand: Inferred, range: SourceRange) -> Inferred {
        let Inferred::Known(kind, qualifier) = operand else {
            return Inferred::Unknown;
        };
        if op == UnaryOperator::Not {
            self.require_bool(operand, "Operand of 'not'", range);
            return Inferred::Known(ValueType::Bool, qualifier);
        }
        if !is_numeric(kind) {
            let symbol = if op == UnaryOperator::Negate {
                "-"
            } else {
                "+"
            };
            self.error(
                "PINE3034",
                format!(
                    "Operator '{symbol}' cannot be applied to {}.",
                    value_type_name(kind)
                ),
                range,
            );
            return Inferred::Unknown;
        }
        operand
    }

    fn binary(
        &mut self,
        op: BinaryOperator,
        a: Inferred,
        b: Inferred,
        left: SourceRange,
        right: SourceRange,
        range: SourceRange,
    ) -> Inferred {
        if op.is_comparison() && (a == Inferred::Na || b == Inferred::Na) {
            return Inferred::Known(ValueType::Bool, Some(Qualifier::Series));
        }
        let (Inferred::Known(x, qx), Inferred::Known(y, qy)) = (a, b) else {
            return Inferred::Unknown;
        };
        let qualifier = max_qualifier(qx, qy);

        match op {
            BinaryOperator::And | BinaryOperator::Or => {
                let symbol = op.symbol();
                self.require_bool(a, &format!("Operand of '{symbol}'"), left);
                self.require_bool(b, &format!("Operand of '{symbol}'"), right);
                return Inferred::Known(ValueType::Bool, qualifier);
            }
            BinaryOperator::Equal | BinaryOperator::NotEqual => {
                if x == y || (is_numeric(x) && is_numeric(y)) {
                    return Inferred::Known(ValueType::Bool, qualifier);
                }
            }
            BinaryOperator::Less
            | BinaryOperator::LessEqual
            | BinaryOperator::Greater
            | BinaryOperator::GreaterEqual => {
                if is_numeric(x) && is_numeric(y) {
                    return Inferred::Known(ValueType::Bool, qualifier);
                }
            }
            BinaryOperator::Add => {
                if x == ValueType::String && y == ValueType::String {
                    return Inferred::Known(ValueType::String, qualifier);
                }
                if is_numeric(x) && is_numeric(y) {
                    let kind = if x == ValueType::Int && y == ValueType::Int {
                        ValueType::Int
                    } else {
                        ValueType::Float
                    };
                    return Inferred::Known(kind, qualifier);
                }
            }
            BinaryOperator::Subtract | BinaryOperator::Multiply | BinaryOperator::Modulo => {
                if is_numeric(x) && is_numeric(y) {
                    let kind = if x == ValueType::Int && y == ValueType::Int {
                        ValueType::Int
                    } else {
                        ValueType::Float
                    };
                    return Inferred::Known(kind, qualifier);
                }
            }
            BinaryOperator::Divide => {
                if is_numeric(x) && is_numeric(y) {
                    // Two ints may divide to an int in what TradingView accepts.
                    return if x == ValueType::Int && y == ValueType::Int {
                        Inferred::Unknown
                    } else {
                        Inferred::Known(ValueType::Float, qualifier)
                    };
                }
            }
            BinaryOperator::Power => {
                if is_numeric(x) && is_numeric(y) {
                    return Inferred::Known(ValueType::Float, qualifier);
                }
            }
        }
        self.error(
            "PINE3034",
            format!(
                "Operator '{}' cannot be applied to {} and {}.",
                op.symbol(),
                value_type_name(x),
                value_type_name(y)
            ),
            range,
        );
        Inferred::Unknown
    }

    // MARK: Calls

    fn call(
        &mut self,
        name: &str,
        arguments: &[super::ast::Argument],
        scope: &Scope,
        range: SourceRange,
    ) -> Inferred {
        let values: Vec<Inferred> = arguments
            .iter()
            .map(|argument| self.infer(&argument.value, scope))
            .collect();
        if self.functions.contains(name) {
            return Inferred::Unknown;
        }
        if let Some(kind) = builtin_types::input_result(name) {
            if name != "input.source" {
                self.check_input_default(name, arguments, &values);
            }
            return Inferred::Known(
                kind,
                if name == "input.source" {
                    Some(Qualifier::Series)
                } else {
                    Some(Qualifier::Input)
                },
            );
        }
        if name == "alert" || name == "alertcondition" {
            self.check_alert_message(name, arguments, &values, range);
        }
        let mut qualifier = Some(Qualifier::Constant);
        for value in &values {
            qualifier = max_qualifier(qualifier, value.qualifier());
        }
        builtin_types::call(name, arguments, &values, qualifier)
    }

    fn check_alert_message(
        &mut self,
        name: &str,
        arguments: &[super::ast::Argument],
        values: &[Inferred],
        range: SourceRange,
    ) {
        let position = if name == "alert" { 0 } else { 2 };
        let positional: Vec<usize> = arguments
            .iter()
            .enumerate()
            .filter(|(_, argument)| argument.name.is_none())
            .map(|(index, _)| index)
            .collect();
        let index = arguments
            .iter()
            .position(|argument| argument.name.as_deref() == Some("message"))
            .or_else(|| positional.get(position).copied());
        let Some(index) = index else {
            return;
        };
        if let Inferred::Known(kind, _) = values[index]
            && kind != ValueType::String
        {
            self.error(
                "PINE3036",
                format!(
                    "{name}() message must be a string, not {}.",
                    value_type_name(kind)
                ),
                range,
            );
        }
    }

    fn check_input_default(
        &mut self,
        name: &str,
        arguments: &[super::ast::Argument],
        values: &[Inferred],
    ) {
        let Some(index) = arguments.iter().position(|argument| {
            argument.name.is_none() || argument.name.as_deref() == Some("defval")
        }) else {
            return;
        };
        let Inferred::Known(kind, _) = values[index] else {
            return;
        };
        let accepted: &[ValueType] = match name {
            "input.float" | "input.price" => &[ValueType::Float, ValueType::Int],
            "input.bool" => &[ValueType::Bool],
            "input.string" | "input.session" | "input.symbol" | "input.timeframe"
            | "input.text_area" => &[ValueType::String],
            "input.color" => &[ValueType::Color],
            _ => &[ValueType::Int],
        };
        if !accepted.contains(&kind) {
            self.error(
                "PINE3035",
                format!(
                    "{name}() default must be {}, got {}.",
                    value_type_name(accepted[0]),
                    value_type_name(kind)
                ),
                arguments[index].value.range(),
            );
        }
    }
}

// MARK: Helpers

fn is_numeric(kind: ValueType) -> bool {
    matches!(kind, ValueType::Int | ValueType::Float)
}

fn max_qualifier(a: Option<Qualifier>, b: Option<Qualifier>) -> Option<Qualifier> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.max(b)),
        _ => None,
    }
}

fn unify(a: Inferred, b: Inferred, test: Option<Qualifier>) -> Inferred {
    match (a, b) {
        (Inferred::Known(x, qx), Inferred::Known(y, qy)) => {
            let qualifier = max_qualifier(test, max_qualifier(qx, qy));
            if x == y {
                return Inferred::Known(x, qualifier);
            }
            if is_numeric(x) && is_numeric(y) {
                return Inferred::Known(ValueType::Float, qualifier);
            }
            Inferred::Unknown
        }
        (Inferred::Known(x, qx), Inferred::Na) | (Inferred::Na, Inferred::Known(x, qx)) => {
            Inferred::Known(x, max_qualifier(test, qx))
        }
        (Inferred::Na, Inferred::Na) => Inferred::Na,
        _ => Inferred::Unknown,
    }
}

fn fits(value: Inferred, declared: ValueType) -> bool {
    let Inferred::Known(kind, _) = value else {
        return true;
    };
    kind == declared || (declared == ValueType::Float && kind == ValueType::Int)
}

fn describe(value: Inferred) -> String {
    match value {
        Inferred::Known(kind, _) => value_type_name(kind).to_owned(),
        Inferred::Na => "na".to_owned(),
        Inferred::Unknown => "an unknown type".to_owned(),
    }
}

fn describe_qualifier(qualifier: Qualifier) -> &'static str {
    match qualifier {
        Qualifier::Constant => "const",
        Qualifier::Input => "input",
        Qualifier::Simple => "simple",
        Qualifier::Series => "series",
    }
}

fn assigned_names(statements: &[Statement]) -> HashSet<String> {
    let mut names = HashSet::new();
    for statement in statements {
        if let Statement::Assignment { name, .. } = statement {
            names.insert(name.clone());
        }
        for block in nested_blocks(statement) {
            names.extend(assigned_names(block));
        }
    }
    names
}

fn nested_blocks(statement: &Statement) -> Vec<&[Statement]> {
    match statement {
        Statement::Conditional {
            when_true,
            when_false,
            ..
        } => vec![when_true.as_slice(), when_false.as_slice()],
        Statement::ForRange { body, .. }
        | Statement::ForIn { body, .. }
        | Statement::WhileLoop { body, .. }
        | Statement::Once { body, .. }
        | Statement::Function { body, .. } => vec![body.as_slice()],
        Statement::Switch { arms, .. } => arms.iter().map(|arm| arm.body.as_slice()).collect(),
        _ => Vec::new(),
    }
}

fn function_names(statements: &[Statement]) -> HashSet<String> {
    statements
        .iter()
        .filter_map(|statement| match statement {
            Statement::Function { name, .. } => Some(name.clone()),
            _ => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pine::{Lexer, Parser};

    fn check(source: &str) -> Vec<Diagnostic> {
        let lexed = Lexer::new(source).lex();
        let mut parser = Parser::new(lexed.tokens, crate::pine::Limits::default(), HashSet::new());
        let (statements, _) = parser.parse();
        TypeChecker::check(&statements)
    }

    #[test]
    fn accepts_a_well_typed_script() {
        let diagnostics = check("x = 1\nfloat y = x\n");
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
    }

    #[test]
    fn rejects_a_string_initializer_for_an_int() {
        let diagnostics = check("int x = \"hi\"\n");
        assert!(
            diagnostics.iter().any(|d| d.code == "PINE3030"),
            "{diagnostics:?}"
        );
    }

    #[test]
    fn rejects_a_non_bool_condition() {
        let diagnostics = check("if 1\n    x = 2\n");
        assert!(
            diagnostics.iter().any(|d| d.code == "PINE3033"),
            "{diagnostics:?}"
        );
    }

    #[test]
    fn rejects_arithmetic_on_strings() {
        let diagnostics = check("x = \"a\" + 1\n");
        assert!(
            diagnostics.iter().any(|d| d.code == "PINE3034"),
            "{diagnostics:?}"
        );
    }

    #[test]
    fn int_fits_float_but_not_the_reverse() {
        let diagnostics = check("float x = 1\nint y = 1.5\n");
        assert_eq!(
            diagnostics.iter().filter(|d| d.code == "PINE3030").count(),
            1
        );
    }
}
