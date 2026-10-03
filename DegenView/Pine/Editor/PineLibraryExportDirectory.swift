import Foundation

/// What the editor knows about the libraries a script can import, read from the app's
/// `PineLibraryRegistry` and nothing else.
///
/// A library's exports come from the same source symbol index the editor builds for any script
/// (`export` declarations with their parameters), so no compile happens. The result is kept per
/// library source, so a lookup costs one string comparison until the library is edited. The
/// library text is lexed with its own snapshot, never the shared one, so asking about a library
/// does not evict the lex of the script being edited.
final class PineLibraryExportDirectory: PineLibraryExportProviding, @unchecked Sendable {
    static let shared = PineLibraryExportDirectory(registry: .shared)

    private let registry: PineLibraryRegistry
    private let lock = NSLock()
    private var remembered: [String: (source: String, exports: [PineLibraryExport])] = [:]

    init(registry: PineLibraryRegistry) {
        self.registry = registry
    }

    func exports(forImportPath path: String) -> [PineLibraryExport]? {
        guard let source = registry.source(forLibrary: path) else { return nil }
        let key = Self.key(of: path)
        if let hit = lock.withLock({ remembered[key] }), hit.source == source { return hit.exports }
        let snapshot = PineLexicalSnapshot(source: source)
        let index = PineSourceSymbolIndex(snapshot: snapshot, source: source)
        let exports = index.exports.compactMap(Self.export)
        lock.withLock { remembered[key] = (source, exports) }
        return exports
    }

    func libraryNames() -> [String] { registry.libraryNames() }

    /// The registry matches on the library name alone, so one entry serves every user and version.
    private static func key(of path: String) -> String {
        let parts = path.split(separator: "/")
        return parts.count == 3 ? String(parts[1]) : path
    }

    private static func export(_ declaration: PineSourceSymbolIndex.Declaration) -> PineLibraryExport? {
        let kind: PineLibraryExport.Kind
        switch declaration.kind {
        case .function: kind = .function
        case .method: kind = .method
        case .variable: kind = .variable
        case .type: kind = .type
        case .enumeration: kind = .enumeration
        case .parameter, .loopVariable: return nil
        }
        return PineLibraryExport(name: declaration.name, kind: kind, parameters: declaration.parameters)
    }
}
