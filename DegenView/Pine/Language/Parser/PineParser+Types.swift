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
}
