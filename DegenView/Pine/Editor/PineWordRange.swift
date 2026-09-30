import Foundation

/// Word boundaries for code: letters, digits and `_`, so dots split `table.cell`.
enum PineWordRange {
    /// The word containing the UTF-16 `index`, or `nil` when it is not on a word character.
    static func range(at index: Int, in source: NSString) -> NSRange? {
        guard index >= 0, index < source.length, isWordCharacter(at: index, in: source) else { return nil }
        var start = index
        var end = index + 1
        while start > 0, isWordCharacter(at: start - 1, in: source) { start -= 1 }
        while end < source.length, isWordCharacter(at: end, in: source) { end += 1 }
        return NSRange(location: start, length: end - start)
    }

    /// Every whole-word occurrence of `word`, so `len` does not match inside `length`.
    static func occurrences(of word: String, in source: NSString) -> [NSRange] {
        guard !word.isEmpty else { return [] }
        var result: [NSRange] = []
        var searchRange = NSRange(location: 0, length: source.length)
        while true {
            let found = source.range(of: word, options: .literal, range: searchRange)
            guard found.location != NSNotFound else { break }
            if range(at: found.location, in: source) == found { result.append(found) }
            let next = NSMaxRange(found)
            searchRange = NSRange(location: next, length: source.length - next)
        }
        return result
    }

    private static func isWordCharacter(at index: Int, in source: NSString) -> Bool {
        let composed = source.rangeOfComposedCharacterSequence(at: index)
        return Character(source.substring(with: composed)).isWordCharacter
    }
}
