import Foundation

/// Groups a snapshot's tokens into statements and gives each its indentation, for tooling that
/// must keep working while the source is half typed.
///
/// The lexer drops every newline, indent and dedent after an opening bracket that never closes,
/// which is exactly the state of a script while a call is being typed. The editor therefore does
/// not read those tokens. A statement starts on a new physical line when the previous token left
/// no bracket open (only brackets that have a partner count), did not end in an operator or comma,
/// and the line is indented by a multiple of four columns: the lexer's own rules, minus the
/// dependence on an unclosed bracket.
struct PineStatementSplitter {
    struct Statement: Equatable {
        /// Positions in `content` of the statement's first and last token.
        let first: Int
        let last: Int
        /// Leading columns of its first line, a tab counting as four.
        let indent: Int
    }

    /// Index into `PineLexicalSnapshot.tokens` of every token that carries text and has an editor
    /// range: everything but newline, indent, dedent and end of file.
    let content: [Int]
    /// The editor range of each `content` token.
    let ranges: [NSRange]
    let statements: [Statement]

    init(snapshot: PineLexicalSnapshot, source: String) {
        let tokens = snapshot.tokens
        var content: [Int] = []
        var ranges: [NSRange] = []
        for (index, token) in tokens.enumerated() {
            switch token.kind {
            case .newline, .indent, .dedent, .eof: continue
            default: break
            }
            guard let range = snapshot.range(of: token) else { continue }
            content.append(index)
            ranges.append(range)
        }

        let widths = Self.leadingWidths(of: PineCompiler.normalizeLineEndings(in: source))
        var statements: [Statement] = []
        var statementStart = 0
        var depth = 0
        var delimiter = 0
        for position in content.indices {
            let token = tokens[content[position]]
            if position > 0 {
                let previous = tokens[content[position - 1]]
                let line = token.range.start.line
                let width = line >= 1 && line <= widths.count ? widths[line - 1] : 0
                if line > previous.range.end.line, depth == 0,
                    !PineLexer.continuationKinds.contains(previous.kind), width % 4 == 0
                {
                    statements.append(
                        Statement(first: statementStart, last: position - 1, indent: indent(of: statementStart)))
                    statementStart = position
                }
            }
            switch token.kind {
            case .leftParen, .leftBracket:
                if delimiter < snapshot.partners.count, snapshot.partners[delimiter] >= 0 { depth += 1 }
                delimiter += 1
            case .rightParen, .rightBracket:
                if delimiter < snapshot.partners.count, snapshot.partners[delimiter] >= 0 {
                    depth = max(0, depth - 1)
                }
                delimiter += 1
            default:
                break
            }
        }
        if !content.isEmpty {
            statements.append(
                Statement(first: statementStart, last: content.count - 1, indent: indent(of: statementStart)))
        }
        self.content = content
        self.ranges = ranges
        self.statements = statements

        func indent(of position: Int) -> Int {
            let line = tokens[content[position]].range.start.line
            return line >= 1 && line <= widths.count ? widths[line - 1] : 0
        }
    }

    /// Leading columns of each line, a tab counting as four (as in the lexer). Index 0 is line 1.
    static func leadingWidths(of normalized: String) -> [Int] {
        var widths: [Int] = []
        var width = 0
        var counting = true
        for byte in normalized.utf8 {
            if byte == 0x0A {
                widths.append(width)
                width = 0
                counting = true
            } else if counting {
                if byte == 0x20 {
                    width += 1
                } else if byte == 0x09 {
                    width += 4
                } else {
                    counting = false
                }
            }
        }
        widths.append(width)
        return widths
    }
}
