//! Scope and structure rules that need no type information, ported from
//! `PineStructureValidator.swift`: duplicate declarations, undeclared reassignment, `break`/`continue`
//! placement, duplicate functions, dangling expression statements, and unsupported `request.*` calls.
//! Name rules (`CE10090`, `CE10190`) come from [`super::identifier_rules`].

use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};

use super::ast::{Expression, Literal, Parameter, Statement};
use super::identifier_rules;
use super::symbol_catalog;
use super::traversal::{for_each_call, for_each_identifier, nested_blocks};
use crate::pine::{Diagnostic, DiagnosticCategory, DiagnosticSeverity, SourceRange, ValueType};

/// Validates `statements`. `methods` is the set of names declared with `method`, which may be
/// overloaded by receiver type; `source` supplies text for the statement-expression diagnostic.
pub fn validate(
    statements: &[Statement],
    methods: &HashSet<String>,
    source: &str,
) -> Vec<Diagnostic> {
    let mut validator = Validator {
        diagnostics: Vec::new(),
        methods,
        source,
        signatures: HashMap::new(),
        first_reads: first_reads(statements),
    };
    validator.validate(statements, &HashSet::new(), &HashSet::new(), false, false);
    for_each_call(statements, &mut |name, range| {
        // The data feeds the app lacks read as `na` (and warn); anything else under `request.` has no
        // meaningful stand-in, since its result is an object or a series the app cannot build.
        if name.starts_with("request.")
            && name != "request.security"
            && name != "request.security_lower_tf"
            && name != "request.footprint"
            && !super::misc_builtins::UNAVAILABLE_FEEDS.contains(&name)
        {
            validator.report(
                "PINE9003",
                DiagnosticCategory::Unsupported,
                format!("Feature '{name}' is not supported in this release."),
                range,
            );
        }
    });
    validator.diagnostics
}

/// What tells two definitions of one name apart: the type of every parameter. A function may be
/// declared again (a library's overloads, a method per receiver type) when its signature is new.
type Signature = Vec<(Option<ValueType>, Option<String>)>;

struct Validator<'a> {
    diagnostics: Vec<Diagnostic>,
    methods: &'a HashSet<String>,
    source: &'a str,
    signatures: HashMap<String, Vec<Signature>>,
    /// Where each name is first read, as a source offset.
    first_reads: HashMap<String, usize>,
}

/// The source offset of the first read of every name in the script.
fn first_reads(statements: &[Statement]) -> HashMap<String, usize> {
    let mut first = HashMap::new();
    for_each_identifier(statements, &mut |name, range| {
        let at = range.start.offset;
        first
            .entry(name.to_owned())
            .and_modify(|seen: &mut usize| *seen = (*seen).min(at))
            .or_insert(at);
    });
    first
}

impl Validator<'_> {
    /// `inherited` are the names visible from the enclosing scopes; `own` the ones that belong to
    /// this scope already (a function's parameters, a loop's counter). A name declared again in
    /// this scope is an error; one that only an enclosing scope has is shadowed, as in Pine.
    fn validate(
        &mut self,
        statements: &[Statement],
        inherited: &HashSet<String>,
        own: &HashSet<String>,
        in_loop: bool,
        is_nested: bool,
    ) {
        let mut declared = inherited.clone();
        let mut local = own.clone();
        for (index, statement) in statements.iter().enumerate() {
            match statement {
                Statement::Declaration {
                    name,
                    annotation,
                    value,
                    range,
                    ..
                } => {
                    self.declare(name, &mut declared, &mut local, *range);
                    self.check_name(name, *range);
                    if annotation.kind == Some(ValueType::Bool)
                        && matches!(value, Expression::Literal(Literal::Na, _))
                    {
                        self.report(
                            "PINE3021",
                            DiagnosticCategory::Semantic,
                            "Boolean values cannot be na in Pine v6.".to_owned(),
                            *range,
                        );
                    }
                }
                Statement::Assignment { name, range, .. } => {
                    if !declared.contains(name) {
                        self.report(
                            "PINE3022",
                            DiagnosticCategory::Semantic,
                            format!("Cannot reassign undeclared variable '{name}'."),
                            *range,
                        );
                    }
                }
                Statement::TupleDeclaration { names, range, .. } => {
                    for name in names.iter().filter(|name| name.as_str() != "_") {
                        self.declare(name, &mut declared, &mut local, *range);
                        self.check_name(name, *range);
                    }
                }
                Statement::LoopControl(_, range) => {
                    if !in_loop {
                        self.report(
                            "PINE3023",
                            DiagnosticCategory::Semantic,
                            "break and continue are only allowed inside a loop.".to_owned(),
                            *range,
                        );
                    }
                }
                Statement::Function {
                    name,
                    parameters,
                    body,
                    range,
                } => self.function(name, parameters, body, *range),
                Statement::Conditional { .. }
                | Statement::ForRange { .. }
                | Statement::ForIn { .. }
                | Statement::WhileLoop { .. }
                | Statement::Switch { .. } => self.validate_nested(statement, &declared, in_loop),
                Statement::Expression(expression) => {
                    // The last statement of a nested block is its value, so it may be any expression.
                    if !(is_nested && index == statements.len() - 1) {
                        self.check_statement_expression(expression);
                    }
                }
                Statement::TypeDeclaration { .. }
                | Statement::EnumDeclaration { .. }
                | Statement::FieldAssignment { .. } => {}
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn function(
        &mut self,
        name: &str,
        parameters: &[Parameter],
        body: &[Statement],
        range: SourceRange,
    ) {
        let signature: Signature = parameters
            .iter()
            .map(|parameter| (parameter.kind, parameter.type_name.clone()))
            .collect();
        // Functions and variables live in different namespaces (`kama = kama(src, len)`), so only an
        // earlier function of the same name counts.
        if self
            .signatures
            .get(name)
            .is_some_and(|known| known.contains(&signature))
        {
            self.report(
                "PINE3024",
                DiagnosticCategory::Semantic,
                format!("Function '{name}' is already declared."),
                range,
            );
        }
        self.signatures
            .entry(name.to_owned())
            .or_default()
            .push(signature);
        // A method may share a builtin's name: it is picked by receiver type.
        if !self.methods.contains(name) {
            self.check_name(name, range);
        }
        for parameter in parameters {
            self.check_parameter_name(&parameter.name, range);
        }
        let parameters: HashSet<String> = parameters
            .iter()
            .map(|parameter| parameter.name.clone())
            .collect();
        self.validate(body, &parameters, &parameters, false, true);
    }

    fn validate_nested(
        &mut self,
        statement: &Statement,
        declared: &HashSet<String>,
        in_loop: bool,
    ) {
        let mut inherited = declared.clone();
        let mut own: HashSet<String> = HashSet::new();
        let mut looped = in_loop;
        match statement {
            Statement::ForRange {
                variable, range, ..
            } => {
                self.check_name(variable, *range);
                inherited.insert(variable.clone());
                own.insert(variable.clone());
                looped = true;
            }
            Statement::ForIn {
                index,
                value,
                range,
                ..
            } => {
                for name in [index.as_ref(), Some(value)].into_iter().flatten() {
                    self.check_name(name, *range);
                }
                if let Some(index) = index {
                    inherited.insert(index.clone());
                    own.insert(index.clone());
                }
                inherited.insert(value.clone());
                own.insert(value.clone());
                looped = true;
            }
            Statement::WhileLoop { .. } => looped = true,
            _ => {}
        }
        for block in nested_blocks(statement) {
            self.validate(block, &inherited, &own, looped, true);
        }
    }

    /// Only calls do something as a statement; a value on its own line is a syntax error.
    fn check_statement_expression(&mut self, expression: &Expression) {
        if matches!(
            expression,
            Expression::Call { .. } | Expression::MethodCall { .. } | Expression::Statement(..)
        ) {
            return;
        }
        let range = expression.range();
        self.report(
            "PINE3048",
            DiagnosticCategory::Syntax,
            format!(
                "\"{}\" is not a valid statement.",
                self.statement_text(range)
            ),
            range,
        );
    }

    /// The statement's source text: from its first token to the end of that line.
    fn statement_text(&self, range: SourceRange) -> String {
        let byte = utf16_to_byte(self.source, range.start.offset);
        let Some(rest) = self.source.get(byte..) else {
            return String::new();
        };
        rest.split('\n').next().unwrap_or("").trim().to_owned()
    }

    fn declare(
        &mut self,
        name: &str,
        declared: &mut HashSet<String>,
        local: &mut HashSet<String>,
        range: SourceRange,
    ) {
        if !local.insert(name.to_owned()) {
            self.report(
                "PINE3020",
                DiagnosticCategory::Semantic,
                format!("Variable '{name}' is already declared in this scope."),
                range,
            );
        }
        declared.insert(name.to_owned());
    }

    fn check_name(&mut self, name: &str, range: SourceRange) {
        // Pine lets a script declare a name that a builtin variable has (`low = shareZ < limit`): from
        // that line on the name is the script's, which is worth a warning but not a refusal.
        if symbol_catalog::is_variable(name) {
            // TradingView refuses it once the script has read the builtin above the declaration:
            // the name would mean two things.
            if self
                .first_reads
                .get(name)
                .is_some_and(|&read| read < range.start.offset)
            {
                self.report(
                    "CE10190",
                    DiagnosticCategory::Semantic,
                    format!(
                        "Cannot shadow the built-in variable \"{name}\" because it has already been used as a built-in."
                    ),
                    range,
                );
                return;
            }
            self.diagnostics.push(Diagnostic {
                code: "PINE7006".to_owned(),
                severity: DiagnosticSeverity::Warning,
                category: DiagnosticCategory::Semantic,
                message: format!(
                    "'{name}' shadows the built-in variable '{name}' from this declaration on; earlier lines still read the built-in, and `{name}[n]` keeps reading the built-in's history."
                ),
                range,
            });
            return;
        }
        if let Some(violation) = identifier_rules::violation(name) {
            self.report(
                violation.code,
                DiagnosticCategory::Semantic,
                violation.message,
                range,
            );
        }
    }

    /// A parameter may reuse a builtin function's name (`color fill`): inside the function the name is
    /// the parameter, which is a warning rather than an error. Builtin variables stay reserved.
    fn check_parameter_name(&mut self, name: &str, range: SourceRange) {
        if symbol_catalog::is_function(name) && !symbol_catalog::is_variable(name) {
            self.diagnostics.push(Diagnostic {
                code: "PINE7006".to_owned(),
                severity: DiagnosticSeverity::Warning,
                category: DiagnosticCategory::Semantic,
                message: format!(
                    "Parameter '{name}' shadows the built-in function '{name}' inside this function."
                ),
                range,
            });
            return;
        }
        self.check_name(name, range);
    }

    fn report(
        &mut self,
        code: &str,
        category: DiagnosticCategory,
        message: String,
        range: SourceRange,
    ) {
        self.diagnostics
            .push(Diagnostic::error(code, category, message, range));
    }
}

fn utf16_to_byte(source: &str, utf16: usize) -> usize {
    let mut offset = 0usize;
    for (index, character) in source.char_indices() {
        if offset == utf16 {
            return index;
        }
        if offset > utf16 {
            break;
        }
        offset += character.len_utf16();
    }
    source.len()
}

#[cfg(test)]
mod tests {
    use super::super::ast::{DeclarationMode, TypeAnnotation};
    use super::*;
    use crate::pine::SourceRange;

    fn decl(name: &str, kind: Option<ValueType>, value: Expression) -> Statement {
        Statement::Declaration {
            name: name.to_owned(),
            annotation: TypeAnnotation {
                kind,
                qualifier: None,
            },
            mode: DeclarationMode::Ordinary,
            value,
            range: SourceRange::ZERO,
        }
    }

    fn literal(value: i64) -> Expression {
        Expression::Literal(Literal::Int(value), SourceRange::ZERO)
    }

    fn run(statements: &[Statement]) -> Vec<Diagnostic> {
        validate(statements, &HashSet::new(), "")
    }

    fn codes(diagnostics: &[Diagnostic]) -> Vec<&str> {
        diagnostics.iter().map(|d| d.code.as_str()).collect()
    }

    #[test]
    fn duplicate_declaration_is_pine3020() {
        let statements = [decl("x", None, literal(1)), decl("x", None, literal(2))];
        assert_eq!(codes(&run(&statements)), ["PINE3020"]);
    }

    #[test]
    fn bool_with_na_is_pine3021() {
        let statements = [decl(
            "flag",
            Some(ValueType::Bool),
            Expression::Literal(Literal::Na, SourceRange::ZERO),
        )];
        assert_eq!(codes(&run(&statements)), ["PINE3021"]);
    }

    #[test]
    fn reassigning_an_undeclared_variable_is_pine3022() {
        let statements = [Statement::Assignment {
            name: "missing".to_owned(),
            op: super::super::ast::AssignmentOperator::Reassign,
            value: literal(1),
            range: SourceRange::ZERO,
        }];
        assert_eq!(codes(&run(&statements)), ["PINE3022"]);
    }

    #[test]
    fn break_outside_a_loop_is_pine3023() {
        let statements = [Statement::LoopControl(
            super::super::ast::LoopControl::Break,
            SourceRange::ZERO,
        )];
        assert_eq!(codes(&run(&statements)), ["PINE3023"]);
    }

    #[test]
    fn a_bare_literal_statement_is_pine3048() {
        let statements = [Statement::Expression(literal(3))];
        assert_eq!(codes(&run(&statements)), ["PINE3048"]);
    }

    #[test]
    fn a_call_statement_is_allowed() {
        let statements = [Statement::Expression(Expression::Call {
            name: "plot".to_owned(),
            arguments: vec![],
            site: 0,
            range: SourceRange::ZERO,
        })];
        assert!(run(&statements).is_empty());
    }

    #[test]
    fn shadowing_a_builtin_variable_is_a_warning() {
        let statements = [decl("close", None, literal(1))];
        assert_eq!(codes(&run(&statements)), ["PINE7006"]);
    }

    #[test]
    fn an_unsupported_request_call_is_pine9003() {
        let statements = [Statement::Expression(Expression::Call {
            name: "request.volume_flow".to_owned(),
            arguments: vec![],
            site: 0,
            range: SourceRange::ZERO,
        })];
        assert_eq!(codes(&run(&statements)), ["PINE9003"]);
    }
}
