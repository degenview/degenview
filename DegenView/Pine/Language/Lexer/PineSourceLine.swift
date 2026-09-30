import Foundation

/// One source line as characters, with the UTF-16 bookkeeping `PineSourceRange` offsets need.
/// Columns count characters; offsets count UTF-16 units (what `NSRange` consumers expect).
struct PineSourceLine {
    let text: [Character]
    let number: Int
    let startOffset: Int
    /// `utf16Prefix[i]` is the UTF-16 length of `text[..<i]`.
    private let utf16Prefix: [Int]

    init(text: [Character], number: Int, startOffset: Int) {
        self.text = text
        self.number = number
        self.startOffset = startOffset
        var prefix = [0]
        prefix.reserveCapacity(text.count + 1)
        for character in text { prefix.append(prefix[prefix.count - 1] + character.utf16.count) }
        self.utf16Prefix = prefix
    }

    var utf16Length: Int { utf16Prefix[text.count] }

    func character(at index: Int) -> Character? {
        index < text.count ? text[index] : nil
    }

    /// Range covering `text[from..<to]`.
    func range(_ from: Int, _ to: Int) -> PineSourceRange {
        .init(
            start: .init(line: number, column: from + 1, offset: startOffset + utf16Prefix[from]),
            end: .init(line: number, column: to + 1, offset: startOffset + utf16Prefix[to]))
    }
}
