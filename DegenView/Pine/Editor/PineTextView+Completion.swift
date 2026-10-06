import AppKit

extension PineTextView {
    /// The caret line's rectangle at a character offset, in screen coordinates: what the
    /// completion list and signature help anchor to. Nil without a window or a layout.
    func screenRect(forCharacterOffset offset: Int) -> NSRect? {
        guard let window else { return nil }
        let length = (string as NSString).length
        let location = min(max(0, offset), length)
        var actual = NSRange()
        let reported = firstRect(forCharacterRange: NSRange(location: location, length: 0), actualRange: &actual)
        if reported.height > 0 { return reported }

        // `firstRect` has nothing for an empty last line; ask the layout manager directly.
        guard let layoutManager, let textContainer else { return nil }
        layoutManager.ensureLayout(for: textContainer)
        var rect = layoutManager.extraLineFragmentRect
        if rect.isEmpty, layoutManager.numberOfGlyphs > 0 {
            let last = layoutManager.numberOfGlyphs - 1
            let glyph = min(layoutManager.glyphIndexForCharacter(at: max(0, location - 1)), last)
            rect = layoutManager.lineFragmentUsedRect(forGlyphAt: glyph, effectiveRange: nil)
        }
        guard !rect.isEmpty else { return nil }
        rect.origin.x += textContainerOrigin.x
        rect.origin.y += textContainerOrigin.y
        return window.convertToScreen(convert(rect, to: nil))
    }
}
