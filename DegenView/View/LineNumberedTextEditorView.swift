import AppKit
import SwiftUI

struct LineNumberedTextEditorView: NSViewRepresentable {
    @Binding var text: String

    func makeCoordinator() -> Coordinator { Coordinator(text: $text) }

    func makeNSView(context: Context) -> EditorContainerView {
        let container = EditorContainerView()
        container.textView.delegate = context.coordinator
        context.coordinator.gutter = container.gutter
        container.setText(text)
        return container
    }

    func updateNSView(_ container: EditorContainerView, context: Context) {
        if container.textView.string != text { container.setText(text) }
        container.gutter.needsDisplay = true
    }

    final class Coordinator: NSObject, NSTextViewDelegate {
        private var text: Binding<String>
        weak var gutter: LineNumberGutterView?

        init(text: Binding<String>) { self.text = text }

        func textDidChange(_ notification: Notification) {
            guard let textView = notification.object as? NSTextView else { return }
            PineSyntaxHighlighter.apply(to: textView)
            text.wrappedValue = textView.string
            gutter?.needsDisplay = true
        }
    }

    final class EditorContainerView: NSView {
        let scrollView: NSScrollView
        let textView: NSTextView
        let gutter: LineNumberGutterView

        override var intrinsicContentSize: NSSize {
            NSSize(width: NSView.noIntrinsicMetric, height: NSView.noIntrinsicMetric)
        }

        override init(frame frameRect: NSRect) {
            scrollView = NSTextView.scrollableTextView()
            textView = scrollView.documentView as! NSTextView
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
            PineSyntaxHighlighter.apply(to: textView)
            gutter.needsDisplay = true
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

    static func apply(to textView: NSTextView) {
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
        storage.endEditing()
        textView.typingAttributes = [.font: font, .foregroundColor: NSColor.labelColor]
    }
}
