//! Pine Script abstract syntax tree: the node types `Parser` produces and the type
//! checker / runtime / editor consume. Mirrors `Pine/Language/AST/*.swift`.

use crate::pine::{SourceRange, ValueType};

/// A literal written in source. Runtime-only kinds (tuples, object handles) are not literals.
#[derive(Clone, Debug, PartialEq)]
pub enum Literal {
    Int(i64),
    Float(f64),
    Bool(bool),
    String(String),
    Color(u32),
    Na,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Expression {
    Literal(Literal, SourceRange),
    Identifier(String, SourceRange),
    Unary {
        op: UnaryOperator,
        operand: Box<Expression>,
        range: SourceRange,
    },
    Binary {
        lhs: Box<Expression>,
        op: BinaryOperator,
        rhs: Box<Expression>,
        range: SourceRange,
    },
    Ternary {
        condition: Box<Expression>,
        when_true: Box<Expression>,
        when_false: Box<Expression>,
        range: SourceRange,
    },
    /// `site` numbers each call in source order; it keys per-call-site runtime state.
    Call {
        name: String,
        arguments: Vec<Argument>,
        site: usize,
        range: SourceRange,
    },
    History {
        base: Box<Expression>,
        offset: Box<Expression>,
        range: SourceRange,
    },
    Tuple(Vec<Expression>, SourceRange),
    /// `array.get(a, i).field`: a field read from something that is not a plain variable path.
    Member {
        base: Box<Expression>,
        name: String,
        range: SourceRange,
    },
    /// A method called on something that is not a plain variable path.
    MethodCall {
        receiver: Box<Expression>,
        name: String,
        arguments: Vec<Argument>,
        site: usize,
        range: SourceRange,
    },
    /// `x = if …` / `x = switch …`: a block statement used for its value.
    Statement(Box<Statement>, SourceRange),
}

impl Expression {
    pub fn range(&self) -> SourceRange {
        match self {
            Self::Literal(_, range)
            | Self::Identifier(_, range)
            | Self::Unary { range, .. }
            | Self::Binary { range, .. }
            | Self::Ternary { range, .. }
            | Self::Call { range, .. }
            | Self::History { range, .. }
            | Self::Tuple(_, range)
            | Self::Member { range, .. }
            | Self::MethodCall { range, .. }
            | Self::Statement(_, range) => *range,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Statement {
    Declaration {
        name: String,
        annotation: TypeAnnotation,
        mode: DeclarationMode,
        value: Expression,
        range: SourceRange,
    },
    Assignment {
        name: String,
        op: AssignmentOperator,
        value: Expression,
        range: SourceRange,
    },
    /// `zone.top := value`, `a.get(i).count += 1`: assignment to a field of a user-defined type.
    FieldAssignment {
        target: Expression,
        op: AssignmentOperator,
        value: Expression,
        range: SourceRange,
    },
    Expression(Expression),
    Conditional {
        condition: Expression,
        when_true: Vec<Statement>,
        when_false: Vec<Statement>,
        range: SourceRange,
    },
    /// `for i = from to end [by step]`
    ForRange {
        variable: String,
        from: Expression,
        to: Expression,
        step: Option<Expression>,
        body: Vec<Statement>,
        range: SourceRange,
    },
    /// `for value in array` / `for [index, value] in array`
    ForIn {
        index: Option<String>,
        value: String,
        collection: Expression,
        body: Vec<Statement>,
        range: SourceRange,
    },
    WhileLoop {
        condition: Expression,
        body: Vec<Statement>,
        range: SourceRange,
    },
    /// `switch subject` compares each arm's condition to the subject; `switch` without a
    /// subject takes the first arm whose condition is true.
    Switch {
        subject: Option<Expression>,
        arms: Vec<SwitchArm>,
        range: SourceRange,
    },
    /// `[a, b] = expression`
    TupleDeclaration {
        names: Vec<String>,
        value: Expression,
        range: SourceRange,
    },
    LoopControl(LoopControl, SourceRange),
    /// `type Name` and its indented field lines.
    TypeDeclaration {
        name: String,
        fields: Vec<TypeField>,
        range: SourceRange,
    },
    /// `enum Name` and its indented member lines.
    EnumDeclaration {
        name: String,
        members: Vec<EnumMember>,
        range: SourceRange,
    },
    /// User-defined function. The body's last statement is the return value.
    Function {
        name: String,
        parameters: Vec<Parameter>,
        body: Vec<Statement>,
        range: SourceRange,
    },
}

impl Statement {
    /// Where the statement starts, as the parser ranged it. Ranges are start-anchored: a
    /// declaration is ranged at its name, a block statement at its keyword, an expression
    /// statement at its first token.
    pub fn range(&self) -> SourceRange {
        match self {
            Self::Declaration { range, .. }
            | Self::Assignment { range, .. }
            | Self::FieldAssignment { range, .. }
            | Self::Conditional { range, .. }
            | Self::ForRange { range, .. }
            | Self::ForIn { range, .. }
            | Self::WhileLoop { range, .. }
            | Self::Switch { range, .. }
            | Self::TupleDeclaration { range, .. }
            | Self::TypeDeclaration { range, .. }
            | Self::EnumDeclaration { range, .. }
            | Self::Function { range, .. }
            | Self::LoopControl(_, range) => *range,
            Self::Expression(expression) => expression.range(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UnaryOperator {
    Negate,
    Plus,
    Not,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BinaryOperator {
    Or,
    And,
    Equal,
    NotEqual,
    Less,
    LessEqual,
    Greater,
    GreaterEqual,
    Add,
    Subtract,
    Multiply,
    Divide,
    Modulo,
    Power,
}

impl BinaryOperator {
    pub fn symbol(self) -> &'static str {
        match self {
            Self::Or => "or",
            Self::And => "and",
            Self::Equal => "==",
            Self::NotEqual => "!=",
            Self::Less => "<",
            Self::LessEqual => "<=",
            Self::Greater => ">",
            Self::GreaterEqual => ">=",
            Self::Add => "+",
            Self::Subtract => "-",
            Self::Multiply => "*",
            Self::Divide => "/",
            Self::Modulo => "%",
            Self::Power => "**",
        }
    }

    pub fn is_comparison(self) -> bool {
        matches!(
            self,
            Self::Equal
                | Self::NotEqual
                | Self::Less
                | Self::LessEqual
                | Self::Greater
                | Self::GreaterEqual
        )
    }

    pub fn is_arithmetic(self) -> bool {
        matches!(
            self,
            Self::Add | Self::Subtract | Self::Multiply | Self::Divide | Self::Modulo | Self::Power
        )
    }

    /// Pratt-parser binding powers. `power` is right-associative.
    pub fn binding_power(self) -> (u8, u8) {
        match self {
            Self::Or => (10, 11),
            Self::And => (20, 21),
            Self::Equal | Self::NotEqual => (30, 31),
            Self::Less | Self::LessEqual | Self::Greater | Self::GreaterEqual => (40, 41),
            Self::Add | Self::Subtract => (50, 51),
            Self::Multiply | Self::Divide | Self::Modulo => (60, 61),
            Self::Power => (70, 70),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AssignmentOperator {
    /// `:=`
    Reassign,
    /// `+=`, `-=`, `*=`, `/=`
    Compound(BinaryOperator),
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum DeclarationMode {
    #[default]
    Ordinary,
    Variable,
    Intrabar,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LoopControl {
    Break,
    Continue,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Qualifier {
    Constant,
    Input,
    Simple,
    Series,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TypeAnnotation {
    pub kind: Option<ValueType>,
    pub qualifier: Option<Qualifier>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Argument {
    pub name: Option<String>,
    pub value: Expression,
    /// Where the `name =` label sits, for diagnostics about the argument itself.
    pub name_range: Option<SourceRange>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Parameter {
    pub name: String,
    pub kind: Option<ValueType>,
    /// The script-defined type (or enum, or `chart.point`) the parameter was annotated with.
    pub type_name: Option<String>,
    pub default_value: Option<Expression>,
}

/// One `switch` arm; a nil condition is the default (`=> value`) arm.
#[derive(Clone, Debug, PartialEq)]
pub struct SwitchArm {
    pub condition: Option<Expression>,
    pub body: Vec<Statement>,
}

/// One field of a script-defined `type`: `[type] name [= default]`.
#[derive(Clone, Debug, PartialEq)]
pub struct TypeField {
    pub name: String,
    pub kind: Option<ValueType>,
    pub default_value: Option<Expression>,
}

/// One member of a script-defined `enum`: `name [= "title"]`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EnumMember {
    pub name: String,
    pub title: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binary_binding_power_precedence() {
        assert!(
            BinaryOperator::Power.binding_power().0 > BinaryOperator::Multiply.binding_power().0
        );
        assert!(BinaryOperator::Multiply.binding_power().0 > BinaryOperator::Add.binding_power().0);
        assert!(BinaryOperator::Add.binding_power().0 > BinaryOperator::Equal.binding_power().0);
        assert!(BinaryOperator::Equal.binding_power().0 > BinaryOperator::And.binding_power().0);
        assert!(BinaryOperator::And.binding_power().0 > BinaryOperator::Or.binding_power().0);
        // Power is right-associative.
        assert_eq!(BinaryOperator::Power.binding_power(), (70, 70));
    }

    #[test]
    fn expression_range_is_preserved() {
        let range = SourceRange::ZERO;
        let expr = Expression::Identifier("x".to_owned(), range);
        assert_eq!(expr.range(), range);
    }
}
