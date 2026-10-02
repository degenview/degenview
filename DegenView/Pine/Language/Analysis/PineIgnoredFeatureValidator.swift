import Foundation

/// Warns about TradingView features the engine accepts and then ignores (`PineIgnoredFeatures`). A
/// warning never makes the script invalid.
///
/// | Code | Warning |
/// |---|---|
/// | `PINE7001` | an argument that has no effect |
/// | `PINE7002` | a function that does nothing |
/// | `PINE7003` | a variable that is always `na` |
/// | `PINE7004` | an argument value that is drawn or handled as something else |
enum PineIgnoredFeatureValidator {
    static func validate(_ statements: [PineStatement]) -> [PineDiagnostic] {
        let own = Set(
            statements.compactMap { statement -> String? in
                if case .function(let name, _, _, _) = statement { return name }
                return nil
            })
        var diagnostics: [PineDiagnostic] = []
        PineStatement.forEachCall(withArgumentsIn: statements) { name, arguments, range in
            guard !own.contains(name) else { return }
            if let entry = PineIgnoredFeatures.functionPrefixes.first(where: { name.hasPrefix($0.prefix) }) {
                diagnostics.append(
                    .warning(
                        "PINE7002", .unsupported, "\(name)() is ignored. \(entry.explanation)",
                        spanning(name, from: range)))
            }
            for argument in arguments {
                guard let argumentName = argument.name else { continue }
                if let explanation = PineIgnoredFeatures.arguments[name]?[argumentName] {
                    diagnostics.append(
                        .warning(
                            "PINE7001", .unsupported,
                            "\(name)(): `\(argumentName)` is ignored. \(explanation)",
                            argument.nameRange ?? argument.value.range))
                }
                if case .identifier(let constant, let valueRange) = argument.value,
                    let explanation = PineIgnoredFeatures.values[name]?[argumentName]?[constant]
                {
                    diagnostics.append(
                        .warning(
                            "PINE7004", .unsupported,
                            "\(name)(): `\(argumentName) = \(constant)` is not supported. \(explanation)",
                            spanning(constant, from: valueRange)))
                }
            }
        }
        PineStatement.forEachIdentifier(in: statements) { name, range in
            guard PineSymbolCatalog.isUnimplementedVariable(name), !own.contains(name) else { return }
            let message = "`\(name)` is not supported. \(PineIgnoredFeatures.unimplementedVariable)"
            diagnostics.append(.warning("PINE7003", .unsupported, message, spanning(name, from: range)))
        }
        return diagnostics.sorted { $0.range.start.offset < $1.range.start.offset }
    }

    /// The parser records only the first token of a dotted name (`plot` of `plot.style_linebr`); the
    /// underline should cover all of it. Names never span lines.
    private static func spanning(_ name: String, from range: PineSourceRange) -> PineSourceRange {
        let length = name.utf16.count
        guard range.end.offset - range.start.offset < length else { return range }
        var end = range.start
        end.column += length
        end.offset += length
        return .init(start: range.start, end: end)
    }
}
