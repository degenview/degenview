import Foundation

enum PineDiagnosticRangeMapper {
    static func nsRange(for range: PineSourceRange, in source: String) -> NSRange? {
        let sourceLength = source.utf16.count
        guard sourceLength > 0 else { return nil }
        let reportedStart = sourceOffset(for: range.start, in: source)
        let reportedEnd = sourceOffset(for: range.end, in: source)
        let start = reportedStart == sourceLength ? sourceLength - 1 : reportedStart
        return NSRange(
            location: start,
            length: min(max(1, reportedEnd - reportedStart), sourceLength - start)
        )
    }

    /// Compiler offsets refer to its line-ending-normalized source. Mapping through line
    /// and column keeps editor ranges correct for CRLF text and other line separators.
    private static func sourceOffset(for position: PineSourcePosition, in source: String) -> Int {
        let nsSource = source as NSString
        var lineStart = 0
        for _ in 1..<max(1, position.line) {
            guard lineStart < nsSource.length else { return nsSource.length }
            let lineRange = nsSource.lineRange(for: NSRange(location: lineStart, length: 0))
            let nextLineStart = NSMaxRange(lineRange)
            guard nextLineStart > lineStart else { return lineStart }
            lineStart = nextLineStart
        }

        guard lineStart < nsSource.length else { return nsSource.length }
        let fullLineRange = nsSource.lineRange(for: NSRange(location: lineStart, length: 0))
        let fullLine = nsSource.substring(with: fullLineRange)
        let line = fullLine.prefix(while: { !$0.isNewline })
        let characterOffset = min(max(0, position.column - 1), line.count)
        let index = line.index(line.startIndex, offsetBy: characterOffset)
        return min(nsSource.length, lineStart + line[..<index].utf16.count)
    }
}
