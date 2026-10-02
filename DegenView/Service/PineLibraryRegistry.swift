import Foundation

/// Answers `import user/Library/version` from the library scripts in the Script Manager, synchronously, so the
/// compiler can run anywhere (the editor, a chart, the preview) without awaiting the script store.
///
/// A path is matched on its library name alone. The user and version parts are TradingView's way to address a
/// published library; a local library has neither, so they are accepted and ignored. The name is the script's
/// file name, or failing that the title in its `library("…")` declaration.
final class PineLibraryRegistry: PineLibraryResolver, @unchecked Sendable {
    static let shared = PineLibraryRegistry()

    private let lock = NSLock()
    private var byName: [String: String] = [:]
    private var byTitle: [String: String] = [:]

    /// Replaces the catalog with the library scripts among `scripts`.
    func publish(_ scripts: [LocalScript]) {
        var names: [String: String] = [:]
        var titles: [String: String] = [:]
        for script in scripts where script.type == .library {
            names[script.name] = script.source
            if let title = Self.title(of: script.source), titles[title] == nil { titles[title] = script.source }
        }
        lock.withLock {
            byName = names
            byTitle = titles
        }
    }

    func source(forLibrary path: String) -> String? {
        let parts = path.split(separator: "/")
        guard parts.count == 3 else { return nil }
        let name = String(parts[1])
        return lock.withLock { byName[name] ?? byTitle[name] }
    }

    private static let titlePattern = try? NSRegularExpression(
        pattern: #"^[ \t]*library[ \t]*\([ \t]*(?:"([^"]+)"|'([^']+)')"#, options: [.anchorsMatchLines])

    /// The name a `library("Title")` declaration gives, which is what an import path uses.
    static func title(of source: String) -> String? {
        let range = NSRange(source.startIndex..., in: source)
        guard let match = titlePattern?.firstMatch(in: source, range: range) else { return nil }
        for group in 1...2 {
            if let found = Range(match.range(at: group), in: source) { return String(source[found]) }
        }
        return nil
    }
}
