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
