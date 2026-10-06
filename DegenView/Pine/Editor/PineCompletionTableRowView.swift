import AppKit

/// Draws the selected row in the accent color even though the panel is never the key window.
final class PineCompletionTableRowView: NSTableRowView {
    override var isEmphasized: Bool {
        get { true }
        set {}
    }
}
