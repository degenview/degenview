import AppKit

/// The signature popup: one compact panel over the caret line, `ta.sma(source, length) → series float`
/// with the argument under the caret in bold, and the summary under it. Like the completion list it
/// is a child window that never takes the keyboard.
final class PineSignaturePanel: NSPanel {
    private let background = NSVisualEffectView()
    private let label = NSTextField(wrappingLabelWithString: "")

    private static let font = NSFont.monospacedSystemFont(ofSize: 12, weight: .regular)
    private static let boldFont = NSFont.monospacedSystemFont(ofSize: 12, weight: .bold)
    private static let maximumWidth: CGFloat = 560

    init() {
        super.init(
            contentRect: NSRect(x: 0, y: 0, width: 300, height: 30),
            styleMask: [.borderless, .nonactivatingPanel], backing: .buffered, defer: true)
        isOpaque = false
        backgroundColor = .clear
        hasShadow = true
        hidesOnDeactivate = false
        becomesKeyOnlyIfNeeded = true
        isReleasedWhenClosed = false
        collectionBehavior = [.transient, .ignoresCycle]

        background.material = .popover
        background.blendingMode = .behindWindow
        background.state = .active
        background.wantsLayer = true
        background.layer?.cornerRadius = 6
        background.layer?.masksToBounds = true
        label.maximumNumberOfLines = 4
        label.lineBreakMode = .byTruncatingTail
        label.preferredMaxLayoutWidth = Self.maximumWidth - 20
        background.addSubview(label)
        contentView = background
        setAccessibilityLabel("Signature help")
    }

    override var canBecomeKey: Bool { false }
    override var canBecomeMain: Bool { false }

    /// The text of the help: the active signature with its active parameter in bold, an
    /// overload counter when there is more than one form, and the summary.
    static func text(for help: PineSignatureHelp) -> NSAttributedString {
        let signature = help.signature
        let color = NSColor.labelColor
        let result = NSMutableAttributedString(
            string: "\(signature.name)(", attributes: [.font: font, .foregroundColor: color])
        for (index, parameter) in signature.parameters.enumerated() {
            if index > 0 { result.append(.init(string: ", ", attributes: [.font: font, .foregroundColor: color])) }
            let isActive = index == help.activeParameter
            result.append(
                .init(
                    string: parameter.name,
                    attributes: isActive
                        ? [
                            .font: boldFont, .foregroundColor: color,
                            .underlineStyle: NSUnderlineStyle.single.rawValue,
                            .underlineColor: NSColor.controlAccentColor,
                        ] : [.font: font, .foregroundColor: color]))
        }
        result.append(.init(string: ")", attributes: [.font: font, .foregroundColor: color]))
        if !signature.returns.isEmpty {
            result.append(
                .init(
                    string: " → \(signature.returns)",
                    attributes: [.font: font, .foregroundColor: NSColor.secondaryLabelColor]))
        }
        if help.signatures.count > 1 {
            result.append(
                .init(
                    string: "   (\(help.activeSignature + 1)/\(help.signatures.count))",
                    attributes: [.font: font, .foregroundColor: NSColor.tertiaryLabelColor]))
        }
        var notes: [String] = []
        if let index = help.activeParameter, signature.parameters.indices.contains(index) {
            notes.append(signature.parameters[index].detail)
        }
        if let summary = signature.summary { notes.append(summary) }
        if !notes.isEmpty {
            result.append(
                .init(
                    string: "\n" + notes.joined(separator: "  ·  "),
                    attributes: [.font: NSFont.systemFont(ofSize: 11), .foregroundColor: NSColor.secondaryLabelColor]))
        }
        return result
    }

    /// Shows the help above the caret line; `caret` is that line's rectangle in screen coordinates.
    func present(_ help: PineSignatureHelp, caret: NSRect, in parent: NSWindow) {
        appearance = parent.effectiveAppearance
        label.attributedStringValue = Self.text(for: help)
        let fitting = label.sizeThatFits(NSSize(width: Self.maximumWidth - 20, height: .greatestFiniteMagnitude))
        let size = NSSize(width: min(Self.maximumWidth, fitting.width + 20), height: fitting.height + 12)
        let visible = (parent.screen ?? NSScreen.main)?.visibleFrame ?? NSRect(x: 0, y: 0, width: 4000, height: 3000)
        var origin = NSPoint(x: caret.minX, y: caret.maxY + 2)
        origin.x = min(max(origin.x, visible.minX + 4), visible.maxX - size.width - 4)
        if origin.y + size.height > visible.maxY { origin.y = caret.minY - size.height - 2 }
        setFrame(NSRect(origin: origin, size: size), display: true)
        background.frame = NSRect(origin: .zero, size: size)
        label.frame = NSRect(x: 10, y: 6, width: size.width - 20, height: size.height - 12)
        if parent.childWindows?.contains(self) != true { parent.addChildWindow(self, ordered: .above) }
        orderFront(nil)
    }

    func dismiss() {
        parent?.removeChildWindow(self)
        orderOut(nil)
    }
}
