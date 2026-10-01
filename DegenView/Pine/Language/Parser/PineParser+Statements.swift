import Foundation

extension PineParser {
    mutating func statement() -> PineStatement? {
        if takeWord("export") { return exportedStatement() }
        if isTypeDeclaration() { return typeDeclaration() }
        if isEnumDeclaration() { return enumDeclaration() }
        if isMethodDefinition() { return methodDefinition() }
        if skipUnsupportedDeclaration() { return nil }
        if take(.ifKeyword) { return ifStatement() }
        if take(.forKeyword) { return forStatement() }
        if take(.whileKeyword) { return whileStatement() }
        if take(.switchKeyword) { return switchStatement() }
        if isTupleDeclaration() { return tupleDeclaration() }
        if take(.breakKeyword) { return .loopControl(.breakLoop, previous.range) }
        if take(.continueKeyword) { return .loopControl(.continueLoop, previous.range) }
        if isFunctionDefinition() { return functionDefinition() }
        if let declaration = declaration() { return declaration }
        if isFieldAssignment() { return fieldAssignment() }
        if let assignment = assignment() { return assignment }
        return expression().map(PineStatement.expression)
    }

    /// `name := value` and the compound forms. Returns nil without consuming anything when the
    /// tokens are not an assignment; a malformed right-hand side also yields nil.
    private mutating func assignment() -> PineStatement? {
        guard let name = variableName(current.kind), let next = peek(1),
            let op = PineAssignmentOperator(token: next.kind)
        else { return nil }
        let range = current.range
        advance()
        advance()
        guard let value = expression() else { return nil }
        return .assignment(name: name, op: op, value: value, range: range)
    }

    // MARK: - Blocks

    /// Consumes the newline and indent that open a block, reporting PINE2002 when absent.
    mutating func beginIndentedBlock(_ owner: String) -> Bool {
        _ = take(.newline)
        guard take(.indent) else {
            error("PINE2002", "Expected an indented block after \(owner).", current.range)
            return false
        }
        return true
    }

    mutating func indentedBlock(_ owner: String) -> [PineStatement]? {
        guard beginIndentedBlock(owner) else { return nil }
        return block(untilDedent: true)
    }

    /// The body after `=>`: an indented block, or one statement on the same line.
    private mutating func blockOrSingleStatement(_ owner: String) -> [PineStatement]? {
        if at(.newline) { return indentedBlock(owner) }
        let statements = statementsOnLine()
        return statements.isEmpty ? nil : statements
    }

    // MARK: - Control flow

    /// Called after `if` has been consumed. `else if` nests the chained `if` as the
    /// sole statement of the else branch.
    mutating func ifStatement() -> PineStatement? {
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
        return .conditional(condition: condition, whenTrue: yes, whenFalse: no, range: start)
    }

    /// Called after `while` has been consumed.
    private mutating func whileStatement() -> PineStatement? {
        let start = previous.range
        guard let condition = expression() else { return nil }
        guard let body = indentedBlock("while") else { return nil }
        return .whileLoop(condition: condition, body: body, range: start)
    }

    /// Called after `switch` has been consumed.
    mutating func switchStatement() -> PineStatement? {
        let start = previous.range
        var subject: PineExpression?
        if !at(.newline) {
            guard let value = expression() else { return nil }
            subject = value
        }
        guard beginIndentedBlock("switch") else { return nil }
        var arms: [PineSwitchArm] = []
        while !at(.eof) && !at(.dedent) {
            if take(.newline) { continue }
            var condition: PineExpression?
            if !take(.arrow) {
                guard let value = expression() else {
                    skipToLineEnd()
                    continue
                }
                condition = value
                expect(.arrow, "Expected '=>' in switch arm.")
            }
            guard let body = blockOrSingleStatement("switch arm") else { return nil }
            arms.append(.init(condition: condition, body: body))
            while take(.newline) {}
        }
        _ = take(.dedent)
        return .switchStatement(subject: subject, arms: arms, range: start)
    }

    /// Called after `for` has been consumed.
    private mutating func forStatement() -> PineStatement? {
        let start = previous.range
        var indexName: String?
        let valueName: String
        if take(.leftBracket) {
            guard let first = loopVariable() else { return nil }
            expect(.comma, "Expected ',' in for...in destructuring.")
            guard let second = loopVariable() else { return nil }
            expect(.rightBracket, "Expected ']'.")
            indexName = first
            valueName = second
        } else {
            guard let name = loopVariable() else { return nil }
            valueName = name
        }
        if indexName == nil, take(.assign) {
            return forRangeTail(variable: valueName, start: start)
        }
        guard takeWord("in") else {
            error("PINE2010", "Expected '=' or 'in' in for loop.", current.range)
            return nil
        }
        guard let collection = expression() else { return nil }
        guard let body = indentedBlock("for") else { return nil }
        return .forIn(index: indexName, value: valueName, collection: collection, body: body, range: start)
    }

    private mutating func loopVariable() -> String? {
        guard case .identifier(let name) = current.kind else {
            error("PINE2009", "Expected a loop variable.", current.range)
            return nil
        }
        advance()
        return name
    }

    /// `from to end [by step]` and the body, after `for name =`.
    private mutating func forRangeTail(variable: String, start: PineSourceRange) -> PineStatement? {
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
        return .forRange(variable: variable, from: from, to: end, step: step, body: body, range: start)
    }

    // MARK: - Tuples and functions

    /// `[a, b] = …` — a bracketed name list directly followed by `=`.
    func isTupleDeclaration() -> Bool {
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

    mutating func tupleDeclaration() -> PineStatement? {
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
        return .tupleDeclaration(names: names, value: rhs, range: range)
    }

    /// `name(` … matching `)` followed by `=>`.
    func isFunctionDefinition() -> Bool {
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

    mutating func functionDefinition() -> PineStatement? {
        guard case .identifier(let name) = current.kind else { return nil }
        let range = current.range
        advance()
        expect(.leftParen, "Expected '('.")
        var parameters: [PineParameter] = []
        if !at(.rightParen) {
            repeat {
                guard let parameter = functionParameter() else { return nil }
                parameters.append(parameter)
            } while take(.comma)
        }
        expect(.rightParen, "Expected ')'.")
        expect(.arrow, "Expected '=>'.")
        guard let body = blockOrSingleStatement("function declaration") else { return nil }
        return .function(name: name, parameters: parameters, body: body, range: range)
    }

    private mutating func functionParameter() -> PineParameter? {
        skipQualifier()
        let type = typeAnnotation()
        let typeName = type == .object ? annotatedTypeName : nil
        guard case .identifier(let name) = current.kind else {
            error("PINE2011", "Expected a parameter name.", current.range)
            return nil
        }
        advance()
        var defaultValue: PineExpression?
        if take(.assign) { defaultValue = expression() }
        return .init(name: name, type: type, typeName: typeName, defaultValue: defaultValue)
    }
}
