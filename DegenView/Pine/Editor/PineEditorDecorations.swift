import AppKit

/// Visual-only highlights: the matching bracket and other occurrences of the identifier under
/// the caret. They are temporary attributes on the layout manager, so the source text, its
/// syntax colours and the diagnostics underlines are never touched, and they survive
/// re-highlighting. Both live here because they share one attribute (`.backgroundColor`), and
/// clearing it for one would otherwise wipe the other.
enum PineEditorDecorations {
    /// Occurrences are painted this far beyond the visible text, so small scrolls need no redo.
    private static let margin = 2000

    static func update(in textView: NSTextView) {
        guard let layoutManager = textView.layoutManager, let storage = textView.textStorage else { return }
        let whole = NSRange(location: 0, length: storage.length)
        layoutManager.removeTemporaryAttribute(.backgroundColor, forCharacterRange: whole)
        guard storage.length > 0, !textView.hasMarkedText(), textView.selectedRanges.count == 1 else { return }

        let context = PineEditorContext(source: storage.string, selection: textView.selectedRange())
        paintOccurrences(in: context, textView: textView, layoutManager: layoutManager)
        paintBracketMatch(in: context, layoutManager: layoutManager)
    }

    // MARK: - Occurrences

    /// Whole-token occurrences of the identifier the caret touches or the selection covers.
    /// Only identifier tokens count, so text in strings and comments, and substrings of longer
    /// names, are never highlighted. `ta.len` (a member) and `len` stay distinct. This is textual,
    /// not a reference search: two same-named variables in different scopes both light up.
    private static func paintOccurrences(
        in context: PineEditorContext, textView: NSTextView, layoutManager: NSLayoutManager
    ) {
        guard let target = target(in: context),
            case .identifier(let name) = target.kind
        else { return }
        let isMember = isMemberAccess(target, in: context)

        let visible = visibleCharacterRange(of: textView)
        let window = NSRange(
            location: max(0, visible.location - margin),
            length: min(context.length, NSMaxRange(visible) + margin) - max(0, visible.location - margin))
        let tint = NSColor.selectedTextBackgroundColor
        for item in context.snapshot.items(in: window) {
            guard case .identifier(let other) = item.kind, other == name,
                isMemberAccess(item, in: context) == isMember
            else { continue }
            layoutManager.addTemporaryAttribute(
                .backgroundColor, value: tint.withAlphaComponent(item.range == target.range ? 0.7 : 0.45),
                forCharacterRange: item.range)
        }
    }

    /// The identifier token under a collapsed caret (touching either side), or exactly selected.
    private static func target(in context: PineEditorContext) -> PineLexicalSnapshot.Item? {
        let selection = context.selection
        if selection.length == 0 {
            for offset in [selection.location, selection.location - 1] {
                if let item = context.token(atUTF16Offset: offset), case .identifier = item.kind {
                    return item
                }
            }
            return nil
        }
        guard let item = context.token(atUTF16Offset: selection.location), case .identifier = item.kind,
            item.range == selection
        else { return nil }
        return item
    }

    private static func isMemberAccess(_ item: PineLexicalSnapshot.Item, in context: PineEditorContext) -> Bool {
        context.unit(at: item.range.location - 1) == 0x2E
    }

    private static func visibleCharacterRange(of textView: NSTextView) -> NSRange {
        guard let layoutManager = textView.layoutManager, let container = textView.textContainer else {
            return NSRange(location: 0, length: 0)
        }
        let origin = textView.textContainerOrigin
        let glyphs = layoutManager.glyphRange(
            forBoundingRect: textView.visibleRect.offsetBy(dx: -origin.x, dy: -origin.y), in: container)
        return layoutManager.characterRange(forGlyphRange: glyphs, actualGlyphRange: nil)
    }

    // MARK: - Brackets

    private static func paintBracketMatch(in context: PineEditorContext, layoutManager: NSLayoutManager) {
        guard context.selection.length == 0,
            let match = PineDelimiterMatcher.match(at: context.caret, in: context.snapshot)
        else { return }
        // An unmatched delimiter gets a faint error tint; the parser's own diagnostic stays the
        // authority, so this only marks the one the caret is touching.
        let color =
            match.isUnmatched
            ? NSColor.systemRed.withAlphaComponent(0.3) : NSColor.labelColor.withAlphaComponent(0.22)
        for range in [match.delimiter, match.partner].compactMap({ $0 }) {
            layoutManager.addTemporaryAttribute(.backgroundColor, value: color, forCharacterRange: range)
        }
    }
}
