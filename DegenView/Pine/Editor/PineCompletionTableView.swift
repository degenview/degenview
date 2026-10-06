import AppKit

/// The list's table. The panel that holds it never becomes key, so a click must be taken from
/// the first mouse-down, and must not move the editor's focus: the table answers it itself
/// instead of going through AppKit's selection and first-responder handling.
final class PineCompletionTableView: NSTableView {
    /// Called with the clicked row and the click count.
    var onClick: ((Int, Int) -> Void)?

    override var acceptsFirstResponder: Bool { false }
    override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }
    override var mouseDownCanMoveWindow: Bool { false }

    override func mouseDown(with event: NSEvent) {
        let row = row(at: convert(event.locationInWindow, from: nil))
        guard row >= 0 else { return }
        onClick?(row, event.clickCount)
    }
}
