import Foundation

/// Which indentation guides each line shows, from the text alone (no views, no layout).
///
/// A guide sits at the real indentation column of the line that opens its block, so it lines up
/// with where that line's text starts even when the indentation is not a multiple of four (a
/// wrapped argument list indented by five spaces gets its guide at column five, not four).
/// Block structure is the usual indentation stack: a line is a child of the nearest line above it
/// that is indented at least one step (four columns) less; a line indented by less than a step
/// more than the previous one is that line's sibling, as the lexer treats it (a wrapped line).
/// A blank line keeps only the guides both of its neighbours share, so a guide runs through a
/// gap inside a block and stops at the block's end.
enum PineIndentGuides {
    struct Line: Equatable {
        /// The line without its terminator.
        let range: NSRange
        /// Columns of the guides drawn on this line: the indentation of each enclosing line.
        let guideColumns: [Int]
        /// This line's own indentation, or `nil` when it is blank.
        let indent: Int?

        var isBlank: Bool { indent == nil }
        var level: Int { guideColumns.count }
    }

    static let width = PineIndentationEngine.width
    /// Lines read above the requested range to find the enclosing blocks, and below it to settle
    /// blank lines and the active guide.
    private static let linesAbove = 300
    private static let linesBelow = 64

    /// Columns of indentation of the line at `range`, or `nil` when the line is blank.
    static func indentColumns(of range: NSRange, in source: NSString) -> Int? {
        var columns = 0
        var index = range.location
        while index < NSMaxRange(range) {
            switch source.character(at: index) {
            case 0x20: columns += 1
            case 0x09: columns += width
            default: return columns
            }
            index += 1
        }
        return nil
    }

    /// The lines that intersect `charRange`, with their guides resolved.
    static func lines(in source: NSString, covering charRange: NSRange) -> [Line] {
        guard source.length > 0 else { return [] }
        var location = lineStart(source, charRange.location)
        let firstVisible = location
        for _ in 0..<linesAbove where location > 0 { location = lineStart(source, location - 1) }
        let reachedTop = location == 0
        let end = min(source.length, NSMaxRange(charRange))

        var raw: [(range: NSRange, indent: Int?, visible: Bool)] = []
        var linesAfter = 0
        while true {
            var start = 0
            var next = 0
            var contentsEnd = 0
            source.getLineStart(
                &start, end: &next, contentsEnd: &contentsEnd,
                for: NSRange(location: location, length: 0))
            // Asking at the very end of text without a final break returns the last line again.
            if !raw.isEmpty, start != location { break }
            let range = NSRange(location: start, length: contentsEnd - start)
            raw.append((range, indentColumns(of: range, in: source), start >= firstVisible && start <= end))
            if start > end { linesAfter += 1 }
            // The last line, or the empty one after a final line break.
            if linesAfter > linesBelow || next == location || (next >= source.length && next == contentsEnd) {
                break
            }
            location = next
        }
        return resolve(raw, seedAncestors: !reachedTop).filter { $0.visible }.map(\.line)
    }

    private static func lineStart(_ source: NSString, _ offset: Int) -> Int {
        var start = 0
        source.getLineStart(&start, end: nil, contentsEnd: nil, for: NSRange(location: offset, length: 0))
        return start
    }

    private static func resolve(
        _ raw: [(range: NSRange, indent: Int?, visible: Bool)], seedAncestors: Bool
    ) -> [(line: Line, visible: Bool)] {
        var stack: [Int] = []
        var ancestors = [[Int]?](repeating: nil, count: raw.count)
        var seeded = !seedAncestors
        for (index, entry) in raw.enumerated() {
            guard let indent = entry.indent else { continue }
            if !seeded {
                // The window starts inside a block whose opener is out of reach: assume steps.
                stack = Array(stride(from: 0, to: indent, by: width)).filter { indent - $0 >= width }
                seeded = true
            }
            while let top = stack.last, top >= indent || indent - top < width { stack.removeLast() }
            ancestors[index] = stack
            stack.append(indent)
        }

        // A blank line keeps the guides its nearest non-blank neighbours both have.
        var before = [[Int]?](repeating: nil, count: raw.count)
        var carried: [Int]?
        for index in raw.indices {
            if let own = ancestors[index] { carried = own }
            before[index] = carried
        }
        carried = nil
        var result: [(line: Line, visible: Bool)] = []
        for index in raw.indices.reversed() {
            if let own = ancestors[index] { carried = own }
            let columns =
                ancestors[index] ?? (before[index] ?? []).filter { (carried ?? []).contains($0) }
            result.append((Line(range: raw[index].range, guideColumns: columns, indent: raw[index].indent), raw[index].visible))
        }
        return result.reversed()
    }

    /// The guide to emphasize for the caret: the one belonging to the block the caret's line is
    /// in (or opens), as its column and the indices of the contiguous lines it spans.
    static func activeGuide(
        in lines: [Line], caretLine: Int
    ) -> (column: Int, lines: ClosedRange<Int>)? {
        guard lines.indices.contains(caretLine) else { return nil }
        let caret = lines[caretLine]
        var column = caret.guideColumns.last
        var start = caretLine
        // A line that opens a block owns the guide at its own indentation, drawn down its body.
        if let indent = caret.indent,
            let body = lines[(caretLine + 1)...].firstIndex(where: { !$0.isBlank }),
            lines[body].guideColumns.contains(indent)
        {
            column = indent
            start = body
        }
        guard let column, lines[start].guideColumns.contains(column) else { return nil }
        var first = start
        var last = start
        while first > 0, lines[first - 1].guideColumns.contains(column) { first -= 1 }
        while last + 1 < lines.count, lines[last + 1].guideColumns.contains(column) { last += 1 }
        return (column, first...last)
    }
}
