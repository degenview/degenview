import AppKit
import SwiftUI

extension Character {
    fileprivate var isWordCharacter: Bool { isLetter || isNumber || self == "_" }
}

struct LineNumberedTextEditorView: NSViewRepresentable {
    @Binding var text: String
    var diagnostics: [PineDiagnostic] = []

    func makeCoordinator() -> Coordinator { Coordinator(text: $text) }

    func makeNSView(context: Context) -> EditorContainerView {
        let container = EditorContainerView()
        container.textView.delegate = context.coordinator
        context.coordinator.gutter = container.gutter
        context.coordinator.container = container
        context.coordinator.diagnostics = diagnostics
        container.setText(text)
        container.setDiagnostics(diagnostics)
        return container
    }

    func updateNSView(_ container: EditorContainerView, context: Context) {
        if container.textView.string != text { container.setText(text) }
        context.coordinator.diagnostics = diagnostics
        container.setDiagnostics(diagnostics)
        container.gutter.needsDisplay = true
    }

    final class Coordinator: NSObject, NSTextViewDelegate {
        private var text: Binding<String>
        var diagnostics: [PineDiagnostic] = []
        weak var gutter: LineNumberGutterView?
        weak var container: EditorContainerView?
        private var lastEditWasWordCharacter: Bool?

        init(text: Binding<String>) { self.text = text }

        func textDidChange(_ notification: Notification) {
            guard let textView = notification.object as? NSTextView else { return }
            PineSyntaxHighlighter.apply(to: textView, diagnostics: diagnostics)
            text.wrappedValue = textView.string
            container?.updateOccurrenceHighlights()
            gutter?.needsDisplay = true
        }

        func textViewDidChangeSelection(_ notification: Notification) {
            container?.updateOccurrenceHighlights()
        }

        /// Groups typing into per-word undo steps, the way Xcode/Sublime/VS Code do,
        /// by closing the current undo group whenever an edit crosses a word boundary
        /// (word character <-> whitespace/punctuation) or is a multi-character paste.
        func textView(
            _ textView: NSTextView, shouldChangeTextIn affectedCharRange: NSRange,
            replacementString: String?
        ) -> Bool {
            defer {
                if let last = replacementString?.last {
                    lastEditWasWordCharacter = last.isWordCharacter
                } else {
                    lastEditWasWordCharacter = nil
                }
            }
            guard let replacementString else { return true }

            let isPaste = replacementString.count > 1
            let isWordCharacterEdit = replacementString.first?.isWordCharacter ?? false
            let crossedBoundary = lastEditWasWordCharacter != nil && lastEditWasWordCharacter != isWordCharacterEdit

            if isPaste || crossedBoundary {
                textView.breakUndoCoalescing()
            }
            return true
        }
    }

    final class EditorContainerView: NSView {
        let scrollView: NSScrollView
        let textView: NSTextView
        let gutter: LineNumberGutterView
        private var diagnostics: [PineDiagnostic] = []

        override var intrinsicContentSize: NSSize {
            NSSize(width: NSView.noIntrinsicMetric, height: NSView.noIntrinsicMetric)
        }

        override init(frame frameRect: NSRect) {
            // An explicit TextKit 1 stack: the gutter and occurrence highlighting rely on
            // the layout manager, and `scrollableTextView()` caps horizontal growth.
            let storage = NSTextStorage()
            let layoutManager = NSLayoutManager()
            storage.addLayoutManager(layoutManager)
            let container = NSTextContainer(
                size: NSSize(width: CGFloat.greatestFiniteMagnitude, height: CGFloat.greatestFiniteMagnitude)
            )
            layoutManager.addTextContainer(container)
            textView = PineTextView(frame: .zero, textContainer: container)
            scrollView = NSScrollView()
            scrollView.documentView = textView
            gutter = LineNumberGutterView()
            super.init(frame: frameRect)

            wantsLayer = true
            layer?.masksToBounds = true

            textView.font = .monospacedSystemFont(ofSize: NSFont.systemFontSize, weight: .regular)
            textView.textColor = .labelColor
            textView.backgroundColor = .textBackgroundColor
            textView.insertionPointColor = .labelColor
            textView.drawsBackground = true
            textView.isEditable = true
            textView.isSelectable = true
            textView.isRichText = false
            textView.allowsUndo = true
            textView.isAutomaticQuoteSubstitutionEnabled = false
            textView.isAutomaticDashSubstitutionEnabled = false
            textView.isAutomaticTextReplacementEnabled = false
            textView.textContainerInset = NSSize(width: 8, height: 8)
            textView.minSize = .zero
            textView.maxSize = NSSize(
                width: CGFloat.greatestFiniteMagnitude,
                height: CGFloat.greatestFiniteMagnitude
            )
            textView.autoresizingMask = [.width, .height]
            textView.isVerticallyResizable = true
            textView.isHorizontallyResizable = true
            textView.textContainer?.widthTracksTextView = false
            textView.usesFindBar = true
            textView.isIncrementalSearchingEnabled = true

            scrollView.hasVerticalScroller = true
            scrollView.hasHorizontalScroller = true
            scrollView.autohidesScrollers = true
            scrollView.borderType = .noBorder
            scrollView.drawsBackground = true
            scrollView.backgroundColor = .textBackgroundColor
            scrollView.findBarPosition = .aboveContent

            gutter.textView = textView
            gutter.translatesAutoresizingMaskIntoConstraints = false
            scrollView.translatesAutoresizingMaskIntoConstraints = false
            addSubview(gutter)
            addSubview(scrollView)
            NSLayoutConstraint.activate([
                gutter.leadingAnchor.constraint(equalTo: leadingAnchor),
                gutter.topAnchor.constraint(equalTo: topAnchor),
                gutter.bottomAnchor.constraint(equalTo: bottomAnchor),
                gutter.widthAnchor.constraint(equalToConstant: 42),
                scrollView.leadingAnchor.constraint(equalTo: gutter.trailingAnchor),
                scrollView.trailingAnchor.constraint(equalTo: trailingAnchor),
                scrollView.topAnchor.constraint(equalTo: topAnchor),
                scrollView.bottomAnchor.constraint(equalTo: bottomAnchor),
            ])

            scrollView.contentView.postsBoundsChangedNotifications = true
            NotificationCenter.default.addObserver(
                gutter,
                selector: #selector(LineNumberGutterView.editorDidScroll),
                name: NSView.boundsDidChangeNotification,
                object: scrollView.contentView
            )
            // Showing or hiding the find bar moves the content view without scrolling it.
            scrollView.contentView.postsFrameChangedNotifications = true
            NotificationCenter.default.addObserver(
                gutter,
                selector: #selector(LineNumberGutterView.editorDidScroll),
                name: NSView.frameDidChangeNotification,
                object: scrollView.contentView
            )
        }

        required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

        private var escapeMonitor: Any?

        override func viewDidMoveToWindow() {
            super.viewDidMoveToWindow()
            if let escapeMonitor { NSEvent.removeMonitor(escapeMonitor) }
            escapeMonitor = nil
            guard window != nil else { return }
            // SwiftUI windows and sheets consume Escape before the find bar's own
            // cancel handling sees it, which otherwise leaves no way to close the bar.
            escapeMonitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown) { [weak self] event in
                guard let self, event.keyCode == 53, self.closeFindBarIfFocused(in: event.window) else {
                    return event
                }
                return nil
            }
        }

        private func closeFindBarIfFocused(in eventWindow: NSWindow?) -> Bool {
            guard scrollView.isFindBarVisible, eventWindow === window,
                let responder = window?.firstResponder as? NSView,
                responder === textView || scrollView.findBarView.map({ responder.isDescendant(of: $0) }) == true
            else { return false }
            let sender = NSMenuItem()
            sender.tag = NSTextFinder.Action.hideFindInterface.rawValue
            textView.performTextFinderAction(sender)
            window?.makeFirstResponder(textView)
            return true
        }

        func setText(_ text: String) {
            let undoManager = textView.undoManager
            undoManager?.disableUndoRegistration()
            textView.textStorage?.setAttributedString(
                NSAttributedString(
                    string: text,
                    attributes: [
                        .font: textView.font
                            ?? NSFont.monospacedSystemFont(ofSize: NSFont.systemFontSize, weight: .regular),
                        .foregroundColor: NSColor.labelColor,
                    ]
                ))
            PineSyntaxHighlighter.apply(to: textView, diagnostics: diagnostics)
            undoManager?.enableUndoRegistration()
            // Replacing storage bypasses the typing path that normally resizes the view.
            textView.sizeToFit()
            updateOccurrenceHighlights()
            gutter.needsDisplay = true
        }

        func setDiagnostics(_ diagnostics: [PineDiagnostic]) {
            self.diagnostics = diagnostics
            PineSyntaxHighlighter.apply(to: textView, diagnostics: diagnostics)
        }

        /// Tints the word under the caret (or the selected word) and every other
        /// whole-word occurrence of it, so uses of a variable are easy to spot.
        func updateOccurrenceHighlights() {
            guard let layoutManager = textView.layoutManager else { return }
            let source = textView.string as NSString
            layoutManager.removeTemporaryAttribute(
                .backgroundColor, forCharacterRange: NSRange(location: 0, length: source.length))

            let selection = textView.selectedRange()
            let token: NSRange?
            if selection.length == 0 {
                token =
                    PineWordRange.range(at: selection.location, in: source)
                    ?? PineWordRange.range(at: selection.location - 1, in: source)
            } else if PineWordRange.range(at: selection.location, in: source) == selection {
                token = selection
            } else {
                token = nil
            }
            guard let token else { return }

            let word = source.substring(with: token)
            let tint = NSColor.selectedTextBackgroundColor
            for range in PineWordRange.occurrences(of: word, in: source) {
                layoutManager.addTemporaryAttribute(
                    .backgroundColor,
                    value: tint.withAlphaComponent(range == token ? 0.7 : 0.45),
                    forCharacterRange: range
                )
            }
        }
    }

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

    final class LineNumberGutterView: NSView {
        weak var textView: NSTextView?
        private let attributes: [NSAttributedString.Key: Any] = [
            .font: NSFont.monospacedDigitSystemFont(ofSize: NSFont.smallSystemFontSize, weight: .regular),
            .foregroundColor: NSColor.secondaryLabelColor,
        ]

        override var isFlipped: Bool { true }

        @objc func editorDidScroll() { needsDisplay = true }

        override func draw(_ dirtyRect: NSRect) {
            NSColor.windowBackgroundColor.setFill()
            dirtyRect.fill()
            guard let textView,
                let layoutManager = textView.layoutManager,
                let textContainer = textView.textContainer,
                let clipView = textView.enclosingScrollView?.contentView
            else { return }
            // Line numbers must not draw beside the find bar when it is shown.
            // Only the vertical extent matters: the clip view sits to the right of the gutter.
            let contentArea = convert(clipView.bounds, from: clipView)
            NSRect(x: bounds.minX, y: contentArea.minY, width: bounds.width, height: contentArea.height).clip()

            let visibleRect = textView.visibleRect
            let glyphRange = layoutManager.glyphRange(forBoundingRect: visibleRect, in: textContainer)
            let source = textView.string as NSString
            var nextLineStart = 0
            var lineNumber = 1

            layoutManager.enumerateLineFragments(forGlyphRange: glyphRange) { _, usedRect, _, lineGlyphRange, _ in
                let characterRange = layoutManager.characterRange(
                    forGlyphRange: lineGlyphRange,
                    actualGlyphRange: nil
                )
                let characterIndex = min(characterRange.location, source.length)

                while nextLineStart < characterIndex {
                    let lineRange = source.lineRange(for: NSRange(location: nextLineStart, length: 0))
                    guard NSMaxRange(lineRange) > nextLineStart else { break }
                    nextLineStart = NSMaxRange(lineRange)
                    lineNumber += 1
                }

                let label = "\(lineNumber)" as NSString
                let size = label.size(withAttributes: self.attributes)
                let y = self.convert(
                    NSPoint(x: 0, y: usedRect.minY + textView.textContainerOrigin.y), from: textView
                ).y
                label.draw(
                    at: NSPoint(x: self.bounds.width - size.width - 8, y: y),
                    withAttributes: self.attributes
                )
            }
        }
    }
}

/// Word boundaries for code: letters, digits and `_`, so dots split `table.cell`.
enum PineWordRange {
    /// The word containing the UTF-16 `index`, or `nil` when it is not on a word character.
    static func range(at index: Int, in source: NSString) -> NSRange? {
        guard index >= 0, index < source.length, isWordCharacter(at: index, in: source) else { return nil }
        var start = index
        var end = index + 1
        while start > 0, isWordCharacter(at: start - 1, in: source) { start -= 1 }
        while end < source.length, isWordCharacter(at: end, in: source) { end += 1 }
        return NSRange(location: start, length: end - start)
    }

    /// Every whole-word occurrence of `word`, so `len` does not match inside `length`.
    static func occurrences(of word: String, in source: NSString) -> [NSRange] {
        guard !word.isEmpty else { return [] }
        var result: [NSRange] = []
        var searchRange = NSRange(location: 0, length: source.length)
        while true {
            let found = source.range(of: word, options: .literal, range: searchRange)
            guard found.location != NSNotFound else { break }
            if range(at: found.location, in: source) == found { result.append(found) }
            let next = NSMaxRange(found)
            searchRange = NSRange(location: next, length: source.length - next)
        }
        return result
    }

    private static func isWordCharacter(at index: Int, in source: NSString) -> Bool {
        let composed = source.rangeOfComposedCharacterSequence(at: index)
        return Character(source.substring(with: composed)).isWordCharacter
    }
}

/// Lightweight, editor-only highlighting. The compiler remains the authority on whether
/// source is valid; this deliberately also colors incomplete tokens while they are typed.
private enum PineSyntaxHighlighter {
    private struct Rule {
        let expression: NSRegularExpression
        let color: NSColor

        init(_ pattern: String, color: NSColor) {
            expression = try! NSRegularExpression(pattern: pattern)
            self.color = color
        }
    }

    // Rules are ordered from general to specific. Later matches win, except that strings
    // and comments are applied last so text inside them never receives token coloring.
    private static let tokenRules = [
        Rule(#"\b(?:\d+(?:\.\d*)?|\.\d+)(?:[eE][+-]?\d+)?\b"#, color: .systemOrange),
        Rule(
            #"\b(?:indicator|strategy|library|plot|plotshape|plotchar|hline|bgcolor|barcolor|input|color|ta)\b"#,
            color: .systemTeal
        ),
        Rule(
            #"\b(?:and|or|not|if|else|var|varip|int|float|bool|string|true|false|na)\b"#,
            color: .systemPurple
        ),
        Rule(#"#[0-9A-Fa-f]{6}(?:[0-9A-Fa-f]{2})?\b"#, color: .systemPink),
    ]
    // One expression is important here: alternation makes a whole string win before a
    // `//` inside it can look like a comment, and a whole comment wins before quoted text
    // inside the comment can look like a string.
    private static let protectedExpression = try! NSRegularExpression(
        pattern: #"(\"(?:\\.|[^\"\\])*\"?)|(//[^\n]*)"#
    )

    static func apply(to textView: NSTextView, diagnostics: [PineDiagnostic] = []) {
        guard let storage = textView.textStorage else { return }
        let range = NSRange(location: 0, length: storage.length)
        let source = storage.string
        let font =
            textView.font
            ?? NSFont.monospacedSystemFont(ofSize: NSFont.systemFontSize, weight: .regular)

        // Attribute-only edits made here happen outside the normal typing edit path;
        // left alone they reset NSTextView's typing-coalescing on every keystroke and
        // turn cmd-Z into per-character undo instead of per-word.
        let undoManager = textView.undoManager
        undoManager?.disableUndoRegistration()
        defer { undoManager?.enableUndoRegistration() }

        storage.beginEditing()
        storage.setAttributes([.font: font, .foregroundColor: NSColor.labelColor], range: range)
        for rule in tokenRules {
            rule.expression.enumerateMatches(in: source, range: range) { match, _, _ in
                guard let match else { return }
                storage.addAttribute(.foregroundColor, value: rule.color, range: match.range)
            }
        }
        protectedExpression.enumerateMatches(in: source, range: range) { match, _, _ in
            guard let match else { return }
            let isString = match.range(at: 1).location != NSNotFound
            storage.addAttribute(
                .foregroundColor,
                value: isString ? NSColor.systemRed : NSColor.secondaryLabelColor,
                range: match.range
            )
        }
        // Messages surface as a native hover tooltip on the underlined text. Diagnostics
        // sharing a range are merged, since a later `.toolTip` would replace an earlier one.
        var messages: [NSRange: [String]] = [:]
        var order: [NSRange] = []
        for diagnostic in diagnostics {
            guard
                let diagnosticRange = PineDiagnosticRangeMapper.nsRange(
                    for: diagnostic.range, in: source
                )
            else { continue }
            storage.addAttributes(
                [
                    .underlineStyle: NSUnderlineStyle.single.rawValue,
                    .underlineColor: diagnostic.severity == .error ? NSColor.systemRed : NSColor.systemOrange,
                ], range: diagnosticRange)
            if messages[diagnosticRange] == nil { order.append(diagnosticRange) }
            messages[diagnosticRange, default: []].append(diagnostic.message)
        }
        for range in order {
            storage.addAttribute(.toolTip, value: messages[range]!.joined(separator: "\n"), range: range)
        }
        storage.endEditing()
        textView.typingAttributes = [.font: font, .foregroundColor: NSColor.labelColor]
    }
}

enum PineDiagnosticRangeMapper {
    static func nsRange(for range: PineSourceRange, in source: String) -> NSRange? {
        let sourceLength = source.utf16.count
        guard sourceLength > 0 else { return nil }
        let reportedStart = sourceOffset(for: range.start, in: source)
        let reportedEnd = sourceOffset(for: range.end, in: source)
        let start = reportedStart == sourceLength ? sourceLength - 1 : reportedStart
        return NSRange(
            location: start,
            length: min(max(1, reportedEnd - reportedStart), sourceLength - start)
        )
    }

    /// Compiler offsets refer to its line-ending-normalized source. Mapping through line
    /// and column keeps editor ranges correct for CRLF text and other line separators.
    private static func sourceOffset(for position: PineSourcePosition, in source: String) -> Int {
        let nsSource = source as NSString
        var lineStart = 0
        for _ in 1..<max(1, position.line) {
            guard lineStart < nsSource.length else { return nsSource.length }
            let lineRange = nsSource.lineRange(for: NSRange(location: lineStart, length: 0))
            let nextLineStart = NSMaxRange(lineRange)
            guard nextLineStart > lineStart else { return lineStart }
            lineStart = nextLineStart
        }

        guard lineStart < nsSource.length else { return nsSource.length }
        let fullLineRange = nsSource.lineRange(for: NSRange(location: lineStart, length: 0))
        let fullLine = nsSource.substring(with: fullLineRange)
        let line = fullLine.prefix(while: { !$0.isNewline })
        let characterOffset = min(max(0, position.column - 1), line.count)
        let index = line.index(line.startIndex, offsetBy: characterOffset)
        return min(nsSource.length, lineStart + line[..<index].utf16.count)
    }
}
