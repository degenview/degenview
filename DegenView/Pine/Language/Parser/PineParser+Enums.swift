import Foundation

extension PineParser {
    /// `enum Name` ending its line, so `enum = 1` and `enum(x)` stay ordinary expressions.
    func isEnumDeclaration() -> Bool {
        guard case .identifier("enum") = current.kind, let name = peek(1), case .identifier = name.kind,
            let end = peek(2)
        else { return false }
        return end.kind == .newline
    }

    /// `enum Name` and its indented member lines: `name [= "title"]`.
    mutating func enumDeclaration() -> PineStatement? {
        advance()
        guard case .identifier(let name) = current.kind else { return nil }
        let range = current.range
        advance()
        enumTypes.insert(name)
        guard beginIndentedBlock("enum \(name)") else { return nil }
        var members: [PineEnumMember] = []
        while !at(.eof) && !at(.dedent) {
            if take(.newline) { continue }
            guard let member = enumMember() else {
                skipToLineEnd()
                continue
            }
            members.append(member)
            if !at(.newline) && !at(.dedent) && !at(.eof) {
                error("PINE2013", "Expected end of statement; found unexpected token.", current.range)
                skipToLineEnd()
            }
        }
        _ = take(.dedent)
        return .enumDeclaration(name: name, members: members, range: range)
    }

    private mutating func enumMember() -> PineEnumMember? {
        guard case .identifier(let name) = current.kind else {
            error("PINE2015", "Expected an enum member name.", current.range)
            return nil
        }
        advance()
        guard take(.assign) else { return PineEnumMember(name: name, title: name) }
        guard case .string(let title) = current.kind else {
            error("PINE2017", "An enum title must be a string literal.", current.range)
            return nil
        }
        advance()
        return PineEnumMember(name: name, title: title)
    }
}
