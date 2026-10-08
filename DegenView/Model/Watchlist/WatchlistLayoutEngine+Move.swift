import Foundation

extension WatchlistLayoutEngine {
    /// A drop in terms of stored entries: move `ids`, in this order, to sit before `before` (nil = the end).
    struct Move: Equatable {
        var ids: [UUID]
        var before: UUID?
    }

    /// Turns a native row move (`onMove`'s source rows and destination index, both in the rows as shown)
    /// into a move of stored entries.
    ///
    /// The stored list is flat, so the destination means exactly what the insertion line showed: before the
    /// row at `destination`, whatever section that row is in. Dropped between a section's last symbol and the
    /// next heading, a symbol lands before that heading, which is the end of its section; dropped under a
    /// heading it lands first in the section. Placeholder rows stand for no entry, so a drop on one resolves
    /// to the next real entry. Returns nil when the drop would change nothing.
    static func resolveMove(rows: [Row], sources: IndexSet, destination: Int) -> Move? {
        let moved = sources.sorted().compactMap { rows.indices.contains($0) ? rows[$0].entryID : nil }
        guard !moved.isEmpty else { return nil }
        let movedSet = Set(moved)

        var before: UUID?
        var index = max(0, destination)
        while index < rows.count {
            if let id = rows[index].entryID, !movedSet.contains(id) {
                before = id
                break
            }
            index += 1
        }

        let current = rows.compactMap(\.entryID)
        var result = current.filter { !movedSet.contains($0) }
        let insertion = before.flatMap { result.firstIndex(of: $0) } ?? result.count
        result.insert(contentsOf: moved, at: insertion)
        guard result != current else { return nil }
        return Move(ids: moved, before: before)
    }
}
