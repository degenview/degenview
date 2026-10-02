import AppKit

/// Draws the editor's under-text decorations, the current-line band and the indentation guides,
/// in the layout manager's background pass. That is the TextKit hook that runs after the view
/// fills its background and before selection, temporary highlights and glyphs, so decorations sit
/// below all of them and need no views of their own.
final class PineLayoutManager: NSLayoutManager {
    override func drawBackground(forGlyphRange glyphsToShow: NSRange, at origin: NSPoint) {
        if let textView = firstTextView {
            let clip = NSGraphicsContext.current?.cgContext.boundingBoxOfClipPath ?? textView.visibleRect
            PineCurrentLineRenderer.draw(in: textView, rect: clip)
            PineIndentGuideRenderer.draw(in: textView, rect: clip)
        }
        super.drawBackground(forGlyphRange: glyphsToShow, at: origin)
    }
}
