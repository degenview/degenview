import Foundation

/// The text, selection and lexical structure an editing decision is made from.
///
/// Everything is UTF-16 (`NSString` / `NSRange`), the unit AppKit and the lexer share, so no
/// offset is ever converted between `String.Index`, `Character` counts and `NSRange`.
/// A context is a snapshot: build a fresh one for each key press instead of holding one.
struct PineEditorContext {
    let source: NSString
    let selection: NSRange
    let snapshot: PineLexicalSnapshot

    init(source: String, selection: NSRange, snapshot: PineLexicalSnapshot? = nil) {
        self.source = source as NSString
        self.selection = NSRange(
            location: min(max(0, selection.location), self.source.length),
            length: min(selection.length, max(0, self.source.length - selection.location)))
        self.snapshot = snapshot ?? PineLexicalSnapshot.shared(for: source)
    }

    var length: Int { source.length }
    var caret: Int { selection.location }

    // MARK: - Characters

    /// The UTF-16 unit at `offset`, or `nil` outside the text.
    func unit(at offset: Int) -> unichar? {
        offset >= 0 && offset < source.length ? source.character(at: offset) : nil
    }

    func isWordUnit(_ unit: unichar?) -> Bool {
        guard let unit, let scalar = Unicode.Scalar(unit) else {
            // A surrogate half belongs to a non-BMP character; treat emoji and the like as words.
            return unit != nil
        }
        return Character(scalar).isWordCharacter
    }

    func isWhitespace(_ unit: unichar?) -> Bool {
        unit == 0x20 || unit == 0x09
    }

    func isLineBreak(_ unit: unichar?) -> Bool {
        guard let unit else { return false }
        return unit == 0x0A || unit == 0x0D || unit == 0x2028 || unit == 0x2029
    }

    // MARK: - Lexical context

    /// Whether a character typed at `offset` would land inside a string literal: between its
    /// quotes, or after the opening quote of one the line never closes.
    func isInsideString(at offset: Int) -> Bool {
        enclosingString(at: offset) != nil
    }

    func enclosingString(at offset: Int) -> PineLexicalSnapshot.StringLiteral? {
        guard offset > 0, let literal = snapshot.stringLiteral(containing: offset - 1) else { return nil }
        let end = NSMaxRange(literal.range)
        return offset < end || (offset == end && !literal.isTerminated) ? literal : nil
    }

    /// Whether a character typed at `offset` would land inside a `//` comment.
    func isInsideComment(at offset: Int) -> Bool {
        offset > 0 && snapshot.comment(containing: offset - 1) != nil
    }

    func token(atUTF16Offset offset: Int) -> PineLexicalSnapshot.Item? {
        snapshot.item(containing: offset)
    }

    // MARK: - Lines

    /// The line holding `offset`, terminator excluded. An offset at the very end of text that
    /// follows a line break is on the empty last line.
    func contentRange(ofLineAt offset: Int) -> NSRange {
        var start = 0
        var end = 0
        var contentsEnd = 0
        source.getLineStart(
            &start, end: &end, contentsEnd: &contentsEnd,
            for: NSRange(location: min(max(0, offset), source.length), length: 0))
        return NSRange(location: start, length: contentsEnd - start)
    }

    /// The line holding `offset`, terminator included.
    func fullRange(ofLineAt offset: Int) -> NSRange {
        source.lineRange(for: NSRange(location: min(max(0, offset), source.length), length: 0))
    }

    /// Length of the run of spaces and tabs that opens the line starting at `lineStart`.
    func leadingWhitespaceLength(ofLineAt lineStart: Int) -> Int {
        var index = lineStart
        while isWhitespace(unit(at: index)) { index += 1 }
        return index - lineStart
    }

    func leadingWhitespace(ofLineAt offset: Int) -> String {
        let line = contentRange(ofLineAt: offset)
        return source.substring(
            with: NSRange(location: line.location, length: leadingWhitespaceLength(ofLineAt: line.location)))
    }

    func isBlank(lineAt offset: Int) -> Bool {
        let line = contentRange(ofLineAt: offset)
        return leadingWhitespaceLength(ofLineAt: line.location) == line.length
    }

    /// The lines a selection covers, as terminator-included ranges. A selection that ends at the
    /// start of a line does not include that line, the way editors treat whole-line selections.
    func coveredLines() -> [NSRange] {
        var end = NSMaxRange(selection)
        if selection.length > 0, end > selection.location, end == contentRange(ofLineAt: end).location,
            end > 0
        {
            end -= 1
        }
        var lines: [NSRange] = []
        var location = fullRange(ofLineAt: selection.location).location
        while true {
            let line = fullRange(ofLineAt: location)
            lines.append(line)
            let next = NSMaxRange(line)
            if next > end || next >= source.length || line.length == 0 { break }
            location = next
        }
        return lines
    }
}
