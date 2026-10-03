import Foundation

/// Where an `import user/Library/version` finds its source. The compiler never reads files: the app supplies the
/// libraries it has, and tests supply their own.
protocol PineLibraryResolver: Sendable {
    /// The source of the library an import path (`user/name/version`) names, or nil when there is none.
    func source(forLibrary path: String) -> String?
}

/// The default: no library can be found.
struct PineNoLibraries: PineLibraryResolver {
    func source(forLibrary path: String) -> String? { nil }
}

/// One `import path as alias` of a script and, when it resolved, the library's compiled program.
struct PineImport: Sendable {
    let path: String
    let alias: String
    let range: PineSourceRange
    let library: PineCompiledProgram?
}

/// Reads a script's `import` lines and links each to a compiled library.
enum PineLibraryLinker {
    struct Declaration {
        var path: String
        var alias: String
        var range: PineSourceRange
    }

    /// How deep libraries may import libraries; a guard against runaway graphs.
    static let maximumDepth = 8

    /// The alias an import without `as` gets: the library name, the middle of `user/Library/version`.
    /// Nil for text that is not a three-part path (an import still being typed).
    static func defaultAlias(forPath path: String) -> String? {
        let parts = path.split(separator: "/", omittingEmptySubsequences: false)
        return parts.count == 3 && !parts[1].isEmpty ? String(parts[1]) : nil
    }

    private static let importLine = try? NSRegularExpression(
        pattern: #"^import[ \t]+(\S+)(?:[ \t]+as[ \t]+([A-Za-z_][A-Za-z0-9_]*))?[ \t]*(?://.*)?$"#)

    /// The `import` lines of `source` (a line that starts with `import `), with diagnostics for malformed ones
    /// and for aliases that clash with each other or with a builtin namespace.
    static func scan(_ source: String) -> (declarations: [Declaration], diagnostics: [PineDiagnostic]) {
        var declarations: [Declaration] = []
        var diagnostics: [PineDiagnostic] = []
        var offset = 0
        for (index, line) in source.split(separator: "\n", omittingEmptySubsequences: false).enumerated() {
            defer { offset += line.utf16.count + 1 }
            guard line.hasPrefix("import ") || line.hasPrefix("import\t") else { continue }
            let text = String(line)
            let range = PineSourceRange(
                start: .init(line: index + 1, column: 1, offset: offset),
                end: .init(line: index + 1, column: text.count + 1, offset: offset + line.utf16.count))
            guard let match = importLine?.firstMatch(in: text, range: NSRange(text.startIndex..., in: text)),
                let pathRange = Range(match.range(at: 1), in: text)
            else {
                diagnostics.append(
                    .error(
                        "PINE3042", .semantic, "An import reads: import user/Library/version [as alias].", range))
                continue
            }
            let path = String(text[pathRange])
            let parts = path.split(separator: "/", omittingEmptySubsequences: false)
            guard parts.count == 3, parts.allSatisfy({ !$0.isEmpty }), parts[2].allSatisfy(\.isNumber) else {
                diagnostics.append(
                    .error(
                        "PINE3042", .semantic,
                        "'\(path)' is not a library path: expected user/Library/version, e.g. me/Math/1.", range))
                continue
            }
            let alias = Range(match.range(at: 2), in: text).map { String(text[$0]) } ?? String(parts[1])
            if declarations.contains(where: { $0.alias == alias }) {
                diagnostics.append(
                    .error("PINE3045", .semantic, "The import alias '\(alias)' is already used.", range))
            } else if PineSymbolCatalog.namespaces.contains(alias) || PineSymbolCatalog.reservedWords.contains(alias) {
                diagnostics.append(
                    .error(
                        "PINE3045", .semantic, "'\(alias)' is a Pine namespace and cannot be an import alias.",
                        range))
            } else {
                declarations.append(Declaration(path: path, alias: alias, range: range))
            }
        }
        return (declarations, diagnostics)
    }

    /// Compiles the library behind each import. `stack` holds the paths being compiled, to catch cycles.
    static func link(
        _ declarations: [Declaration], resolver: PineLibraryResolver, limits: PineLimits, stack: [String]
    ) -> (imports: [PineImport], diagnostics: [PineDiagnostic]) {
        var imports: [PineImport] = []
        var diagnostics: [PineDiagnostic] = []
        for declaration in declarations {
            func fail(_ code: String, _ message: String) {
                diagnostics.append(.error(code, .semantic, message, declaration.range))
                imports.append(
                    PineImport(path: declaration.path, alias: declaration.alias, range: declaration.range, library: nil)
                )
            }
            if stack.contains(declaration.path) {
                fail("PINE3041", "Library '\(declaration.path)' imports itself, directly or through another library.")
                continue
            }
            if stack.count >= maximumDepth {
                fail("PINE3047", "Libraries import each other more than \(maximumDepth) levels deep.")
                continue
            }
            guard let source = resolver.source(forLibrary: declaration.path) else {
                fail("PINE3040", "Library '\(declaration.path)' was not found.")
                continue
            }
            let program = PineCompiler.compile(
                source: source, limits: limits, libraries: resolver, importStack: stack + [declaration.path])
            guard program.declaration.type == .library else {
                fail(
                    "PINE3043",
                    "'\(declaration.path)' is not a library: it declares an \(program.declaration.type.rawValue).")
                continue
            }
            if let problem = program.diagnostics.first(where: { $0.severity == .error }) {
                // A cycle or a too-deep chain is found below this library; say so instead of blaming it.
                if ["PINE3041", "PINE3047"].contains(problem.code) {
                    fail(problem.code, problem.message)
                } else {
                    fail(
                        "PINE3043",
                        "Library '\(declaration.path)' has errors (line \(problem.range.start.line): \(problem.message))."
                    )
                }
                continue
            }
            imports.append(
                PineImport(path: declaration.path, alias: declaration.alias, range: declaration.range, library: program)
            )
        }
        return (imports, diagnostics)
    }

    /// Calls such as `alias.name(…)` or `alias.Type.new(…)` must name something the library exports.
    static func validateUsage(_ statements: [PineStatement], imports: [PineImport]) -> [PineDiagnostic] {
        let libraries = Dictionary(
            imports.compactMap { entry in entry.library.map { (entry.alias, $0) } },
            uniquingKeysWith: { first, _ in first })
        guard !libraries.isEmpty else { return [] }
        var diagnostics: [PineDiagnostic] = []
        PineStatement.forEachCall(in: statements) { name, range in
            guard let dot = name.firstIndex(of: "."), let library = libraries[String(name[..<dot])] else { return }
            let rest = name[name.index(after: dot)...]
            let member = String(rest.prefix { $0 != "." })
            if !library.exportedNames.contains(member) {
                diagnostics.append(
                    .error(
                        "PINE3044", .semantic,
                        "'\(name[..<dot])' does not export '\(member)'.", range))
            }
        }
        return diagnostics
    }
}
