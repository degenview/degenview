import AppKit

/// The line commands bound to key equivalents. Their keys are handled before AppKit's own
/// bindings, which is what lets ⌥↑/⌥↓ move lines instead of jumping by paragraph.
enum PineEditorShortcut {
    case toggleComment, moveUp, moveDown, duplicateUp, duplicateDown

    private static let upArrow: UInt16 = 126
    private static let downArrow: UInt16 = 125

    init?(keyCode: UInt16, key: String, modifiers: NSEvent.ModifierFlags) {
        // Arrow keys carry `.function`/`.numericPad`; only the real modifiers decide.
        let modifiers = modifiers.intersection([.command, .option, .shift, .control])
        switch (modifiers, keyCode) {
        case ([.command], _) where key == "/": self = .toggleComment
        case ([.option], Self.upArrow): self = .moveUp
        case ([.option], Self.downArrow): self = .moveDown
        case ([.option, .shift], Self.upArrow): self = .duplicateUp
        case ([.option, .shift], Self.downArrow): self = .duplicateDown
        default: return nil
        }
    }

    func edit(in context: PineEditorContext) -> PineEditorEdit? {
        switch self {
        case .toggleComment: PineEditorCommands.toggleComment(in: context)
        case .moveUp: PineEditorCommands.move(down: false, in: context)
        case .moveDown: PineEditorCommands.move(down: true, in: context)
        case .duplicateUp: PineEditorCommands.duplicate(down: false, in: context)
        case .duplicateDown: PineEditorCommands.duplicate(down: true, in: context)
        }
    }

    var actionName: String {
        switch self {
        case .toggleComment: "Toggle Comment"
        case .moveUp, .moveDown: "Move Line"
        case .duplicateUp, .duplicateDown: "Duplicate Line"
        }
    }
}
