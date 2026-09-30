import Foundation

indirect enum PineExpression: Sendable {
    case literal(PineRuntimeValue, PineSourceRange)
    case identifier(String, PineSourceRange)
    case unary(PineTokenKind, PineExpression, PineSourceRange)
    case binary(PineExpression, PineTokenKind, PineExpression, PineSourceRange)
    case ternary(PineExpression, PineExpression, PineExpression, PineSourceRange)
    case call(String, [PineArgument], Int, PineSourceRange)
    case history(PineExpression, PineExpression, PineSourceRange)
    case tuple([PineExpression], PineSourceRange)
    /// `x = if …` / `x = switch …`: a block statement used for its value.
    case statementExpression(PineStatement, PineSourceRange)
    var range: PineSourceRange {
        switch self {
        case .literal(_, let r), .identifier(_, let r), .unary(_, _, let r), .binary(_, _, _, let r),
            .ternary(_, _, _, let r), .call(_, _, _, let r), .history(_, _, let r), .tuple(_, let r),
            .statementExpression(_, let r):
            return r
        }
    }
}
