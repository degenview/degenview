import AppKit

final class PineTextView: NSTextView {
    /// What the user just did, for the completion controller to read when the text has changed.
    var pendingCompletionCause: PineCompletionTrigger.Cause?

    /// The completion list and signature help of this editor.
    private(set) lazy var completion = PineCompletionController(textView: self)

    /// Double-clicking `table` in `table.cell` selects only `table`: AppKit's word
    /// breaking treats dotted names as one word, which is wrong for code.
    override func selectionRange(
        forProposedRange proposedCharRange: NSRange, granularity: NSSelectionGranularity
    ) -> NSRange {
        if granularity == .selectByWord,
            let word = PineWordRange.range(at: proposedCharRange.location, in: string as NSString)
        {
            return word
        }
        return super.selectionRange(forProposedRange: proposedCharRange, granularity: granularity)
    }

    /// Backstop for double-click paths that bypass `selectionRange(forProposedRange:)`:
    /// when a double-click lands AppKit's own dotted "word" (`table.cell`), narrow it to
    /// the part under the pointer. Word-wise drags that extend past it are left alone.
    override func setSelectedRanges(
        _ ranges: [NSValue], affinity: NSSelectionAffinity, stillSelecting: Bool
    ) {
        var ranges = ranges
        if ranges.count == 1, let word = doubleClickedWord(),
            let storage = textStorage,
            ranges[0].rangeValue == storage.doubleClick(at: word.location),
            ranges[0].rangeValue != word
        {
            ranges = [NSValue(range: word)]
        }
        super.setSelectedRanges(ranges, affinity: affinity, stillSelecting: stillSelecting)
        // A typed character moves the caret too; `didChangeText` handles that case with the cause.
        if !stillSelecting, pendingCompletionCause == nil { completion.selectionDidChange() }
    }

    private func doubleClickedWord() -> NSRange? {
        guard let event = NSApp.currentEvent, event.window === window,
            [.leftMouseDown, .leftMouseDragged, .leftMouseUp].contains(event.type),
            event.clickCount == 2,
            let layoutManager, let textContainer
        else { return nil }
        var point = convert(event.locationInWindow, from: nil)
        point.x -= textContainerOrigin.x
        point.y -= textContainerOrigin.y
        let index = layoutManager.characterIndex(
            for: point, in: textContainer, fractionOfDistanceBetweenInsertionPoints: nil)
        return PineWordRange.range(at: index, in: string as NSString)
    }

    // MARK: - Editing assistance
    //
    // Each entry point asks a pure `PineEditor…` type for an edit and applies it as one undoable
    // change (`PineTextView+Editing`). `nil`, marked text (IME composition) or a read-only view
    // all fall through to AppKit's own behaviour.

    override func insertText(_ string: Any, replacementRange: NSRange) {
        if replacementRange.location == NSNotFound, !hasMarkedText(),
            let text = (string as? String) ?? (string as? NSAttributedString)?.string
        {
            pendingCompletionCause = .typed(text)
        }
        defer { pendingCompletionCause = nil }
        if replacementRange.location == NSNotFound, let edit = pairingEdit(forTyped: string) {
            perform(edit)
            return
        }
        super.insertText(string, replacementRange: replacementRange)
    }

    override func didChangeText() {
        super.didChangeText()
        completion.textDidChange(hasMarkedText() ? nil : pendingCompletionCause)
    }

    // The completion list takes Return, Tab, Up, Down and Escape only while it is open with a row
    // selected; closed, each falls through to the editing behaviour below.

    override func insertNewline(_ sender: Any?) {
        if completion.acceptSelected() { return }
        guard let context = editingContext else { return super.insertNewline(sender) }
        perform(PineIndentationEngine.newline(in: context))
    }

    override func insertTab(_ sender: Any?) {
        if completion.acceptSelected() { return }
        guard let context = editingContext else { return super.insertTab(sender) }
        perform(PineIndentationEngine.indent(in: context), actionName: "Indent")
    }

    override func moveDown(_ sender: Any?) {
        if !completion.moveSelection(by: 1) { super.moveDown(sender) }
    }

    override func moveUp(_ sender: Any?) {
        if !completion.moveSelection(by: -1) { super.moveUp(sender) }
    }

    override func cancelOperation(_ sender: Any?) {
        // NSTextView has no handler of its own; an Escape nothing here wants goes up the chain.
        if !completion.cancel() {
            _ = nextResponder?.tryToPerform(#selector(NSResponder.cancelOperation(_:)), with: sender)
        }
    }

    /// Option-Escape and F5: the standard "complete" command.
    override func complete(_ sender: Any?) {
        completion.explicitInvoke()
    }

    override func resignFirstResponder() -> Bool {
        completion.dismissAll()
        return super.resignFirstResponder()
    }

    override func mouseDown(with event: NSEvent) {
        completion.dismissAll()
        super.mouseDown(with: event)
    }

    override func viewWillMove(toWindow newWindow: NSWindow?) {
        completion.dismissAll()
        super.viewWillMove(toWindow: newWindow)
    }

    override func insertBacktab(_ sender: Any?) {
        guard let context = editingContext else { return super.insertBacktab(sender) }
        if let edit = PineIndentationEngine.outdent(in: context) { perform(edit, actionName: "Outdent") }
    }

    override func deleteBackward(_ sender: Any?) {
        if !hasMarkedText() { pendingCompletionCause = .deleted }
        defer { pendingCompletionCause = nil }
        guard let context = editingContext, let edit = PineEditorPairing.backspace(in: context) else {
            return super.deleteBackward(sender)
        }
        perform(edit)
    }

    override func paste(_ sender: Any?) {
        guard let context = editingContext, let text = NSPasteboard.general.string(forType: .string),
            let edit = PineIndentationEngine.reindentPaste(text, in: context)
        else { return super.paste(sender) }
        perform(edit, actionName: "Paste")
    }

    override func performKeyEquivalent(with event: NSEvent) -> Bool {
        let modifiers = event.modifierFlags.intersection(.deviceIndependentFlagsMask)
        guard window?.firstResponder === self,
            let key = event.charactersIgnoringModifiers?.lowercased()
        else { return super.performKeyEquivalent(with: event) }

        // Control-Space asks for completion (macOS may reserve it for input sources; Option-Escape and
        // F5 reach the same command through `complete(_:)`).
        if event.keyCode == 49, modifiers == [.control], !hasMarkedText() {
            completion.explicitInvoke()
            return true
        }

        if let command = PineEditorShortcut(keyCode: event.keyCode, key: key, modifiers: modifiers),
            let context = editingContext
        {
            if let edit = command.edit(in: context) { perform(edit, actionName: command.actionName) }
            return true
        }

        let action: NSTextFinder.Action?
        switch (key, modifiers) {
        case ("f", [.command]): action = .showFindInterface
        case ("g", [.command]): action = .nextMatch
        case ("g", [.command, .shift]): action = .previousMatch
        default: action = nil
        }
        guard let action else { return super.performKeyEquivalent(with: event) }
        // Like Xcode: cmd+F with a single-line selection searches for that text.
        let selection = selectedRange()
        if action == .showFindInterface, selection.length > 0,
            (string as NSString).substring(with: selection).rangeOfCharacter(from: .newlines) == nil
        {
            let useSelection = NSMenuItem()
            useSelection.tag = NSTextFinder.Action.setSearchString.rawValue
            performTextFinderAction(useSelection)
        }
        let sender = NSMenuItem()
        sender.tag = action.rawValue
        performTextFinderAction(sender)
        return true
    }

    override func performTextFinderAction(_ sender: Any?) {
        super.performTextFinderAction(sender)
        styleFindBarCloseButton()
    }

    /// Turns the find bar's "Done" button into a trailing X. AppKit offers no API for
    /// this, so the button is found by its (non-localized) action and keeps it, meaning
    /// clicking it still closes the bar the native way. If AppKit's layout changes,
    /// the lookup simply fails and the stock button stays.
    private func styleFindBarCloseButton() {
        guard let findBar = enclosingScrollView?.findBarView,
            let done = Self.buttons(in: findBar).first(where: {
                $0.action == NSSelectorFromString("_doneButton:")
            }),
            let stack = done.superview as? NSStackView,
            stack.arrangedSubviews.last !== done || done.image == nil
        else { return }
        done.title = ""
        done.image = NSImage(systemSymbolName: "xmark", accessibilityDescription: "Close")
        done.imagePosition = .imageOnly
        done.isBordered = false
        done.toolTip = "Close"
        stack.removeArrangedSubview(done)
        stack.addArrangedSubview(done)
    }

    private static func buttons(in view: NSView) -> [NSButton] {
        ((view as? NSButton).map { [$0] } ?? []) + view.subviews.flatMap(buttons(in:))
    }
}
