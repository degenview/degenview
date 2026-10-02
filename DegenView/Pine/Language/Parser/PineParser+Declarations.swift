import Foundation

extension PineParser {
    static let objectTypes: [String: PineValueType] = [
        "line": .line, "label": .label, "box": .box, "table": .table, "array": .array,
        // Typed `.object` rather than a kind of its own: the checker does not track handles of this kind.
        "linefill": .object, "map": .map, "polyline": .object, "matrix": .matrix,
    ]

    static let qualifiers: [String: PineQualifier] = [
        "const": .constant, "input": .input, "simple": .simple, "series": .series,
    ]

    /// The variable a token names. A type keyword is allowed when nothing else claimed it as an
    /// annotation (`color = …`): scripts do call their variables `color`.
    func variableName(_ kind: PineTokenKind) -> String? {
        switch kind {
        case .identifier(let name): name
        case .typeKeyword(let type): type.rawValue
        default: nil
        }
    }

    /// `[var|varip] [type] name = expression`. Parsed speculatively: when the tokens do
    /// not form a declaration header the cursor is restored so they parse as an expression.
    mutating func declaration() -> PineStatement? {
        let start = index
        var mode = PineDeclarationMode.ordinary
        if take(.varKeyword) { mode = .variable } else if take(.varipKeyword) { mode = .intrabar }
        let qualifier = skipQualifier()
        let type = typeAnnotation()
        guard let name = variableName(current.kind), peek(1)?.kind == .assign else {
            index = start
            return nil
        }
        let range = current.range
        advance()
        advance()
        guard let value = expression() else { return nil }
        return .declaration(
            name: name, annotation: .init(type: type, qualifier: qualifier), mode: mode, value: value,
            range: range)
    }

    /// Parses `int`, `float[]`, `line`, `box[]`, `array<int>`. Only consumes tokens when
    /// they are followed by a name (or `[]`/`<`), so `line.new(...)` and `int(x)` still
    /// parse as expressions.
    mutating func typeAnnotation() -> PineValueType? {
        annotatedTypeName = nil
        var type: PineValueType
        switch current.kind {
        case .typeKeyword(let t): type = t
        case .identifier(let alias) where importAliases.contains(alias) && isLibraryTypeAnnotation():
            // `alias.Type name`
            advance()
            advance()
            if case .identifier(let name) = current.kind { annotatedTypeName = "\(alias).\(name)" }
            advance()
            return .object
        case .identifier("chart") where isChartPointAnnotation():
            // `chart.point p = …`
            advance()
            advance()
            advance()
            annotatedTypeName = "chart.point"
            return .object
        case .identifier(let word):
            if let t = Self.objectTypes[word] {
                type = t
            } else if userTypes.contains(word) || enumTypes.contains(word) {
                type = .object
                annotatedTypeName = word
            } else {
                return nil
            }
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
            _ = typeArgument()
            expect(.greater, "Expected '>'.")
            return .array
        }
        if type == .matrix, next.kind == .less {
            advance()
            advance()
            _ = typeArgument()
            expect(.greater, "Expected '>'.")
            return .matrix
        }
        if type == .map, next.kind == .less {
            advance()
            advance()
            _ = typeArgument()
            expect(.comma, "Expected ',' between the key and value types.")
            _ = typeArgument()
            expect(.greater, "Expected '>'.")
            return .map
        }
        return nil
    }

    /// `alias.Type name`: an imported type followed by a variable name.
    private func isLibraryTypeAnnotation() -> Bool {
        guard peek(1)?.kind == .dot, case .identifier? = peek(2)?.kind, case .identifier? = peek(3)?.kind else {
            return false
        }
        return true
    }

    /// `chart.point name`: the dotted type name followed by a variable name.
    private func isChartPointAnnotation() -> Bool {
        guard peek(1)?.kind == .dot, case .identifier("point")? = peek(2)?.kind,
            case .identifier? = peek(3)?.kind
        else { return false }
        return true
    }

    /// One type argument of `array<…>` or `map<…, …>`: a type name, a dotted one (`chart.point`), or a nested
    /// generic (`array<float>`). Returns its first word, which is all the runtime needs.
    mutating func typeArgument() -> String? {
        var word: String
        switch current.kind {
        case .typeKeyword(let type): word = type.rawValue
        case .identifier(let name): word = name
        default:
            error("PINE2012", "Expected a type.", current.range)
            return nil
        }
        advance()
        while at(.dot), case .identifier(let part)? = peek(1)?.kind {
            word += "." + part
            advance()
            advance()
        }
        if take(.less) {
            _ = typeArgument()
            while take(.comma) { _ = typeArgument() }
            expect(.greater, "Expected '>'.")
        }
        return word
    }

    @discardableResult
    mutating func skipQualifier() -> PineQualifier? {
        guard case .identifier(let word) = current.kind, let qualifier = Self.qualifiers[word],
            let next = peek(1)
        else { return nil }
        switch next.kind {
        case .typeKeyword, .identifier:
            advance()
            return qualifier
        default: return nil
        }
    }

    // MARK: - Imports

    /// `import user/lib/1 as alias`: the compiler reads these lines itself (a library path is not an
    /// expression), so the parser only steps over one.
    mutating func skipImportDeclaration() -> Bool {
        guard case .identifier("import") = current.kind, let next = peek(1), case .identifier = next.kind
        else { return false }
        skipToLineEnd()
        return true
    }
}
