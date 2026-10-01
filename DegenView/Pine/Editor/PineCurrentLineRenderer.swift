import AppKit

/// The subtle band behind the caret's line. Drawn in the background pass, not stored as a text
/// attribute, so it never touches the source, survives re-highlighting and sits under both
/// selection and diagnostics.
enum PineCurrentLineRenderer {
    static func draw(in textView: NSTextView, rect: NSRect) {
        guard textView.selectedRanges.count == 1, textView.selectedRange().length == 0,
            let layoutManager = textView.layoutManager, let storage = textView.textStorage
        else { return }
        let caret = textView.selectedRange().location
        var line: NSRect
        if storage.length == 0 || (caret >= storage.length && layoutManager.extraLineFragmentRect.height > 0) {
            line = layoutManager.extraLineFragmentRect
        } else {
            let glyph = layoutManager.glyphIndexForCharacter(at: min(caret, storage.length - 1))
            line = layoutManager.lineFragmentRect(forGlyphAt: glyph, effectiveRange: nil)
        }
        guard line.height > 0 else { return }
        let band = NSRect(
            x: 0, y: line.minY + textView.textContainerOrigin.y, width: textView.bounds.width,
            height: line.height)
        guard band.intersects(rect) else { return }
        NSColor.labelColor.withAlphaComponent(0.06).setFill()
        band.fill(using: .sourceOver)
    }
}
