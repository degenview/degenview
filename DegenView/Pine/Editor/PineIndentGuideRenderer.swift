import AppKit

/// Draws indentation guides straight into the text view's background pass.
///
/// Drawing there, instead of adding views, keeps guides out of hit-testing (they cannot steal
/// clicks or disturb selection), scroll with the text for free, and lets them be placed from
/// the layout manager's own line fragments and glyph positions, so they stay aligned with the
/// text whatever the font, line height or tab stops. Only the lines in the dirty rect are read.
enum PineIndentGuideRenderer {
    static func draw(in textView: NSTextView, rect: NSRect) {
        guard let layoutManager = textView.layoutManager, let container = textView.textContainer,
            let storage = textView.textStorage, storage.length > 0
        else { return }
        let source = storage.mutableString
        let origin = textView.textContainerOrigin
        let glyphRange = layoutManager.glyphRange(
            forBoundingRect: rect.offsetBy(dx: -origin.x, dy: -origin.y), in: container)
        let charRange = layoutManager.characterRange(forGlyphRange: glyphRange, actualGlyphRange: nil)
        let lines = PineIndentGuides.lines(in: source, covering: charRange)

        let caretLine = lines.firstIndex { line in
            let caret = textView.selectedRange().location
            return caret >= line.range.location && caret <= NSMaxRange(line.range)
        }
        let active = caretLine.flatMap { PineIndentGuides.activeGuide(in: lines, caretLine: $0) }

        let font = textView.font ?? .monospacedSystemFont(ofSize: NSFont.systemFontSize, weight: .regular)
        let spaceWidth = " ".size(withAttributes: [.font: font]).width
        let padding = container.lineFragmentPadding

        for (index, line) in lines.enumerated()
        where !line.guideColumns.isEmpty && line.range.location < source.length {
            let glyph = layoutManager.glyphIndexForCharacter(at: line.range.location)
            let fragment = layoutManager.lineFragmentRect(forGlyphAt: glyph, effectiveRange: nil)
            let top = fragment.minY + origin.y
            guard top <= rect.maxY, fragment.maxY + origin.y >= rect.minY else { continue }

            for column in line.guideColumns {
                let x =
                    glyphX(
                        ofColumn: column, in: line, fragment: fragment, source: source,
                        layoutManager: layoutManager)
                    ?? (padding + CGFloat(column) * spaceWidth)
                let isActive = active.map { $0.column == column && $0.lines.contains(index) } ?? false
                (isActive ? NSColor.secondaryLabelColor : NSColor.quaternaryLabelColor).setFill()
                NSRect(x: (x + origin.x).rounded(.down), y: top, width: 1, height: fragment.height)
                    .fill(using: .sourceOver)
            }
        }
    }

    /// The x of the glyph at indentation `column`, for lines whose own leading whitespace
    /// reaches that far; `nil` for blank lines, whose guides are placed by width.
    private static func glyphX(
        ofColumn target: Int, in line: PineIndentGuides.Line, fragment: NSRect, source: NSString,
        layoutManager: NSLayoutManager
    ) -> CGFloat? {
        guard !line.isBlank else { return nil }
        var columns = 0
        var index = line.range.location
        while index < NSMaxRange(line.range), columns < target {
            switch source.character(at: index) {
            case 0x20: columns += 1
            case 0x09: columns += PineIndentGuides.width
            default: return nil
            }
            index += 1
        }
        guard columns == target, index < source.length else { return nil }
        let glyph = layoutManager.glyphIndexForCharacter(at: index)
        return fragment.minX + layoutManager.location(forGlyphAt: glyph).x
    }
}
