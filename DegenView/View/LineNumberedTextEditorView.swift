import AppKit
import SwiftUI

struct LineNumberedTextEditorView: NSViewRepresentable {
    @Binding var text: String
    var diagnostics: [PineDiagnostic] = []
    /// A position to scroll to and flash; each new request (by id) is handled once.
    var reveal: ScriptEditorViewModel.Reveal?

    func makeCoordinator() -> Coordinator { Coordinator(text: $text) }

    func makeNSView(context: Context) -> EditorContainerView {
        let container = EditorContainerView()
        container.textView.delegate = context.coordinator
        context.coordinator.gutter = container.gutter
        context.coordinator.container = container
        context.coordinator.diagnostics = diagnostics
        // A request made before this editor existed is stale.
        context.coordinator.handledReveal = reveal?.id
        container.setText(text)
        container.setDiagnostics(diagnostics)
        return container
    }

    func updateNSView(_ container: EditorContainerView, context: Context) {
        if container.textView.string != text { container.setText(text) }
        context.coordinator.diagnostics = diagnostics
        container.setDiagnostics(diagnostics)
        container.gutter.needsDisplay = true
        if let reveal, reveal.id != context.coordinator.handledReveal {
            context.coordinator.handledReveal = reveal.id
            DispatchQueue.main.async { container.reveal(line: reveal.line, column: reveal.column) }
        }
    }

    final class Coordinator: NSObject, NSTextViewDelegate {
        private var text: Binding<String>
        var diagnostics: [PineDiagnostic] = []
        weak var gutter: LineNumberGutterView?
        weak var container: EditorContainerView?
        private var lastEditWasWordCharacter: Bool?
        var handledReveal: UUID?

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
            let layoutManager = PineLayoutManager()
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
            NotificationCenter.default.addObserver(
                self,
                selector: #selector(editorDidScroll),
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
                guard let self, event.keyCode == 53 else { return event }
                // Escape closes the completion list or signature help before anything else.
                if self.cancelCompletionIfFocused(in: event.window) || self.closeFindBarIfFocused(in: event.window) {
                    return nil
                }
                return event
            }
        }

        private func cancelCompletionIfFocused(in eventWindow: NSWindow?) -> Bool {
            guard eventWindow === window, window?.firstResponder === textView,
                let editor = textView as? PineTextView
            else { return false }
            return editor.completion.cancel()
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

        /// Scrolls to a 1-based line and column, puts the caret there and flashes the line. The
        /// current-line band then keeps marking it until the caret moves.
        func reveal(line: Int, column: Int) {
            let source = textView.string as NSString
            var start = 0
            for _ in 1..<max(line, 1) {
                let next = NSMaxRange(source.lineRange(for: NSRange(location: start, length: 0)))
                if next >= source.length { break }
                start = next
            }
            let lineRange = source.lineRange(for: NSRange(location: start, length: 0))
            var content = lineRange
            while content.length > 0, source.character(at: NSMaxRange(content) - 1) == 0x0A { content.length -= 1 }
            let caret = start + min(max(column - 1, 0), content.length)
            window?.makeFirstResponder(textView)
            textView.setSelectedRange(NSRange(location: caret, length: 0))
            textView.scrollRangeToVisible(lineRange)
            textView.showFindIndicator(for: content)
        }

        func setDiagnostics(_ diagnostics: [PineDiagnostic]) {
            self.diagnostics = diagnostics
            PineSyntaxHighlighter.apply(to: textView, diagnostics: diagnostics)
        }

        /// Refreshes the decorations that follow the caret: the matching bracket and other
        /// occurrences of the identifier at it (`PineEditorDecorations`), the current-line band
        /// and the active indentation guide (drawn by `PineTextView`).
        func updateOccurrenceHighlights() {
            PineEditorDecorations.update(in: textView)
            textView.setNeedsDisplay(textView.visibleRect)
        }

        /// Scrolling exposes text whose occurrences were not painted yet.
        @objc private func editorDidScroll() { updateOccurrenceHighlights() }
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
