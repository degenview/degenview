import Foundation

/// Whole-line commands: toggle comment, duplicate, move. Each returns one contiguous edit.
enum PineEditorCommands {
    private static let marker = "//"

    // MARK: - Comments

    /// ⌘/ — comments every selected line at the shallowest indentation among them, or, when
    /// they are all commented already, removes the `//` (and the space after it).
    static func toggleComment(in context: PineEditorContext) -> PineEditorEdit? {
        let lines = context.coveredLines()
        var candidates = lines.filter { !context.isBlank(lineAt: $0.location) }
        // A lone empty line can still take a comment; blank lines inside a block are skipped.
        if candidates.isEmpty, lines.count == 1 { candidates = lines }
        guard !candidates.isEmpty else { return nil }

        let isCommented: (NSRange) -> Bool = { line in
            let start = line.location + context.leadingWhitespaceLength(ofLineAt: line.location)
            return context.source.length >= start + 2
                && context.source.substring(with: NSRange(location: start, length: 2)) == marker
        }
        var edits = PineLineEditSet()
        if candidates.allSatisfy(isCommented) {
            for line in candidates {
                let start = line.location + context.leadingWhitespaceLength(ofLineAt: line.location)
                let spaceFollows = context.unit(at: start + 2) == 0x20
                edits.add(location: start, remove: spaceFollows ? 3 : 2)
            }
        } else {
            let column = candidates.map { context.leadingWhitespaceLength(ofLineAt: $0.location) }.min() ?? 0
            for line in candidates { edits.add(location: line.location + column, insert: marker + " ") }
        }
        return edits.makeEdit(in: context.source, selection: context.selection)
    }

    // MARK: - Duplicate

    /// ⇧⌥↓ / ⇧⌥↑ — copies the selected lines (the caret's line when nothing is selected)
    /// below or above themselves. The selection follows the copy when going down and stays
    /// put, now on the upper copy, when going up.
    static func duplicate(down: Bool, in context: PineEditorContext) -> PineEditorEdit? {
        guard let block = lineBlock(of: context) else { return nil }
        let text = context.source.substring(with: block)
        let endsWithBreak = context.isLineBreak(context.unit(at: NSMaxRange(block) - 1))
        if down {
            let inserted = endsWithBreak ? text : "\n" + text
            return PineEditorEdit(
                range: NSRange(location: NSMaxRange(block), length: 0), replacement: inserted,
                selection: NSRange(
                    location: context.selection.location + inserted.utf16.count,
                    length: context.selection.length))
        }
        return PineEditorEdit(
            range: NSRange(location: block.location, length: 0),
            replacement: endsWithBreak ? text : text + "\n",
            selection: context.selection)
    }

    // MARK: - Move

    /// ⌥↑ / ⌥↓ — swaps the selected lines with the line above or below. Selection and caret move
    /// with the text; indentation is left exactly as it was.
    static func move(down: Bool, in context: PineEditorContext) -> PineEditorEdit? {
        guard let block = lineBlock(of: context) else { return nil }
        let neighbourOffset = down ? NSMaxRange(block) : block.location - 1
        guard neighbourOffset >= 0, neighbourOffset < context.length else { return nil }
        let neighbour = context.fullRange(ofLineAt: neighbourOffset)
        var blockText = context.source.substring(with: block)
        var neighbourText = context.source.substring(with: neighbour)

        if down {
            // The neighbour may be the unterminated last line; the block then takes its place
            // at the end and loses the break it carried.
            if !context.isLineBreak(context.unit(at: NSMaxRange(neighbour) - 1)) {
                neighbourText += "\n"
                blockText = removingTrailingBreak(blockText)
            }
            let range = NSRange(location: block.location, length: NSMaxRange(neighbour) - block.location)
            return PineEditorEdit(
                range: range, replacement: neighbourText + blockText,
                selection: NSRange(
                    location: context.selection.location + neighbourText.utf16.count,
                    length: context.selection.length))
        }
        // Moving up: the block may be the unterminated last line, which hands its end to the neighbour.
        if !context.isLineBreak(context.unit(at: NSMaxRange(block) - 1)) {
            blockText += "\n"
            neighbourText = removingTrailingBreak(neighbourText)
        }
        let range = NSRange(location: neighbour.location, length: NSMaxRange(block) - neighbour.location)
        return PineEditorEdit(
            range: range, replacement: blockText + neighbourText,
            selection: NSRange(
                location: context.selection.location - neighbour.length, length: context.selection.length))
    }

    // MARK: - Helpers

    /// The selected lines as one terminator-included range.
    private static func lineBlock(of context: PineEditorContext) -> NSRange? {
        let lines = context.coveredLines()
        guard let first = lines.first, let last = lines.last else { return nil }
        return NSRange(location: first.location, length: NSMaxRange(last) - first.location)
    }

    private static func removingTrailingBreak(_ text: String) -> String {
        guard let last = text.last, last.isNewline else { return text }
        return String(text.dropLast())
    }
}
