//! Recursive-descent statement parser with a Pratt expression parser, ported from
//! `PineParser.swift` (+Declarations/+Enums/+Expressions/+Statements/+Types).

use std::collections::HashSet;

use super::ast::{
    Argument, AssignmentOperator, BinaryOperator, DeclarationMode, EnumMember, Expression, Literal,
    LoopControl, Parameter, Qualifier, Statement, SwitchArm, TypeAnnotation, TypeField,
    UnaryOperator,
};
use super::program::StatementEnd;
use super::token::SourcePosition;
use crate::pine::{
    Diagnostic, DiagnosticCategory, Limits, SourceRange, Token, TokenKind, ValueType,
};

/// Ternary binds looser than every binary operator.
const TERNARY_BINDING_POWER: u8 = 5;
const UNARY_BINDING_POWER: u8 = 80;

pub struct Parser {
    tokens: Vec<Token>,
    index: usize,
    diagnostics: Vec<Diagnostic>,
    call_site: usize,
    limits: Limits,
    statement_count: usize,
    export_ranges: Vec<SourceRange>,
    user_types: HashSet<String>,
    enum_types: HashSet<String>,
    annotated_type_name: Option<String>,
    method_names: HashSet<String>,
    import_aliases: HashSet<String>,
    exported_names: HashSet<String>,
    /// Where each statement parsed so far ends, by the offset it starts at.
    statement_ends: Vec<StatementEnd>,
}

impl Parser {
    pub fn new(tokens: Vec<Token>, limits: Limits, import_aliases: HashSet<String>) -> Self {
        let mut user_types = HashSet::new();
        // TradingView's opaque built-in types scripts may name in signatures.
        user_types.insert("footprint".to_owned());
        user_types.insert("volume_row".to_owned());
        Self {
            tokens,
            index: 0,
            diagnostics: Vec::new(),
            call_site: 0,
            limits,
            statement_count: 0,
            export_ranges: Vec::new(),
            user_types,
            enum_types: HashSet::new(),
            annotated_type_name: None,
            method_names: HashSet::new(),
            import_aliases,
            exported_names: HashSet::new(),
            statement_ends: Vec::new(),
        }
    }

    /// Numbers this parser's call sites from `base` instead of 1, so expressions parsed apart from
    /// the script (the debugger's watches) never share a call-site key with one of its calls.
    pub fn with_call_site_base(mut self, base: usize) -> Self {
        self.call_site = base;
        self
    }

    pub fn parse(&mut self) -> (Vec<Statement>, Vec<Diagnostic>) {
        let result = self.block(false);
        (result, std::mem::take(&mut self.diagnostics))
    }

    /// Where every statement ends, sorted by the offset it starts at. Statement ranges are
    /// start-anchored, so this is the only record of how far a block reaches.
    pub fn statement_ends(&self) -> Vec<StatementEnd> {
        let mut ends = self.statement_ends.clone();
        ends.sort_by_key(|end| end.offset);
        ends.dedup_by_key(|end| end.offset);
        ends
    }

    pub fn export_ranges(&self) -> &[SourceRange] {
        &self.export_ranges
    }

    pub fn exported_names(&self) -> &HashSet<String> {
        &self.exported_names
    }

    pub fn method_names(&self) -> &HashSet<String> {
        &self.method_names
    }

    // MARK: Cursor

    fn current(&self) -> &Token {
        &self.tokens[self.index.min(self.tokens.len().saturating_sub(1))]
    }

    fn previous(&self) -> &Token {
        &self.tokens[self.index.saturating_sub(1)]
    }

    fn peek(&self, n: usize) -> Option<&Token> {
        self.tokens.get(self.index + n)
    }

    fn advance(&mut self) {
        self.index = self.index.saturating_add(1).min(self.tokens.len());
    }

    fn at(&self, kind: TokenKind) -> bool {
        self.current().kind == kind
    }

    fn take(&mut self, kind: TokenKind) -> bool {
        if self.at(kind) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn take_word(&mut self, word: &str) -> bool {
        if matches!(&self.current().kind, TokenKind::Identifier(value) if value == word) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn expect(&mut self, kind: TokenKind, message: &str) {
        if !self.take(kind) {
            self.error("PINE2008", message, self.current().range);
        }
    }

    fn error(&mut self, code: &str, message: impl Into<String>, range: SourceRange) {
        self.diagnostics.push(Diagnostic::error(
            code,
            DiagnosticCategory::Syntax,
            message,
            range,
        ));
    }

    // MARK: Blocks

    fn block(&mut self, until_dedent: bool) -> Vec<Statement> {
        let mut out = Vec::new();
        while !self.at(TokenKind::Eof) && !(until_dedent && self.at(TokenKind::Dedent)) {
            if self.take(TokenKind::Newline) || self.take(TokenKind::Indent) {
                continue;
            }
            if self.at(TokenKind::Dedent) {
                self.advance();
                continue;
            }
            let before = self.index;
            let parsed = self.statements_on_line();
            let parsed_len = parsed.len();
            let parsed_empty = parsed.is_empty();
            out.extend(parsed);
            self.statement_count += parsed_len;
            if self.index == before {
                self.error("PINE2001", "Expected a statement.", self.current().range);
                self.advance();
            } else {
                self.end_statement(!parsed_empty);
            }
            while self.take(TokenKind::Newline) {}
            if self.statement_count > self.limits.ast_nodes {
                self.error("PINE8003", "AST node limit exceeded.", self.current().range);
                break;
            }
        }
        if until_dedent {
            self.take(TokenKind::Dedent);
        }
        out
    }

    fn statements_on_line(&mut self) -> Vec<Statement> {
        let Some(first) = self.statement() else {
            return Vec::new();
        };
        let mut statements = vec![first];
        while self.take(TokenKind::Comma) {
            let Some(next) = self.statement() else {
                break;
            };
            statements.push(next);
        }
        statements
    }

    fn exported_statement(&mut self) -> Option<Statement> {
        self.export_ranges.push(self.previous().range);
        let exported = self.statement();
        if let Some(name) = exported.as_ref().and_then(exportable_name) {
            self.exported_names.insert(name.to_owned());
        }
        exported
    }

    fn end_statement(&mut self, report: bool) {
        if self.at(TokenKind::Newline)
            || self.at(TokenKind::Eof)
            || self.at(TokenKind::Dedent)
            || self.previous().kind == TokenKind::Dedent
        {
            return;
        }
        if report {
            self.error(
                "PINE2013",
                "Expected end of statement; found unexpected token.",
                self.current().range,
            );
        }
        self.skip_to_line_end();
    }

    fn skip_to_line_end(&mut self) {
        while !self.at(TokenKind::Newline)
            && !self.at(TokenKind::Dedent)
            && !self.at(TokenKind::Eof)
        {
            self.advance();
        }
    }

    // MARK: Statements

    fn statement(&mut self) -> Option<Statement> {
        let statement = self.statement_unrecorded()?;
        self.record_end(&statement);
        Some(statement)
    }

    /// Notes where `statement`, just parsed, ends: the end of the last token it consumed that is not
    /// layout (a dedent that closes its block, or the newline after it, is not part of it).
    fn record_end(&mut self, statement: &Statement) {
        let end = self.tokens[..self.index.min(self.tokens.len())]
            .iter()
            .rev()
            .find(|token| {
                !matches!(
                    token.kind,
                    TokenKind::Newline | TokenKind::Indent | TokenKind::Dedent | TokenKind::Eof
                )
            })
            .map_or(statement.range().end, |token| token.range.end);
        self.statement_ends.push(StatementEnd {
            offset: statement.range().start.offset,
            end,
        });
    }

    fn statement_unrecorded(&mut self) -> Option<Statement> {
        if self.take_word("export") {
            return self.exported_statement();
        }
        if self.is_type_declaration() {
            return self.type_declaration();
        }
        if self.is_enum_declaration() {
            return self.enum_declaration();
        }
        if self.is_method_definition() {
            return self.method_definition();
        }
        if self.skip_import_declaration() {
            return None;
        }
        if self.take(TokenKind::If) {
            return self.if_statement();
        }
        if self.take(TokenKind::For) {
            return self.for_statement();
        }
        if self.take(TokenKind::While) {
            return self.while_statement();
        }
        if self.take(TokenKind::Once) {
            return self.once_statement();
        }
        if self.take(TokenKind::Switch) {
            return self.switch_statement();
        }
        if self.is_tuple_declaration() {
            return self.tuple_declaration();
        }
        if self.take(TokenKind::Break) {
            return Some(Statement::LoopControl(
                LoopControl::Break,
                self.previous().range,
            ));
        }
        if self.take(TokenKind::Continue) {
            return Some(Statement::LoopControl(
                LoopControl::Continue,
                self.previous().range,
            ));
        }
        if self.is_function_definition() {
            return self.function_definition();
        }
        if let Some(declaration) = self.declaration() {
            return Some(declaration);
        }
        if self.is_field_assignment() {
            return self.field_assignment();
        }
        if let Some(assignment) = self.assignment() {
            return Some(assignment);
        }
        self.expression(0).map(Statement::Expression)
    }

    fn assignment(&mut self) -> Option<Statement> {
        let name = self.variable_name(&self.current().kind)?;
        let op = assignment_operator_from_token(&self.peek(1)?.kind)?;
        let range = self.current().range;
        self.advance();
        self.advance();
        let value = self.expression(0)?;
        Some(Statement::Assignment {
            name,
            op,
            value,
            range,
        })
    }

    fn begin_indented_block(&mut self, owner: &str) -> bool {
        self.take(TokenKind::Newline);
        if !self.take(TokenKind::Indent) {
            self.error(
                "PINE2002",
                format!("Expected an indented block after {owner}."),
                self.current().range,
            );
            return false;
        }
        true
    }

    fn indented_block(&mut self, owner: &str) -> Option<Vec<Statement>> {
        if !self.begin_indented_block(owner) {
            return None;
        }
        Some(self.block(true))
    }

    fn block_or_single_statement(&mut self, owner: &str) -> Option<Vec<Statement>> {
        if self.at(TokenKind::Newline) {
            return self.indented_block(owner);
        }
        let statements = self.statements_on_line();
        if statements.is_empty() {
            None
        } else {
            Some(statements)
        }
    }

    fn if_statement(&mut self) -> Option<Statement> {
        let start = self.previous().range;
        let condition = self.expression(0)?;
        let when_true = self.indented_block("if")?;
        let mut when_false = Vec::new();
        if self.take(TokenKind::Else) {
            if self.take(TokenKind::If) {
                if let Some(chained) = self.if_statement() {
                    self.record_end(&chained);
                    when_false = vec![chained];
                }
            } else {
                self.take(TokenKind::Newline);
                if self.take(TokenKind::Indent) {
                    when_false = self.block(true);
                }
            }
        }
        Some(Statement::Conditional {
            condition,
            when_true,
            when_false,
            range: start,
        })
    }

    fn while_statement(&mut self) -> Option<Statement> {
        let start = self.previous().range;
        let condition = self.expression(0)?;
        let body = self.indented_block("while")?;
        Some(Statement::WhileLoop {
            condition,
            body,
            range: start,
        })
    }

    fn once_statement(&mut self) -> Option<Statement> {
        let start = self.previous().range;
        let condition = if self.at(TokenKind::Newline) {
            None
        } else {
            Some(self.expression(0)?)
        };
        let body = self.indented_block("once")?;
        Some(Statement::Once {
            condition,
            body,
            range: start,
        })
    }

    fn switch_statement(&mut self) -> Option<Statement> {
        let start = self.previous().range;
        let mut subject = None;
        if !self.at(TokenKind::Newline) {
            subject = Some(self.expression(0)?);
        }
        if !self.begin_indented_block("switch") {
            return None;
        }
        let mut arms = Vec::new();
        while !self.at(TokenKind::Eof) && !self.at(TokenKind::Dedent) {
            if self.take(TokenKind::Newline) {
                continue;
            }
            let mut condition = None;
            if !self.take(TokenKind::Arrow) {
                let Some(value) = self.expression(0) else {
                    self.skip_to_line_end();
                    continue;
                };
                condition = Some(value);
                self.expect(TokenKind::Arrow, "Expected '=>' in switch arm.");
            }
            let body = self.block_or_single_statement("switch arm")?;
            arms.push(SwitchArm { condition, body });
            while self.take(TokenKind::Newline) {}
        }
        self.take(TokenKind::Dedent);
        Some(Statement::Switch {
            subject,
            arms,
            range: start,
        })
    }

    fn for_statement(&mut self) -> Option<Statement> {
        let start = self.previous().range;
        let mut index_name = None;
        let value_name;
        if self.take(TokenKind::LeftBracket) {
            let first = self.loop_variable()?;
            self.expect(TokenKind::Comma, "Expected ',' in for...in destructuring.");
            let second = self.loop_variable()?;
            self.expect(TokenKind::RightBracket, "Expected ']'.");
            index_name = Some(first);
            value_name = second;
        } else {
            value_name = self.loop_variable()?;
        }
        if index_name.is_none() && self.take(TokenKind::Assign) {
            return self.for_range_tail(value_name, start);
        }
        if !self.take_word("in") {
            self.error(
                "PINE2010",
                "Expected '=' or 'in' in for loop.",
                self.current().range,
            );
            return None;
        }
        let collection = self.expression(0)?;
        let body = self.indented_block("for")?;
        Some(Statement::ForIn {
            index: index_name,
            value: value_name,
            collection,
            body,
            range: start,
        })
    }

    /// A loop counter or element name, with the optional type in front of it
    /// (`for int i = …`, `for Zone z in zones`).
    fn loop_variable(&mut self) -> Option<String> {
        let typed_by_keyword = matches!(self.current().kind, TokenKind::TypeKeyword(_));
        let typed_by_name = matches!(self.current().kind, TokenKind::Identifier(_))
            && matches!(
                self.peek(1).map(|token| &token.kind),
                Some(TokenKind::Identifier(next)) if next != "in"
            );
        if typed_by_keyword || typed_by_name {
            self.advance();
        }
        let TokenKind::Identifier(name) = &self.current().kind else {
            self.error(
                "PINE2009",
                "Expected a loop variable.",
                self.current().range,
            );
            return None;
        };
        let name = name.clone();
        self.advance();
        Some(name)
    }

    fn for_range_tail(&mut self, variable: String, start: SourceRange) -> Option<Statement> {
        let from = self.expression(0)?;
        if !self.take_word("to") {
            self.error(
                "PINE2010",
                "Expected 'to' in for loop.",
                self.current().range,
            );
            return None;
        }
        let to = self.expression(0)?;
        let mut step = None;
        if self.take_word("by") {
            step = Some(self.expression(0)?);
        }
        let body = self.indented_block("for")?;
        Some(Statement::ForRange {
            variable,
            from,
            to,
            step,
            body,
            range: start,
        })
    }

    fn is_tuple_declaration(&self) -> bool {
        if !self.at(TokenKind::LeftBracket) {
            return false;
        }
        let mut i = self.index + 1;
        let mut expect_name = true;
        while i < self.tokens.len() {
            match &self.tokens[i].kind {
                TokenKind::Identifier(_) if expect_name => expect_name = false,
                TokenKind::Comma if !expect_name => expect_name = true,
                TokenKind::RightBracket => {
                    return !expect_name
                        && i + 1 < self.tokens.len()
                        && self.tokens[i + 1].kind == TokenKind::Assign;
                }
                _ => return false,
            }
            i += 1;
        }
        false
    }

    fn tuple_declaration(&mut self) -> Option<Statement> {
        let range = self.current().range;
        self.advance();
        let mut names = Vec::new();
        while let TokenKind::Identifier(name) = &self.current().kind {
            names.push(name.clone());
            self.advance();
            if !self.take(TokenKind::Comma) {
                break;
            }
        }
        self.expect(TokenKind::RightBracket, "Expected ']'.");
        self.expect(TokenKind::Assign, "Expected '='.");
        let value = self.expression(0)?;
        Some(Statement::TupleDeclaration {
            names,
            value,
            range,
        })
    }

    fn is_function_definition(&self) -> bool {
        if !matches!(&self.current().kind, TokenKind::Identifier(_)) {
            return false;
        }
        if self.peek(1).is_none_or(|t| t.kind != TokenKind::LeftParen) {
            return false;
        }
        let mut depth = 0;
        let mut i = self.index + 1;
        while i < self.tokens.len() {
            match &self.tokens[i].kind {
                TokenKind::LeftParen => depth += 1,
                TokenKind::RightParen => {
                    depth -= 1;
                    if depth == 0 {
                        return i + 1 < self.tokens.len()
                            && self.tokens[i + 1].kind == TokenKind::Arrow;
                    }
                }
                TokenKind::Newline | TokenKind::Eof => return false,
                _ => {}
            }
            i += 1;
        }
        false
    }

    fn function_definition(&mut self) -> Option<Statement> {
        let TokenKind::Identifier(name) = &self.current().kind else {
            return None;
        };
        let name = name.clone();
        let range = self.current().range;
        self.advance();
        self.expect(TokenKind::LeftParen, "Expected '('.");
        let mut parameters = Vec::new();
        if !self.at(TokenKind::RightParen) {
            loop {
                let parameter = self.function_parameter()?;
                parameters.push(parameter);
                if !self.take(TokenKind::Comma) {
                    break;
                }
            }
        }
        self.expect(TokenKind::RightParen, "Expected ')'.");
        self.expect(TokenKind::Arrow, "Expected '=>'.");
        let body = self.block_or_single_statement("function declaration")?;
        Some(Statement::Function {
            name,
            parameters,
            body,
            range,
        })
    }

    fn function_parameter(&mut self) -> Option<Parameter> {
        self.skip_qualifier();
        let kind = self.type_annotation();
        let type_name = if kind == Some(ValueType::Object) {
            self.annotated_type_name.clone()
        } else {
            None
        };
        let TokenKind::Identifier(name) = &self.current().kind else {
            self.error(
                "PINE2011",
                "Expected a parameter name.",
                self.current().range,
            );
            return None;
        };
        let name = name.clone();
        self.advance();
        let mut default_value = None;
        if self.take(TokenKind::Assign) {
            default_value = self.expression(0);
        }
        Some(Parameter {
            name,
            kind,
            type_name,
            default_value,
        })
    }

    // MARK: Expressions

    fn expression(&mut self, min_bp: u8) -> Option<Expression> {
        let token = self.current().clone();
        self.advance();
        if token.kind == TokenKind::If || token.kind == TokenKind::Switch {
            let block = if token.kind == TokenKind::If {
                self.if_statement()
            } else {
                self.switch_statement()
            }?;
            self.record_end(&block);
            return Some(Expression::Statement(Box::new(block), token.range));
        }
        let mut lhs = self.prefix_expression(&token)?;
        loop {
            match self.extend(lhs, min_bp) {
                Extension::Extended(grown) => lhs = grown,
                Extension::Finished(done) => return Some(done),
            }
        }
    }

    fn prefix_expression(&mut self, token: &Token) -> Option<Expression> {
        match &token.kind {
            TokenKind::Number { value, is_integer } => {
                Some(self.number_literal(*value, *is_integer, token.range))
            }
            TokenKind::Color(value) => {
                Some(Expression::Literal(Literal::Color(*value), token.range))
            }
            TokenKind::String(value) => Some(Expression::Literal(
                Literal::String(value.clone()),
                token.range,
            )),
            TokenKind::Bool(value) => Some(Expression::Literal(Literal::Bool(*value), token.range)),
            TokenKind::Na => {
                if self.at(TokenKind::LeftParen) {
                    Some(Expression::Identifier("na".to_owned(), token.range))
                } else {
                    Some(Expression::Literal(Literal::Na, token.range))
                }
            }
            TokenKind::Identifier(name) => Some(Expression::Identifier(name.clone(), token.range)),
            TokenKind::TypeKeyword(kind) => Some(Expression::Identifier(
                value_type_name(*kind).to_owned(),
                token.range,
            )),
            TokenKind::Minus | TokenKind::Plus | TokenKind::Not => {
                let op = unary_operator_from_token(&token.kind)?;
                let operand = self.expression(UNARY_BINDING_POWER)?;
                Some(Expression::Unary {
                    op,
                    operand: Box::new(operand),
                    range: token.range,
                })
            }
            TokenKind::LeftParen => {
                let inner = self.expression(0)?;
                self.expect(TokenKind::RightParen, "Expected ')'.");
                Some(inner)
            }
            TokenKind::LeftBracket => Some(self.tuple_literal(token.range)),
            _ => {
                self.error("PINE2003", "Expected an expression.", token.range);
                None
            }
        }
    }

    fn number_literal(&mut self, value: f64, integer: bool, range: SourceRange) -> Expression {
        if !integer {
            return Expression::Literal(Literal::Float(value), range);
        }
        if value >= i64::MIN as f64 && value <= i64::MAX as f64 {
            return Expression::Literal(Literal::Int(value as i64), range);
        }
        self.error("PINE2014", "Integer literal is out of range.", range);
        Expression::Literal(Literal::Float(value), range)
    }

    fn tuple_literal(&mut self, range: SourceRange) -> Expression {
        let mut values = Vec::new();
        if !self.at(TokenKind::RightBracket) {
            loop {
                if let Some(value) = self.expression(0) {
                    values.push(value);
                }
                if !self.take(TokenKind::Comma) {
                    break;
                }
            }
        }
        self.expect(TokenKind::RightBracket, "Expected ']'.");
        Expression::Tuple(values, range)
    }

    fn extend(&mut self, lhs: Expression, min_bp: u8) -> Extension {
        if self.take(TokenKind::Dot) {
            return self.member_access(lhs);
        }
        if matches!(&lhs, Expression::Identifier(name, _) if name == "array.new")
            && self.at(TokenKind::Less)
        {
            return self.generic_array_constructor(lhs);
        }
        if matches!(&lhs, Expression::Identifier(name, _) if name == "map.new")
            && self.at(TokenKind::Less)
        {
            return self.generic_map_constructor(lhs);
        }
        if matches!(&lhs, Expression::Identifier(name, _) if name == "matrix.new")
            && self.at(TokenKind::Less)
        {
            return self.generic_matrix_constructor(lhs);
        }
        if self.take(TokenKind::LeftParen) {
            return self.call_suffix(lhs);
        }
        if self.take(TokenKind::LeftBracket) {
            return self.history_suffix(lhs);
        }
        if min_bp <= TERNARY_BINDING_POWER && self.take(TokenKind::Question) {
            return self.ternary_suffix(lhs);
        }
        self.binary_suffix(lhs, min_bp)
    }

    fn member_access(&mut self, lhs: Expression) -> Extension {
        let member = match &self.current().kind {
            TokenKind::Identifier(value) => value.clone(),
            TokenKind::TypeKeyword(kind) => value_type_name(*kind).to_owned(),
            _ => {
                self.error("PINE2004", "Expected member name.", self.current().range);
                return Extension::Finished(lhs);
            }
        };
        // The whole access is what a diagnostic points at: `array.resize`, not only `array`.
        let to_member = |range: SourceRange, member_end: SourcePosition| SourceRange {
            start: range.start,
            end: member_end,
        };
        let member_end = self.current().range.end;
        self.advance();
        if let Expression::Identifier(base, range) = lhs {
            return Extension::Extended(Expression::Identifier(
                format!("{base}.{member}"),
                to_member(range, member_end),
            ));
        }
        let range = to_member(lhs.range(), member_end);
        Extension::Extended(Expression::Member {
            base: Box::new(lhs),
            name: member,
            range,
        })
    }

    fn generic_array_constructor(&mut self, lhs: Expression) -> Extension {
        self.advance();
        let Some(element) = self.type_argument() else {
            return Extension::Finished(lhs);
        };
        self.expect(TokenKind::Greater, "Expected '>'.");
        let range = lhs.range();
        if !self.take(TokenKind::LeftParen) {
            self.error("PINE2008", "Expected '('.", self.current().range);
            return Extension::Finished(lhs);
        }
        self.call_suffix(Expression::Identifier(
            format!("array.new_{element}"),
            range,
        ))
    }

    fn generic_map_constructor(&mut self, lhs: Expression) -> Extension {
        self.advance();
        let _ = self.type_argument();
        self.expect(
            TokenKind::Comma,
            "Expected ',' between the key and value types.",
        );
        let _ = self.type_argument();
        self.expect(TokenKind::Greater, "Expected '>'.");
        if !self.take(TokenKind::LeftParen) {
            self.error("PINE2008", "Expected '('.", self.current().range);
            return Extension::Finished(lhs);
        }
        self.call_suffix(lhs)
    }

    fn generic_matrix_constructor(&mut self, lhs: Expression) -> Extension {
        self.advance();
        let _ = self.type_argument();
        self.expect(TokenKind::Greater, "Expected '>'.");
        if !self.take(TokenKind::LeftParen) {
            self.error("PINE2008", "Expected '('.", self.current().range);
            return Extension::Finished(lhs);
        }
        self.call_suffix(lhs)
    }

    fn call_suffix(&mut self, lhs: Expression) -> Extension {
        let arguments = self.call_arguments();
        self.expect(TokenKind::RightParen, "Expected ')'.");
        match lhs {
            Expression::Member { base, name, range } => {
                self.call_site += 1;
                Extension::Extended(Expression::MethodCall {
                    receiver: base,
                    name,
                    arguments,
                    site: self.call_site,
                    range,
                })
            }
            Expression::Identifier(name, range) => {
                self.call_site += 1;
                Extension::Extended(Expression::Call {
                    name,
                    arguments,
                    site: self.call_site,
                    range,
                })
            }
            other => {
                self.error(
                    "PINE2006",
                    "Only named functions can be called.",
                    other.range(),
                );
                Extension::Finished(other)
            }
        }
    }

    fn call_arguments(&mut self) -> Vec<Argument> {
        let mut arguments = Vec::new();
        if self.at(TokenKind::RightParen) {
            return arguments;
        }
        loop {
            let label_range = self.current().range;
            let name = self.argument_name();
            let Some(value) = self.expression(0) else {
                break;
            };
            arguments.push(Argument {
                name: name.clone(),
                value,
                name_range: name.map(|_| label_range),
            });
            if !self.take(TokenKind::Comma) {
                break;
            }
        }
        arguments
    }

    fn argument_name(&mut self) -> Option<String> {
        if self.peek(1).is_none_or(|t| t.kind != TokenKind::Assign) {
            return None;
        }
        let name = match &self.current().kind {
            TokenKind::Identifier(name) => name.clone(),
            TokenKind::TypeKeyword(kind) => value_type_name(*kind).to_owned(),
            _ => return None,
        };
        self.advance();
        self.advance();
        Some(name)
    }

    fn history_suffix(&mut self, lhs: Expression) -> Extension {
        let Some(offset) = self.expression(0) else {
            return Extension::Finished(lhs);
        };
        self.expect(TokenKind::RightBracket, "Expected ']'.");
        let range = lhs.range();
        Extension::Extended(Expression::History {
            base: Box::new(lhs),
            offset: Box::new(offset),
            range,
        })
    }

    fn ternary_suffix(&mut self, lhs: Expression) -> Extension {
        let Some(when_true) = self.expression(0) else {
            self.error(
                "PINE2007",
                "Malformed ternary expression.",
                self.current().range,
            );
            return Extension::Finished(lhs);
        };
        if !self.take(TokenKind::Colon) {
            self.error(
                "PINE2007",
                "Malformed ternary expression.",
                self.current().range,
            );
            return Extension::Finished(lhs);
        }
        let Some(when_false) = self.expression(0) else {
            self.error(
                "PINE2007",
                "Malformed ternary expression.",
                self.current().range,
            );
            return Extension::Finished(lhs);
        };
        let range = lhs.range();
        Extension::Extended(Expression::Ternary {
            condition: Box::new(lhs),
            when_true: Box::new(when_true),
            when_false: Box::new(when_false),
            range,
        })
    }

    fn binary_suffix(&mut self, lhs: Expression, min_bp: u8) -> Extension {
        let Some(op) = binary_operator_from_token(&self.current().kind) else {
            return Extension::Finished(lhs);
        };
        if op.binding_power().0 < min_bp {
            return Extension::Finished(lhs);
        }
        self.advance();
        let Some(rhs) = self.expression(op.binding_power().1) else {
            return Extension::Finished(lhs);
        };
        let range = lhs.range();
        Extension::Extended(Expression::Binary {
            lhs: Box::new(lhs),
            op,
            rhs: Box::new(rhs),
            range,
        })
    }

    // MARK: Declarations

    fn variable_name(&self, kind: &TokenKind) -> Option<String> {
        match kind {
            TokenKind::Identifier(name) => Some(name.clone()),
            TokenKind::TypeKeyword(kind) => Some(value_type_name(*kind).to_owned()),
            _ => None,
        }
    }

    fn declaration(&mut self) -> Option<Statement> {
        let start = self.index;
        let mut mode = DeclarationMode::Ordinary;
        if self.take(TokenKind::Var) {
            mode = DeclarationMode::Variable;
        } else if self.take(TokenKind::Varip) {
            mode = DeclarationMode::Intrabar;
        }
        let qualifier = self.skip_qualifier();
        let kind = self.type_annotation();
        let Some((name, range, token_count)) = self.declaration_target() else {
            self.index = start;
            return None;
        };
        for _ in 0..=token_count {
            self.advance();
        }
        let value = self.expression(0)?;
        Some(Statement::Declaration {
            name,
            annotation: TypeAnnotation { kind, qualifier },
            mode,
            value,
            range,
        })
    }

    fn declaration_target(&self) -> Option<(String, SourceRange, usize)> {
        let mut name = self.variable_name(&self.current().kind)?;
        let mut range = self.current().range;
        let mut count = 1;
        while self.peek(count).is_some_and(|t| t.kind == TokenKind::Dot) {
            let Some(next) = self.peek(count + 1) else {
                break;
            };
            let TokenKind::Identifier(part) = &next.kind else {
                break;
            };
            name.push('.');
            name.push_str(part);
            range.end = next.range.end;
            count += 2;
        }
        if self.peek(count).is_none_or(|t| t.kind != TokenKind::Assign) {
            return None;
        }
        Some((name, range, count))
    }

    fn type_annotation(&mut self) -> Option<ValueType> {
        self.annotated_type_name = None;
        let kind;
        match &self.current().kind {
            TokenKind::TypeKeyword(value) => kind = *value,
            TokenKind::Identifier(alias)
                if self.import_aliases.contains(alias) && self.is_library_type_annotation() =>
            {
                let alias = alias.clone();
                self.advance();
                self.advance();
                if let TokenKind::Identifier(name) = &self.current().kind {
                    self.annotated_type_name = Some(format!("{alias}.{name}"));
                }
                self.advance();
                return Some(ValueType::Object);
            }
            TokenKind::Identifier(word) if word == "chart" && self.is_chart_point_annotation() => {
                self.advance();
                self.advance();
                self.advance();
                self.annotated_type_name = Some("chart.point".to_owned());
                return Some(ValueType::Object);
            }
            TokenKind::Identifier(word) => {
                if let Some(value) = object_type(word) {
                    kind = value;
                } else if self.user_types.contains(word) || self.enum_types.contains(word) {
                    kind = ValueType::Object;
                    self.annotated_type_name = Some(word.clone());
                } else {
                    return None;
                }
            }
            _ => return None,
        }
        let next = self.peek(1)?;
        if matches!(&next.kind, TokenKind::Identifier(_)) {
            self.advance();
            return Some(kind);
        }
        if next.kind == TokenKind::LeftBracket
            && self
                .peek(2)
                .is_some_and(|t| t.kind == TokenKind::RightBracket)
        {
            self.advance();
            self.advance();
            self.advance();
            return Some(ValueType::Array);
        }
        if kind == ValueType::Array && next.kind == TokenKind::Less {
            self.advance();
            self.advance();
            let _ = self.type_argument();
            self.expect(TokenKind::Greater, "Expected '>'.");
            return Some(ValueType::Array);
        }
        if kind == ValueType::Matrix && next.kind == TokenKind::Less {
            self.advance();
            self.advance();
            let _ = self.type_argument();
            self.expect(TokenKind::Greater, "Expected '>'.");
            return Some(ValueType::Matrix);
        }
        if kind == ValueType::Map && next.kind == TokenKind::Less {
            self.advance();
            self.advance();
            let _ = self.type_argument();
            self.expect(
                TokenKind::Comma,
                "Expected ',' between the key and value types.",
            );
            let _ = self.type_argument();
            self.expect(TokenKind::Greater, "Expected '>'.");
            return Some(ValueType::Map);
        }
        None
    }

    fn is_library_type_annotation(&self) -> bool {
        self.peek(1).is_some_and(|t| t.kind == TokenKind::Dot)
            && self
                .peek(2)
                .is_some_and(|t| matches!(&t.kind, TokenKind::Identifier(_)))
            && self
                .peek(3)
                .is_some_and(|t| matches!(&t.kind, TokenKind::Identifier(_)))
    }

    fn is_chart_point_annotation(&self) -> bool {
        self.peek(1).is_some_and(|t| t.kind == TokenKind::Dot)
            && self
                .peek(2)
                .is_some_and(|t| matches!(&t.kind, TokenKind::Identifier(word) if word == "point"))
            && self
                .peek(3)
                .is_some_and(|t| matches!(&t.kind, TokenKind::Identifier(_)))
    }

    fn type_argument(&mut self) -> Option<String> {
        let mut word = match &self.current().kind {
            TokenKind::TypeKeyword(kind) => value_type_name(*kind).to_owned(),
            TokenKind::Identifier(name) => name.clone(),
            _ => {
                self.error("PINE2012", "Expected a type.", self.current().range);
                return None;
            }
        };
        self.advance();
        while self.at(TokenKind::Dot) {
            let Some(next) = self.peek(1) else { break };
            let TokenKind::Identifier(part) = &next.kind else {
                break;
            };
            word.push('.');
            word.push_str(part);
            self.advance();
            self.advance();
        }
        if self.take(TokenKind::Less) {
            let _ = self.type_argument();
            while self.take(TokenKind::Comma) {
                let _ = self.type_argument();
            }
            self.expect(TokenKind::Greater, "Expected '>'.");
        }
        Some(word)
    }

    fn skip_qualifier(&mut self) -> Option<Qualifier> {
        let TokenKind::Identifier(word) = &self.current().kind else {
            return None;
        };
        let qualifier = qualifier_from_name(word)?;
        let next = self.peek(1)?;
        match &next.kind {
            TokenKind::TypeKeyword(_) | TokenKind::Identifier(_) => {
                self.advance();
                Some(qualifier)
            }
            _ => None,
        }
    }

    fn skip_import_declaration(&mut self) -> bool {
        let is_import = matches!(&self.current().kind, TokenKind::Identifier(word) if word == "import")
            && self
                .peek(1)
                .is_some_and(|t| matches!(&t.kind, TokenKind::Identifier(_)));
        if !is_import {
            return false;
        }
        self.skip_to_line_end();
        true
    }

    // MARK: Enums

    fn is_enum_declaration(&self) -> bool {
        let Some(name) = self.peek(1) else {
            return false;
        };
        let Some(end) = self.peek(2) else {
            return false;
        };
        matches!(&self.current().kind, TokenKind::Identifier(word) if word == "enum")
            && matches!(&name.kind, TokenKind::Identifier(_))
            && end.kind == TokenKind::Newline
    }

    fn enum_declaration(&mut self) -> Option<Statement> {
        self.advance();
        let TokenKind::Identifier(name) = &self.current().kind else {
            return None;
        };
        let name = name.clone();
        let range = self.current().range;
        self.advance();
        self.enum_types.insert(name.clone());
        let owner = format!("enum {name}");
        if !self.begin_indented_block(&owner) {
            return None;
        }
        let mut members = Vec::new();
        while !self.at(TokenKind::Eof) && !self.at(TokenKind::Dedent) {
            if self.take(TokenKind::Newline) {
                continue;
            }
            let Some(member) = self.enum_member() else {
                self.skip_to_line_end();
                continue;
            };
            members.push(member);
            if !self.at(TokenKind::Newline)
                && !self.at(TokenKind::Dedent)
                && !self.at(TokenKind::Eof)
            {
                self.error(
                    "PINE2013",
                    "Expected end of statement; found unexpected token.",
                    self.current().range,
                );
                self.skip_to_line_end();
            }
        }
        self.take(TokenKind::Dedent);
        Some(Statement::EnumDeclaration {
            name,
            members,
            range,
        })
    }

    fn enum_member(&mut self) -> Option<EnumMember> {
        let TokenKind::Identifier(name) = &self.current().kind else {
            self.error(
                "PINE2015",
                "Expected an enum member name.",
                self.current().range,
            );
            return None;
        };
        let name = name.clone();
        self.advance();
        if !self.take(TokenKind::Assign) {
            return Some(EnumMember {
                name: name.clone(),
                title: name,
            });
        }
        let TokenKind::String(title) = &self.current().kind else {
            self.error(
                "PINE2017",
                "An enum title must be a string literal.",
                self.current().range,
            );
            return None;
        };
        let title = title.clone();
        self.advance();
        Some(EnumMember { name, title })
    }

    // MARK: Types

    fn is_type_declaration(&self) -> bool {
        let Some(name) = self.peek(1) else {
            return false;
        };
        let Some(end) = self.peek(2) else {
            return false;
        };
        matches!(&self.current().kind, TokenKind::Identifier(word) if word == "type")
            && matches!(&name.kind, TokenKind::Identifier(_))
            && end.kind == TokenKind::Newline
    }

    fn type_declaration(&mut self) -> Option<Statement> {
        self.advance();
        let TokenKind::Identifier(name) = &self.current().kind else {
            return None;
        };
        let name = name.clone();
        let range = self.current().range;
        self.advance();
        self.user_types.insert(name.clone());
        let owner = format!("type {name}");
        if !self.begin_indented_block(&owner) {
            return None;
        }
        let mut fields = Vec::new();
        while !self.at(TokenKind::Eof) && !self.at(TokenKind::Dedent) {
            if self.take(TokenKind::Newline) {
                continue;
            }
            let Some(field) = self.type_field() else {
                self.skip_to_line_end();
                continue;
            };
            fields.push(field);
            if !self.at(TokenKind::Newline)
                && !self.at(TokenKind::Dedent)
                && !self.at(TokenKind::Eof)
            {
                self.error(
                    "PINE2013",
                    "Expected end of statement; found unexpected token.",
                    self.current().range,
                );
                self.skip_to_line_end();
            }
        }
        self.take(TokenKind::Dedent);
        Some(Statement::TypeDeclaration {
            name,
            fields,
            range,
        })
    }

    fn type_field(&mut self) -> Option<TypeField> {
        self.skip_qualifier();
        let kind = self.type_annotation();
        let TokenKind::Identifier(name) = &self.current().kind else {
            self.error("PINE2015", "Expected a field name.", self.current().range);
            return None;
        };
        let name = name.clone();
        self.advance();
        let mut default_value = None;
        if self.take(TokenKind::Assign) {
            default_value = Some(self.expression(0)?);
        }
        Some(TypeField {
            name,
            kind,
            default_value,
        })
    }

    fn is_field_assignment(&self) -> bool {
        if !matches!(&self.current().kind, TokenKind::Identifier(_)) {
            return false;
        }
        if self.peek(1).is_none_or(|t| t.kind != TokenKind::Dot) {
            return false;
        }
        let mut depth = 0;
        for token in &self.tokens[self.index..] {
            match &token.kind {
                TokenKind::LeftParen | TokenKind::LeftBracket => depth += 1,
                TokenKind::RightParen | TokenKind::RightBracket => depth -= 1,
                TokenKind::Newline | TokenKind::Eof | TokenKind::Indent | TokenKind::Dedent => {
                    return false;
                }
                TokenKind::Comma if depth == 0 => return false,
                _ => {
                    if depth == 0 && assignment_operator_from_token(&token.kind).is_some() {
                        return true;
                    }
                }
            }
        }
        false
    }

    fn field_assignment(&mut self) -> Option<Statement> {
        let range = self.current().range;
        let target = self.expression(0)?;
        let Some(op) = assignment_operator_from_token(&self.current().kind) else {
            self.error(
                "PINE2013",
                "Expected end of statement; found unexpected token.",
                self.current().range,
            );
            return None;
        };
        self.advance();
        let valid = match &target {
            Expression::Identifier(name, _) => name.contains('.'),
            Expression::Member { .. } => true,
            _ => false,
        };
        if !valid {
            self.error(
                "PINE2016",
                "Only a variable or a field can be assigned to.",
                target.range(),
            );
            return None;
        }
        let value = self.expression(0)?;
        Some(Statement::FieldAssignment {
            target,
            op,
            value,
            range,
        })
    }

    fn is_method_definition(&self) -> bool {
        let Some(name) = self.peek(1) else {
            return false;
        };
        matches!(&self.current().kind, TokenKind::Identifier(word) if word == "method")
            && matches!(&name.kind, TokenKind::Identifier(_))
            && self.peek(2).is_some_and(|t| t.kind == TokenKind::LeftParen)
    }

    fn method_definition(&mut self) -> Option<Statement> {
        self.advance();
        if let TokenKind::Identifier(name) = &self.current().kind {
            self.method_names.insert(name.clone());
        }
        self.function_definition()
    }
}

/// What one round of the postfix/infix loop produced.
enum Extension {
    Extended(Expression),
    Finished(Expression),
}

fn exportable_name(statement: &Statement) -> Option<&str> {
    match statement {
        Statement::Function { name, .. }
        | Statement::Declaration { name, .. }
        | Statement::TypeDeclaration { name, .. }
        | Statement::EnumDeclaration { name, .. } => Some(name),
        _ => None,
    }
}

fn value_type_name(kind: ValueType) -> &'static str {
    match kind {
        ValueType::Int => "int",
        ValueType::Float => "float",
        ValueType::Bool => "bool",
        ValueType::String => "string",
        ValueType::Color => "color",
        ValueType::Line => "line",
        ValueType::Label => "label",
        ValueType::Box => "box",
        ValueType::Table => "table",
        ValueType::Linefill => "linefill",
        ValueType::Polyline => "polyline",
        ValueType::Array => "array",
        ValueType::Object => "object",
        ValueType::Map => "map",
        ValueType::Matrix => "matrix",
        ValueType::Time => "time",
    }
}

fn object_type(name: &str) -> Option<ValueType> {
    match name {
        "line" => Some(ValueType::Line),
        "label" => Some(ValueType::Label),
        "box" => Some(ValueType::Box),
        "table" => Some(ValueType::Table),
        "array" => Some(ValueType::Array),
        "linefill" => Some(ValueType::Linefill),
        "polyline" => Some(ValueType::Polyline),
        "map" => Some(ValueType::Map),
        "matrix" => Some(ValueType::Matrix),
        _ => None,
    }
}

fn qualifier_from_name(name: &str) -> Option<Qualifier> {
    match name {
        "const" => Some(Qualifier::Constant),
        "input" => Some(Qualifier::Input),
        "simple" => Some(Qualifier::Simple),
        "series" => Some(Qualifier::Series),
        _ => None,
    }
}

fn unary_operator_from_token(kind: &TokenKind) -> Option<UnaryOperator> {
    match kind {
        TokenKind::Minus => Some(UnaryOperator::Negate),
        TokenKind::Plus => Some(UnaryOperator::Plus),
        TokenKind::Not => Some(UnaryOperator::Not),
        _ => None,
    }
}

fn binary_operator_from_token(kind: &TokenKind) -> Option<BinaryOperator> {
    match kind {
        TokenKind::Or => Some(BinaryOperator::Or),
        TokenKind::And => Some(BinaryOperator::And),
        TokenKind::Equal => Some(BinaryOperator::Equal),
        TokenKind::NotEqual => Some(BinaryOperator::NotEqual),
        TokenKind::Less => Some(BinaryOperator::Less),
        TokenKind::LessEqual => Some(BinaryOperator::LessEqual),
        TokenKind::Greater => Some(BinaryOperator::Greater),
        TokenKind::GreaterEqual => Some(BinaryOperator::GreaterEqual),
        TokenKind::Plus => Some(BinaryOperator::Add),
        TokenKind::Minus => Some(BinaryOperator::Subtract),
        TokenKind::Star => Some(BinaryOperator::Multiply),
        TokenKind::Slash => Some(BinaryOperator::Divide),
        TokenKind::Percent => Some(BinaryOperator::Modulo),
        TokenKind::Power => Some(BinaryOperator::Power),
        _ => None,
    }
}

fn assignment_operator_from_token(kind: &TokenKind) -> Option<AssignmentOperator> {
    match kind {
        TokenKind::Reassign => Some(AssignmentOperator::Reassign),
        TokenKind::PlusAssign => Some(AssignmentOperator::Compound(BinaryOperator::Add)),
        TokenKind::MinusAssign => Some(AssignmentOperator::Compound(BinaryOperator::Subtract)),
        TokenKind::StarAssign => Some(AssignmentOperator::Compound(BinaryOperator::Multiply)),
        TokenKind::SlashAssign => Some(AssignmentOperator::Compound(BinaryOperator::Divide)),
        TokenKind::PercentAssign => Some(AssignmentOperator::Compound(BinaryOperator::Modulo)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pine::Lexer;

    fn parse(source: &str) -> (Vec<Statement>, Vec<Diagnostic>) {
        let lexer = Lexer::new(source);
        let lexed = lexer.lex();
        let mut parser = Parser::new(lexed.tokens, Limits::default(), HashSet::new());
        parser.parse()
    }

    #[test]
    fn parses_a_declaration() {
        let (statements, diagnostics) = parse("x = 1\n");
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        assert!(matches!(
            statements.as_slice(),
            [Statement::Declaration { name, .. }] if name == "x"
        ));
    }

    #[test]
    fn parses_operator_precedence() {
        let (statements, diagnostics) = parse("a = 1 + 2 * 3\n");
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        let Statement::Declaration { value, .. } = &statements[0] else {
            panic!("expected declaration");
        };
        // `1 + (2 * 3)` — the top-level operator is `+`.
        assert!(matches!(
            value,
            Expression::Binary {
                op: BinaryOperator::Add,
                ..
            }
        ));
    }

    #[test]
    fn parses_a_call_with_arguments() {
        let (statements, diagnostics) = parse("plot(ta.sma(close, length = 14))\n");
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        let Statement::Expression(Expression::Call {
            name, arguments, ..
        }) = &statements[0]
        else {
            panic!("expected call");
        };
        assert_eq!(name, "plot");
        assert_eq!(arguments.len(), 1);
    }

    #[test]
    fn parses_an_if_block() {
        let source = "if close > 0\n    plot(close)\n";
        let (statements, diagnostics) = parse(source);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        assert!(matches!(
            statements.as_slice(),
            [Statement::Conditional { when_true, when_false, .. }]
                if when_true.len() == 1 && when_false.is_empty()
        ));
    }

    #[test]
    fn a_dotted_name_spans_the_whole_name() {
        let (statements, diagnostics) = parse("array.resize(a, 5)\n");
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        let Statement::Expression(Expression::Call { name, range, .. }) = &statements[0] else {
            panic!("expected call");
        };
        assert_eq!(name, "array.resize");
        assert_eq!((range.start.column, range.end.column), (1, 13));
        assert_eq!((range.start.offset, range.end.offset), (0, 12));
    }

    #[test]
    fn a_longer_dotted_name_and_a_member_of_a_call_span_what_they_read() {
        let (statements, _) =
            parse("x = strategy.closedtrades.profit(0)\ny = array.get(a, 0).top\n");
        let Statement::Declaration { value, .. } = &statements[0] else {
            panic!("expected declaration");
        };
        let Expression::Call { name, range, .. } = value else {
            panic!("expected call, got {value:?}");
        };
        assert_eq!(name, "strategy.closedtrades.profit");
        assert_eq!(range.end.offset - range.start.offset, name.len());
        let Statement::Declaration { value, .. } = &statements[1] else {
            panic!("expected declaration");
        };
        let Expression::Member { range, .. } = value else {
            panic!("expected member, got {value:?}");
        };
        // From the start of `array.get(a, 0)` to the end of `.top`.
        assert_eq!(
            range.end.offset - range.start.offset,
            "array.get(a, 0).top".len()
        );
    }

    #[test]
    fn a_plain_name_keeps_its_range() {
        let (statements, _) = parse("plot(close)\n");
        let Statement::Expression(Expression::Call { range, .. }) = &statements[0] else {
            panic!("expected call");
        };
        assert_eq!((range.start.offset, range.end.offset), (0, 4));
    }
}
