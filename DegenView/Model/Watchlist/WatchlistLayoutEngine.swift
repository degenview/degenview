import Foundation

/// Turns a watchlist plus the viewer's filter and sort into the rows the sidebar draws.
///
/// Pure: it never changes the stored order. Sorting happens inside each section, so
/// section headers stay where the user put them, and filtering hides sections left empty.
enum WatchlistLayoutEngine {
    enum Row: Identifiable, Equatable {
        case section(WatchlistSection, count: Int)
        case instrument(WatchlistInstrument, sectionID: UUID?)
        /// Placeholder under an expanded section that holds nothing.
        case emptySection(sectionID: UUID)

        var id: String {
            switch self {
            case .section(let section, _): return section.id.uuidString
            case .instrument(let item, _): return item.id.uuidString
            case .emptySection(let id): return "empty-\(id.uuidString)"
            }
        }

        /// The stored entry this row stands for, if any.
        var entryID: UUID? {
            switch self {
            case .section(let section, _): return section.id
            case .instrument(let item, _): return item.id
            case .emptySection: return nil
            }
        }
    }

    struct Filter: Equatable {
        var text = ""
        var flag: WatchlistFlag?

        var isActive: Bool { !trimmedText.isEmpty || flag != nil }
        var trimmedText: String { text.trimmingCharacters(in: .whitespacesAndNewlines) }
    }

    static func rows(
        in list: Watchlist, filter: Filter = Filter(), flags: [String: WatchlistFlag] = [:],
        sort: WatchlistSort = .manual, quote: (InstrumentID) -> WatchlistQuote? = { _ in nil }
    ) -> [Row] {
        var rows: [Row] = []
        for block in blocks(of: list) {
            var items = block.items.filter { matches($0, filter: filter, flags: flags) }
            if sort.key != .manual { items = sorted(items, by: sort, quote: quote) }

            guard let section = block.section else {
                rows.append(contentsOf: items.map { .instrument($0, sectionID: nil) })
                continue
            }
            if filter.isActive && items.isEmpty { continue }

            rows.append(.section(section, count: items.count))
            guard !section.isCollapsed else { continue }
            if items.isEmpty {
                rows.append(.emptySection(sectionID: section.id))
            } else {
                rows.append(contentsOf: items.map { .instrument($0, sectionID: section.id) })
            }
        }
        return rows
    }

    // MARK: Pieces

    private struct Block {
        var section: WatchlistSection?
        var items: [WatchlistInstrument] = []
    }

    /// The root block, then one block per section, in stored order.
    private static func blocks(of list: Watchlist) -> [Block] {
        var blocks = [Block()]
        for entry in list.entries {
            switch entry {
            case .section(let section): blocks.append(Block(section: section))
            case .instrument(let item): blocks[blocks.count - 1].items.append(item)
            }
        }
        return blocks
    }

    static func matches(
        _ item: WatchlistInstrument, filter: Filter, flags: [String: WatchlistFlag]
    ) -> Bool {
        if let flag = filter.flag, flags[item.instrument.key] != flag { return false }
        let needle = filter.trimmedText
        guard !needle.isEmpty else { return true }
        let haystack = [
            item.name, item.label, item.instrument.symbol, item.displayName ?? "",
            item.instrument.source.displayName,
        ]
        return haystack.contains { $0.range(of: needle, options: [.caseInsensitive, .diacriticInsensitive]) != nil }
    }

    /// Stable, with markets that have no value for the key always last.
    private static func sorted(
        _ items: [WatchlistInstrument], by sort: WatchlistSort, quote: (InstrumentID) -> WatchlistQuote?
    ) -> [WatchlistInstrument] {
        let keyed = items.enumerated().map { (offset: $0.offset, item: $0.element, value: value(of: $0.element, sort.key, quote)) }
        return keyed.sorted { lhs, rhs in
            switch (lhs.value, rhs.value) {
            case (nil, nil): return lhs.offset < rhs.offset
            case (nil, _): return false
            case (_, nil): return true
            case (let a?, let b?):
                if a == b { return lhs.offset < rhs.offset }
                return sort.ascending ? a < b : a > b
            }
        }
        .map(\.item)
    }

    private enum SortValue: Comparable {
        case number(Double)
        case text(String)

        static func < (lhs: SortValue, rhs: SortValue) -> Bool {
            switch (lhs, rhs) {
            case (.number(let a), .number(let b)): return a < b
            case (.text(let a), .text(let b)): return a.localizedStandardCompare(b) == .orderedAscending
            case (.number, .text): return true
            case (.text, .number): return false
            }
        }
    }

    private static func value(
        of item: WatchlistInstrument, _ key: WatchlistSortKey, _ quote: (InstrumentID) -> WatchlistQuote?
    ) -> SortValue? {
        switch key {
        case .manual: return nil
        case .symbol: return .text(item.label)
        case .last: return quote(item.instrument)?.last.map(SortValue.number)
        case .change: return quote(item.instrument)?.change.map(SortValue.number)
        case .changePercent: return quote(item.instrument)?.changePercent.map(SortValue.number)
        case .volume: return quote(item.instrument)?.volume.map(SortValue.number)
        }
    }
}
