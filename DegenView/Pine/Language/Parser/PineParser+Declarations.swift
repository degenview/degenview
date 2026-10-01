import Foundation

extension PineParser {
    static let objectTypes: [String: PineValueType] = [
        "line": .line, "label": .label, "box": .box, "table": .table, "array": .array,
        // Typed `.object` rather than a kind of its own: the checker does not track handles of this kind.
        "linefill": .object,
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
        var type: PineValueType
        switch current.kind {
        case .typeKeyword(let t): type = t
        case .identifier(let word):
            if let t = Self.objectTypes[word] {
                type = t
            } else if userTypes.contains(word) {
                type = .object
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
            skipElementType()
            expect(.greater, "Expected '>'.")
            return .array
        }
        return nil
    }

    /// The `T` of `array<T>`; only one-word element types are supported.
    private mutating func skipElementType() {
        switch current.kind {
        case .typeKeyword, .identifier: advance()
        default: error("PINE2012", "Expected an element type.", current.range)
        }
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

    // MARK: - Declarations outside this release

    private static let unsupportedDeclarations: [String: (code: String, message: String)] = [
        "enum": ("PINE9007", "Enums are not supported in this release."),
        "import": ("PINE9008", "Library imports are not supported in this release."),
    ]

    /// `type Name`, `enum Name`, `method name(…) =>` and `import user/lib/1 as alias` are valid Pine
    /// the engine does not implement. Reporting the declaration once and skipping its body keeps one
    /// clear diagnostic from turning into one syntax error per field line. Returns whether it consumed
    /// anything; ordinary code that merely uses one of these words as a name is left alone.
    mutating func skipUnsupportedDeclaration() -> Bool {
        guard case .identifier(let word) = current.kind,
            let entry = Self.unsupportedDeclarations[word],
            let next = peek(1), case .identifier = next.kind
        else { return false }
        diagnostics.append(.error(entry.code, .unsupported, entry.message, current.range))
        skipToLineEnd()
        guard at(.newline), peek(1)?.kind == .indent else { return true }
        advance()
        advance()
        var depth = 1
        while depth > 0, !at(.eof) {
            if at(.indent) { depth += 1 } else if at(.dedent) { depth -= 1 }
            advance()
        }
        return true
    }
}
