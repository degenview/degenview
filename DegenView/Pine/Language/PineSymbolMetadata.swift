import Foundation

/// Signatures and one-line documentation for Pine builtins, for editor tooling.
///
/// `PineSymbolCatalog` owns which names exist; this adds how to call them. Entries are written in a
/// one-line notation, one string per overload, grouped by family in `PineSymbolMetadata+*.swift`:
///
///     ta.rsi(source: series float, length: simple int) -> series float :: Relative Strength Index.
///     ta.sma(source: series float, length: series int = 14) -> series float
///     plot(series: series float, title?: const string) -> void
///
/// A parameter with a default (`= x`) or a trailing `?` on its name is optional. A line without
/// parentheses documents a variable: `close :: Closing price of the current bar.`
/// `PineSymbolMetadataTests` guards that every line parses and that the names agree with the catalog.
enum PineSymbolMetadata {
    private enum Entry {
        case callable(PineSymbolSignature)
        case variable(name: String, summary: String)
    }

    /// Signatures by qualified name; several for an overloaded builtin, in declaration order.
    static let table: [String: [PineSymbolSignature]] = parsed.signatures

    /// One-line documentation for variables and constants, by qualified name.
    static let variableSummaries: [String: String] = parsed.summaries

    /// Source lines that did not parse. Always empty; the test fails on a malformed entry rather
    /// than the editor silently losing it.
    static let rejectedLines: [String] = parsed.rejected

    private struct Parsed {
        var signatures: [String: [PineSymbolSignature]] = [:]
        var summaries: [String: String] = [:]
        var rejected: [String] = []
    }

    private static let parsed: Parsed = {
        var result = Parsed()
        for line in allLines {
            switch parse(line) {
            case .callable(let signature)?: result.signatures[signature.name, default: []].append(signature)
            case .variable(let name, let summary)?: result.summaries[name] = summary
            case nil: result.rejected.append(line)
            }
        }
        return result
    }()

    private static var allLines: [String] {
        [
            globalEntries, declarationEntries, visualEntries, taEntries, mathEntries, stringEntries,
            colorEntries, inputEntries, strategyEntries, requestEntries, drawingEntries,
            collectionEntries, variableEntries,
        ]
        .flatMap { $0.split(separator: "\n").map { $0.trimmingCharacters(in: .whitespaces) } }
        .filter { !$0.isEmpty }
    }

    // MARK: - Parsing

    private static func parse(_ line: String) -> Entry? {
        var head = line
        var summary: String?
        if let separator = line.range(of: " :: ") {
            head = String(line[..<separator.lowerBound])
            summary = String(line[separator.upperBound...]).trimmingCharacters(in: .whitespaces)
        }
        guard let open = head.firstIndex(of: "(") else {
            guard let summary, !head.isEmpty, !head.contains(" ") else { return nil }
            return .variable(name: head, summary: summary)
        }
        let name = String(head[..<open]).trimmingCharacters(in: .whitespaces)
        guard isQualifiedName(name), let close = head.lastIndex(of: ")"), close > open else { return nil }
        let parameterText = String(head[head.index(after: open)..<close])
        var returns = "void"
        let rest = head[head.index(after: close)...].trimmingCharacters(in: .whitespaces)
        if !rest.isEmpty {
            guard rest.hasPrefix("->") else { return nil }
            returns = String(rest.dropFirst(2)).trimmingCharacters(in: .whitespaces)
            guard !returns.isEmpty else { return nil }
        }
        var parameters: [PineSymbolSignature.Parameter] = []
        for piece in splitTopLevel(parameterText) {
            guard let parameter = parseParameter(piece) else { return nil }
            parameters.append(parameter)
        }
        return .callable(
            PineSymbolSignature(name: name, parameters: parameters, returns: returns, summary: summary))
    }

    private static func parseParameter(_ text: String) -> PineSymbolSignature.Parameter? {
        guard let colon = text.range(of: ": ") else { return nil }
        var name = String(text[..<colon.lowerBound]).trimmingCharacters(in: .whitespaces)
        var rest = String(text[colon.upperBound...]).trimmingCharacters(in: .whitespaces)
        var optional = false
        if name.hasSuffix("?") {
            optional = true
            name.removeLast()
        }
        var defaultValue: String?
        if let equals = rest.range(of: " = ") {
            defaultValue = String(rest[equals.upperBound...]).trimmingCharacters(in: .whitespaces)
            rest = String(rest[..<equals.lowerBound]).trimmingCharacters(in: .whitespaces)
        }
        guard isIdentifier(name), !rest.isEmpty, defaultValue?.isEmpty != true else { return nil }
        return .init(
            name: name, type: rest, defaultValue: defaultValue, isOptional: optional || defaultValue != nil)
    }

    /// Splits on commas that sit outside `<>`, `()`, `[]` and string quotes.
    private static func splitTopLevel(_ text: String) -> [String] {
        var pieces: [String] = []
        var depth = 0
        var inString = false
        var current = ""
        for character in text {
            if character == "\"" { inString.toggle() }
            if !inString {
                switch character {
                case "<", "(", "[": depth += 1
                case ">", ")", "]": depth -= 1
                default: break
                }
            }
            if character == ",", depth == 0, !inString {
                pieces.append(current.trimmingCharacters(in: .whitespaces))
                current = ""
            } else {
                current.append(character)
            }
        }
        let last = current.trimmingCharacters(in: .whitespaces)
        if !last.isEmpty { pieces.append(last) }
        return pieces
    }

    private static func isIdentifier(_ text: String) -> Bool {
        guard let first = text.first, first.isLetter || first == "_" else { return false }
        return text.allSatisfy { $0.isLetter || $0.isNumber || $0 == "_" }
    }

    private static func isQualifiedName(_ text: String) -> Bool {
        let parts = text.split(separator: ".", omittingEmptySubsequences: false)
        return !parts.isEmpty && parts.allSatisfy { isIdentifier(String($0)) }
    }
}
