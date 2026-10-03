import Foundation

/// Keeps the analysis of the last source it was asked about. A different text replaces it, so the
/// cost is one lex and one index per edit, shared by everyone reading the same text.
///
/// The lexical snapshot comes from `PineLexicalSnapshot.shared`, so the highlighter, the editing
/// helpers and this cache all use one lex per text version.
final class PineEditorAnalysisCache: @unchecked Sendable {
    static let shared = PineEditorAnalysisCache()

    private let lock = NSLock()
    private var entry: PineEditorAnalysisSnapshot?
    private var builds = 0

    /// How many analyses this cache has built. Tests assert it stays put across caret moves.
    var buildCount: Int {
        lock.lock()
        defer { lock.unlock() }
        return builds
    }

    func analysis(for source: String) -> PineEditorAnalysisSnapshot {
        lock.lock()
        defer { lock.unlock() }
        if let entry, entry.source == source { return entry }
        let lexical = PineLexicalSnapshot.shared(for: source)
        builds += 1
        let snapshot = PineEditorAnalysisSnapshot(
            source: source, lexical: lexical, index: PineSourceSymbolIndex(snapshot: lexical, source: source),
            version: builds)
        entry = snapshot
        return snapshot
    }
}
