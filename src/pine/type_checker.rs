//! Static type checker, run after parsing and before the script ever executes. Ported
//! from `PineTypeChecker.swift` + `PineTypeChecker+Expressions.swift`.

use std::collections::{HashMap, HashSet};

use super::ast::{
    Argument, AssignmentOperator, BinaryOperator, DeclarationMode, Expression, FunctionKind,
    Literal, Qualifier, Statement, TypeAnnotation, UnaryOperator,
};
use super::builtin_types::{self, value_type_name};
use super::callables::{Callables, Given, NoFit, Resolution};
use super::registry;
use super::types::{Ty, TyId, Types};
use crate::pine::{Diagnostic, DiagnosticCategory, SourceRange, ValueType};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Inferred {
    /// A qualifier of None means the type is certain but the qualifier is not.
    Known(ValueType, Option<Qualifier>),
    /// A type with more to it than its kind (`array<float>`, a user type), by handle into the
    /// checker's [`Types`]. `kind` is what the checks that only know kinds see: None for types they
    /// have never judged (user types, enums, maps, matrices).
    Typed {
        ty: TyId,
        kind: Option<ValueType>,
        qualifier: Option<Qualifier>,
    },
    Na,
    Unknown,
}

impl Inferred {
    pub fn kind(self) -> Option<ValueType> {
        match self {
            Self::Known(kind, _) => Some(kind),
            Self::Typed { kind, .. } => kind,
            _ => None,
        }
    }

    pub fn qualifier(self) -> Option<Qualifier> {
        match self {
            Self::Known(_, qualifier) | Self::Typed { qualifier, .. } => qualifier,
            Self::Na => Some(Qualifier::Constant),
            Self::Unknown => None,
        }
    }
}

/// What checking a script found.
pub struct Checked {
    pub diagnostics: Vec<Diagnostic>,
    /// The callable each call site selected, by call-site number and by position among the script's
    /// top-level functions (see [`Callables`]). A call the checker could not settle is absent.
    pub resolutions: HashMap<usize, usize>,
}

type Scope = HashMap<String, Inferred>;

pub struct TypeChecker {
    diagnostics: Vec<Diagnostic>,
    reassigned: HashSet<String>,
    types: Types,
    enums: HashSet<String>,
    /// The fields of each user type the script declares, in order.
    udts: HashMap<String, Vec<(String, Ty)>>,
    callables: Callables,
    /// What each callable's body evaluates to, once the body has been checked.
    returns: Vec<Inferred>,
    /// The callable whose body is being checked.
    current: Option<usize>,
    /// Calls between callables, for finding the ones that reach themselves.
    edges: Vec<(usize, usize, SourceRange)>,
    resolutions: HashMap<usize, usize>,
    /// The value of the statement just checked, which a function returns when it is the last.
    last: Inferred,
    /// Whether the script imports a library, whose methods this checker cannot see.
    imports: bool,
}

impl TypeChecker {
    pub fn check(statements: &[Statement]) -> Vec<Diagnostic> {
        Self::check_program(statements, false).diagnostics
    }

    /// Checks `statements`; `imports` says the script imports libraries.
    pub fn check_program(statements: &[Statement], imports: bool) -> Checked {
        let enums: HashSet<String> = statements
            .iter()
            .filter_map(|statement| match statement {
                Statement::EnumDeclaration { name, .. } => Some(name.clone()),
                _ => None,
            })
            .collect();
        let udts = statements
            .iter()
            .filter_map(|statement| match statement {
                Statement::TypeDeclaration { name, fields, .. } => Some((
                    name.clone(),
                    fields
                        .iter()
                        .map(|field| {
                            let ty = field
                                .type_ref
                                .as_ref()
                                .map_or(Ty::Any, |reference| Ty::from_ref(reference, &enums));
                            (field.name.clone(), ty)
                        })
                        .collect(),
                )),
                _ => None,
            })
            .collect();
        let callables = Callables::collect(statements, &enums);
        let mut checker = Self {
            diagnostics: Vec::new(),
            reassigned: assigned_names(statements),
            types: Types::default(),
            enums,
            udts,
            returns: vec![Inferred::Unknown; callables.len()],
            callables,
            current: None,
            edges: Vec::new(),
            resolutions: HashMap::new(),
            last: Inferred::Unknown,
            imports,
        };
        let mut scope = Scope::new();
        checker.check_statements(statements, &mut scope);
        checker.report_recursion();
        Checked {
            diagnostics: checker.diagnostics,
            resolutions: checker.resolutions,
        }
    }

    // MARK: Statements

    fn check_statements(&mut self, statements: &[Statement], scope: &mut Scope) {
        for statement in statements {
            self.last = Inferred::Unknown;
            match statement {
                Statement::Declaration {
                    name,
                    annotation,
                    mode,
                    value,
                    ..
                } => {
                    self.declare(name, *annotation, *mode, value, scope);
                    let declared = scope.get(name).copied().unwrap_or(Inferred::Unknown);
                    self.last = declared;
                }
                Statement::Assignment {
                    name, op, value, ..
                } => {
                    self.assign(name, *op, value, scope);
                }
                Statement::Expression(expression) => {
                    let value = self.infer(expression, scope);
                    self.last = value;
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
                    let collection = self.infer(collection, scope);
                    let mut inner = scope.clone();
                    if let Some(index) = index {
                        inner.insert(
                            index.clone(),
                            Inferred::Known(ValueType::Int, Some(Qualifier::Series)),
                        );
                    }
                    let element = match self.ty_of(collection) {
                        Ty::Array(element) => self.inferred_of(&element, Some(Qualifier::Series)),
                        _ => Inferred::Unknown,
                    };
                    inner.insert(value.clone(), element);
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
                    range,
                    ..
                } => {
                    let mut inner = scope.clone();
                    for parameter in parameters {
                        if let Some(default) = &parameter.default_value {
                            self.infer(default, scope);
                        }
                        let declared = parameter
                            .type_ref
                            .as_ref()
                            .map(|reference| Ty::from_ref(reference, &self.enums));
                        let value = match declared {
                            Some(Ty::Scalar(kind)) => Inferred::Known(kind, None),
                            Some(ty) => self.inferred_of(&ty, None),
                            None => Inferred::Unknown,
                        };
                        inner.insert(parameter.name.clone(), value);
                    }
                    let outer = self.current;
                    self.current = self.callables.at(range.start.offset);
                    self.check_statements(body, &mut inner);
                    if let Some(id) = self.current {
                        self.returns[id] = self.last;
                    }
                    self.current = outer;
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
            // Only an expression or a declaration hands its value to the function around it.
            if !matches!(
                statement,
                Statement::Expression(_) | Statement::Declaration { .. }
            ) {
                self.last = Inferred::Unknown;
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
            // Whether a value fits the annotated type is not judged, but a value whose type is
            // known keeps it.
            let kept = match value {
                Inferred::Typed { ty, kind, .. } => Inferred::Typed {
                    ty,
                    kind,
                    qualifier: Some(Qualifier::Series),
                },
                _ => Inferred::Unknown,
            };
            scope.insert(name.to_owned(), kept);
        } else if let Inferred::Typed { ty, kind, .. } = value {
            scope.insert(
                name.to_owned(),
                Inferred::Typed {
                    ty,
                    kind,
                    qualifier: Some(Qualifier::Series),
                },
            );
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
            Expression::Identifier(name, _) => {
                if let Some(value) = scope.get(name) {
                    *value
                } else if let Some(value) = self.field_path(name, scope) {
                    value
                } else {
                    builtin_types::identifier(name)
                }
            }
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
                site,
                range,
                ..
            } => self.call(name, arguments, *site, scope, *range),
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
            Expression::Member { base, name, .. } => {
                let base = self.infer(base, scope);
                let ty = self.ty_of(base);
                match self.field_type(&ty, name) {
                    Some(field) => self.inferred_of(&field, Some(Qualifier::Series)),
                    None => Inferred::Unknown,
                }
            }
            Expression::MethodCall {
                receiver,
                name,
                arguments,
                site,
                range,
            } => {
                let receiver = self.infer(receiver, scope);
                let values: Vec<Inferred> = arguments
                    .iter()
                    .map(|argument| self.infer(&argument.value, scope))
                    .collect();
                self.method_call(receiver, name, arguments, &values, *site, *range)
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
        let (Some(kind), qualifier) = (operand.kind(), operand.qualifier()) else {
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
        let (Some(x), Some(y)) = (a.kind(), b.kind()) else {
            return Inferred::Unknown;
        };
        let qualifier = max_qualifier(a.qualifier(), b.qualifier());

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
        arguments: &[Argument],
        site: usize,
        scope: &Scope,
        range: SourceRange,
    ) -> Inferred {
        let values: Vec<Inferred> = arguments
            .iter()
            .map(|argument| self.infer(&argument.value, scope))
            .collect();
        // `levels.push(x)` reads as one dotted name: a variable (or a builtin series) and a method.
        if let Some((path, member)) = name.rsplit_once('.') {
            // A variable named like a namespace (`timeframe.change(timeframe)` with a `timeframe`
            // parameter) does not hide the namespace's function: a method it could be would have to
            // be declared by the script.
            let builtin = registry::is_function(name) && self.callables.declared(member).is_empty();
            if !builtin && let Some(receiver) = self.receiver_value(path, scope) {
                return self.method_call(receiver, member, arguments, &values, site, range);
            }
            if !self.callables.declared(member).is_empty() {
                return Inferred::Unknown;
            }
        }
        if !self.callables.declared(name).is_empty() {
            return self.plain_call(name, arguments, &values, site, range);
        }
        if let Some(object) = name.strip_suffix(".new")
            && self.udts.contains_key(object)
        {
            let ty = Ty::Udt(object.to_owned());
            return self.inferred_of(&ty, Some(Qualifier::Series));
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
        let result = builtin_types::call(name, arguments, &values, qualifier);
        self.refine_collection(name, &values, result)
    }

    // MARK: Types

    /// The type of an inferred value as a `Ty`; `Any` where the checker does not know it.
    fn ty_of(&self, value: Inferred) -> Ty {
        match value {
            Inferred::Known(ValueType::Array, _) => Ty::Array(Box::new(Ty::Any)),
            Inferred::Known(ValueType::Matrix, _) => Ty::Matrix(Box::new(Ty::Any)),
            Inferred::Known(ValueType::Map, _) => Ty::Map(Box::new(Ty::Any), Box::new(Ty::Any)),
            Inferred::Known(ValueType::Object, _) => Ty::Any,
            Inferred::Known(kind, _) => Ty::Scalar(kind),
            Inferred::Typed { ty, .. } => self.types.get(ty).clone(),
            Inferred::Na | Inferred::Unknown => Ty::Any,
        }
    }

    /// The inferred value of type `ty`. A scalar stays a plain `Known`, as it always was.
    fn inferred_of(&mut self, ty: &Ty, qualifier: Option<Qualifier>) -> Inferred {
        match ty {
            Ty::Any => Inferred::Unknown,
            Ty::Scalar(kind) => Inferred::Known(*kind, qualifier),
            Ty::Array(_) => Inferred::Typed {
                ty: self.types.intern(ty.clone()),
                kind: Some(ValueType::Array),
                qualifier,
            },
            Ty::Matrix(_) | Ty::Map(..) | Ty::Udt(_) | Ty::Enum(_) => Inferred::Typed {
                ty: self.types.intern(ty.clone()),
                kind: None,
                qualifier,
            },
        }
    }

    /// The type of `field` in a user type the script declares.
    fn field_type(&self, ty: &Ty, field: &str) -> Option<Ty> {
        let Ty::Udt(name) = ty else {
            return None;
        };
        self.udts
            .get(name)?
            .iter()
            .find(|(candidate, _)| candidate == field)
            .map(|(_, ty)| ty.clone())
    }

    /// `zone.top`, `this.items`: a variable and the fields read through it. None when the head of
    /// the path is not a variable.
    fn field_path(&mut self, path: &str, scope: &Scope) -> Option<Inferred> {
        let (head, rest) = path.split_once('.')?;
        let mut ty = self.ty_of(scope.get(head).copied()?);
        for field in rest.split('.') {
            let Some(next) = self.field_type(&ty, field) else {
                return Some(Inferred::Unknown);
            };
            ty = next;
        }
        Some(self.inferred_of(&ty, Some(Qualifier::Series)))
    }

    /// What a method is called on, given the path before the method name: a variable, a field path
    /// or a builtin series such as `close`. None when the path is none of those (a namespace such
    /// as `ta`).
    fn receiver_value(&mut self, path: &str, scope: &Scope) -> Option<Inferred> {
        if let Some(value) = scope.get(path) {
            return Some(*value);
        }
        if let Some(value) = self.field_path(path, scope) {
            return Some(value);
        }
        // A dotted builtin such as `strategy.closedtrades` is also a namespace of functions.
        if path.contains('.') {
            return None;
        }
        match builtin_types::identifier(path) {
            Inferred::Unknown => None,
            known => Some(known),
        }
    }

    /// Gives `array.new_float`, `array.from` and `array.copy` the element type they carry.
    fn refine_collection(&mut self, name: &str, values: &[Inferred], result: Inferred) -> Inferred {
        let element = if let Some(element) = name.strip_prefix("array.new_") {
            Some(match element {
                "int" => Ty::Scalar(ValueType::Int),
                "float" => Ty::Scalar(ValueType::Float),
                "bool" => Ty::Scalar(ValueType::Bool),
                "string" => Ty::Scalar(ValueType::String),
                "color" => Ty::Scalar(ValueType::Color),
                "line" => Ty::Scalar(ValueType::Line),
                "label" => Ty::Scalar(ValueType::Label),
                "box" => Ty::Scalar(ValueType::Box),
                "table" => Ty::Scalar(ValueType::Table),
                "linefill" => Ty::Scalar(ValueType::Linefill),
                "polyline" => Ty::Scalar(ValueType::Polyline),
                object if self.udts.contains_key(object) => Ty::Udt(object.to_owned()),
                _ => Ty::Any,
            })
        } else if name == "array.from" {
            values.first().map(|first| self.ty_of(*first))
        } else if name == "array.copy" || name == "array.slice" {
            match values.first().map(|first| self.ty_of(*first)) {
                Some(Ty::Array(element)) => Some(*element),
                _ => None,
            }
        } else {
            None
        };
        match element {
            Some(element) => {
                let qualifier = result.qualifier();
                self.inferred_of(&Ty::Array(Box::new(element)), qualifier)
            }
            None => result,
        }
    }

    // MARK: User callables

    fn given(
        &self,
        receiver: Option<Inferred>,
        arguments: &[Argument],
        values: &[Inferred],
    ) -> Vec<Given> {
        receiver
            .into_iter()
            .map(|receiver| Given {
                label: None,
                ty: self.ty_of(receiver),
                qualifier: receiver.qualifier(),
            })
            .chain(arguments.iter().zip(values).map(|(argument, value)| Given {
                label: argument.name.clone(),
                ty: self.ty_of(*value),
                qualifier: value.qualifier(),
            }))
            .collect()
    }

    /// What a call to callable `id` evaluates to. A scalar's qualifier is not carried over: it
    /// depends on the arguments.
    fn select(&mut self, id: usize, site: usize, range: SourceRange) -> Inferred {
        self.resolutions.insert(site, id);
        if let Some(from) = self.current {
            self.edges.push((from, id, range));
        }
        match self.returns[id] {
            Inferred::Known(kind, _) => Inferred::Known(kind, None),
            other => other,
        }
    }

    /// `name(a, b)` where `name` is declared by the script.
    fn plain_call(
        &mut self,
        name: &str,
        arguments: &[Argument],
        values: &[Inferred],
        site: usize,
        range: SourceRange,
    ) -> Inferred {
        // A name declared once can only mean that declaration, so a call from its own body is
        // recursion whether or not its arguments fit.
        if let [only] = self.callables.declared(name)
            && let Some(from) = self.current
        {
            self.edges.push((from, *only, range));
        }
        let given = self.given(None, arguments, values);
        match self.callables.resolve(name, &given, false) {
            Resolution::Selected(id) => self.select(id, site, range),
            Resolution::Dynamic => Inferred::Unknown,
            Resolution::Ambiguous(ids) => {
                self.ambiguous(name, &ids, range);
                Inferred::Unknown
            }
            Resolution::NoFit(reasons) => {
                let only_methods = !reasons.is_empty()
                    && reasons
                        .iter()
                        .all(|(id, _)| self.callables.get(*id).kind == FunctionKind::Method);
                if only_methods {
                    self.refused(name, &reasons, arguments, false, range);
                }
                Inferred::Unknown
            }
        }
    }

    /// `receiver.member(a, b)`.
    fn method_call(
        &mut self,
        receiver: Inferred,
        member: &str,
        arguments: &[Argument],
        values: &[Inferred],
        site: usize,
        range: SourceRange,
    ) -> Inferred {
        let receiver_ty = self.ty_of(receiver);
        // Nothing is known of the receiver, so which method runs is for the run to say.
        if receiver_ty == Ty::Any {
            return Inferred::Unknown;
        }
        let given = self.given(Some(receiver), arguments, values);
        match self.callables.resolve(member, &given, true) {
            Resolution::Selected(id) => self.select(id, site, range),
            Resolution::Dynamic => Inferred::Unknown,
            Resolution::Ambiguous(ids) => {
                self.ambiguous(member, &ids, range);
                Inferred::Unknown
            }
            Resolution::NoFit(reasons) => {
                // A builtin method of the same name takes the call when no declaration does.
                match self.builtin_method(&receiver_ty, receiver, member, values) {
                    Some(result) => result,
                    None => {
                        if reasons.is_empty() {
                            self.unknown_method(member, &receiver_ty, range);
                        } else {
                            self.refused(member, &reasons, arguments, true, range);
                        }
                        Inferred::Unknown
                    }
                }
            }
        }
    }

    /// The result of a builtin method on a receiver of known type, or None when the type has no such
    /// method. A receiver the checker has no list of methods for always has it.
    fn builtin_method(
        &mut self,
        receiver: &Ty,
        value: Inferred,
        member: &str,
        values: &[Inferred],
    ) -> Option<Inferred> {
        let namespace = match receiver {
            Ty::Array(_) => "array",
            Ty::Matrix(_) => "matrix",
            Ty::Map(..) => "map",
            Ty::Scalar(ValueType::Line) => "line",
            Ty::Scalar(ValueType::Label) => "label",
            Ty::Scalar(ValueType::Box) => "box",
            Ty::Scalar(ValueType::Table) => "table",
            Ty::Scalar(ValueType::Linefill) => "linefill",
            Ty::Scalar(ValueType::Polyline) => "polyline",
            Ty::Udt(name) => {
                // A library's types and methods are out of sight.
                let known = self.udts.contains_key(name) && !self.imports;
                return (!known || member == "copy").then(|| {
                    if member == "copy" {
                        value
                    } else {
                        Inferred::Unknown
                    }
                });
            }
            Ty::Scalar(_) | Ty::Enum(_) => {
                return self.imports.then_some(Inferred::Unknown);
            }
            Ty::Any => return Some(Inferred::Unknown),
        };
        let exists = !registry::is_closed(namespace)
            || self.imports
            || registry::is_function(&format!("{namespace}.{member}"));
        if !exists {
            return None;
        }
        let _ = values;
        Some(match (receiver, member) {
            (Ty::Array(element), "get" | "first" | "last" | "pop" | "shift" | "remove") => {
                let element = (**element).clone();
                self.inferred_of(&element, Some(Qualifier::Series))
            }
            (Ty::Array(_) | Ty::Map(..) | Ty::Matrix(_), "copy") => value,
            _ => Inferred::Unknown,
        })
    }

    fn unknown_method(&mut self, member: &str, receiver: &Ty, range: SourceRange) {
        self.error(
            "PINE3064",
            format!("There is no method '{member}' for {}.", receiver.describe()),
            range,
        );
    }

    fn ambiguous(&mut self, name: &str, ids: &[usize], range: SourceRange) {
        let signatures: Vec<String> = ids.iter().map(|id| self.signature(*id)).collect();
        self.error(
            "PINE3068",
            format!(
                "The call to '{name}' is ambiguous: {} fit equally well.",
                signatures.join(" and ")
            ),
            range,
        );
    }

    fn signature(&self, id: usize) -> String {
        let callable = self.callables.get(id);
        let parameters: Vec<String> = callable
            .params
            .iter()
            .map(|parameter| match &parameter.ty {
                Some(ty) => format!("{} {}", ty.describe(), parameter.name),
                None => parameter.name.clone(),
            })
            .collect();
        format!("{}({})", callable.name, parameters.join(", "))
    }

    /// Reports why no declaration of `name` takes the call. `method_syntax` says the first given is
    /// the receiver written before the dot.
    fn refused(
        &mut self,
        name: &str,
        reasons: &[(usize, NoFit)],
        arguments: &[Argument],
        method_syntax: bool,
        range: SourceRange,
    ) {
        // Where argument `index` of the given list is: the receiver has no range of its own.
        let at = |index: usize| {
            let position = if method_syntax {
                index.checked_sub(1)
            } else {
                Some(index)
            };
            position
                .and_then(|position| arguments.get(position))
                .map_or(range, |argument| argument.value.range())
        };
        let receiver_refused = method_syntax
            && reasons
                .iter()
                .all(|(_, reason)| matches!(reason, NoFit::Type { index: 0, .. }));
        if receiver_refused {
            let given = match &reasons[0].1 {
                NoFit::Type { given, .. } => given.describe(),
                _ => String::new(),
            };
            self.error(
                "PINE3065",
                format!("No method '{name}' takes a receiver of type {given}."),
                range,
            );
            return;
        }
        let (message, at) = match reasons {
            [(id, reason)] => match reason {
                NoFit::Arity => (
                    format!("Wrong number of arguments for '{}'.", self.signature(*id)),
                    range,
                ),
                NoFit::Label(label) => (
                    format!(
                        "'{}' has no parameter named '{label}'.",
                        self.signature(*id)
                    ),
                    range,
                ),
                NoFit::Type {
                    index,
                    expected,
                    given,
                } => (
                    format!(
                        "Argument {} of '{name}' must be {}, got {}.",
                        index + usize::from(!method_syntax),
                        expected.describe(),
                        given.describe()
                    ),
                    at(*index),
                ),
                NoFit::Qualifier { index, expected } => (
                    format!(
                        "Argument {} of '{name}' must be {} or lower, but it changes from bar to bar.",
                        index + usize::from(!method_syntax),
                        describe_qualifier(*expected),
                    ),
                    at(*index),
                ),
            },
            _ => (
                format!("No overload of '{name}' takes these arguments."),
                range,
            ),
        };
        let code = if matches!(reasons, [(_, NoFit::Qualifier { .. })]) {
            "PINE3067"
        } else {
            "PINE3066"
        };
        self.error(code, message, at);
    }

    /// Reports each callable that reaches itself through the calls the checker resolved.
    fn report_recursion(&mut self) {
        let mut out: Vec<Vec<(usize, SourceRange)>> = vec![Vec::new(); self.callables.len()];
        for (from, to, range) in &self.edges {
            out[*from].push((*to, *range));
        }
        let reaches = |from: usize, target: usize| {
            let mut seen = vec![false; out.len()];
            let mut pending = vec![from];
            while let Some(current) = pending.pop() {
                if current == target {
                    return true;
                }
                if !std::mem::replace(&mut seen[current], true) {
                    pending.extend(out[current].iter().map(|(next, _)| *next));
                }
            }
            false
        };
        let mut found = Vec::new();
        for (id, calls) in out.iter().enumerate() {
            if let Some((callee, range)) = calls.iter().find(|(callee, _)| reaches(*callee, id)) {
                found.push((id, *callee, *range));
            }
        }
        for (id, callee, range) in found {
            let callable = self.callables.get(id);
            let noun = match callable.kind {
                FunctionKind::Function => "function",
                FunctionKind::Method => "method",
            };
            let how = if callee == id {
                "itself".to_owned()
            } else {
                format!("itself through '{}'", self.callables.get(callee).name)
            };
            let message = format!(
                "The {noun} '{}' calls {how}: Pine functions and methods cannot be recursive, so use a loop.",
                callable.name
            );
            self.error("PINE3063", message, range);
        }
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
        Inferred::Typed { kind, .. } => kind.map_or("an object".to_owned(), |kind| {
            value_type_name(kind).to_owned()
        }),
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
