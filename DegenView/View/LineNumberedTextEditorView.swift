import AppKit
import SwiftUI

struct LineNumberedTextEditorView: NSViewRepresentable {
    @Binding var text: String
    var diagnostics: [PineDiagnostic] = []

    func makeCoordinator() -> Coordinator { Coordinator(text: $text) }

    func makeNSView(context: Context) -> EditorContainerView {
        let container = EditorContainerView()
        container.textView.delegate = context.coordinator
        context.coordinator.gutter = container.gutter
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

        init(text: Binding<String>) { self.text = text }

        func textDidChange(_ notification: Notification) {
            guard let textView = notification.object as? NSTextView else { return }
            PineSyntaxHighlighter.apply(to: textView, diagnostics: diagnostics)
            text.wrappedValue = textView.string
            gutter?.needsDisplay = true
        }
    }

    final class EditorContainerView: NSView {
        let scrollView: NSScrollView
        let textView: NSTextView
        let gutter: LineNumberGutterView
        let diagnosticOverlay: InlineDiagnosticView

        override var intrinsicContentSize: NSSize {
            NSSize(width: NSView.noIntrinsicMetric, height: NSView.noIntrinsicMetric)
        }

        override init(frame frameRect: NSRect) {
            scrollView = NSTextView.scrollableTextView()
            textView = scrollView.documentView as! NSTextView
            gutter = LineNumberGutterView()
            diagnosticOverlay = InlineDiagnosticView()
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
            textView.isAutomaticQuoteSubstitutionEnabled = false
            textView.isAutomaticDashSubstitutionEnabled = false
            textView.isAutomaticTextReplacementEnabled = false
            textView.textContainerInset = NSSize(width: 8, height: 8)
            textView.isVerticallyResizable = true
            textView.isHorizontallyResizable = true
            textView.textContainer?.widthTracksTextView = false
            textView.textContainer?.containerSize = NSSize(
                width: CGFloat.greatestFiniteMagnitude,
                height: CGFloat.greatestFiniteMagnitude
            )

            scrollView.hasVerticalScroller = true
            scrollView.hasHorizontalScroller = true
            scrollView.autohidesScrollers = true
            scrollView.borderType = .noBorder

            gutter.textView = textView
            diagnosticOverlay.textView = textView
            diagnosticOverlay.frame = textView.bounds
            diagnosticOverlay.autoresizingMask = [.width, .height]
            textView.addSubview(diagnosticOverlay)
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
                scrollView.bottomAnchor.constraint(equalTo: bottomAnchor)
            ])

            scrollView.contentView.postsBoundsChangedNotifications = true
            NotificationCenter.default.addObserver(
                gutter,
                selector: #selector(LineNumberGutterView.editorDidScroll),
                name: NSView.boundsDidChangeNotification,
                object: scrollView.contentView
            )
            NotificationCenter.default.addObserver(
                diagnosticOverlay,
                selector: #selector(InlineDiagnosticView.editorDidScroll),
                name: NSView.boundsDidChangeNotification,
                object: scrollView.contentView
            )
        }

        required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

        func setText(_ text: String) {
            textView.textStorage?.setAttributedString(NSAttributedString(
                string: text,
                attributes: [
                    .font: textView.font ?? NSFont.monospacedSystemFont(ofSize: NSFont.systemFontSize, weight: .regular),
                    .foregroundColor: NSColor.labelColor
                ]
            ))
            PineSyntaxHighlighter.apply(to: textView, diagnostics: diagnosticOverlay.diagnostics)
            gutter.needsDisplay = true
        }

        func setDiagnostics(_ diagnostics: [PineDiagnostic]) {
            diagnosticOverlay.diagnostics = diagnostics
            PineSyntaxHighlighter.apply(to: textView, diagnostics: diagnostics)
        }
    }

    final class LineNumberGutterView: NSView {
        weak var textView: NSTextView?
        private let attributes: [NSAttributedString.Key: Any] = [
            .font: NSFont.monospacedDigitSystemFont(ofSize: NSFont.smallSystemFontSize, weight: .regular),
            .foregroundColor: NSColor.secondaryLabelColor
        ]

        override var isFlipped: Bool { true }

        @objc func editorDidScroll() { needsDisplay = true }

        override func draw(_ dirtyRect: NSRect) {
            NSColor.windowBackgroundColor.setFill()
            dirtyRect.fill()
            guard let textView,
                  let layoutManager = textView.layoutManager,
                  let textContainer = textView.textContainer else { return }

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
                let y = usedRect.minY + textView.textContainerOrigin.y - visibleRect.minY
                label.draw(
                    at: NSPoint(x: self.bounds.width - size.width - 8, y: y),
                    withAttributes: self.attributes
                )
            }
        }
    }

    final class InlineDiagnosticView: NSView {
        weak var textView: NSTextView?
        var diagnostics: [PineDiagnostic] = [] { didSet { needsDisplay = true } }

        override var isFlipped: Bool { true }
        override func hitTest(_ point: NSPoint) -> NSView? { nil }
        @objc func editorDidScroll() { needsDisplay = true }

        override func draw(_ dirtyRect: NSRect) {
            guard let textView, let layoutManager = textView.layoutManager,
                  let textContainer = textView.textContainer else { return }
            let grouped = Dictionary(grouping: diagnostics, by: { $0.range.start.line })
            let source = textView.string as NSString

            for (line, items) in grouped where line > 0 {
                var lineStart = 0
                for _ in 1..<line {
                    let range = source.lineRange(for: NSRange(location: lineStart, length: 0))
                    lineStart = NSMaxRange(range)
                    if lineStart >= source.length { break }
                }
                guard lineStart <= source.length else { continue }
                let lineRange = source.lineRange(for: NSRange(location: lineStart, length: 0))
                let glyphRange = layoutManager.glyphRange(forCharacterRange: lineRange, actualCharacterRange: nil)
                let used = layoutManager.boundingRect(forGlyphRange: glyphRange, in: textContainer)
                let isError = items.contains { $0.severity == .error }
                let message = items.map(\.message).joined(separator: " • ") as NSString
                let attributes: [NSAttributedString.Key: Any] = [
                    .font: NSFont.systemFont(ofSize: NSFont.smallSystemFontSize),
                    .foregroundColor: isError ? NSColor.systemRed : NSColor.systemOrange,
                ]
                let visible = textView.visibleRect
                let naturalWidth = message.size(withAttributes: attributes).width + 12
                let width = min(naturalWidth, max(120, visible.width * 0.6))
                let preferredX = used.maxX + textView.textContainerOrigin.x + 14
                let x = max(visible.minX + 8, min(preferredX, visible.maxX - width - 8))
                let y = used.minY + textView.textContainerOrigin.y
                let background = NSRect(x: x - 5, y: y - 1, width: width + 10, height: used.height + 2)
                (isError ? NSColor.systemRed : NSColor.systemOrange).withAlphaComponent(0.12).setFill()
                NSBezierPath(roundedRect: background, xRadius: 4, yRadius: 4).fill()
                let paragraph = NSMutableParagraphStyle()
                paragraph.lineBreakMode = .byTruncatingTail
                var drawingAttributes = attributes
                drawingAttributes[.paragraphStyle] = paragraph
                message.draw(
                    in: NSRect(x: x, y: y, width: width, height: used.height),
                    withAttributes: drawingAttributes
                )
            }
        }
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
        let font = textView.font
            ?? NSFont.monospacedSystemFont(ofSize: NSFont.systemFontSize, weight: .regular)

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
        for diagnostic in diagnostics {
            guard let diagnosticRange = PineDiagnosticRangeMapper.nsRange(
                for: diagnostic.range, in: source
            ) else { continue }
            storage.addAttributes([
                .underlineStyle: NSUnderlineStyle.single.rawValue,
                .underlineColor: diagnostic.severity == .error ? NSColor.systemRed : NSColor.systemOrange,
            ], range: diagnosticRange)
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
