import Foundation

// A compact, deliberately self-contained compiler pipeline. Tokens and AST retain source
// ranges; the VM never evaluates source text and has no access to Foundation I/O APIs.
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

struct PineToken: Equatable, Sendable {
    let kind: PineTokenKind
    let range: PineSourceRange
}

struct PineLexer {
    let source: String
    let limits: PineLimits

    func lex() -> (tokens: [PineToken], diagnostics: [PineDiagnostic]) {
        if source.count > limits.sourceCharacters {
            return (
                [],
                [
                    diag(
                        "PINE8001", .resource, "Source exceeds the \(limits.sourceCharacters)-character limit.",
                        .zero)
                ]
            )
        }
        var tokens: [PineToken] = []
        var diagnostics: [PineDiagnostic] = []
        var offsets = 0
        var indents = [0]
        var delimiterDepth = 0
        var continued = false
        let lines = source.split(separator: "\n", omittingEmptySubsequences: false)
        for (lineIndex, raw) in lines.enumerated() {
            let text = String(raw)
            let lineNo = lineIndex + 1
            let leading = text.prefix { $0 == " " }.count
            let trimmed = text.dropFirst(leading)
            if trimmed.isEmpty || trimmed.hasPrefix("//") {
                offsets += text.utf16.count + 1
                continue
            }
            if delimiterDepth == 0 && !continued && leading % 4 != 0 {
                diagnostics.append(
                    diag(
                        "PINE1002", .lexical, "Indentation must use multiples of four spaces.",
                        range(lineNo, 1, offsets, max(1, leading))))
            }
            if delimiterDepth == 0 && !continued {
                if leading > indents.last! {
                    indents.append(leading)
                    tokens.append(.init(kind: .indent, range: range(lineNo, 1, offsets, leading)))
                }
                while leading < indents.last! {
                    indents.removeLast()
                    tokens.append(.init(kind: .dedent, range: range(lineNo, 1, offsets, leading)))
                }
            }
            var i = leading
            while i < text.count {
                let chars = Array(text)
                let c = chars[i]
                if c == " " || c == "\t" {
                    i += 1
                    continue
                }
                if c == "/", i + 1 < chars.count, chars[i + 1] == "/" { break }
                let start = i
                if c.isLetter || c == "_" {
                    i += 1
                    while i < chars.count && (chars[i].isLetter || chars[i].isNumber || chars[i] == "_") {
                        i += 1
                    }
                    let word = String(chars[start..<i])
                    let kind: PineTokenKind
                    switch word {
                    case "true": kind = .bool(true)
                    case "false": kind = .bool(false)
                    case "na": kind = .na
                    case "and": kind = .and
                    case "or": kind = .or
                    case "not": kind = .not
                    case "if": kind = .ifKeyword
                    case "else": kind = .elseKeyword
                    case "var": kind = .varKeyword
                    case "varip": kind = .varipKeyword
                    case "for": kind = .forKeyword
                    case "break": kind = .breakKeyword
                    case "continue": kind = .continueKeyword
                    case "while": kind = .whileKeyword
                    case "switch": kind = .switchKeyword
                    case "int": kind = .typeKeyword(.int)
                    case "float": kind = .typeKeyword(.float)
                    case "bool": kind = .typeKeyword(.bool)
                    case "string": kind = .typeKeyword(.string)
                    case "color": kind = .typeKeyword(.color)
                    default: kind = .identifier(word)
                    }
                    tokens.append(
                        .init(kind: kind, range: range(lineNo, start + 1, offsets + start, i - start)))
                    continue
                }
                if c.isNumber || (c == "." && i + 1 < chars.count && chars[i + 1].isNumber) {
                    i += 1
                    var dot = c == "."
                    while i < chars.count && (chars[i].isNumber || (!dot && chars[i] == ".")) {
                        if chars[i] == "." { dot = true }
                        i += 1
                    }
                    if i < chars.count && (chars[i] == "e" || chars[i] == "E") {
                        i += 1
                        if i < chars.count && (chars[i] == "+" || chars[i] == "-") { i += 1 }
                        while i < chars.count && chars[i].isNumber { i += 1 }
                        dot = true
                    }
                    let rawNumber = String(chars[start..<i])
                    tokens.append(
                        .init(
                            kind: .number(Double(rawNumber) ?? 0, isInteger: !dot),
                            range: range(lineNo, start + 1, offsets + start, i - start)))
                    continue
                }
                if c == "\"" {
                    i += 1
                    var value = ""
                    var closed = false
                    while i < chars.count {
                        if chars[i] == "\"" {
                            i += 1
                            closed = true
                            break
                        }
                        if chars[i] == "\\", i + 1 < chars.count {
                            i += 1
                            value.append(chars[i] == "n" ? "\n" : chars[i])
                        } else {
                            value.append(chars[i])
                        }
                        i += 1
                    }
                    if !closed {
                        diagnostics.append(
                            diag(
                                "PINE1003", .lexical, "Unterminated string literal.",
                                range(lineNo, start + 1, offsets + start, i - start)))
                    }
                    tokens.append(
                        .init(kind: .string(value), range: range(lineNo, start + 1, offsets + start, i - start))
                    )
                    continue
                }
                if c == "#" {
                    var end = i + 1
                    while end < chars.count, chars[end].isHexDigit, end - i <= 8 { end += 1 }
                    let digitCount = end - i - 1
                    guard digitCount == 6 || digitCount == 8 else {
                        diagnostics.append(
                            diag(
                                "PINE1004", .lexical, "A hex color requires exactly six or eight digits.",
                                range(lineNo, start + 1, offsets + start, max(1, digitCount + 1))))
                        i = end
                        continue
                    }

                    let digits = String(chars[(i + 1)..<end])
                    let raw = UInt32(digits, radix: 16)!

                    let rgba = digitCount == 6 ? (raw << 8) | 0xFF : raw
                    tokens.append(
                        .init(
                            kind: .color(rgba),
                            range: range(lineNo, start + 1, offsets + start, digitCount + 1)))
                    i += digitCount + 1
                    continue
                }
                let pair = i + 1 < chars.count ? String(chars[i...i + 1]) : ""
                let two: [String: PineTokenKind] = [
                    ":=": .reassign, "+=": .plusAssign, "-=": .minusAssign, "*=": .starAssign,
                    "/=": .slashAssign, "==": .equal, "!=": .notEqual, "<=": .lessEqual, ">=": .greaterEqual,
                    "**": .power, "=>": .arrow,
                ]
                if let kind = two[pair] {
                    tokens.append(.init(kind: kind, range: range(lineNo, start + 1, offsets + start, 2)))
                    i += 2
                    continue
                }
                let one: [Character: PineTokenKind] = [
                    "(": .leftParen, ")": .rightParen, "[": .leftBracket, "]": .rightBracket, ",": .comma,
                    ".": .dot, "?": .question, ":": .colon, "=": .assign, "+": .plus, "-": .minus, "*": .star,
                    "/": .slash, "%": .percent, "<": .less, ">": .greater,
                ]
                if let kind = one[c] {
                    tokens.append(.init(kind: kind, range: range(lineNo, start + 1, offsets + start, 1)))
                    if c == "(" || c == "[" { delimiterDepth += 1 }
                    if c == ")" || c == "]" { delimiterDepth = max(0, delimiterDepth - 1) }
                } else {
                    diagnostics.append(
                        diag(
                            "PINE1001", .lexical, "Unexpected character '\(c)'.",
                            range(lineNo, start + 1, offsets + start, 1)))
                }
                i += 1
            }
            let continuationKinds: [PineTokenKind] = [
                .assign, .reassign, .plus, .minus, .star, .slash, .percent, .power, .and, .or, .comma,
                .question, .colon,
            ]
            continued =
                delimiterDepth > 0 || tokens.last.map { continuationKinds.contains($0.kind) } == true
            if !continued {
                tokens.append(
                    .init(kind: .newline, range: range(lineNo, text.count + 1, offsets + text.count, 0)))
            }
            offsets += text.utf16.count + 1
            if tokens.count > limits.tokens {
                diagnostics.append(
                    diag("PINE8002", .resource, "Token limit exceeded.", tokens.last?.range ?? .zero))
                break
            }
        }
        let eofRange = range(
            lines.count,
            (lines.last?.count ?? 0) + 1,
            source.utf16.count,
            0
        )
        while indents.count > 1 {
            indents.removeLast()
            tokens.append(.init(kind: .dedent, range: eofRange))
        }
        tokens.append(.init(kind: .eof, range: eofRange))
        return (tokens, diagnostics)
    }

    private func range(_ line: Int, _ column: Int, _ offset: Int, _ length: Int) -> PineSourceRange {
        .init(
            start: .init(line: line, column: column, offset: offset),
            end: .init(line: line, column: column + length, offset: offset + length))
    }
}

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
struct PineArgument: Sendable {
    var name: String?
    var value: PineExpression
}
enum PineDeclarationMode: Sendable { case ordinary, variable, intrabar }
enum PineLoopControl: Sendable { case breakLoop, continueLoop }
struct PineParameter: Sendable {
    var name: String
    var type: PineValueType?
    var defaultValue: PineExpression?
}
/// One `switch` arm; a nil condition is the default (`=> value`) arm.
struct PineSwitchArm: Sendable {
    var condition: PineExpression?
    var body: [PineStatement]
}
indirect enum PineStatement: Sendable {
    case declaration(String, PineTypeAnnotation, PineDeclarationMode, PineExpression, PineSourceRange)
    case assignment(String, PineTokenKind, PineExpression, PineSourceRange)
    case expression(PineExpression)
    case conditional(PineExpression, [PineStatement], [PineStatement], PineSourceRange)
    /// `for i = from to end [by step]`
    case forRange(String, PineExpression, PineExpression, PineExpression?, [PineStatement], PineSourceRange)
    /// `for value in array` / `for [index, value] in array`
    case forIn(String?, String, PineExpression, [PineStatement], PineSourceRange)
    case whileLoop(PineExpression, [PineStatement], PineSourceRange)
    /// `switch subject` compares each arm's condition to the subject; `switch` without a
    /// subject takes the first arm whose condition is true.
    case switchStatement(PineExpression?, [PineSwitchArm], PineSourceRange)
    /// `[a, b] = expression`
    case tupleDeclaration([String], PineExpression, PineSourceRange)
    case loopControl(PineLoopControl, PineSourceRange)
    /// User-defined function. The body's last statement is the return value.
    case function(String, [PineParameter], [PineStatement], PineSourceRange)
}

struct PineParser {
    var tokens: [PineToken]
    var index = 0
    var diagnostics: [PineDiagnostic] = []
    var callSite = 0
    let limits: PineLimits
    mutating func parse() -> ([PineStatement], [PineDiagnostic]) {
        let result = block(untilDedent: false)
        return (result, diagnostics)
    }
    mutating private func block(untilDedent: Bool) -> [PineStatement] {
        var out: [PineStatement] = []
        while !at(.eof) && !(untilDedent && at(.dedent)) {
            if take(.newline) || take(.indent) { continue }
            if at(.dedent) {
                advance()
                continue
            }
            let before = index
            let parsed = statement()
            if let parsed { out.append(parsed) }
            if index == before {
                error("PINE2001", "Expected a statement.", current.range)
                advance()
            } else {
                endStatement(report: parsed != nil)
            }
            while take(.newline) {}
            if out.count > limits.astNodes {
                error("PINE8003", "AST node limit exceeded.", current.range)
                break
            }
        }
        if untilDedent { _ = take(.dedent) }
        return out
    }
    /// A statement must end the line. Block statements (`if`, `for`, block functions) end
    /// on the dedent their body consumed, so the next line's first token follows directly.
    /// Leftover tokens are reported once (unless the statement already failed) and skipped
    /// to the end of the line.
    mutating private func endStatement(report: Bool) {
        if at(.newline) || at(.eof) || at(.dedent) || previous.kind == .dedent { return }
        if report { error("PINE2013", "Expected end of statement; found unexpected token.", current.range) }
        while !at(.newline) && !at(.dedent) && !at(.eof) { advance() }
    }

    mutating private func statement() -> PineStatement? {
        if take(.ifKeyword) { return ifStatement() }
        if take(.forKeyword) { return forStatement() }
        if take(.whileKeyword) { return whileStatement() }
        if take(.switchKeyword) { return switchStatement() }
        if isTupleDeclaration() { return tupleDeclaration() }
        if take(.breakKeyword) { return .loopControl(.breakLoop, previous.range) }
        if take(.continueKeyword) { return .loopControl(.continueLoop, previous.range) }
        if isFunctionDefinition() { return functionDefinition() }
        if let declaration = declaration() { return declaration }
        if case .identifier(let name) = current.kind, let next = peek(1),
            [.reassign, .plusAssign, .minusAssign, .starAssign, .slashAssign].contains(next.kind)
        {
            let range = current.range
            advance()
            let op = current.kind
            advance()
            guard let rhs = expression() else { return nil }
            return .assignment(name, op, rhs, range)
        }
        return expression().map(PineStatement.expression)
    }

    /// Called after `if` has been consumed. `else if` nests the chained `if` as the
    /// sole statement of the else branch.
    mutating private func ifStatement() -> PineStatement? {
        let start = previous.range
        guard let condition = expression() else { return nil }
        guard let yes = indentedBlock("if") else { return nil }
        var no: [PineStatement] = []
        if take(.elseKeyword) {
            if take(.ifKeyword) {
                if let chained = ifStatement() { no = [chained] }
            } else {
                _ = take(.newline)
                if take(.indent) { no = block(untilDedent: true) }
            }
        }
        return .conditional(condition, yes, no, start)
    }

    mutating private func indentedBlock(_ owner: String) -> [PineStatement]? {
        _ = take(.newline)
        guard take(.indent) else {
            error("PINE2002", "Expected an indented block after \(owner).", current.range)
            return nil
        }
        return block(untilDedent: true)
    }

    /// Called after `while` has been consumed.
    mutating private func whileStatement() -> PineStatement? {
        let start = previous.range
        guard let condition = expression() else { return nil }
        guard let body = indentedBlock("while") else { return nil }
        return .whileLoop(condition, body, start)
    }

    /// Called after `switch` has been consumed.
    mutating private func switchStatement() -> PineStatement? {
        let start = previous.range
        var subject: PineExpression?
        if !at(.newline) {
            guard let value = expression() else { return nil }
            subject = value
        }
        _ = take(.newline)
        guard take(.indent) else {
            error("PINE2002", "Expected an indented block after switch.", current.range)
            return nil
        }
        var arms: [PineSwitchArm] = []
        while !at(.eof) && !at(.dedent) {
            if take(.newline) { continue }
            var condition: PineExpression?
            if !take(.arrow) {
                guard let value = expression() else {
                    while !at(.newline) && !at(.dedent) && !at(.eof) { advance() }
                    continue
                }
                condition = value
                expect(.arrow, "Expected '=>' in switch arm.")
            }
            let body: [PineStatement]
            if at(.newline) {
                guard let block = indentedBlock("switch arm") else { return nil }
                body = block
            } else if let single = statement() {
                body = [single]
            } else {
                return nil
            }
            arms.append(.init(condition: condition, body: body))
            while take(.newline) {}
        }
        _ = take(.dedent)
        return .switchStatement(subject, arms, start)
    }

    /// `[a, b] = …` — a bracketed name list directly followed by `=`.
    private func isTupleDeclaration() -> Bool {
        guard at(.leftBracket) else { return false }
        var i = index + 1
        var expectName = true
        while i < tokens.count {
            switch tokens[i].kind {
            case .identifier where expectName: expectName = false
            case .comma where !expectName: expectName = true
            case .rightBracket: return !expectName && i + 1 < tokens.count && tokens[i + 1].kind == .assign
            default: return false
            }
            i += 1
        }
        return false
    }

    mutating private func tupleDeclaration() -> PineStatement? {
        let range = current.range
        advance()
        var names: [String] = []
        while case .identifier(let name) = current.kind {
            names.append(name)
            advance()
            if !take(.comma) { break }
        }
        expect(.rightBracket, "Expected ']'.")
        expect(.assign, "Expected '='.")
        guard let rhs = expression() else { return nil }
        return .tupleDeclaration(names, rhs, range)
    }

    /// Called after `for` has been consumed.
    mutating private func forStatement() -> PineStatement? {
        let start = previous.range
        var indexName: String?
        let valueName: String
        if take(.leftBracket) {
            guard case .identifier(let first) = current.kind else {
                error("PINE2009", "Expected a loop variable.", current.range)
                return nil
            }
            advance()
            expect(.comma, "Expected ',' in for...in destructuring.")
            guard case .identifier(let second) = current.kind else {
                error("PINE2009", "Expected a loop variable.", current.range)
                return nil
            }
            advance()
            expect(.rightBracket, "Expected ']'.")
            indexName = first
            valueName = second
        } else {
            guard case .identifier(let name) = current.kind else {
                error("PINE2009", "Expected a loop variable.", current.range)
                return nil
            }
            advance()
            valueName = name
        }
        if indexName == nil, take(.assign) {
            guard let from = expression() else { return nil }
            guard takeWord("to") else {
                error("PINE2010", "Expected 'to' in for loop.", current.range)
                return nil
            }
            guard let end = expression() else { return nil }
            var step: PineExpression?
            if takeWord("by") {
                guard let value = expression() else { return nil }
                step = value
            }
            guard let body = indentedBlock("for") else { return nil }
            return .forRange(valueName, from, end, step, body, start)
        }
        guard takeWord("in") else {
            error("PINE2010", "Expected '=' or 'in' in for loop.", current.range)
            return nil
        }
        guard let collection = expression() else { return nil }
        guard let body = indentedBlock("for") else { return nil }
        return .forIn(indexName, valueName, collection, body, start)
    }

    /// `name(` … matching `)` followed by `=>`.
    private func isFunctionDefinition() -> Bool {
        guard case .identifier = current.kind, peek(1)?.kind == .leftParen else { return false }
        var depth = 0
        var i = index + 1
        while i < tokens.count {
            switch tokens[i].kind {
            case .leftParen: depth += 1
            case .rightParen:
                depth -= 1
                if depth == 0 { return i + 1 < tokens.count && tokens[i + 1].kind == .arrow }
            case .newline, .eof: return false
            default: break
            }
            i += 1
        }
        return false
    }

    mutating private func functionDefinition() -> PineStatement? {
        guard case .identifier(let name) = current.kind else { return nil }
        let range = current.range
        advance()
        expect(.leftParen, "Expected '('.")
        var parameters: [PineParameter] = []
        if !at(.rightParen) {
            repeat {
                skipQualifier()
                let type = typeAnnotation()
                guard case .identifier(let parameter) = current.kind else {
                    error("PINE2011", "Expected a parameter name.", current.range)
                    return nil
                }
                advance()
                var defaultValue: PineExpression?
                if take(.assign) { defaultValue = expression() }
                parameters.append(.init(name: parameter, type: type, defaultValue: defaultValue))
            } while take(.comma)
        }
        expect(.rightParen, "Expected ')'.")
        expect(.arrow, "Expected '=>'.")
        let body: [PineStatement]
        if at(.newline) {
            guard let block = indentedBlock("function declaration") else { return nil }
            body = block
        } else {
            guard let single = statement() else { return nil }
            body = [single]
        }
        return .function(name, parameters, body, range)
    }

    /// `[var|varip] [type] name = expression`. Parsed speculatively: when the tokens do
    /// not form a declaration header the cursor is restored so they parse as an expression.
    mutating private func declaration() -> PineStatement? {
        let start = index
        var mode = PineDeclarationMode.ordinary
        if take(.varKeyword) { mode = .variable } else if take(.varipKeyword) { mode = .intrabar }
        let qualifier = skipQualifier()
        let type = typeAnnotation()
        guard case .identifier(let name) = current.kind, peek(1)?.kind == .assign else {
            index = start
            return nil
        }
        let range = current.range
        advance()
        advance()
        guard let rhs = expression() else { return nil }
        return .declaration(name, .init(type: type, qualifier: qualifier), mode, rhs, range)
    }

    private static let objectTypes: [String: PineValueType] = [
        "line": .line, "label": .label, "box": .box, "table": .table, "array": .array,
    ]

    /// Parses `int`, `float[]`, `line`, `box[]`, `array<int>`. Only consumes tokens when
    /// they are followed by a name (or `[]`/`<`), so `line.new(...)` and `int(x)` still
    /// parse as expressions.
    mutating private func typeAnnotation() -> PineValueType? {
        var type: PineValueType
        switch current.kind {
        case .typeKeyword(let t): type = t
        case .identifier(let word):
            guard let t = Self.objectTypes[word] else { return nil }
            type = t
        default: return nil
        }
        guard let next = peek(1) else { return nil }
        if case .identifier = next.kind {
            advance()
            return type
        }
        if next.kind == .leftBracket, peek(2)?.kind == .rightBracket {
            advance()
            advance()
            advance()
            return .array
        }
        if type == .array, next.kind == .less {
            advance()
            advance()
            // Element type; only one-word element types are supported.
            switch current.kind {
            case .typeKeyword, .identifier: advance()
            default: error("PINE2012", "Expected an element type.", current.range)
            }
            expect(.greater, "Expected '>'.")
            return .array
        }
        return nil
    }

    @discardableResult
    mutating private func skipQualifier() -> PineQualifier? {
        let qualifiers: [String: PineQualifier] = [
            "const": .constant, "input": .input, "simple": .simple, "series": .series,
        ]
        guard case .identifier(let word) = current.kind, let qualifier = qualifiers[word],
            let next = peek(1)
        else { return nil }
        switch next.kind {
        case .typeKeyword, .identifier:
            advance()
            return qualifier
        default: return nil
        }
    }

    mutating private func takeWord(_ word: String) -> Bool {
        guard case .identifier(let value) = current.kind, value == word else { return false }
        advance()
        return true
    }

    mutating private func expression(_ minBP: Int = 0) -> PineExpression? {
        var lhs: PineExpression
        let token = current
        advance()
        switch token.kind {
        case .number(let n, let integer):
            lhs = .literal(integer ? .int(Int(n)) : .float(n), token.range)
        case .color(let value): lhs = .literal(.color(value), token.range)
        case .string(let s): lhs = .literal(.string(s), token.range)
        case .bool(let b): lhs = .literal(.bool(b), token.range)
        case .na:
            // `na(x)` is the builtin test; a bare `na` is the value.
            lhs = at(.leftParen) ? .identifier("na", token.range) : .literal(.na, token.range)
        case .identifier(let name): lhs = .identifier(name, token.range)
        case .typeKeyword(let type): lhs = .identifier(type.rawValue, token.range)
        case .minus, .plus, .not:
            guard let rhs = expression(80) else { return nil }
            lhs = .unary(token.kind, rhs, token.range)
        case .ifKeyword, .switchKeyword:
            // The block consumes its own dedent, so the postfix/binary loop below must not
            // run: the next line's first token could be mistaken for an operator.
            guard let block = token.kind == .ifKeyword ? ifStatement() : switchStatement() else {
                return nil
            }
            return .statementExpression(block, token.range)
        case .leftParen:
            guard let inner = expression() else { return nil }
            lhs = inner
            expect(.rightParen, "Expected ')'.")
        case .leftBracket:
            var values: [PineExpression] = []
            if !at(.rightBracket) {
                repeat { if let e = expression() { values.append(e) } } while take(.comma)
            }
            expect(.rightBracket, "Expected ']'.")
            lhs = .tuple(values, token.range)
        default:
            error("PINE2003", "Expected an expression.", token.range)
            return nil
        }
        while true {
            if take(.dot) {
                let member: String
                switch current.kind {
                case .identifier(let value): member = value
                case .typeKeyword(let type): member = type.rawValue
                default:
                    error("PINE2004", "Expected member name.", current.range)
                    return lhs
                }
                advance()
                guard case .identifier(let base, let r) = lhs else {
                    error("PINE2005", "Member access requires a namespace.", lhs.range)
                    return lhs
                }
                lhs = .identifier(base + "." + member, r)
                continue
            }
            if take(.leftParen) {
                var args: [PineArgument] = []
                if !at(.rightParen) {
                    repeat {
                        var name: String?
                        if peek(1)?.kind == .assign {
                            switch current.kind {
                            case .identifier(let n): name = n
                            case .typeKeyword(let type): name = type.rawValue
                            default: break
                            }
                            if name != nil {
                                advance()
                                advance()
                            }
                        }
                        guard let value = expression() else { break }
                        args.append(.init(name: name, value: value))
                    } while take(.comma)
                }
                expect(.rightParen, "Expected ')'.")
                guard case .identifier(let name, let r) = lhs else {
                    error("PINE2006", "Only named functions can be called.", lhs.range)
                    return lhs
                }
                callSite += 1
                lhs = .call(name, args, callSite, r)
                continue
            }
            if take(.leftBracket) {
                guard let offset = expression() else { return lhs }
                expect(.rightBracket, "Expected ']'.")
                lhs = .history(lhs, offset, lhs.range)
                continue
            }
            // Ternary has lower precedence than every binary operator. In particular,
            // `a and b ? x : y` must parse as `(a and b) ? x : y`, not
            // `a and (b ? x : y)`.
            if minBP <= 5, take(.question) {
                guard let yes = expression(), take(.colon), let no = expression() else {
                    error("PINE2007", "Malformed ternary expression.", current.range)
                    return lhs
                }
                lhs = .ternary(lhs, yes, no, lhs.range)
                continue
            }
            guard let (lbp, rbp) = binding(current.kind), lbp >= minBP else { break }
            let op = current.kind
            advance()
            guard let rhs = expression(rbp) else { return lhs }
            lhs = .binary(lhs, op, rhs, lhs.range)
        }
        return lhs
    }
    private func binding(_ kind: PineTokenKind) -> (Int, Int)? {
        switch kind {
        case .or: return (10, 11)
        case .and: return (20, 21)
        case .equal, .notEqual: return (30, 31)
        case .less, .lessEqual, .greater, .greaterEqual: return (40, 41)
        case .plus, .minus: return (50, 51)
        case .star, .slash, .percent: return (60, 61)
        case .power: return (70, 70)
        default: return nil
        }
    }
    var current: PineToken { tokens[min(index, tokens.count - 1)] }
    var previous: PineToken { tokens[max(0, index - 1)] }
    func peek(_ n: Int) -> PineToken? { index + n < tokens.count ? tokens[index + n] : nil }
    mutating func advance() { index = min(index + 1, tokens.count) }
    func at(_ kind: PineTokenKind) -> Bool { current.kind == kind }
    @discardableResult mutating func take(_ kind: PineTokenKind) -> Bool {
        if at(kind) {
            advance()
            return true
        }
        return false
    }
    mutating func expect(_ kind: PineTokenKind, _ message: String) {
        if !take(kind) { error("PINE2008", message, current.range) }
    }
    mutating func error(_ code: String, _ message: String, _ range: PineSourceRange) {
        diagnostics.append(diag(code, .syntax, message, range))
    }
}

enum PineRefKind: String, Equatable, Sendable { case array, line, label, box, table, plot }

indirect enum PineRuntimeValue: Equatable, Sendable {
    case int(Int)
    case float(Double)
    case bool(Bool)
    case string(String)
    case color(UInt32)
    case tuple([PineRuntimeValue])
    /// Handle to a runtime-owned object (array, drawing, plot).
    case ref(PineRefKind, Int)
    case na, void
    var number: Double? {
        switch self {
        case .int(let v): return Double(v)
        case .float(let v): return v
        default: return nil
        }
    }
    var bool: Bool? {
        if case .bool(let v) = self { return v }
        return nil
    }
}

struct PineCompiledProgram: Sendable {
    let source: String
    let statements: [PineStatement]
    let declaration: PineDeclarationMetadata
    let inputSchema: PineInputSchema
    let diagnostics: [PineDiagnostic]
    var isValid: Bool { !diagnostics.contains { $0.severity == .error } }
}

enum PineCompiler {
    static func compile(source: String, limits: PineLimits = .default) -> PineCompiledProgram {
        let normalizedSource = normalizeLineEndings(in: source)
        var diagnostics: [PineDiagnostic] = []
        let versionMatches = normalizedSource.split(separator: "\n").compactMap { line -> String? in
            let s = line.trimmingCharacters(in: .whitespacesAndNewlines)
            return s.hasPrefix("//@version=") ? String(s.dropFirst(11)) : nil
        }
        if versionMatches.isEmpty {
            diagnostics.append(diag("PINE0001", .semantic, "Missing //@version=6 annotation.", .zero))
        } else if versionMatches.first != "6" {
            diagnostics.append(
                diag(
                    "PINE0002", .semantic,
                    "Pine Script version \(versionMatches.first!) is not supported; this runtime targets v6.",
                    .zero))
        }
        let lexed = PineLexer(source: normalizedSource, limits: limits).lex()
        diagnostics += lexed.diagnostics
        var parser = PineParser(
            tokens: lexed.tokens, index: 0, diagnostics: [], callSite: 0, limits: limits)
        let (statements, parseDiagnostics) = parser.parse()
        diagnostics += parseDiagnostics
        var declarations: [(ScriptType, PineExpression, PineSourceRange)] = []
        for statement in statements {
            if case .expression(let e) = statement, case .call(let name, _, _, let r) = e,
                name == "indicator"
            {
                declarations.append((.indicator, e, r))
            } else if case .expression(let e) = statement,
                case .call(let name, _, _, let r) = e,
                let type = ScriptType(rawValue: name)
            {
                declarations.append((type, e, r))
            }
        }
        if declarations.count != 1 {
            diagnostics.append(
                diag(
                    "PINE3001", .semantic,
                    "A script must contain exactly one indicator(), strategy(), or library() declaration.",
                    declarations.first?.2 ?? .zero))
        }
        let environment = constantEnvironment(statements)
        var metadata = PineDeclarationMetadata(
            type: declarations.first?.0 ?? .indicator,
            pineVersion: versionMatches.first.flatMap(Int.init),
            title: "Untitled", shortTitle: nil, overlay: false, format: nil, precision: nil,
            maxBarsBack: nil)
        if metadata.type == .strategy { metadata.strategy = PineStrategySettings() }
        if let expression = declarations.first?.1, case .call(let declarationName, let args, _, let range) = expression
        {
            if let first = args.first, first.name == nil || first.name == "title",
                case .string(let title)? = constantValue(first.value, environment)
            {
                metadata.title = title
            } else {
                diagnostics.append(
                    diag("PINE3002", .semantic, "\(declarationName)() title must be a constant string.", range))
            }
            var supported: Set<String> = [
                "title", "shorttitle", "overlay", "format", "precision", "max_bars_back",
                "max_lines_count", "max_labels_count", "max_boxes_count",
            ]
            if metadata.type == .strategy {
                supported.formUnion([
                    "initial_capital", "default_qty_type", "default_qty_value", "commission_type",
                    "commission_value", "slippage", "pyramiding", "currency", "process_orders_on_close",
                    "calc_on_order_fills", "calc_on_every_tick", "close_entries_rule",
                ])
            }
            for arg in args where arg.name != nil {
                let name = arg.name!
                if !supported.contains(name) {
                    diagnostics.append(
                        diag(
                            "PINE9001", .unsupported, "Unsupported \(declarationName)() argument '\(name)'.",
                            arg.value.range))
                    continue
                }
                guard let value = constantValue(arg.value, environment) else { continue }
                switch (name, value) {
                case ("shorttitle", .string(let v)): metadata.shortTitle = v
                case ("overlay", .bool(let v)): metadata.overlay = v
                case ("format", .string(let v)): metadata.format = v
                case ("precision", .int(let v)): metadata.precision = v
                case ("max_bars_back", .int(let v)): metadata.maxBarsBack = v
                case ("max_lines_count", .int(let v)): metadata.maxLinesCount = v
                case ("max_labels_count", .int(let v)): metadata.maxLabelsCount = v
                case ("max_boxes_count", .int(let v)): metadata.maxBoxesCount = v
                default: applyStrategySetting(name, value, &metadata)
                }
            }
        }
        var schema = PineInputSchema()
        collectInputs(statements, environment, &schema, &diagnostics)
        validate(statements, diagnostics: &diagnostics)
        // Type errors on top of a broken parse would only be noise from a half-built tree.
        if !diagnostics.contains(where: { $0.category == .lexical || $0.category == .syntax }) {
            diagnostics += PineTypeChecker.check(statements)
        }
        return .init(
            source: normalizedSource, statements: statements, declaration: metadata, inputSchema: schema,
            diagnostics: diagnostics)
    }

    /// Text copied from browsers and editors can contain CR-only or Unicode line separators.
    /// Normalize them before both annotation discovery and lexing so a leading `//` comment
    /// cannot accidentally consume the entire script.
    private static func normalizeLineEndings(in source: String) -> String {
        source
            .replacingOccurrences(of: "\r\n", with: "\n")
            .replacingOccurrences(of: "\r", with: "\n")
            .replacingOccurrences(of: "\u{2028}", with: "\n")
            .replacingOccurrences(of: "\u{2029}", with: "\n")
    }

    /// Values of top-level declarations that fold to a constant and are never reassigned, so
    /// `group = g_sr` and `default_qty_value = riskPercent` resolve at compile time.
    private static func constantEnvironment(_ statements: [PineStatement]) -> [String: PineRuntimeValue] {
        var reassigned = Set<String>()
        func collect(_ statements: [PineStatement]) {
            for statement in statements {
                switch statement {
                case .assignment(let name, _, _, _): reassigned.insert(name)
                case .conditional(_, let a, let b, _):
                    collect(a)
                    collect(b)
                case .forRange(_, _, _, _, let body, _), .forIn(_, _, _, let body, _),
                    .whileLoop(_, let body, _), .function(_, _, let body, _):
                    collect(body)
                case .switchStatement(_, let arms, _): arms.forEach { collect($0.body) }
                default: break
                }
            }
        }
        collect(statements)
        var environment: [String: PineRuntimeValue] = [:]
        for statement in statements {
            guard case .declaration(let name, _, .ordinary, let expression, _) = statement,
                !reassigned.contains(name), let value = constantValue(expression, environment)
            else { continue }
            environment[name] = value
        }
        return environment
    }

    /// Folds compile-time expressions: literals, builtin constants, earlier constants,
    /// arithmetic on numbers, string concatenation, colors, and `timestamp()`.
    static func constantValue(_ e: PineExpression, _ environment: [String: PineRuntimeValue])
        -> PineRuntimeValue?
    {
        switch e {
        case .literal(let value, _): return value == .na ? nil : value
        case .identifier(let name, _):
            if let value = environment[name] { return value }
            if let value = PineBuiltins.constants[name] { return value }
            return PineBuiltins.colors[name].map(PineRuntimeValue.color)
        case .unary(.minus, let inner, _):
            switch constantValue(inner, environment) {
            case .int(let x)?: return .int(-x)
            case .float(let x)?: return .float(-x)
            default: return nil
            }
        case .binary(let l, let op, let r, _):
            guard let a = constantValue(l, environment), let b = constantValue(r, environment) else {
                return nil
            }
            if op == .plus, case .string(let x) = a, case .string(let y) = b { return .string(x + y) }
            if case .int(let x) = a, case .int(let y) = b {
                switch op {
                case .plus: return .int(x + y)
                case .minus: return .int(x - y)
                case .star: return .int(x * y)
                default: break
                }
            }
            guard let x = a.number, let y = b.number else { return nil }
            switch op {
            case .plus: return .float(x + y)
            case .minus: return .float(x - y)
            case .star: return .float(x * y)
            case .slash: return y == 0 ? nil : .float(x / y)
            default: return nil
            }
        case .call("timestamp", let arguments, _, _):
            var positional: [PineRuntimeValue] = []
            var named: [String: PineRuntimeValue] = [:]
            for argument in arguments {
                guard let value = constantValue(argument.value, environment) else { return nil }
                if let name = argument.name { named[name] = value } else { positional.append(value) }
            }
            return PineTimestamp.evaluate(positional: positional, named: named).map(PineRuntimeValue.int)
        case .call("color.new", _, _, _), .call("color.rgb", _, _, _):
            return constantColor(e).map(PineRuntimeValue.color)
        default: return nil
        }
    }

    private static func applyStrategySetting(
        _ name: String, _ value: PineRuntimeValue, _ metadata: inout PineDeclarationMetadata
    ) {
        guard var settings = metadata.strategy else { return }
        switch (name, value) {
        case ("initial_capital", _):
            if let n = value.number, n > 0 { settings.initialCapital = n }
        case ("default_qty_type", .string(let v)):
            switch v {
            case "strategy.cash": settings.quantityType = .cash
            case "strategy.percent_of_equity": settings.quantityType = .percentOfEquity
            default: settings.quantityType = .fixed
            }
        case ("default_qty_value", _):
            if let n = value.number, n >= 0 { settings.quantityValue = n }
        case ("commission_type", .string(let v)):
            switch v {
            case "strategy.commission.cash_per_order": settings.commissionType = .cashPerOrder
            case "strategy.commission.cash_per_contract": settings.commissionType = .cashPerContract
            default: settings.commissionType = .percent
            }
        case ("commission_value", _):
            if let n = value.number, n >= 0 { settings.commissionValue = n }
        case ("slippage", _):
            if let n = value.number, n >= 0, n < 1e6 { settings.slippage = Int(n) }
        case ("pyramiding", _):
            if let n = value.number, n >= 0, n < 1e6 { settings.pyramiding = Int(n) }
        case ("currency", .string(let v)): settings.currency = v
        case ("process_orders_on_close", .bool(let v)): settings.processOrdersOnClose = v
        default: break
        }
        metadata.strategy = settings
    }

    private static func collectInputs(
        _ statements: [PineStatement], _ environment: [String: PineRuntimeValue],
        _ schema: inout PineInputSchema, _ diagnostics: inout [PineDiagnostic]
    ) {
        for statement in statements {
            if case .declaration(let variable, _, _, let expr, _) = statement,
                case .call(let name, let args, _, let range) = expr, name.hasPrefix("input.")
            {
                let type: PineValueType
                switch name {
                case "input.int": type = .int
                case "input.float": type = .float
                case "input.bool": type = .bool
                case "input.color": type = .color
                case "input.time": type = .time
                default: type = .string
                }
                guard let first = args.first,
                    let defaultValue = inputValue(first.value, function: name, environment)
                else {
                    diagnostics.append(
                        diag("PINE3010", .semantic, "\(name) requires a constant default value.", range))
                    continue
                }
                func string(_ key: String) -> String? {
                    args.first { $0.name == key }.flatMap {
                        if case .string(let v)? = constantValue($0.value, environment) { return v }
                        return nil
                    }
                }
                func number(_ key: String) -> Double? {
                    args.first { $0.name == key }.flatMap { constantValue($0.value, environment)?.number }
                }
                func boolean(_ key: String) -> Bool {
                    args.first { $0.name == key }.flatMap {
                        if case .literal(.bool(let v), _) = $0.value { return v }
                        return nil
                    } ?? false
                }
                let title =
                    string("title")
                    ?? (args.count > 1 && args[1].name == nil
                        ? {
                            if case .string(let v)? = constantValue(args[1].value, environment) { return v }
                            return nil
                        }() : nil)
                schema.inputs.append(
                    .init(
                        id: variable, type: type, defaultValue: defaultValue, title: title,
                        tooltip: string("tooltip"), group: string("group"), inline: string("inline"),
                        confirm: boolean("confirm"), minValue: number("minval"), maxValue: number("maxval"),
                        step: number("step"), options: options(args, function: name, environment)))
            }
            if case .conditional(_, let a, let b, _) = statement {
                collectInputs(a, environment, &schema, &diagnostics)
                collectInputs(b, environment, &schema, &diagnostics)
            }
        }
    }
    private static func options(
        _ args: [PineArgument], function: String, _ environment: [String: PineRuntimeValue]
    ) -> [PineInputValue]? {
        guard let argument = args.first(where: { $0.name == "options" }),
            case .tuple(let values, _) = argument.value
        else { return nil }
        let options = values.compactMap { inputValue($0, function: function, environment) }
        return options.count == values.count && !options.isEmpty ? options : nil
    }

    private static func inputValue(
        _ e: PineExpression, function: String, _ environment: [String: PineRuntimeValue]
    ) -> PineInputValue? {
        if function == "input.source", case .identifier(let name, _) = e {
            return .source(name)
        }
        switch constantValue(e, environment) {
        case .int(let x)?: return .int(x)
        case .float(let x)?: return .float(x)
        case .bool(let x)?: return .bool(x)
        case .string(let x)?: return .string(x)
        case .color(let x)?: return .color(x)
        default: return nil
        }
    }

    /// Folds compile-time color expressions: literals, `color.*` constants, and
    /// `color.new`/`color.rgb` with constant arguments.
    static func constantColor(_ e: PineExpression) -> UInt32? {
        switch e {
        case .literal(.color(let rgba), _): return rgba
        case .identifier(let name, _): return PineBuiltins.colors[name]
        case .call("color.new", let arguments, _, _):
            guard arguments.count >= 2, let base = constantColor(arguments[0].value),
                let transparency = constantNumber(arguments[1].value)
            else { return nil }
            return PineBuiltins.withTransparency(base, transparency)
        case .call("color.rgb", let arguments, _, _):
            let numbers = arguments.map { constantNumber($0.value) }
            guard numbers.count >= 3, !numbers.contains(where: { $0 == nil }) else { return nil }
            return PineBuiltins.rgb(numbers[0]!, numbers[1]!, numbers[2]!, numbers.count > 3 ? numbers[3]! : 0)
        default: return nil
        }
    }

    private static func constantNumber(_ e: PineExpression) -> Double? {
        switch e {
        case .literal(let value, _): return value.number
        case .unary(.minus, let inner, _): return constantNumber(inner).map { -$0 }
        default: return nil
        }
    }
    private static func validate(_ statements: [PineStatement], diagnostics: inout [PineDiagnostic]) {
        validate(statements, inheritedDeclarations: [], inLoop: false, diagnostics: &diagnostics)
    }

    private static func validate(
        _ statements: [PineStatement], inheritedDeclarations: Set<String>, inLoop: Bool,
        diagnostics: inout [PineDiagnostic]
    ) {
        var declared = inheritedDeclarations
        for statement in statements {
            switch statement {
            case .declaration(let n, let annotation, _, let e, let r):
                if declared.contains(n) {
                    diagnostics.append(
                        diag("PINE3020", .semantic, "Variable '\(n)' is already declared in this scope.", r))
                }
                declared.insert(n)
                if annotation.type == .bool, case .literal(.na, _) = e {
                    diagnostics.append(
                        diag("PINE3021", .semantic, "Boolean values cannot be na in Pine v6.", r))
                }
            case .assignment(let n, _, _, let r):
                if !declared.contains(n) {
                    diagnostics.append(
                        diag("PINE3022", .semantic, "Cannot reassign undeclared variable '\(n)'.", r))
                }
            case .conditional(_, let a, let b, _):
                validate(a, inheritedDeclarations: declared, inLoop: inLoop, diagnostics: &diagnostics)
                validate(b, inheritedDeclarations: declared, inLoop: inLoop, diagnostics: &diagnostics)
            case .forRange(let variable, _, _, _, let body, _):
                validate(
                    body, inheritedDeclarations: declared.union([variable]), inLoop: true,
                    diagnostics: &diagnostics)
            case .forIn(let index, let value, _, let body, _):
                validate(
                    body, inheritedDeclarations: declared.union([index, value].compactMap { $0 }),
                    inLoop: true, diagnostics: &diagnostics)
            case .whileLoop(_, let body, _):
                validate(body, inheritedDeclarations: declared, inLoop: true, diagnostics: &diagnostics)
            case .switchStatement(_, let arms, _):
                for arm in arms {
                    validate(
                        arm.body, inheritedDeclarations: declared, inLoop: inLoop, diagnostics: &diagnostics)
                }
            case .tupleDeclaration(let names, _, let r):
                for name in names where name != "_" {
                    if declared.contains(name) {
                        diagnostics.append(
                            diag("PINE3020", .semantic, "Variable '\(name)' is already declared in this scope.", r))
                    }
                    declared.insert(name)
                }
            case .loopControl(_, let r):
                if !inLoop {
                    diagnostics.append(
                        diag("PINE3023", .semantic, "break and continue are only allowed inside a loop.", r))
                }
            case .function(let n, let parameters, let body, let r):
                if declared.contains(n) {
                    diagnostics.append(
                        diag("PINE3024", .semantic, "Function '\(n)' is already declared.", r))
                }
                declared.insert(n)
                // Function bodies are their own scope: locals may reuse global names, and
                // globals cannot be reassigned from inside a function.
                validate(
                    body, inheritedDeclarations: Set(parameters.map(\.name)), inLoop: false,
                    diagnostics: &diagnostics)
            case .expression(let e):
                if case .call(let n, _, _, let r) = e,
                    n.hasPrefix("request.")
                {
                    diagnostics.append(
                        diag("PINE9003", .unsupported, "Feature '\(n)' is not supported in this release.", r))
                }
            }
        }
    }
}

/// Builtin constants shared by compile-time input folding and the runtime.
enum PineBuiltins {
    static let colors: [String: UInt32] = [
        "color.aqua": 0x00bc_d4ff, "color.black": 0x0000_00ff, "color.blue": 0x2196_f3ff,
        "color.fuchsia": 0xe040_fbff, "color.gray": 0x787b_86ff, "color.green": 0x4caf_50ff,
        "color.lime": 0x00e6_76ff, "color.maroon": 0x880e_4fff, "color.navy": 0x0d47_a1ff,
        "color.olive": 0x8277_17ff, "color.orange": 0xff98_00ff, "color.purple": 0x9c27_b0ff,
        "color.red": 0xf236_45ff, "color.silver": 0xb2b5_beff, "color.teal": 0x0089_7bff,
        "color.white": 0xffff_ffff, "color.yellow": 0xffeb_3bff,
    ]

    /// Named constants that are not colors. Enumeration-like values are their own name, so a
    /// script compares and passes them exactly as it would in Pine; `display.*` and
    /// `dayofweek.*` are integers because scripts do arithmetic on them.
    static let constants: [String: PineRuntimeValue] = {
        var table: [String: PineRuntimeValue] = [
            "display.none": .int(PineDisplay.none), "display.pane": .int(PineDisplay.pane),
            "display.data_window": .int(PineDisplay.dataWindow),
            "display.price_scale": .int(PineDisplay.priceScale),
            "display.status_line": .int(PineDisplay.statusLine), "display.all": .int(PineDisplay.all),
            "dayofweek.sunday": .int(1), "dayofweek.monday": .int(2), "dayofweek.tuesday": .int(3),
            "dayofweek.wednesday": .int(4), "dayofweek.thursday": .int(5), "dayofweek.friday": .int(6),
            "dayofweek.saturday": .int(7), "math.pi": .float(Double.pi), "math.e": .float(M_E),
            "math.phi": .float((1 + 5.0.squareRoot()) / 2),
        ]
        let names = [
            "strategy.long", "strategy.short", "strategy.cash", "strategy.fixed",
            "strategy.percent_of_equity", "strategy.commission.percent",
            "strategy.commission.cash_per_order", "strategy.commission.cash_per_contract",
            "strategy.oca.none", "strategy.oca.cancel", "strategy.oca.reduce",
            "alert.freq_all", "alert.freq_once_per_bar", "alert.freq_once_per_bar_close",
            "format.inherit", "format.price", "format.volume", "format.percent", "format.mintick",
            "order.ascending", "order.descending",
        ]
        for name in names { table[name] = .string(name) }
        return table
    }()

    /// Pine transparency is 0 (opaque) … 100 (invisible).
    static func withTransparency(_ rgba: UInt32, _ transparency: Double) -> UInt32 {
        (rgba & 0xFFFF_FF00) | UInt32(((100 - min(100, max(0, transparency))) * 2.55).rounded())
    }

    static func rgb(_ r: Double, _ g: Double, _ b: Double, _ transparency: Double) -> UInt32 {
        func channel(_ v: Double) -> UInt32 { UInt32(min(255, max(0, v.isFinite ? v : 0))) }
        return withTransparency(
            (channel(r) << 24) | (channel(g) << 16) | (channel(b) << 8) | 0xFF, transparency)
    }
}

func diag(
    _ code: String, _ category: PineDiagnosticCategory, _ message: String, _ range: PineSourceRange
) -> PineDiagnostic {
    .init(code: code, severity: .error, category: category, message: message, range: range)
}
