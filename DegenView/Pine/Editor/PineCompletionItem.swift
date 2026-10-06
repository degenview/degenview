import Foundation

/// One row of the completion list.
struct PineCompletionItem: Identifiable, Equatable, Sendable {
    /// Stable across refreshes, so the selection survives re-filtering: origin, path and kind.
    let id: String
    /// What the row shows and what is matched against the prefix: `rsi`, not `ta.rsi`.
    let label: String
    /// The text that replaces `replacementRange`, before any call or dot is added.
    let insertText: String
    let kind: PineCompletionKind
    let origin: PineCompletionOrigin
    let style: PineCompletionInsertionStyle
    /// The signature (`ta.rsi(source, length) → series float`) or the type of a variable.
    let detail: String?
    let documentation: String?
    /// The call forms of a callable, for the popup footer and signature help that follows acceptance.
    let signatures: [PineSymbolSignature]
    /// The identifier being completed, as an editor range.
    let replacementRange: NSRange
    /// Where the candidate lives, nearest scope first. Ranking reads it; see `PineCompletionRanking`.
    let tier: PineCompletionRanking.Tier

    var isCallable: Bool { style == .callable }
}
