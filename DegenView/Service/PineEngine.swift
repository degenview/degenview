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
    var range: PineSourceRange {
        switch self {
        case .literal(_, let r), .identifier(_, let r), .unary(_, _, let r), .binary(_, _, _, let r),
            .ternary(_, _, _, let r), .call(_, _, _, let r), .history(_, _, let r), .tuple(_, let r):
            return r
        }
    }
}
struct PineArgument: Sendable {
    var name: String?
    var value: PineExpression
}
enum PineDeclarationMode: Sendable { case ordinary, variable, intrabar }
indirect enum PineStatement: Sendable {
    case declaration(String, PineValueType?, PineDeclarationMode, PineExpression, PineSourceRange)
    case assignment(String, PineTokenKind, PineExpression, PineSourceRange)
    case expression(PineExpression)
    case conditional(PineExpression, [PineStatement], [PineStatement], PineSourceRange)
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
            if let statement = statement() { out.append(statement) }
            if index == before {
                error("PINE2001", "Expected a statement.", current.range)
                advance()
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
    mutating private func statement() -> PineStatement? {
        if take(.ifKeyword) {
            let start = previous.range
            guard let condition = expression() else { return nil }
            _ = take(.newline)
            guard take(.indent) else {
                error("PINE2002", "Expected an indented block after if.", current.range)
                return nil
            }
            let yes = block(untilDedent: true)
            var no: [PineStatement] = []
            if take(.elseKeyword) {
                _ = take(.newline)
                if take(.indent) { no = block(untilDedent: true) }
            }
            return .conditional(condition, yes, no, start)
        }
        var mode = PineDeclarationMode.ordinary
        var type: PineValueType?
        if take(.varKeyword) { mode = .variable } else if take(.varipKeyword) { mode = .intrabar }
        if case .typeKeyword(let t) = current.kind {
            type = t
            advance()
        }
        if case .identifier(let name) = current.kind, let next = peek(1) {
            if next.kind == .assign {
                let range = current.range
                advance()
                advance()
                guard let rhs = expression() else { return nil }
                return .declaration(name, type, mode, rhs, range)
            }
            if [.reassign, .plusAssign, .minusAssign, .starAssign, .slashAssign].contains(next.kind) {
                let range = current.range
                advance()
                let op = current.kind
                advance()
                guard let rhs = expression() else { return nil }
                return .assignment(name, op, rhs, range)
            }
        }
        return expression().map(PineStatement.expression)
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
        case .na: lhs = .literal(.na, token.range)
        case .identifier(let name): lhs = .identifier(name, token.range)
        case .typeKeyword(let type): lhs = .identifier(type.rawValue, token.range)
        case .minus, .plus, .not:
            guard let rhs = expression(80) else { return nil }
            lhs = .unary(token.kind, rhs, token.range)
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

indirect enum PineRuntimeValue: Equatable, Sendable {
    case int(Int)
    case float(Double)
    case bool(Bool)
    case string(String)
    case color(UInt32)
    case tuple([PineRuntimeValue])
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
        var metadata = PineDeclarationMetadata(
            type: declarations.first?.0 ?? .indicator,
            pineVersion: versionMatches.first.flatMap(Int.init),
            title: "Untitled", shortTitle: nil, overlay: false, format: nil, precision: nil,
            maxBarsBack: nil)
        if let expression = declarations.first?.1, case .call(let declarationName, let args, _, let range) = expression
        {
            if let first = args.first, case .literal(.string(let title), _) = first.value {
                metadata.title = title
            } else {
                diagnostics.append(
                    diag("PINE3002", .semantic, "indicator() title must be a constant string.", range))
            }
            let supported: Set<String> = [
                "title", "shorttitle", "overlay", "format", "precision", "max_bars_back",
            ]
            for arg in args where arg.name != nil {
                let name = arg.name!
                if !supported.contains(name) {
                    diagnostics.append(
                        diag(
                            "PINE9001", .unsupported, "Unsupported \(declarationName)() argument '\(name)'.",
                            arg.value.range))
                    continue
                }
                switch (name, arg.value) {
                case ("shorttitle", .literal(.string(let v), _)): metadata.shortTitle = v
                case ("overlay", .literal(.bool(let v), _)): metadata.overlay = v
                case ("format", .literal(.string(let v), _)): metadata.format = v
                case ("precision", .literal(.int(let v), _)): metadata.precision = v
                case ("max_bars_back", .literal(.int(let v), _)): metadata.maxBarsBack = v
                default: break
                }
            }
        }
        var schema = PineInputSchema()
        collectInputs(statements, &schema, &diagnostics)
        validate(statements, diagnostics: &diagnostics)
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

    private static func collectInputs(
        _ statements: [PineStatement], _ schema: inout PineInputSchema,
        _ diagnostics: inout [PineDiagnostic]
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
                default: type = .string
                }
                guard let first = args.first, let defaultValue = inputValue(first.value, function: name) else {
                    diagnostics.append(
                        diag("PINE3010", .semantic, "\(name) requires a constant default value.", range))
                    continue
                }
                func string(_ key: String) -> String? {
                    args.first { $0.name == key }.flatMap {
                        if case .literal(.string(let v), _) = $0.value { return v }
                        return nil
                    }
                }
                func number(_ key: String) -> Double? {
                    args.first { $0.name == key }.flatMap {
                        if case .literal(let v, _) = $0.value { return v.number }
                        return nil
                    }
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
                            if case .literal(.string(let v), _) = args[1].value { return v }
                            return nil
                        }() : nil)
                schema.inputs.append(
                    .init(
                        id: variable, type: type, defaultValue: defaultValue, title: title,
                        tooltip: string("tooltip"), group: string("group"), inline: string("inline"),
                        confirm: boolean("confirm"), minValue: number("minval"), maxValue: number("maxval"),
                        step: number("step"), options: nil))
            }
            if case .conditional(_, let a, let b, _) = statement {
                collectInputs(a, &schema, &diagnostics)
                collectInputs(b, &schema, &diagnostics)
            }
        }
    }
    private static func inputValue(_ e: PineExpression, function: String) -> PineInputValue? {
        if function == "input.source", case .identifier(let name, _) = e {
            return .source(name)
        }
        if case .literal(let v, _) = e {
            switch v {
            case .int(let x): return .int(x)
            case .float(let x): return .float(x)
            case .bool(let x): return .bool(x)
            case .string(let x): return .string(x)
            case .color(let x): return .color(x)
            default: return nil
            }
        }
        if function == "input.color", case .call("color.new", let arguments, _, _) = e,
            arguments.count >= 2,
            case .literal(.color(let rgba), _) = arguments[0].value,
            case .literal(let transparency, _) = arguments[1].value,
            let percent = transparency.number
        {
            let alpha = UInt32((100 - min(100, max(0, percent))) * 2.55)
            return .color((rgba & 0xFFFF_FF00) | alpha)
        }
        return nil
    }
    private static func validate(_ statements: [PineStatement], diagnostics: inout [PineDiagnostic]) {
        validate(statements, inheritedDeclarations: [], diagnostics: &diagnostics)
    }

    private static func validate(
        _ statements: [PineStatement], inheritedDeclarations: Set<String>,
        diagnostics: inout [PineDiagnostic]
    ) {
        var declared = inheritedDeclarations
        for statement in statements {
            switch statement {
            case .declaration(let n, let type, _, let e, let r):
                if declared.contains(n) {
                    diagnostics.append(
                        diag("PINE3020", .semantic, "Variable '\(n)' is already declared in this scope.", r))
                }
                declared.insert(n)
                if type == .bool, case .literal(.na, _) = e {
                    diagnostics.append(
                        diag("PINE3021", .semantic, "Boolean values cannot be na in Pine v6.", r))
                }
            case .assignment(let n, _, _, let r):
                if !declared.contains(n) {
                    diagnostics.append(
                        diag("PINE3022", .semantic, "Cannot reassign undeclared variable '\(n)'.", r))
                }
            case .conditional(_, let a, let b, _):
                validate(a, inheritedDeclarations: declared, diagnostics: &diagnostics)
                validate(b, inheritedDeclarations: declared, diagnostics: &diagnostics)
            case .expression(let e):
                if case .call(let n, _, _, let r) = e,
                    n.hasPrefix("request.") || ["alert", "alertcondition"].contains(n)
                {
                    diagnostics.append(
                        diag("PINE9003", .unsupported, "Feature '\(n)' is not supported in this release.", r))
                }
            }
        }
    }
}

func diag(
    _ code: String, _ category: PineDiagnosticCategory, _ message: String, _ range: PineSourceRange
) -> PineDiagnostic {
    .init(code: code, severity: .error, category: category, message: message, range: range)
}
