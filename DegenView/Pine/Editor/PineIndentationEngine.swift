import Foundation

/// Indentation decisions for Return, Tab, Shift-Tab, a closing delimiter and paste.
///
/// A level is four spaces: the lexer reads any other indent on a fresh line as a wrapped
/// continuation (PINE1002), so four is the only width that keeps block structure intact.
///
/// What opens a block comes from lexer *token kinds* of the line being left, never from the
/// `.indent`/`.dedent` tokens: an unclosed `(` mid-typing turns every later line into a
/// continuation, which erases those tokens exactly when the editor needs them.
enum PineIndentationEngine {
    static let unit = "    "
    static let width = 4

    // MARK: - Return

    /// Replaces the selection with a line break and the indentation the next line should have.
    static func newline(in context: PineEditorContext) -> PineEditorEdit {
        let caret = context.caret
        let line = context.contentRange(ofLineAt: caret)
        let leadingEnd = line.location + context.leadingWhitespaceLength(ofLineAt: line.location)
        var replaceStart = caret
        var replaceEnd = NSMaxRange(context.selection)

        // Return inside, or at the end of, a line's indentation drops that indentation from the
        // line being left (a blank indented line becomes empty) and rebuilds it on the new line.
        let inIndentation = caret <= leadingEnd
        if inIndentation {
            replaceStart = line.location
            replaceEnd = max(replaceEnd, leadingEnd)
        }
        let base = context.source.substring(
            with: NSRange(location: line.location, length: min(caret, leadingEnd) - line.location))

        let snapshot = context.snapshot
        if let opener = PineDelimiterMatcher.enclosingOpener(before: caret, in: snapshot) {
            let openerIndent = context.leadingWhitespace(ofLineAt: opener.location)
            let onOpenerLine = context.contentRange(ofLineAt: opener.location).location == line.location
            let child = onOpenerLine ? openerIndent + unit : base

            // `(|)`: open the pair onto its own lines, the closer back at the opener's indent.
            let next = NSMaxRange(context.selection)
            if caret - 1 == opener.location,
                PineDelimiterMatcher.partner(of: opener, in: snapshot)?.location == next
            {
                return PineEditorEdit(
                    range: NSRange(location: replaceStart, length: replaceEnd - replaceStart),
                    replacement: "\n" + child + "\n" + openerIndent,
                    selection: NSRange(location: replaceStart + 1 + child.utf16.count, length: 0))
            }
            return lineBreak(
                replaceStart: replaceStart, replaceEnd: replaceEnd, indent: child)
        }

        let lineEnd = NSMaxRange(line)
        let before = snapshot.items(in: NSRange(location: line.location, length: caret - line.location))
        // The selection is replaced by the line break, so only text past it moves down.
        let afterStart = NSMaxRange(context.selection)
        let after = snapshot.items(
            in: NSRange(location: afterStart, length: max(0, lineEnd - afterStart)))
        var indent = base
        if after.isEmpty, opensBlock(before.map(\.kind)) || continues(before.last?.kind) {
            indent += unit
        }
        return lineBreak(replaceStart: replaceStart, replaceEnd: replaceEnd, indent: indent)
    }

    private static func lineBreak(
        replaceStart: Int, replaceEnd: Int, indent: String
    ) -> PineEditorEdit {
        PineEditorEdit(
            range: NSRange(location: replaceStart, length: replaceEnd - replaceStart),
            replacement: "\n" + indent,
            selection: NSRange(location: replaceStart + 1 + indent.utf16.count, length: 0))
    }

    /// Whether a line made of these tokens expects an indented body: `if`, `else`, `for`,
    /// `while`, `switch`, a trailing `=>`, a `type`/`enum` header, or `x = if …` / `x = switch …`.
    static func opensBlock(_ kinds: [PineTokenKind]) -> Bool {
        var kinds = kinds[...]
        if case .identifier("export")? = kinds.first, kinds.count > 1 { kinds = kinds.dropFirst() }
        guard let first = kinds.first else { return false }
        switch first {
        case .ifKeyword, .elseKeyword, .forKeyword, .whileKeyword, .switchKeyword:
            return true
        default:
            break
        }
        if kinds.last == .arrow { return true }
        if case .identifier(let word) = first, word == "type" || word == "enum", kinds.count == 2,
            case .identifier = kinds[kinds.startIndex + 1]
        {
            return true
        }
        // `x = if cond` and `x = switch value` take an indented body too.
        if let assign = kinds.firstIndex(where: { $0 == .assign || $0 == .reassign }),
            assign + 1 < kinds.endIndex
        {
            return kinds[assign + 1] == .ifKeyword || kinds[assign + 1] == .switchKeyword
        }
        return false
    }

    /// Whether a line ending in this token continues onto the next one (the lexer's rule).
    static func continues(_ kind: PineTokenKind?) -> Bool {
        guard let kind else { return false }
        switch kind {
        case .assign, .reassign, .plus, .minus, .star, .slash, .percent, .power, .and, .or,
            .comma, .question, .colon:
            return true
        default:
            return false
        }
    }

    // MARK: - Closing delimiter

    /// Typing `)` or `]` on an otherwise empty line puts it at the indentation of the line that
    /// holds its opener, so a multiline call closes flush with where it began.
    static func alignClosing(typed unit: unichar, in context: PineEditorContext) -> PineEditorEdit? {
        guard context.selection.length == 0,
            let opener = PineDelimiterMatcher.enclosingOpener(before: context.caret, in: context.snapshot),
            opener.kind == (unit == 0x29 ? .paren : .bracket)
        else { return nil }
        let line = context.contentRange(ofLineAt: context.caret)
        let leadingEnd = line.location + context.leadingWhitespaceLength(ofLineAt: line.location)
        // Only when nothing but indentation sits before the caret, and nothing but space after.
        guard context.caret <= leadingEnd, line.location > 0,
            context.contentRange(ofLineAt: opener.location).location != line.location,
            context.isBlank(lineAt: context.caret)
        else { return nil }
        let indent = context.leadingWhitespace(ofLineAt: opener.location)
        let closer = String(utf16CodeUnits: [unit], count: 1)
        return PineEditorEdit(
            range: line,
            replacement: indent + closer,
            selection: NSRange(location: line.location + indent.utf16.count + 1, length: 0))
    }

    // MARK: - Tab / Shift-Tab

    /// Tab: one indentation step at the caret (spaces to the next multiple of four), or every
    /// selected line indented when the selection spans lines.
    static func indent(in context: PineEditorContext) -> PineEditorEdit {
        let lines = context.coveredLines()
        guard spansLines(context) else {
            let line = context.contentRange(ofLineAt: context.caret)
            let column = context.caret - line.location
            let spaces = String(repeating: " ", count: width - column % width)
            return PineEditorEdit(
                range: context.selection, replacement: spaces,
                selection: NSRange(location: context.caret + spaces.utf16.count, length: 0))
        }
        var edits = PineLineEditSet()
        for line in lines where context.contentRange(ofLineAt: line.location).length > 0 {
            // Empty lines stay empty; lines of only whitespace are still shifted.
            edits.add(location: line.location, insert: unit)
        }
        return edits.makeEdit(in: context.source, selection: context.selection)
            ?? PineEditorEdit.moveCaret(to: context.caret)
    }

    /// Shift-Tab: removes up to one indentation step from the caret's line or every selected line.
    static func outdent(in context: PineEditorContext) -> PineEditorEdit? {
        var edits = PineLineEditSet()
        for line in context.coveredLines() {
            let removable = removableIndent(ofLineAt: line.location, in: context)
            if removable > 0 { edits.add(location: line.location, remove: removable) }
        }
        return edits.makeEdit(in: context.source, selection: context.selection)
    }

    /// Up to four leading spaces, or one leading tab.
    private static func removableIndent(ofLineAt lineStart: Int, in context: PineEditorContext) -> Int {
        if context.unit(at: lineStart) == 0x09 { return 1 }
        var count = 0
        while count < width, context.unit(at: lineStart + count) == 0x20 { count += 1 }
        return count
    }

    /// Whether the selection touches more than one line.
    private static func spansLines(_ context: PineEditorContext) -> Bool {
        context.selection.length > 0 && context.coveredLines().count > 1
    }

    // MARK: - Paste

    /// Re-bases multiline pasted code onto the indentation of the line it lands on, keeping its
    /// own relative structure. Deliberately narrow: only when the paste starts after nothing but
    /// indentation, and that indentation is not empty. Anything else is pasted verbatim (`nil`).
    static func reindentPaste(_ text: String, in context: PineEditorContext) -> PineEditorEdit? {
        guard text.contains(where: \.isNewline) else { return nil }
        let line = context.contentRange(ofLineAt: context.selection.location)
        let prefixLength = context.selection.location - line.location
        guard prefixLength > 0, prefixLength <= context.leadingWhitespaceLength(ofLineAt: line.location)
        else { return nil }
        let base = context.source.substring(with: NSRange(location: line.location, length: prefixLength))

        let lines = text.replacingOccurrences(of: "\r\n", with: "\n")
            .replacingOccurrences(of: "\r", with: "\n")
            .components(separatedBy: "\n")
        func indentWidth(_ line: String) -> Int {
            var total = 0
            for character in line {
                if character == " " { total += 1 } else if character == "\t" { total += width } else { break }
            }
            return total
        }
        func isBlank(_ line: String) -> Bool { line.allSatisfy { $0 == " " || $0 == "\t" } }

        let first = lines[0]
        let rest = lines.dropFirst().filter { !isBlank($0) }
        guard !rest.isEmpty, !isBlank(first) else { return nil }
        let restMinimum = rest.map(indentWidth).min() ?? 0

        // Where the first line sat in its source: its own indentation if it was copied whole,
        // otherwise the shallowest later line, one level up when the first line opens a block.
        let firstIndent = indentWidth(first)
        let reference: Int
        if firstIndent > 0 {
            reference = min(firstIndent, restMinimum)
        } else {
            let lexed = PineLexicalSnapshot(source: first)
            let opens = opensBlock(lexed.items.map(\.kind)) || continues(lexed.items.last?.kind)
            reference = max(0, restMinimum - (opens ? width : 0))
        }

        var result = [String(first.drop(while: { $0 == " " || $0 == "\t" }))]
        for line in lines.dropFirst() {
            if isBlank(line) {
                result.append("")
                continue
            }
            let relative = max(0, indentWidth(line) - reference)
            result.append(
                base + String(repeating: " ", count: relative)
                    + line.drop(while: { $0 == " " || $0 == "\t" }))
        }
        let replacement = result.joined(separator: "\n")
        guard replacement != text else { return nil }
        return PineEditorEdit(
            range: context.selection, replacement: replacement,
            selection: NSRange(
                location: context.selection.location + replacement.utf16.count, length: 0))
    }
}
