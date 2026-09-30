import Foundation

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
        diagnostics.append(PineDiagnostic.error(code, .syntax, message, range))
    }
}
