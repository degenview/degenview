import Foundation

/// Recursive-descent statement parser with a Pratt expression parser. Statements live in
/// `PineParser+Statements`, declarations and type annotations in `PineParser+Declarations`,
/// expressions in `PineParser+Expressions`.
struct PineParser {
    var tokens: [PineToken]
    var index = 0
    var diagnostics: [PineDiagnostic] = []
    var callSite = 0
    let limits: PineLimits
    /// Statements produced so far, at any nesting depth; bounded by `limits.astNodes`.
    private var statementCount = 0
    /// Where `export` prefixed a statement. The keyword is only legal in a library, which the
    /// parser cannot know, so the compiler checks these against the declaration.
    private(set) var exportRanges: [PineSourceRange] = []

    init(tokens: [PineToken], limits: PineLimits) {
        self.tokens = tokens
        self.limits = limits
    }

    mutating func parse() -> ([PineStatement], [PineDiagnostic]) {
        let result = block(untilDedent: false)
        return (result, diagnostics)
    }

    mutating func block(untilDedent: Bool) -> [PineStatement] {
        var out: [PineStatement] = []
        while !at(.eof) && !(untilDedent && at(.dedent)) {
            if take(.newline) || take(.indent) { continue }
            if at(.dedent) {
                advance()
                continue
            }
            let before = index
            let parsed = statementsOnLine()
            out += parsed
            statementCount += parsed.count
            if index == before {
                error("PINE2001", "Expected a statement.", current.range)
                advance()
            } else {
                endStatement(report: !parsed.isEmpty)
            }
            while take(.newline) {}
            if statementCount > limits.astNodes {
                error("PINE8003", "AST node limit exceeded.", current.range)
                break
            }
        }
        if untilDedent { _ = take(.dedent) }
        return out
    }

    /// One statement, plus any that follow it on the same line after a comma
    /// (`a := 1, b := 2`). Empty when the first one does not parse.
    mutating func statementsOnLine() -> [PineStatement] {
        guard let first = statement() else { return [] }
        var statements = [first]
        while take(.comma), let next = statement() { statements.append(next) }
        return statements
    }

    /// Notes an `export` prefix and parses the statement it applies to.
    mutating func exportedStatement() -> PineStatement? {
        exportRanges.append(previous.range)
        return statement()
    }

    /// A statement must end the line. Block statements (`if`, `for`, block functions) end
    /// on the dedent their body consumed, so the next line's first token follows directly.
    /// Leftover tokens are reported once (unless the statement already failed) and skipped
    /// to the end of the line.
    private mutating func endStatement(report: Bool) {
        if at(.newline) || at(.eof) || at(.dedent) || previous.kind == .dedent { return }
        if report {
            error("PINE2013", "Expected end of statement; found unexpected token.", current.range)
        }
        skipToLineEnd()
    }

    mutating func skipToLineEnd() {
        while !at(.newline) && !at(.dedent) && !at(.eof) { advance() }
    }

    // MARK: - Cursor

    var current: PineToken { tokens[min(index, tokens.count - 1)] }
    var previous: PineToken { tokens[max(0, index - 1)] }

    func peek(_ n: Int) -> PineToken? { index + n < tokens.count ? tokens[index + n] : nil }

    mutating func advance() { index = min(index + 1, tokens.count) }

    func at(_ kind: PineTokenKind) -> Bool { current.kind == kind }

    @discardableResult
    mutating func take(_ kind: PineTokenKind) -> Bool {
        if at(kind) {
            advance()
            return true
        }
        return false
    }

    mutating func takeWord(_ word: String) -> Bool {
        guard case .identifier(let value) = current.kind, value == word else { return false }
        advance()
        return true
    }

    mutating func expect(_ kind: PineTokenKind, _ message: String) {
        if !take(kind) { error("PINE2008", message, current.range) }
    }

    mutating func error(_ code: String, _ message: String, _ range: PineSourceRange) {
        diagnostics.append(.error(code, .syntax, message, range))
    }
}
