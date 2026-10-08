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

    /// A drop while the visible order is derived (a sort or a filter): symbols change section, not position.
    struct SectionMove: Equatable {
        /// The symbols that are not already in the target section, in displayed order.
        var ids: [UUID]
        /// The target section; nil is the root, above the first heading.
        var section: UUID?
    }

    /// Resolves a native row move of *symbols* when the rows on screen are not in stored order. A symbol's place inside
    /// a section is decided by the sort, so only its section can change: the section that owns the row just above the
    /// insertion line (a symbol's section, a heading's own, an empty-section placeholder's, or the root when the drop is
    /// above everything). Symbols already in that section stay put. Headings are not handled here; they move through
    /// `resolveMove`, since section order is never sorted.
    static func resolveSectionMove(rows: [Row], sources: IndexSet, destination: Int) -> SectionMove? {
        var moved: [(id: UUID, section: UUID?)] = []
        for index in sources.sorted() where rows.indices.contains(index) {
            if case .instrument(let item, let sectionID) = rows[index] { moved.append((item.id, sectionID)) }
        }
        guard !moved.isEmpty else { return nil }
        let above = min(max(destination, 0), rows.count)
        let target = above > 0 ? rows[above - 1].owningSectionID : nil
        let ids = moved.filter { $0.section != target }.map(\.id)
        guard !ids.isEmpty else { return nil }
        return SectionMove(ids: ids, section: target)
    }
}

extension WatchlistLayoutEngine.Row {
    /// The section this row belongs to or heads; nil for a symbol at the root.
    var owningSectionID: UUID? {
        switch self {
        case .instrument(_, let sectionID): return sectionID
        case .section(let section, _): return section.id
        case .emptySection(let sectionID): return sectionID
        }
    }
}
