import Foundation

extension PineParser {
    /// `type Name` ending its line, so `type = 1` and `type(x)` stay ordinary expressions.
    func isTypeDeclaration() -> Bool {
        guard case .identifier("type") = current.kind, let name = peek(1), case .identifier = name.kind,
            let end = peek(2)
        else { return false }
        return end.kind == .newline
    }

    /// `type Name` and its indented field lines: `[type] name [= default]`. The name is known
    /// before the fields are read, so a type may refer to itself.
    mutating func typeDeclaration() -> PineStatement? {
        advance()
        guard case .identifier(let name) = current.kind else { return nil }
        let range = current.range
        advance()
        userTypes.insert(name)
        guard beginIndentedBlock("type \(name)") else { return nil }
        var fields: [PineTypeField] = []
        while !at(.eof) && !at(.dedent) {
            if take(.newline) { continue }
            guard let field = typeField() else {
                skipToLineEnd()
                continue
            }
            fields.append(field)
            if !at(.newline) && !at(.dedent) && !at(.eof) {
                error("PINE2013", "Expected end of statement; found unexpected token.", current.range)
                skipToLineEnd()
            }
        }
        _ = take(.dedent)
        return .typeDeclaration(name: name, fields: fields, range: range)
    }

    private mutating func typeField() -> PineTypeField? {
        skipQualifier()
        let type = typeAnnotation()
        guard case .identifier(let name) = current.kind else {
            error("PINE2015", "Expected a field name.", current.range)
            return nil
        }
        advance()
        var defaultValue: PineExpression?
        if take(.assign) {
            guard let value = expression() else { return nil }
            defaultValue = value
        }
        return PineTypeField(name: name, type: type, defaultValue: defaultValue)
    }

    // MARK: - Field assignment

    /// Whether the line is `path.to.field := value` (or a compound form): a dotted target, then an
    /// assignment operator outside any brackets. Looked at before parsing, so ordinary statements that
    /// start with a namespace (`ta.ema(close, 3)`) are parsed exactly once.
    func isFieldAssignment() -> Bool {
        guard case .identifier = current.kind, peek(1)?.kind == .dot else { return false }
        var depth = 0
        for token in tokens[index...] {
            switch token.kind {
            case .leftParen, .leftBracket: depth += 1
            case .rightParen, .rightBracket: depth -= 1
            case .newline, .eof, .indent, .dedent: return false
            case .comma where depth == 0: return false
            default:
                if depth == 0, PineAssignmentOperator(token: token.kind) != nil { return true }
            }
        }
        return false
    }

    mutating func fieldAssignment() -> PineStatement? {
        let range = current.range
        guard let target = expression() else { return nil }
        guard let op = PineAssignmentOperator(token: current.kind) else {
            error("PINE2013", "Expected end of statement; found unexpected token.", current.range)
            return nil
        }
        advance()
        switch target {
        case .identifier(let name, _) where name.contains("."): break
        default:
            error("PINE2016", "Only a variable or a field can be assigned to.", target.range)
            return nil
        }
        guard let value = expression() else { return nil }
        return .fieldAssignment(target: target, op: op, value: value, range: range)
    }
}
