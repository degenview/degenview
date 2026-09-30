import AppKit

final class PineTextView: NSTextView {
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

    override func performKeyEquivalent(with event: NSEvent) -> Bool {
        let modifiers = event.modifierFlags.intersection(.deviceIndependentFlagsMask)
        guard window?.firstResponder === self,
            let key = event.charactersIgnoringModifiers?.lowercased()
        else { return super.performKeyEquivalent(with: event) }

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
