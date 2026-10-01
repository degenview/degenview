import Foundation

extension PineParser {
    static let objectTypes: [String: PineValueType] = [
        "line": .line, "label": .label, "box": .box, "table": .table, "array": .array,
    ]

    static let qualifiers: [String: PineQualifier] = [
        "const": .constant, "input": .input, "simple": .simple, "series": .series,
    ]

    /// `[var|varip] [type] name = expression`. Parsed speculatively: when the tokens do
    /// not form a declaration header the cursor is restored so they parse as an expression.
    mutating func declaration() -> PineStatement? {
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
}
