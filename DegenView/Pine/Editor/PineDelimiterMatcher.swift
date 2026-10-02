import Foundation

/// Finds the partner of the bracket next to the caret.
///
/// Pairing is resolved once per snapshot (`PineLexicalSnapshot.partners`) from lexer tokens, so
/// strings and comments cannot interfere, and a lookup here is a binary search. `depth` is
/// reported with every match so nesting-level styling can be added without touching the matcher.
enum PineDelimiterMatcher {
    struct Match: Equatable {
        /// The delimiter adjacent to the caret.
        let delimiter: NSRange
        /// Its partner, or `nil` when it has none.
        let partner: NSRange?
        let depth: Int

        var isUnmatched: Bool { partner == nil }
    }

    /// The match for a delimiter immediately before the caret, else one immediately after it.
    static func match(at caret: Int, in snapshot: PineLexicalSnapshot) -> Match? {
        for location in [caret - 1, caret] {
            guard location >= 0, let index = snapshot.delimiterIndex(at: location) else { continue }
            let partner = snapshot.partners[index]
            return Match(
                delimiter: snapshot.delimiters[index].range,
                partner: partner >= 0 ? snapshot.delimiters[partner].range : nil,
                depth: snapshot.depths[index])
        }
        return nil
    }

    /// The nearest opening delimiter before `offset` that is still open there: unclosed, or
    /// closed only at or after `offset`.
    static func enclosingOpener(
        before offset: Int, in snapshot: PineLexicalSnapshot
    ) -> PineLexicalSnapshot.Delimiter? {
        var index = snapshot.firstDelimiterIndex(atOrAfter: offset) - 1
        while index >= 0 {
            let delimiter = snapshot.delimiters[index]
            let partner = snapshot.partners[index]
            if delimiter.isOpening {
                if partner < 0 || snapshot.delimiters[partner].location >= offset { return delimiter }
                index -= 1
            } else {
                // A closed pair before the caret cannot enclose it: jump to before its opener.
                index = partner >= 0 ? partner - 1 : index - 1
            }
        }
        return nil
    }

    /// The closing delimiter paired with `opener`, if any.
    static func partner(
        of opener: PineLexicalSnapshot.Delimiter, in snapshot: PineLexicalSnapshot
    ) -> PineLexicalSnapshot.Delimiter? {
        guard let index = snapshot.delimiterIndex(at: opener.location), snapshot.partners[index] >= 0
        else { return nil }
        return snapshot.delimiters[snapshot.partners[index]]
    }
}
