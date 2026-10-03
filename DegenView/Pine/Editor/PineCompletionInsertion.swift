import Foundation

/// Builds the one edit that accepting a completion makes, so it is one undo step and the text
/// view applies it like any other editor assistance.
///
/// What is inserted follows the item's kind, never its spelling: a variable is the bare name, a
/// callable gets its parenthesis, a namespace gets its dot. Whether `(` arrives paired is
/// decided by `PineEditorPairing.canOpenPair`, the rule typing `(` there would follow, so
/// accepting `rsi` in `plot(ta.rs|)` gives `plot(ta.rsi(|))` and never an unbalanced call.
enum PineCompletionInsertion {
    /// What the controller shows after the text changes.
    enum FollowUp: Equatable {
        case none
        /// A namespace was accepted: list its members.
        case memberCompletion
        /// A call was opened: show its signature.
        case signatureHelp
    }

    struct Acceptance: Equatable {
        let edit: PineEditorEdit
        let followUp: FollowUp
    }

    /// The acceptance of `item` in the text `context` describes, or nil when the range the item was
    /// computed for no longer holds a word (the text changed underneath it).
    static func accept(_ item: PineCompletionItem, in context: PineEditorContext) -> Acceptance? {
        let range = item.replacementRange
        guard range.location >= 0, NSMaxRange(range) <= context.length,
            isWord(context.source.substring(with: range))
        else { return nil }
        let end = NSMaxRange(range)
        let next = context.unit(at: end)
        let name = item.insertText
        let length = (name as NSString).length

        func edit(_ replacement: String, caret: Int) -> PineEditorEdit {
            PineEditorEdit(
                range: range, replacement: replacement,
                selection: NSRange(location: range.location + caret, length: 0))
        }

        switch item.style {
        case .identifier:
            return Acceptance(edit: edit(name, caret: length), followUp: .none)
        case .argumentName:
            // `title` followed by an existing `=` only completes the name.
            if next == 0x3D || (context.isWhitespace(next) && context.unit(at: end + 1) == 0x3D) {
                return Acceptance(edit: edit(name, caret: length), followUp: .none)
            }
            return Acceptance(edit: edit(name + " = ", caret: length + 3), followUp: .none)
        case .namespace:
            if next == 0x2E {
                // `ta|.rsi`: the dot is already there; step over it.
                return Acceptance(
                    edit: PineEditorEdit(
                        range: range, replacement: name,
                        selection: NSRange(location: range.location + length + 1, length: 0)),
                    followUp: .memberCompletion)
            }
            return Acceptance(edit: edit(name + ".", caret: length + 1), followUp: .memberCompletion)
        case .callable:
            if next == 0x28 {
                return Acceptance(
                    edit: PineEditorEdit(
                        range: range, replacement: name,
                        selection: NSRange(location: range.location + length + 1, length: 0)),
                    followUp: .signatureHelp)
            }
            if PineEditorPairing.canOpenPair(before: next, in: context) {
                return Acceptance(edit: edit(name + "()", caret: length + 1), followUp: .signatureHelp)
            }
            return Acceptance(edit: edit(name + "(", caret: length + 1), followUp: .signatureHelp)
        }
    }

    private static func isWord(_ text: String) -> Bool {
        text.allSatisfy { $0.isLetter || $0.isNumber || $0 == "_" }
    }
}
