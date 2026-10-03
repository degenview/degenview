import AppKit

/// The completion list's window: a borderless panel that is never key, so the text view keeps
/// the keyboard and the caret while the list is open. It is a child of the editor's window, which
/// moves, orders and closes it with the editor.
final class PineCompletionPanel: NSPanel {
    enum Placement: Equatable {
        case below
        case above
    }

    let list = PineCompletionListView()

    init() {
        super.init(
            contentRect: NSRect(x: 0, y: 0, width: PineCompletionListView.width, height: 100),
            styleMask: [.borderless, .nonactivatingPanel], backing: .buffered, defer: true)
        isOpaque = false
        backgroundColor = .clear
        hasShadow = true
        hidesOnDeactivate = false
        becomesKeyOnlyIfNeeded = true
        isReleasedWhenClosed = false
        collectionBehavior = [.transient, .ignoresCycle]
        contentView = list
        setAccessibilityRole(.popUpButton)
        setAccessibilityLabel("Completions")
    }

    override var canBecomeKey: Bool { false }
    override var canBecomeMain: Bool { false }

    /// Shows the list under the caret line, or above it when the screen has no room below, and
    /// keeps it inside the screen. `caret` is the caret line's rectangle in screen coordinates.
    @discardableResult
    func present(
        _ items: [PineCompletionItem], prefix: String, selected: Int, caret: NSRect, in parent: NSWindow
    ) -> Placement {
        appearance = parent.effectiveAppearance
        list.show(items, prefix: prefix, selected: selected)
        let size = NSSize(width: PineCompletionListView.width, height: list.preferredHeight)
        let visible = (parent.screen ?? NSScreen.main)?.visibleFrame ?? NSRect(x: 0, y: 0, width: 4000, height: 3000)

        var placement = Placement.below
        var origin = NSPoint(x: caret.minX - 28, y: caret.minY - size.height - 2)
        if origin.y < visible.minY {
            placement = .above
            origin.y = caret.maxY + 2
        }
        origin.x = min(max(origin.x, visible.minX + 4), visible.maxX - size.width - 4)
        origin.y = min(max(origin.y, visible.minY + 4), visible.maxY - size.height - 4)
        setFrame(NSRect(origin: origin, size: size), display: true)
        if parent.childWindows?.contains(self) != true { parent.addChildWindow(self, ordered: .above) }
        orderFront(nil)
        return placement
    }

    func dismiss() {
        parent?.removeChildWindow(self)
        orderOut(nil)
    }
}
