import Foundation

/// Source text -> `PineCompiledProgram`: lex, parse, read the declaration, fold constants,
/// collect inputs, then run the structural and type checks.
enum PineCompiler {
    private static let versionAnnotation = "//@version="
    private static let supportedVersion = "6"

    static func compile(source: String, limits: PineLimits = .default) -> PineCompiledProgram {
        let normalizedSource = normalizeLineEndings(in: source)
        var diagnostics: [PineDiagnostic] = []
        let version = declaredVersion(in: normalizedSource, &diagnostics)
        let lexed = PineLexer(source: normalizedSource, limits: limits).lex()
        diagnostics += lexed.diagnostics
        var parser = PineParser(tokens: lexed.tokens, limits: limits)
        let (statements, parseDiagnostics) = parser.parse()
        diagnostics += parseDiagnostics

        let environment = constantEnvironment(statements)
        let metadata = declarationMetadata(
            statements, version: version, environment: environment, &diagnostics)
        if metadata.type != .library {
            diagnostics += parser.exportRanges.map {
                .error("PINE3037", .semantic, "export is only allowed in a library().", $0)
            }
        }
        var schema = PineInputSchema()
        collectInputs(statements, environment, &schema, &diagnostics)
        diagnostics += PineStructureValidator.validate(statements)
        diagnostics += PineAlertCallValidator.validate(statements)
        // Type errors on top of a broken parse would only be noise from a half-built tree.
        if !diagnostics.contains(where: { $0.category == .lexical || $0.category == .syntax }) {
            diagnostics += PineTypeChecker.check(statements)
        }
        return .init(
            source: normalizedSource, statements: statements, declaration: metadata,
            inputSchema: schema, diagnostics: diagnostics, methodNames: parser.methodNames)
    }

    /// Text copied from browsers and editors can contain CR-only or Unicode line separators.
    /// Normalize them before both annotation discovery and lexing so a leading `//` comment
    /// cannot accidentally consume the entire script.
    static func normalizeLineEndings(in source: String) -> String {
        source
            .replacingOccurrences(of: "\r\n", with: "\n")
            .replacingOccurrences(of: "\r", with: "\n")
            .replacingOccurrences(of: "\u{2028}", with: "\n")
            .replacingOccurrences(of: "\u{2029}", with: "\n")
    }

    /// The first `//@version=N` annotation's value, reporting when it is missing or unsupported.
    private static func declaredVersion(
        in source: String, _ diagnostics: inout [PineDiagnostic]
    ) -> String? {
        let versions = source.split(separator: "\n").compactMap { line -> String? in
            let trimmed = line.trimmingCharacters(in: .whitespacesAndNewlines)
            return trimmed.hasPrefix(versionAnnotation)
                ? String(trimmed.dropFirst(versionAnnotation.count)) : nil
        }
        guard let first = versions.first else {
            diagnostics.append(
                .error("PINE0001", .semantic, "Missing //@version=6 annotation.", .zero))
            return nil
        }
        if first != supportedVersion {
            diagnostics.append(
                .error(
                    "PINE0002", .semantic,
                    "Pine Script version \(first) is not supported; this runtime targets v6.", .zero))
        }
        return first
    }
}
