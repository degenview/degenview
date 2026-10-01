import Foundation

struct PineLexer {
    let source: String
    let limits: PineLimits

    /// Mutable result of a lexing run, threaded through the per-token helpers.
    struct State {
        var tokens: [PineToken] = []
        var diagnostics: [PineDiagnostic] = []
        /// UTF-16 offset of the current line's first character in `source`.
        var offset = 0
        var indents = [0]
        var delimiterDepth = 0
        /// The previous line ended in an operator or inside brackets, so this one continues it.
        var continued = false

        var isAtLogicalLineStart: Bool { delimiterDepth == 0 && !continued }
    }

    func lex() -> (tokens: [PineToken], diagnostics: [PineDiagnostic]) {
        if source.count > limits.sourceCharacters {
            let message = "Source exceeds the \(limits.sourceCharacters)-character limit."
            // The parser needs a terminating token even when there is nothing else to parse.
            return ([.init(kind: .eof, range: .zero)], [.error("PINE8001", .resource, message, .zero)])
        }
        var state = State()
        let lines = source.split(separator: "\n", omittingEmptySubsequences: false)
        var lastLine = PineSourceLine(text: [], number: 1, startOffset: 0)
        for (index, raw) in lines.enumerated() {
            let line = PineSourceLine(text: Array(raw), number: index + 1, startOffset: state.offset)
            lastLine = line
            lexLine(line, &state)
            state.offset += line.utf16Length + 1
            if state.tokens.count > limits.tokens {
                let range = state.tokens.last?.range ?? .zero
                state.diagnostics.append(.error("PINE8002", .resource, "Token limit exceeded.", range))
                break
            }
        }
        let eof = PineSourceRange(
            start: .init(
                line: lines.count, column: lastLine.text.count + 1, offset: source.utf16.count),
            end: .init(
                line: lines.count, column: lastLine.text.count + 1, offset: source.utf16.count))
        while state.indents.count > 1 {
            state.indents.removeLast()
            state.tokens.append(.init(kind: .dedent, range: eof))
        }
        state.tokens.append(.init(kind: .eof, range: eof))
        return (state.tokens, state.diagnostics)
    }

    // MARK: - Lines

    private func lexLine(_ line: PineSourceLine, _ state: inout State) {
        let leading = line.text.prefix { $0 == " " || $0 == "\t" }.count
        let rest = line.text[leading...]
        if rest.isEmpty || rest.starts(with: ["/", "/"]) { return }
        if state.isAtLogicalLineStart {
            if Self.isWrappedLine(leading: leading, line, state) {
                // Joins the previous logical line by dropping the newline that ended it.
                state.tokens.removeLast()
            } else {
                lexIndentation(leading: leading, line, &state)
            }
        }
        var i = leading
        while i < line.text.count { i = lexToken(at: i, line, &state) }
        state.continued =
            state.delimiterDepth > 0
            || state.tokens.last.map { Self.continuationKinds.contains($0.kind) } == true
        if !state.continued {
            let end = line.text.count
            state.tokens.append(.init(kind: .newline, range: line.range(end, end)))
        }
    }

    /// Columns of leading whitespace, where a tab is one four-space level, as in TradingView.
    private static func indentWidth(_ line: PineSourceLine, leading: Int) -> Int {
        line.text.prefix(leading).reduce(0) { $0 + ($1 == "\t" ? 4 : 1) }
    }

    /// Pine wraps a long line by indenting its continuation by anything but a multiple of four
    /// columns. A first line has nothing to continue, so it still reports PINE1002.
    private static func isWrappedLine(leading: Int, _ line: PineSourceLine, _ state: State) -> Bool {
        indentWidth(line, leading: leading) % 4 != 0 && state.tokens.last?.kind == .newline
    }

    /// `leading` counts the line's leading space/tab characters (for ranges); the indent level
    /// compares columns.
    private func lexIndentation(leading: Int, _ line: PineSourceLine, _ state: inout State) {
        let width = Self.indentWidth(line, leading: leading)
        if width % 4 != 0 {
            state.diagnostics.append(
                .error(
                    "PINE1002", .lexical, "Indentation must use multiples of four spaces.",
                    line.range(0, max(1, leading))))
        }
        if width > state.indents[state.indents.count - 1] {
            state.indents.append(width)
            state.tokens.append(.init(kind: .indent, range: line.range(0, leading)))
        }
        while width < state.indents[state.indents.count - 1] {
            state.indents.removeLast()
            state.tokens.append(.init(kind: .dedent, range: line.range(0, leading)))
        }
    }

    /// Lexes the token starting at `i` and returns the index of the next unread character.
    private func lexToken(at i: Int, _ line: PineSourceLine, _ state: inout State) -> Int {
        let c = line.text[i]
        if c == " " || c == "\t" { return i + 1 }
        if c == "/", line.character(at: i + 1) == "/" { return line.text.count }
        if c.isLetter || c == "_" { return lexWord(at: i, line, &state) }
        if Self.isDigit(c) || (c == "." && Self.isDigit(line.character(at: i + 1))) {
            return lexNumber(at: i, line, &state)
        }
        switch c {
        case "\"", "'": return lexString(at: i, quote: c, line, &state)
        case "#": return lexColor(at: i, line, &state)
        default: return lexOperator(at: i, line, &state)
        }
    }

    static func isDigit(_ c: Character?) -> Bool {
        guard let c else { return false }
        return c.isASCII && c.isNumber
    }
}
