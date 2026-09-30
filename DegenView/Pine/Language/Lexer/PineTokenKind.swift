import Foundation

enum PineTokenKind: Equatable, Sendable {
    case identifier(String)
    case number(Double, isInteger: Bool)
    case string(String)
    case bool(Bool)
    case color(UInt32)
    case na
    case newline, indent, dedent, eof
    case leftParen, rightParen, leftBracket, rightBracket, comma, dot, question, colon
    case assign, reassign, plus, minus, star, slash, percent, power
    case plusAssign, minusAssign, starAssign, slashAssign
    case equal, notEqual, less, lessEqual, greater, greaterEqual
    case and, or, not, ifKeyword, elseKeyword, varKeyword, varipKeyword
    case forKeyword, breakKeyword, continueKeyword, whileKeyword, switchKeyword
    case typeKeyword(PineValueType)
    case arrow
}
