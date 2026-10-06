import Foundation

/// When completion opens by itself. One place, so the feel can be tuned without touching the
/// engine or the view.
enum PineCompletionTrigger {
    /// Letters of a word typed before the list opens on its own.
    static let minimumPrefixLength = 2

    enum Cause: Equatable {
        /// The user typed this text.
        case typed(String)
        /// Backspace or delete.
        case deleted
        /// The user asked (Control-Space, Option-Escape, F5).
        case explicit
    }

    /// Whether `cause` opens a closed list at the caret `context` describes.
    ///
    /// A dot after a namespace or alias opens at once; so does the request itself. A word opens
    /// after `minimumPrefixLength` letters. Deleting, pasting and undoing never open it.
    static func opens(for cause: Cause, in context: PineCompletionContext) -> Bool {
        guard context.suppression == nil else { return false }
        switch cause {
        case .explicit:
            return true
        case .deleted:
            return false
        case .typed(let text):
            guard let last = text.last, text.count == 1 else { return false }
            let isWord = isWordCharacter(last)
            switch context.position {
            case .member:
                return last == "." || isWord
            case .importPath(let typed):
                if context.prefix.isEmpty { return last == "/" && typed == "user/" }
                return isWord && context.prefix.count >= minimumPrefixLength
            default:
                return isWord && context.prefix.count >= minimumPrefixLength
            }
        }
    }

    /// Whether a list that has been computed should be shown. An automatic list whose only entry is
    /// what was just typed would offer nothing.
    static func shouldPresent(
        _ items: [PineCompletionItem], for cause: Cause, in context: PineCompletionContext
    ) -> Bool {
        guard !items.isEmpty else { return false }
        if cause != .explicit, items.count == 1, items[0].label == context.prefix, !context.isMember {
            return false
        }
        return true
    }

    private static func isWordCharacter(_ character: Character) -> Bool {
        character.isLetter || character.isNumber || character == "_"
    }
}
