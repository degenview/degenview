import Foundation

extension PineExpression {
    /// Direct subexpressions. A `statementExpression`'s body is statements, reached through
    /// `PineStatement.forEachCall(in:_:)` instead.
    var children: [PineExpression] {
        switch self {
        case .literal, .identifier, .statementExpression: []
        case .unary(_, let operand, _): [operand]
        case .binary(let lhs, _, let rhs, _): [lhs, rhs]
        case .ternary(let condition, let whenTrue, let whenFalse, _): [condition, whenTrue, whenFalse]
        case .call(_, let arguments, _, _): arguments.map(\.value)
        case .history(let base, let offset, _): [base, offset]
        case .tuple(let values, _): values
        }
    }
}

extension PineStatement {
    /// Expressions this statement owns directly (not those of nested statements).
    var expressions: [PineExpression] {
        switch self {
        case .declaration(_, _, _, let value, _), .assignment(_, _, let value, _),
            .tupleDeclaration(_, let value, _):
            [value]
        case .fieldAssignment(let target, _, let value, _): [target, value]
        case .expression(let expression): [expression]
        case .conditional(let condition, _, _, _), .whileLoop(let condition, _, _): [condition]
        case .forRange(_, let from, let to, let step, _, _): [from, to] + (step.map { [$0] } ?? [])
        case .forIn(_, _, let collection, _, _): [collection]
        case .switchStatement(let subject, let arms, _):
            (subject.map { [$0] } ?? []) + arms.compactMap(\.condition)
        case .loopControl, .function: []
        case .typeDeclaration(_, let fields, _): fields.compactMap(\.defaultValue)
        }
    }

    /// Directly nested statement lists: branches, loop and function bodies, switch arms.
    var nestedBlocks: [[PineStatement]] {
        switch self {
        case .conditional(_, let whenTrue, let whenFalse, _): [whenTrue, whenFalse]
        case .forRange(_, _, _, _, let body, _), .forIn(_, _, _, let body, _),
            .whileLoop(_, let body, _), .function(_, _, let body, _):
            [body]
        case .switchStatement(_, let arms, _): arms.map(\.body)
        case .declaration, .assignment, .expression, .tupleDeclaration, .loopControl, .typeDeclaration,
            .fieldAssignment:
            []
        }
    }

    /// Names that are the target of `:=` or a compound assignment anywhere in `statements`.
    static func assignedNames(in statements: [PineStatement]) -> Set<String> {
        var names = Set<String>()
        for statement in statements {
            if case .assignment(let name, _, _, _) = statement { names.insert(name) }
            for block in statement.nestedBlocks { names.formUnion(assignedNames(in: block)) }
        }
        return names
    }

    /// Visits every call expression in `statements`, at any depth.
    static func forEachCall(
        in statements: [PineStatement], _ visit: (_ name: String, _ range: PineSourceRange) -> Void
    ) {
        forEachCall(withArgumentsIn: statements) { name, _, range in visit(name, range) }
    }

    /// Like ``forEachCall(in:_:)``, with each call's arguments.
    static func forEachCall(
        withArgumentsIn statements: [PineStatement],
        _ visit: (_ name: String, _ arguments: [PineArgument], _ range: PineSourceRange) -> Void
    ) {
        for statement in statements {
            for expression in statement.expressions { forEachCall(in: expression, visit) }
            for block in statement.nestedBlocks { forEachCall(withArgumentsIn: block, visit) }
        }
    }

    private static func forEachCall(
        in expression: PineExpression, _ visit: (String, [PineArgument], PineSourceRange) -> Void
    ) {
        if case .call(let name, let arguments, _, let range) = expression { visit(name, arguments, range) }
        if case .statementExpression(let statement, _) = expression {
            forEachCall(withArgumentsIn: [statement], visit)
        }
        for child in expression.children { forEachCall(in: child, visit) }
    }
}
