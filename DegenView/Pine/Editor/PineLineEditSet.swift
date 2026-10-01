import Foundation

/// Several small edits at known places (indent every selected line, comment every selected
/// line) folded into the one contiguous replacement a text view can undo in a single step.
struct PineLineEditSet {
    struct Edit: Equatable {
        let location: Int
        let removeLength: Int
        let insert: String
    }

    /// Ascending and non-overlapping.
    private(set) var edits: [Edit] = []

    var isEmpty: Bool { edits.isEmpty }

    mutating func add(location: Int, remove: Int = 0, insert: String = "") {
        edits.append(Edit(location: location, removeLength: remove, insert: insert))
    }

    /// Applies the edits to `source` as one replacement and carries `selection` across them.
    ///
    /// An offset inside removed text lands where that text began. The offset of a collapsed
    /// caret moves past an insertion made exactly at it; the ends of a real selection do not,
    /// so a selection that starts at the beginning of a line keeps covering the new indentation.
    func makeEdit(in source: NSString, selection: NSRange) -> PineEditorEdit? {
        guard let first = edits.first, let last = edits.last else { return nil }
        let blockEnd = last.location + last.removeLength
        var replacement = ""
        var cursor = first.location
        for edit in edits {
            replacement += source.substring(with: NSRange(location: cursor, length: edit.location - cursor))
            replacement += edit.insert
            cursor = edit.location + edit.removeLength
        }
        replacement += source.substring(with: NSRange(location: cursor, length: blockEnd - cursor))

        let collapsed = selection.length == 0
        let start = map(selection.location, inclusive: collapsed)
        let end = collapsed ? start : map(NSMaxRange(selection), inclusive: false)
        return PineEditorEdit(
            range: NSRange(location: first.location, length: blockEnd - first.location),
            replacement: replacement,
            selection: NSRange(location: start, length: end - start))
    }

    private func map(_ offset: Int, inclusive: Bool) -> Int {
        var delta = 0
        for edit in edits {
            if inclusive ? offset < edit.location : offset <= edit.location { break }
            let removedEnd = edit.location + edit.removeLength
            let inserted = edit.insert.utf16.count
            if offset >= removedEnd {
                delta += inserted - edit.removeLength
            } else {
                return edit.location + delta + inserted
            }
        }
        return offset + delta
    }
}
