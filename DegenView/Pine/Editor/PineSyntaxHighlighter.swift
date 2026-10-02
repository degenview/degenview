import AppKit

/// Lightweight, editor-only highlighting. The compiler remains the authority on whether
/// source is valid; this deliberately also colors incomplete tokens while they are typed.
/// Classification lives in `PineSyntaxClassifier` and colors in `PineSyntaxTheme`.
enum PineSyntaxHighlighter {
    /// The editor re-applies highlighting for the same text when diagnostics arrive, so the last
    /// classification is kept. Only used from the main thread.
    private static var cache: (source: String, spans: [PineHighlightSpan])?

    private static func spans(for source: String) -> [PineHighlightSpan] {
        if let cache, cache.source == source { return cache.spans }
        let spans = PineSyntaxClassifier.classify(source)
        cache = (source, spans)
        return spans
    }

    static func apply(
        to textView: NSTextView, diagnostics: [PineDiagnostic] = [], theme: PineSyntaxTheme = .standard
    ) {
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
        for span in spans(for: source) {
            guard NSMaxRange(span.range) <= range.length,
                let color = theme.color(for: span.category)
            else { continue }
            storage.addAttribute(.foregroundColor, value: color, range: span.range)
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
                    .underlineColor: diagnostic.severity.nsColor,
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
