import AppKit

extension PineTextView {
    /// The context editing assistance decides from, or `nil` when it must stay out of the way:
    /// a read-only view, several selections, or an IME composition in progress (marked text),
    /// where rewriting the text would corrupt the input method's state.
    var editingContext: PineEditorContext? {
        guard isEditable, !hasMarkedText(), selectedRanges.count == 1 else { return nil }
        return PineEditorContext(source: string, selection: selectedRange())
    }

    func pairingEdit(forTyped input: Any) -> PineEditorEdit? {
        guard let text = (input as? String) ?? (input as? NSAttributedString)?.string,
            let context = editingContext
        else { return nil }
        return PineEditorPairing.typed(text, in: context)
    }

    /// Applies an edit as exactly one undoable change, through the path AppKit needs for undo,
    /// the delegate and `textDidChange` (and so highlighting and diagnostics) to all run.
    func perform(_ edit: PineEditorEdit, actionName: String? = nil) {
        if edit.isSelectionOnly {
            setSelectedRange(edit.selection)
            return
        }
        // Keep this change from merging into the typing before it or the typing after it:
        // one Undo takes back exactly what the editor did, `(` and its `)` together.
        breakUndoCoalescing()
        guard shouldChangeText(in: edit.range, replacementString: edit.replacement) else { return }
        replaceCharacters(in: edit.range, with: edit.replacement)
        setSelectedRange(edit.selection)
        didChangeText()
        if let actionName { undoManager?.setActionName(actionName) }
        breakUndoCoalescing()
        scrollRangeToVisible(edit.selection)
    }
}
