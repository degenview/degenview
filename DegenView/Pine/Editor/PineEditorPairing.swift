import Foundation

/// Automatic pairing of `( ) [ ] " '`: insert, wrap, overtype and delete.
///
/// Pine has no braces, so `{` and `}` are deliberately not paired. Every decision lives here,
/// in one precedence order, so the text view never carries special cases of its own. For an
/// opening delimiter or quote:
///
/// 1. a selection wraps, and keeps the wrapped text selected;
/// 2. a comment is plain text, and a string is plain text apart from its own closing quote;
/// 3. otherwise a pair is inserted when the next character cannot be mistaken for the
///    start of something the new delimiter should enclose.
///
/// For a closing delimiter: step over an existing one that closes a real opener, else align it
/// on a line of its own, else type it. Callers handle marked text before getting here.
enum PineEditorPairing {
    private static let closers: [unichar: unichar] = [0x28: 0x29, 0x5B: 0x5D]
    private static let quotes: Set<unichar> = [0x22, 0x27]
    private static let closingUnits: Set<unichar> = [0x29, 0x5D]
    /// Characters after which a new pair may open without swallowing the following text.
    private static let autoCloseBefore: Set<unichar> = [0x29, 0x5D, 0x2C, 0x3B, 0x3A, 0x2E]
    private static let backslash: unichar = 0x5C

    // MARK: - Typing

    /// The edit for typing `text`, or `nil` to let the text view insert it normally.
    static func typed(_ text: String, in context: PineEditorContext) -> PineEditorEdit? {
        guard text.utf16.count == 1, let unit = text.utf16.first else { return nil }
        if closers[unit] != nil || quotes.contains(unit) {
            return opening(unit, in: context)
        }
        if closingUnits.contains(unit) {
            return closing(unit, in: context)
        }
        return nil
    }

    private static func opening(_ unit: unichar, in context: PineEditorContext) -> PineEditorEdit? {
        let isQuote = quotes.contains(unit)
        let closer = isQuote ? unit : closers[unit]!
        let open = String(utf16CodeUnits: [unit], count: 1)
        let close = String(utf16CodeUnits: [closer], count: 1)
        let selection = context.selection

        // 1. Wrap a selection.
        if selection.length > 0 {
            if context.isInsideComment(at: selection.location + 1) { return nil }
            if isQuote, let literal = context.enclosingString(at: selection.location),
                literal.quote == unit
            {
                return nil
            }
            return PineEditorEdit(
                range: selection,
                replacement: open + context.source.substring(with: selection) + close,
                selection: NSRange(location: selection.location + 1, length: selection.length))
        }

        let caret = context.caret
        // 2. Comments and strings are literal text.
        if context.isInsideComment(at: caret) { return nil }
        if let literal = context.enclosingString(at: caret) {
            guard isQuote, literal.quote == unit else { return nil }
            // The string's own closing quote is stepped over.
            if literal.isTerminated, context.unit(at: caret) == unit, caret == NSMaxRange(literal.range) - 1 {
                return .moveCaret(to: caret + 1)
            }
            // Mid-string, the same quote needs an escape unless it already has one.
            if literal.isTerminated, context.unit(at: caret - 1) != backslash {
                return .insert("\\" + open, at: caret, caret: caret + 2)
            }
            return nil
        }

        // 3. Pair, but not before text it would swallow, and not as an apostrophe or `"""`.
        let next = context.unit(at: caret)
        guard next == nil || context.isWhitespace(next) || context.isLineBreak(next)
            || autoCloseBefore.contains(next!)
        else { return nil }
        if isQuote {
            let previous = context.unit(at: caret - 1)
            if context.isWordUnit(previous) || previous == unit || previous == backslash { return nil }
        }
        return .insert(open + close, at: caret, caret: caret + 1)
    }

    private static func closing(_ unit: unichar, in context: PineEditorContext) -> PineEditorEdit? {
        guard context.selection.length == 0 else { return nil }
        let caret = context.caret
        // Overtype a closer that closes an opener, not a stray one.
        if context.unit(at: caret) == unit, let index = context.snapshot.delimiterIndex(at: caret),
            context.snapshot.partners[index] >= 0
        {
            return .moveCaret(to: caret + 1)
        }
        if context.isInsideString(at: caret) || context.isInsideComment(at: caret) { return nil }
        return PineIndentationEngine.alignClosing(typed: unit, in: context)
    }

    // MARK: - Backspace

    /// Backspace between an empty pair removes both halves; anywhere else `nil`.
    static func backspace(in context: PineEditorContext) -> PineEditorEdit? {
        guard context.selection.length == 0, context.caret > 0,
            let previous = context.unit(at: context.caret - 1),
            let next = context.unit(at: context.caret)
        else { return nil }
        let caret = context.caret
        let pair = NSRange(location: caret - 1, length: 2)
        let removal = PineEditorEdit(
            range: pair, replacement: "", selection: NSRange(location: caret - 1, length: 0))

        if closers[previous] == next {
            // Both must be real delimiters, not characters inside a string or comment.
            guard let open = context.snapshot.delimiterIndex(at: caret - 1),
                let close = context.snapshot.delimiterIndex(at: caret),
                context.snapshot.partners[open] == close
            else { return nil }
            return removal
        }
        if quotes.contains(previous), previous == next,
            let literal = context.snapshot.stringLiteral(containing: caret - 1),
            literal.range == pair
        {
            return removal
        }
        return nil
    }
}
