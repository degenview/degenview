import Foundation

/// One window's view of the shared watchlists: which list it shows, its filter and its
/// selection. The lists themselves live in `WatchlistStore`; none of this is shared or saved.
@MainActor
final class WatchlistSidebarViewModel: ObservableObject {
    let store: WatchlistStore
    let quotes: WatchlistQuoteBook
    private let coordinator: WatchlistQuoteCoordinator
    /// This window's identity as a quote consumer.
    private let consumerID = UUID()

    @Published private(set) var selectedID: UUID?
    @Published var filterText = ""
    @Published var isFilterVisible = false
    @Published var flagFilter: WatchlistFlag?
    /// The highlighted row (an entry id), used by the detail panel and the keyboard.
    @Published var selectedEntryID: UUID?
    @Published var showsDetail = false
    @Published var errorMessage: String?

    init(
        store: WatchlistStore? = nil, quotes: WatchlistQuoteBook? = nil,
        coordinator: WatchlistQuoteCoordinator? = nil
    ) {
        let store = store ?? .shared
        self.store = store
        self.quotes = quotes ?? .shared
        self.coordinator = coordinator ?? .shared
        selectedID = store.resolvedSelection(nil)
    }

    /// The list shown, falling back when the chosen one was deleted (here or in another window).
    var list: Watchlist? {
        store.resolvedSelection(selectedID).flatMap(store.list)
    }

    var filter: WatchlistLayoutEngine.Filter {
        .init(text: filterText, flag: flagFilter)
    }

    var isFiltering: Bool { filter.isActive }

    /// Dragging reorders the stored list, which only makes sense while it is shown unsorted and whole.
    var canReorder: Bool {
        !isFiltering && (list?.display.sort.key ?? .manual) == .manual
    }

    var rows: [WatchlistLayoutEngine.Row] {
        guard let list else { return [] }
        return WatchlistLayoutEngine.rows(
            in: list, filter: filter, flags: store.flags, sort: list.display.sort,
            quote: { [quotes] in quotes.quote(for: $0) })
    }

    var selectedInstrument: WatchlistInstrument? {
        guard let id = selectedEntryID else { return nil }
        return list?.instruments.first { $0.id == id }
    }

    /// The narrowest the panel can be while every column of the list still fits, with room left for a name.
    var minimumWidth: CGFloat {
        let columns = list?.display.columns ?? WatchlistDisplaySettings().columns
        // Row padding, icon and gaps, a name's minimum, then each column and the gap before it.
        return 124 + columns.reduce(0) { $0 + $1.width + 6 }
    }

    /// The width before the user drags the divider: roomy for the list's columns.
    var defaultWidth: CGFloat {
        max(minimumWidth, UI.watchlistSidebarWidth(extraColumns: max(0, (list?.display.columns.count ?? 2) - 2)))
    }

    /// The widest the user may drag it.
    static let maximumWidth: CGFloat = 640

    // MARK: Quotes

    /// Markets worth pricing now: every row that is on screen, meaning not hidden in a folded section.
    var quoteInstruments: [InstrumentID] {
        list?.visibleInstruments.map(\.instrument) ?? []
    }

    /// Tells the quote coordinator what this window needs. Called when the sidebar appears, the list's
    /// contents change, or the window is shown or hidden.
    func syncQuotes(isWindowVisible: Bool) {
        coordinator.update(consumer: consumerID, instruments: quoteInstruments, isActive: isWindowVisible)
    }

    /// The sidebar went away: stop asking for prices on its behalf.
    func stopQuotes() {
        coordinator.release(consumer: consumerID)
    }

    // MARK: Selection

    func select(_ id: UUID) {
        guard store.list(id) != nil else { return }
        selectedID = id
        selectedEntryID = nil
        flagFilter = nil
        filterText = ""
        store.lastSelectedID = id
    }

    func clearFilter() {
        filterText = ""
        flagFilter = nil
        isFilterVisible = false
    }

    // MARK: Actions

    /// Runs a store change, surfacing its error text instead of throwing into a view.
    @discardableResult
    func perform(_ change: () throws -> Void) -> Bool {
        do {
            try change()
            return true
        } catch {
            errorMessage = error.localizedDescription
            return false
        }
    }

    func createWatchlist(named name: String) {
        perform {
            let created = try store.createWatchlist(name: name)
            select(created.id)
        }
    }

    func renameCurrent(to name: String) {
        guard let list else { return }
        perform { try store.renameWatchlist(id: list.id, name: name) }
    }

    func duplicateCurrent(named name: String) {
        guard let list else { return }
        perform {
            let copy = try store.duplicateWatchlist(id: list.id, name: name)
            select(copy.id)
        }
    }

    func deleteCurrent() {
        guard let list else { return }
        perform {
            try store.deleteWatchlist(id: list.id)
            selectedID = store.resolvedSelection(nil)
        }
    }

    func addSection(named title: String) {
        guard let list else { return }
        perform { try store.addSection(title: title, in: list.id) }
    }

    func toggleCollapsed(_ section: WatchlistSection) {
        guard let list else { return }
        perform { try store.setSectionCollapsed(section.id, !section.isCollapsed, in: list.id) }
    }

    func remove(_ item: WatchlistInstrument) {
        guard let list else { return }
        if selectedEntryID == item.id { selectedEntryID = nil }
        perform { try store.removeEntry(item.id, from: list.id) }
    }

    func deleteSection(_ section: WatchlistSection) {
        guard let list else { return }
        perform { try store.deleteSection(section.id, in: list.id) }
    }

    func add(_ result: TickerSearchResult) throws {
        guard let list else { throw WatchlistError.notFound }
        try store.add(result, to: list.id)
    }

    /// Click on a column header: new key first, then flip direction, then back to manual.
    func toggleSort(_ key: WatchlistSortKey) {
        guard let list else { return }
        let current = list.display.sort
        let next: WatchlistSort
        if current.key != key {
            // Prices and moves read best biggest-first; names read A to Z.
            next = WatchlistSort(key: key, ascending: key == .symbol)
        } else if current.ascending == (key == .symbol) {
            next = WatchlistSort(key: key, ascending: key != .symbol)
        } else {
            next = .manual
        }
        setSort(next)
    }

    func setSort(_ sort: WatchlistSort) {
        guard let list else { return }
        perform { try store.updateDisplay(of: list.id) { $0.sort = sort } }
    }

    func setColumn(_ column: WatchlistColumn, visible: Bool) {
        guard let list else { return }
        perform {
            try store.updateDisplay(of: list.id) { display in
                var columns = display.columns.filter { $0 != column }
                if visible { columns.append(column) }
                // Keep the fixed on-screen order whatever order they were switched on in.
                display.columns = WatchlistColumn.allCases.filter(columns.contains)
                if !visible, display.sort.key.column == column { display.sort = .manual }
            }
        }
    }

    func setShowsDescription(_ shows: Bool) {
        guard let list else { return }
        perform { try store.updateDisplay(of: list.id) { $0.showsDescription = shows } }
    }

    // MARK: Reordering

    /// The list's native row move (`ForEach.onMove`): `toOffset` is an index into the rows as shown, which
    /// maps straight onto the flat entry list, so a drop between sections, under a heading or after a
    /// section's last symbol all land where the insertion line was. One drop is one store write.
    ///
    /// While a sort or filter is on, the rows are not in stored order, so a symbol's place inside a section is not
    /// the user's to choose: a drop only changes its section (it goes last in that section's stored order and the
    /// sort places it). Headings are never sorted, so they reorder in every mode.
    func moveRows(fromOffsets: IndexSet, toOffset: Int) {
        guard let list else { return }
        let shown = rows
        let movesHeading = fromOffsets.contains { index in
            guard shown.indices.contains(index), case .section = shown[index] else { return false }
            return true
        }
        if canReorder || movesHeading {
            guard let move = WatchlistLayoutEngine.resolveMove(rows: shown, sources: fromOffsets, destination: toOffset)
            else { return }
            perform { try store.moveEntries(move.ids, before: move.before, in: list.id) }
        } else if let move = WatchlistLayoutEngine.resolveSectionMove(
            rows: shown, sources: fromOffsets, destination: toOffset)
        {
            perform { try store.moveInstruments(move.ids, toSection: move.section, in: list.id) }
        }
    }

    /// Rows can be dragged whenever a list is shown and editable; what a drop means depends on `canReorder`.
    var canDrag: Bool { list != nil && !store.loadFailed }

    /// Highlights the row for the market the focused chart shows, or nothing when the list does not hold it.
    /// Selecting a row opens it, so this also makes clicking a symbol again work after another chart took focus.
    func syncSelection(to market: InstrumentID?) {
        let target = market.flatMap { market in
            list?.instruments.first { $0.instrument.isSameMarket(as: market) }?.id
        }
        if selectedEntryID != target { selectedEntryID = target }
    }

    // MARK: Import and export

    func importText(_ text: String) -> WatchlistImportReport? {
        guard let list else { return nil }
        var report: WatchlistImportReport?
        perform { report = try store.importText(text, into: list.id) }
        return report
    }

    var exportText: String? {
        list.flatMap { store.exportText(of: $0.id) }
    }
}

extension WatchlistSortKey {
    /// The column a sort key belongs to, if it has one.
    var column: WatchlistColumn? {
        switch self {
        case .manual, .symbol: return nil
        case .last: return .last
        case .change: return .change
        case .changePercent: return .changePercent
        case .volume: return .volume
        }
    }
}

extension WatchlistColumn {
    var sortKey: WatchlistSortKey? {
        switch self {
        case .last: return .last
        case .change: return .change
        case .changePercent: return .changePercent
        case .volume: return .volume
        case .exchange, .status: return nil
        }
    }

    /// Width the column takes in the sidebar.
    var width: CGFloat {
        switch self {
        case .last: return 70
        case .change: return 62
        case .changePercent: return 64
        case .volume: return 58
        case .exchange: return 72
        case .status: return 66
        }
    }
}
