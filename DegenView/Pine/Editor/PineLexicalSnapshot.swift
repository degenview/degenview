import Foundation

/// What the editor needs to know about the lexical structure of one version of the source:
/// tokens, strings, comments and bracket pairing, all as UTF-16 ranges of the editor text.
///
/// It is built from `PineLexer`, so the editor never keeps its own idea of Pine's grammar:
/// strings are the lexer's `.string` tokens (either quote), and delimiters are the lexer's
/// parenthesis/bracket tokens, which by construction never appear inside strings or comments.
/// Comments are not tokens; they are the `//` runs found in the gaps between strings.
///
/// Building one costs a lex. The last snapshot is cached, so highlighting and every editing
/// helper working on the same text share it.
struct PineLexicalSnapshot {
    /// A token that carries text, in editor coordinates (structural tokens are left out).
    struct Item: Equatable {
        let kind: PineTokenKind
        let range: NSRange
    }

    struct Delimiter: Equatable {
        enum Kind { case paren, bracket }
        let kind: Kind
        let isOpening: Bool
        let location: Int

        var range: NSRange { NSRange(location: location, length: 1) }
    }

    struct StringLiteral: Equatable {
        let range: NSRange
        let quote: unichar
        let isTerminated: Bool
    }

    /// The lexer's output, with offsets in the compiler's line-ending-normalized text.
    let tokens: [PineToken]
    let items: [Item]
    let delimiters: [Delimiter]
    /// Index into `delimiters` of each delimiter's partner, or -1 when it has none.
    let partners: [Int]
    /// Nesting depth of each delimiter: 0 for an outermost pair, counted from its opener.
    let depths: [Int]
    let strings: [StringLiteral]
    let comments: [NSRange]
    private let offsets: PineOffsetMap?

    init(source: String) {
        Self.recordBuild()
        let normalized = PineCompiler.normalizeLineEndings(in: source)
        let offsets = normalized == source ? nil : PineOffsetMap(original: source, normalized: normalized)
        let tokens = PineLexer(source: normalized, limits: .default).lex().tokens
        self.offsets = offsets
        self.tokens = tokens

        let text = source as NSString
        var items: [Item] = []
        var delimiters: [Delimiter] = []
        var strings: [StringLiteral] = []
        for token in tokens {
            switch token.kind {
            case .newline, .indent, .dedent, .eof:
                continue
            default:
                break
            }
            guard let range = Self.range(of: token, offsets: offsets), NSMaxRange(range) <= text.length
            else { continue }
            items.append(Item(kind: token.kind, range: range))
            switch token.kind {
            case .leftParen: delimiters.append(.init(kind: .paren, isOpening: true, location: range.location))
            case .rightParen: delimiters.append(.init(kind: .paren, isOpening: false, location: range.location))
            case .leftBracket:
                delimiters.append(.init(kind: .bracket, isOpening: true, location: range.location))
            case .rightBracket:
                delimiters.append(.init(kind: .bracket, isOpening: false, location: range.location))
            case .string:
                strings.append(Self.stringLiteral(at: range, in: text))
            default:
                break
            }
        }
        self.items = items
        self.delimiters = delimiters
        self.strings = strings
        comments = Self.commentRanges(in: text, strings: strings)
        (partners, depths) = Self.pair(delimiters)
    }

    // MARK: - Instrumentation

    private static let buildLock = NSLock()
    private static var builds = 0

    /// How many snapshots have been built (each is one lex). Tests assert it does not move while
    /// the caret does.
    static var buildCount: Int {
        buildLock.lock()
        defer { buildLock.unlock() }
        return builds
    }

    private static func recordBuild() {
        buildLock.lock()
        builds += 1
        buildLock.unlock()
    }

    // MARK: - Cache

    private static let lock = NSLock()
    private static var cache: (source: String, snapshot: PineLexicalSnapshot)?

    /// The snapshot of `source`, reusing the previous one when the text is unchanged.
    static func shared(for source: String) -> PineLexicalSnapshot {
        lock.lock()
        defer { lock.unlock() }
        if let cache, cache.source == source { return cache.snapshot }
        let snapshot = PineLexicalSnapshot(source: source)
        cache = (source, snapshot)
        return snapshot
    }

    // MARK: - Queries

    /// The editor range of a lexer token.
    func range(of token: PineToken) -> NSRange? {
        Self.range(of: token, offsets: offsets)
    }

    /// Index into `delimiters` of the delimiter at `location`.
    func delimiterIndex(at location: Int) -> Int? {
        let index = lowerBound(delimiters.count) { delimiters[$0].location < location }
        guard index < delimiters.count, delimiters[index].location == location else { return nil }
        return index
    }

    /// Index of the first delimiter at or after `location`.
    func firstDelimiterIndex(atOrAfter location: Int) -> Int {
        lowerBound(delimiters.count) { delimiters[$0].location < location }
    }

    /// The string literal covering the character at `location`.
    func stringLiteral(containing location: Int) -> StringLiteral? {
        let index = lowerBound(strings.count) { NSMaxRange(strings[$0].range) <= location }
        guard index < strings.count, strings[index].range.location <= location else { return nil }
        return strings[index]
    }

    /// The comment covering the character at `location`.
    func comment(containing location: Int) -> NSRange? {
        let index = lowerBound(comments.count) { NSMaxRange(comments[$0]) <= location }
        guard index < comments.count, comments[index].location <= location else { return nil }
        return comments[index]
    }

    /// Tokens that start inside `range`, in source order.
    func items(in range: NSRange) -> ArraySlice<Item> {
        let start = lowerBound(items.count) { items[$0].range.location < range.location }
        let end = lowerBound(items.count) { items[$0].range.location < NSMaxRange(range) }
        return items[start..<max(start, end)]
    }

    /// The token whose range contains `location`.
    func item(containing location: Int) -> Item? {
        itemIndex(containing: location).map { items[$0] }
    }

    /// Index into `items` of the token whose range contains `location`.
    func itemIndex(containing location: Int) -> Int? {
        let index = lowerBound(items.count) { NSMaxRange(items[$0].range) <= location }
        guard index < items.count, items[index].range.location <= location else { return nil }
        return index
    }

    /// First index in `0..<count` for which `isBefore` is false, assuming it is monotonic.
    private func lowerBound(_ count: Int, _ isBefore: (Int) -> Bool) -> Int {
        var low = 0
        var high = count
        while low < high {
            let mid = (low + high) / 2
            if isBefore(mid) { low = mid + 1 } else { high = mid }
        }
        return low
    }

    // MARK: - Construction

    private static func range(of token: PineToken, offsets: PineOffsetMap?) -> NSRange? {
        let start = token.range.start
        let end = token.range.end
        if let offsets {
            guard let from = offsets.original(line: start.line, offset: start.offset),
                let to = offsets.original(line: end.line, offset: end.offset)
            else { return nil }
            return NSRange(location: from, length: max(0, to - from))
        }
        return NSRange(location: start.offset, length: max(0, end.offset - start.offset))
    }

    private static func stringLiteral(at range: NSRange, in text: NSString) -> StringLiteral {
        let quote = text.character(at: range.location)
        var terminated = false
        if range.length >= 2, text.character(at: NSMaxRange(range) - 1) == quote {
            // The closing quote counts only when it is not escaped by an odd run of backslashes.
            var backslashes = 0
            var index = NSMaxRange(range) - 2
            while index > range.location, text.character(at: index) == 0x5C {
                backslashes += 1
                index -= 1
            }
            terminated = backslashes % 2 == 0
        }
        return StringLiteral(range: range, quote: quote, isTerminated: terminated)
    }

    /// `//` runs outside strings, each to the end of its line (terminator excluded).
    private static func commentRanges(in text: NSString, strings: [StringLiteral]) -> [NSRange] {
        var result: [NSRange] = []
        var search = 0
        var stringIndex = 0
        while search < text.length {
            let found = text.range(
                of: "//", options: .literal,
                range: NSRange(location: search, length: text.length - search))
            guard found.location != NSNotFound else { break }
            while stringIndex < strings.count, NSMaxRange(strings[stringIndex].range) <= found.location {
                stringIndex += 1
            }
            if stringIndex < strings.count, strings[stringIndex].range.location <= found.location {
                search = NSMaxRange(strings[stringIndex].range)
                continue
            }
            var lineStart = 0
            var lineEnd = 0
            var contentsEnd = 0
            text.getLineStart(&lineStart, end: &lineEnd, contentsEnd: &contentsEnd, for: found)
            result.append(NSRange(location: found.location, length: contentsEnd - found.location))
            search = max(lineEnd, found.location + 2)
        }
        return result
    }

    /// Pairs delimiters with a stack. A closer whose kind differs from the innermost opener is
    /// left unmatched rather than guessing which of them is the typo.
    private static func pair(_ delimiters: [Delimiter]) -> ([Int], [Int]) {
        var partners = [Int](repeating: -1, count: delimiters.count)
        var depths = [Int](repeating: 0, count: delimiters.count)
        var stack: [Int] = []
        for (index, delimiter) in delimiters.enumerated() {
            if delimiter.isOpening {
                depths[index] = stack.count
                stack.append(index)
            } else if let top = stack.last, delimiters[top].kind == delimiter.kind {
                stack.removeLast()
                partners[index] = top
                partners[top] = index
                depths[index] = stack.count
            } else {
                depths[index] = stack.count
            }
        }
        return (partners, depths)
    }
}
