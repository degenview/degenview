import Foundation

indirect enum PineExpression: Sendable {
    case literal(PineRuntimeValue, PineSourceRange)
    case identifier(String, PineSourceRange)
    case unary(PineUnaryOperator, operand: PineExpression, range: PineSourceRange)
    case binary(lhs: PineExpression, op: PineBinaryOperator, rhs: PineExpression, range: PineSourceRange)
    case ternary(
        condition: PineExpression, whenTrue: PineExpression, whenFalse: PineExpression,
        range: PineSourceRange)
    /// `site` numbers each call in source order; it keys per-call-site runtime state.
    case call(name: String, arguments: [PineArgument], site: Int, range: PineSourceRange)
    case history(base: PineExpression, offset: PineExpression, range: PineSourceRange)
    case tuple([PineExpression], PineSourceRange)
    /// `array.get(a, i).field`: a field read from something that is not a plain variable path.
    case member(base: PineExpression, name: String, range: PineSourceRange)
    /// `array.get(a, i).method(args)`, `zones.get(k).kill()`: a method called on something that is not
    /// a plain variable path. `site` numbers it like a call.
    case methodCall(
        receiver: PineExpression, name: String, arguments: [PineArgument], site: Int,
        range: PineSourceRange)
    /// `x = if …` / `x = switch …`: a block statement used for its value.
    case statementExpression(PineStatement, PineSourceRange)

    var range: PineSourceRange {
        switch self {
        case .literal(_, let r), .identifier(_, let r), .unary(_, _, let r), .binary(_, _, _, let r),
            .ternary(_, _, _, let r), .call(_, _, _, let r), .history(_, _, let r), .tuple(_, let r),
            .member(_, _, let r), .methodCall(_, _, _, _, let r), .statementExpression(_, let r):
            return r
        }
    }
}
