import AppKit

/// Lightweight, editor-only highlighting. The compiler remains the authority on whether
/// source is valid; this deliberately also colors incomplete tokens while they are typed.
enum PineSyntaxHighlighter {
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
            #"\b(?:indicator|strategy|library|plot|plotshape|plotchar|hline|bgcolor|barcolor|alert|alertcondition|input|color|ta)\b"#,
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
