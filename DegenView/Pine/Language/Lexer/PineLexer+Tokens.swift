import Foundation

extension PineLexer {
    static let keywords: [String: PineTokenKind] = [
        "true": .bool(true), "false": .bool(false), "na": .na, "and": .and, "or": .or,
        "not": .not, "if": .ifKeyword, "else": .elseKeyword, "var": .varKeyword,
        "varip": .varipKeyword, "for": .forKeyword, "break": .breakKeyword,
        "continue": .continueKeyword, "while": .whileKeyword, "switch": .switchKeyword,
        "int": .typeKeyword(.int), "float": .typeKeyword(.float), "bool": .typeKeyword(.bool),
        "string": .typeKeyword(.string), "color": .typeKeyword(.color),
    ]

    static let twoCharacterOperators: [String: PineTokenKind] = [
        ":=": .reassign, "+=": .plusAssign, "-=": .minusAssign, "*=": .starAssign,
        "/=": .slashAssign, "==": .equal, "!=": .notEqual, "<=": .lessEqual, ">=": .greaterEqual,
        "**": .power, "=>": .arrow,
    ]

    static let oneCharacterOperators: [Character: PineTokenKind] = [
        "(": .leftParen, ")": .rightParen, "[": .leftBracket, "]": .rightBracket, ",": .comma,
        ".": .dot, "?": .question, ":": .colon, "=": .assign, "+": .plus, "-": .minus, "*": .star,
        "/": .slash, "%": .percent, "<": .less, ">": .greater,
    ]

    /// Tokens after which a line continues onto the next one.
    static let continuationKinds: [PineTokenKind] = [
        .assign, .reassign, .plus, .minus, .star, .slash, .percent, .power, .and, .or, .comma,
        .question, .colon,
    ]

    /// Longest hex color body (`#RRGGBBAA`).
    private static let maxColorDigits = 8

    func lexWord(at start: Int, _ line: PineSourceLine, _ state: inout State) -> Int {
        var i = start + 1
        while i < line.text.count,
            line.text[i].isLetter || line.text[i].isNumber || line.text[i] == "_"
        {
            i += 1
        }
        let word = String(line.text[start..<i])
        state.tokens.append(
            .init(kind: Self.keywords[word] ?? .identifier(word), range: line.range(start, i)))
        return i
    }

    func lexNumber(at start: Int, _ line: PineSourceLine, _ state: inout State) -> Int {
        var i = start + 1
        var hasFraction = line.text[start] == "."
        while i < line.text.count,
            Self.isDigit(line.text[i]) || (!hasFraction && line.text[i] == ".")
        {
            if line.text[i] == "." { hasFraction = true }
            i += 1
        }
        if let e = line.character(at: i), e == "e" || e == "E" {
            i += 1
            if let sign = line.character(at: i), sign == "+" || sign == "-" { i += 1 }
            while Self.isDigit(line.character(at: i)) { i += 1 }
            hasFraction = true
        }
        let value = Double(String(line.text[start..<i])) ?? 0
        state.tokens.append(
            .init(
                kind: .number(value, isInteger: !hasFraction), range: line.range(start, i)))
        return i
    }

    func lexString(at start: Int, _ line: PineSourceLine, _ state: inout State) -> Int {
        var i = start + 1
        var value = ""
        var closed = false
        while i < line.text.count {
            let c = line.text[i]
            if c == "\"" {
                i += 1
                closed = true
                break
            }
            if c == "\\", let escaped = line.character(at: i + 1) {
                i += 1
                value.append(escaped == "n" ? "\n" : escaped)
            } else {
                value.append(c)
            }
            i += 1
        }
        let range = line.range(start, i)
        if !closed {
            state.diagnostics.append(
                .error("PINE1003", .lexical, "Unterminated string literal.", range))
        }
        state.tokens.append(.init(kind: .string(value), range: range))
        return i
    }

    func lexColor(at start: Int, _ line: PineSourceLine, _ state: inout State) -> Int {
        var end = start + 1
        while end < line.text.count, line.text[end].isASCII, line.text[end].isHexDigit,
            end - start <= Self.maxColorDigits
        {
            end += 1
        }
        let digitCount = end - start - 1
        guard digitCount == 6 || digitCount == 8,
            let raw = UInt32(String(line.text[(start + 1)..<end]), radix: 16)
        else {
            state.diagnostics.append(
                .error(
                    "PINE1004", .lexical, "A hex color requires exactly six or eight digits.",
                    line.range(start, end)))
            return end
        }
        let rgba = digitCount == 6 ? (raw << 8) | 0xFF : raw
        state.tokens.append(.init(kind: .color(rgba), range: line.range(start, end)))
        return end
    }

    func lexOperator(at start: Int, _ line: PineSourceLine, _ state: inout State) -> Int {
        let c = line.text[start]
        if let next = line.character(at: start + 1),
            let kind = Self.twoCharacterOperators[String([c, next])]
        {
            state.tokens.append(.init(kind: kind, range: line.range(start, start + 2)))
            return start + 2
        }
        if let kind = Self.oneCharacterOperators[c] {
            state.tokens.append(.init(kind: kind, range: line.range(start, start + 1)))
            if c == "(" || c == "[" { state.delimiterDepth += 1 }
            if c == ")" || c == "]" { state.delimiterDepth = max(0, state.delimiterDepth - 1) }
        } else {
            state.diagnostics.append(
                .error(
                    "PINE1001", .lexical, "Unexpected character '\(c)'.",
                    line.range(start, start + 1)))
        }
        return start + 1
    }
}
