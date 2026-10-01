import Foundation

/// Maps UTF-16 offsets in the compiler's line-ending-normalized source back to the editor text.
/// Only needed when the text holds `\r`, U+2028 or U+2029; plain `\n` text maps to itself.
struct PineOffsetMap {
    private let originalStarts: [Int]
    private let normalizedStarts: [Int]

    init(original: String, normalized: String) {
        originalStarts = Self.lineStarts(in: original)
        normalizedStarts = Self.lineStarts(in: normalized)
    }

    /// `line` is 1-based, as in `PineSourcePosition`.
    func original(line: Int, offset: Int) -> Int? {
        guard line >= 1, line <= originalStarts.count, line <= normalizedStarts.count else {
            return nil
        }
        return originalStarts[line - 1] + (offset - normalizedStarts[line - 1])
    }

    /// UTF-16 offset of each line's first character; `\r\n` is one separator.
    private static func lineStarts(in text: String) -> [Int] {
        var starts = [0]
        var previousWasCR = false
        var offset = 0
        for unit in text.utf16 {
            offset += 1
            switch unit {
            case 0x0D:
                starts.append(offset)
                previousWasCR = true
                continue
            case 0x0A:
                if previousWasCR { starts[starts.count - 1] = offset } else { starts.append(offset) }
            case 0x2028, 0x2029:
                starts.append(offset)
            default:
                break
            }
            previousWasCR = false
        }
        return starts
    }
}
