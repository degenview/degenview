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
        guard let target = declarationTarget() else {
            index = start
            return nil
        }
        for _ in 0...target.tokenCount { advance() }
        guard let value = expression() else { return nil }
        return .declaration(
            name: target.name, annotation: .init(type: type, qualifier: qualifier), mode: mode,
            value: value, range: target.range)
    }

    /// The name before a declaration's `=`. A dotted chain (`math.max = 44`) is kept whole so
    /// the validator can reject it with CE10090 rather than the parser failing on the `=`.
    /// `color` is a type keyword but also a namespace, so `color = 44` names a variable.
    private func declarationTarget() -> (name: String, range: PineSourceRange, tokenCount: Int)? {
        var name: String
        switch current.kind {
        case .identifier(let word): name = word
        case .typeKeyword(.color): name = PineValueType.color.rawValue
        default: return nil
        }
        var range = current.range
        var count = 1
        while peek(count)?.kind == .dot, let next = peek(count + 1),
            case .identifier(let part) = next.kind
        {
            name += "." + part
            range.end = next.range.end
            count += 2
        }
        guard peek(count)?.kind == .assign else { return nil }
        return (name, range, count)
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
