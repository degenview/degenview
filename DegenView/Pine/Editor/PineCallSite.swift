import Foundation

/// The call a caret sits inside: which function, and which argument.
///
/// It reads lexer tokens and the bracket pairing of the snapshot, so a comma inside a string, a
/// comment or a nested call never moves the argument, and a half-typed call still resolves.
struct PineCallSite: Equatable {
    /// The callee as written: `["ta", "sma"]`, `["foo"]`.
    let callee: [String]
    /// Editor offset of the call's opening parenthesis.
    let opener: Int
    /// Zero-based index of the argument the caret is in, counting positional and named alike.
    let argumentIndex: Int
    /// How many arguments before the caret's are positional (no `name =`).
    let positionalBefore: Int
    /// Names already supplied anywhere in the call (`title = …`).
    let namedArguments: Set<String>
    /// The name of the argument the caret is in, when it starts with `name =`.
    let activeName: String?
    /// Whether nothing but the word being typed precedes the caret in its argument: the place for
    /// a parameter name.
    let isAtArgumentStart: Bool

    var calleeName: String { callee.joined(separator: ".") }

    /// The call enclosing `caret`, or nil. `statementStart` bounds the search so a stray
    /// unclosed parenthesis far above cannot enclose every later line.
    static func locate(
        at caret: Int, statementStart: Int, in snapshot: PineLexicalSnapshot
    ) -> PineCallSite? {
        var position = caret
        while let opener = PineDelimiterMatcher.enclosingOpener(before: position, in: snapshot),
            opener.location >= statementStart
        {
            position = opener.location
            // A bracket, a grouping parenthesis or a keyword's condition is not a call: look outward.
            guard opener.kind == .paren, let callee = chain(before: opener.location, in: snapshot) else {
                continue
            }
            return site(of: callee, opener: opener, caret: caret, in: snapshot)
        }
        return nil
    }

    // MARK: - Callee

    /// The dotted name directly before `location`: `ta.sma` before its `(`. Nil when no name is
    /// there, or the call is a method of a value (`f().g(`), whose receiver is not a name.
    private static func chain(before location: Int, in snapshot: PineLexicalSnapshot) -> [String]? {
        let items = snapshot.items
        func touches(_ first: Int, _ second: Int) -> Bool {
            NSMaxRange(items[first].range) == items[second].range.location
        }
        guard let last = snapshot.itemIndex(containing: location - 1),
            NSMaxRange(items[last].range) == location, let name = PineCompletionContext.name(of: items[last].kind)
        else { return nil }
        var parts = [name]
        var current = last
        while current >= 1, items[current - 1].kind == .dot, touches(current - 1, current) {
            guard current >= 2, touches(current - 2, current - 1),
                let part = PineCompletionContext.name(of: items[current - 2].kind)
            else { return nil }
            parts.insert(part, at: 0)
            current -= 2
        }
        return parts
    }

    // MARK: - Arguments

    private static func site(
        of callee: [String], opener: PineLexicalSnapshot.Delimiter, caret: Int, in snapshot: PineLexicalSnapshot
    ) -> PineCallSite {
        // Without a closer only what precedes the caret belongs to the call.
        let end = PineDelimiterMatcher.partner(of: opener, in: snapshot)?.location ?? caret
        let start = opener.location + 1
        let inside = snapshot.items(in: NSRange(location: start, length: max(0, end - start)))

        var segments: [[PineLexicalSnapshot.Item]] = [[]]
        var caretSegment = 0
        var nesting = 0
        for item in inside {
            switch item.kind {
            case .leftParen, .leftBracket: nesting += 1
            case .rightParen, .rightBracket: nesting = max(0, nesting - 1)
            default: break
            }
            if item.kind == .comma, nesting == 0 {
                segments.append([])
                if item.range.location < caret { caretSegment += 1 }
            } else {
                segments[segments.count - 1].append(item)
            }
        }

        func argumentName(_ segment: [PineLexicalSnapshot.Item]) -> String? {
            guard segment.count >= 2, segment[1].kind == .assign else { return nil }
            return PineCompletionContext.name(of: segment[0].kind)
        }
        var named = Set<String>()
        var positional = 0
        for (index, segment) in segments.enumerated() {
            if let name = argumentName(segment) {
                named.insert(name)
            } else if index < caretSegment, !segment.isEmpty {
                positional += 1
            }
        }
        let current = caretSegment < segments.count ? segments[caretSegment] : []
        let before = current.filter { $0.range.location < caret }
        let atStart =
            before.isEmpty
            || (before.count == 1 && PineCompletionContext.name(of: before[0].kind) != nil
                && NSMaxRange(before[0].range) >= caret)
        return PineCallSite(
            callee: callee, opener: opener.location, argumentIndex: caretSegment, positionalBefore: positional,
            namedArguments: named, activeName: argumentName(current), isAtArgumentStart: atStart)
    }
}
