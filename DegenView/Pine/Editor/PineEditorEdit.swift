import Foundation

/// One contiguous text replacement plus the selection that follows it, in UTF-16 units of the
/// text it was computed from. Editor assistance produces these as plain values; the text view
/// applies them as a single undoable change, so no range outlives the text it described.
struct PineEditorEdit: Equatable {
    var range: NSRange
    var replacement: String
    var selection: NSRange

    /// Moves the selection without touching the text (overtyping a closing delimiter).
    var isSelectionOnly: Bool { range.length == 0 && replacement.isEmpty }

    static func moveCaret(to location: Int) -> PineEditorEdit {
        PineEditorEdit(
            range: NSRange(location: location, length: 0), replacement: "",
            selection: NSRange(location: location, length: 0))
    }

    static func insert(
        _ text: String, at location: Int, caret: Int
    ) -> PineEditorEdit {
        PineEditorEdit(
            range: NSRange(location: location, length: 0), replacement: text,
            selection: NSRange(location: caret, length: 0))
    }

    /// The text and selection after applying the edit to `source`.
    func applied(to source: String) -> (text: String, selection: NSRange) {
        (
            (source as NSString).replacingCharacters(in: range, with: replacement),
            selection
        )
    }
}
