import Foundation

/// Everything the editor derives from one version of the source: the lexical snapshot and the
/// symbol index built on it. Highlighting, completion and signature help read this, so moving the
/// caret never lexes or indexes again.
struct PineEditorAnalysisSnapshot {
    let source: String
    let lexical: PineLexicalSnapshot
    let index: PineSourceSymbolIndex
    /// Counts snapshots in the cache that built this one, so a stale result can be recognised.
    let version: Int
}
