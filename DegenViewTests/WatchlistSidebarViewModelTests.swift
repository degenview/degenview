import XCTest

@testable import DegenView

@MainActor
final class WatchlistSidebarViewModelTests: XCTestCase {
    private var store: WatchlistStore!
    private var viewModel: WatchlistSidebarViewModel!

    override func setUpWithError() throws {
        store = WatchlistStore(database: try AppDatabase.makeInMemory())
        viewModel = WatchlistSidebarViewModel(store: store, quotes: WatchlistQuoteBook())
    }

    private func item(_ symbol: String) -> WatchlistInstrument {
        WatchlistInstrument(instrument: InstrumentID(source: .binance, symbol: symbol), name: symbol, label: symbol)
    }

    func testStartsOnFavoritesAndFollowsExplicitSelection() throws {
        XCTAssertEqual(viewModel.list?.isFavorites, true)
        let crypto = try store.createWatchlist(name: "Crypto")

        viewModel.select(crypto.id)

        XCTAssertEqual(viewModel.list?.id, crypto.id)
        XCTAssertEqual(store.lastSelectedID, crypto.id)
    }

    func testSelectionFallsBackWhenAnotherWindowDeletesTheList() throws {
        let crypto = try store.createWatchlist(name: "Crypto")
        viewModel.select(crypto.id)

        try store.deleteWatchlist(id: crypto.id)

        XCTAssertEqual(viewModel.list?.isFavorites, true)
    }

    func testTwoWindowsKeepTheirOwnSelectionButShareContent() throws {
        let other = WatchlistSidebarViewModel(store: store, quotes: WatchlistQuoteBook())
        let crypto = try store.createWatchlist(name: "Crypto")
        viewModel.select(crypto.id)

        XCTAssertEqual(other.list?.isFavorites, true)
        try store.addInstrument(item("BTCUSDT"), to: crypto.id)
        other.select(crypto.id)
        XCTAssertEqual(other.list?.instruments.count, 1)
        try store.renameWatchlist(id: crypto.id, name: "Majors")
        XCTAssertEqual(viewModel.list?.name, "Majors")
        XCTAssertEqual(other.list?.name, "Majors")
    }

    func testSortHeaderCyclesNewKeyThenFlipThenManual() throws {
        viewModel.toggleSort(.changePercent)
        XCTAssertEqual(viewModel.list?.display.sort, WatchlistSort(key: .changePercent, ascending: false))
        viewModel.toggleSort(.changePercent)
        XCTAssertEqual(viewModel.list?.display.sort, WatchlistSort(key: .changePercent, ascending: true))
        viewModel.toggleSort(.changePercent)
        XCTAssertEqual(viewModel.list?.display.sort, .manual)

        viewModel.toggleSort(.symbol)
        XCTAssertEqual(viewModel.list?.display.sort, WatchlistSort(key: .symbol, ascending: true))
    }

    func testReorderingIsOnlyAllowedWhileUnsortedAndUnfiltered() throws {
        XCTAssertTrue(viewModel.canReorder)
        viewModel.filterText = "btc"
        XCTAssertFalse(viewModel.canReorder)
        viewModel.clearFilter()
        viewModel.toggleSort(.last)
        XCTAssertFalse(viewModel.canReorder)
        viewModel.setSort(.manual)
        XCTAssertTrue(viewModel.canReorder)
    }

    func testSortingNeverChangesTheStoredOrder() throws {
        let favorites = try XCTUnwrap(store.favorites)
        for symbol in ["CCC", "AAA", "BBB"] { try store.addInstrument(item(symbol), to: favorites.id) }

        viewModel.toggleSort(.symbol)

        XCTAssertEqual(store.favorites?.instruments.map(\.instrument.symbol), ["CCC", "AAA", "BBB"])
        let shown = viewModel.rows.compactMap { row -> String? in
            if case .instrument(let item, _) = row { return item.instrument.symbol }
            return nil
        }
        XCTAssertEqual(shown, ["AAA", "BBB", "CCC"])
    }

    func testColumnsKeepTheirFixedOrderAndDroppingASortedColumnResetsSort() throws {
        viewModel.setColumn(.volume, visible: true)
        viewModel.setColumn(.change, visible: true)
        XCTAssertEqual(viewModel.list?.display.columns, [.last, .change, .changePercent, .volume])

        viewModel.setSort(WatchlistSort(key: .volume, ascending: false))
        viewModel.setColumn(.volume, visible: false)

        XCTAssertEqual(viewModel.list?.display.sort, .manual)
        XCTAssertEqual(viewModel.list?.display.columns, [.last, .change, .changePercent])
    }

    func testMinimumWidthLeavesRoomForEveryColumnAndGrowsWithMore() throws {
        let base = viewModel.minimumWidth
        XCTAssertGreaterThanOrEqual(viewModel.defaultWidth, base)
        viewModel.setColumn(.volume, visible: true)
        XCTAssertGreaterThan(viewModel.minimumWidth, base)
        XCTAssertGreaterThanOrEqual(viewModel.defaultWidth, viewModel.minimumWidth)
        for column in WatchlistColumn.allCases { viewModel.setColumn(column, visible: true) }
        XCTAssertLessThan(viewModel.minimumWidth, WatchlistSidebarViewModel.maximumWidth)
    }

    func testTheDefaultWidthNeverClipsTheDefaultColumns() {
        XCTAssertGreaterThanOrEqual(viewModel.defaultWidth, viewModel.minimumWidth)
        XCTAssertEqual(viewModel.defaultWidth, UI.watchlistSidebarBaseWidth, accuracy: 0.5)
    }

    /// root: R | Majors: A B | Watching: C — rows in the order the list shows them.
    private func seedSections() throws -> (list: Watchlist, majors: WatchlistSection, watching: WatchlistSection) {
        let favorites = try XCTUnwrap(store.favorites)
        try store.addInstrument(item("ROOT"), to: favorites.id)
        let majors = try store.addSection(title: "Majors", in: favorites.id)
        try store.addInstrument(item("AAA"), to: favorites.id, section: majors.id)
        try store.addInstrument(item("BBB"), to: favorites.id, section: majors.id)
        let watching = try store.addSection(title: "Watching", in: favorites.id)
        try store.addInstrument(item("CCC"), to: favorites.id, section: watching.id)
        return (try XCTUnwrap(store.favorites), majors, watching)
    }

    private func order() -> [String] {
        (store.favorites?.entries ?? []).map {
            switch $0 {
            case .instrument(let item): return item.instrument.symbol
            case .section(let section): return "#" + section.title
            }
        }
    }

    private func rowIndex(of symbol: String) throws -> Int {
        try XCTUnwrap(
            viewModel.rows.firstIndex {
                if case .instrument(let item, _) = $0 { return item.instrument.symbol == symbol }
                return false
            })
    }

    func testDraggingASymbolToAnotherSectionMovesItThere() throws {
        _ = try seedSections()
        // Rows: ROOT, #Majors, AAA, BBB, #Watching, CCC. Drop AAA between #Watching and CCC.
        viewModel.moveRows(fromOffsets: IndexSet(integer: try rowIndex(of: "AAA")), toOffset: try rowIndex(of: "CCC"))

        XCTAssertEqual(order(), ["ROOT", "#Majors", "BBB", "#Watching", "AAA", "CCC"])
    }

    func testDroppingAfterASectionsLastSymbolKeepsItInThatSection() throws {
        _ = try seedSections()
        // Drop ROOT just above #Watching, which is the end of Majors.
        let heading = try XCTUnwrap(viewModel.rows.firstIndex { if case .section(let s, _) = $0 { return s.title == "Watching" } else { return false } })
        viewModel.moveRows(fromOffsets: IndexSet(integer: try rowIndex(of: "ROOT")), toOffset: heading)

        XCTAssertEqual(order(), ["#Majors", "AAA", "BBB", "ROOT", "#Watching", "CCC"])
    }

    func testDroppingUnderAHeadingPutsTheSymbolFirstInThatSection() throws {
        _ = try seedSections()
        // Destination = the row right under #Majors, i.e. before AAA.
        viewModel.moveRows(fromOffsets: IndexSet(integer: try rowIndex(of: "CCC")), toOffset: try rowIndex(of: "AAA"))

        XCTAssertEqual(order(), ["ROOT", "#Majors", "CCC", "AAA", "BBB", "#Watching"])
    }

    func testDroppingAtTheBottomOfTheListAppendsToTheLastSection() throws {
        _ = try seedSections()
        viewModel.moveRows(fromOffsets: IndexSet(integer: try rowIndex(of: "AAA")), toOffset: viewModel.rows.count)

        XCTAssertEqual(order(), ["ROOT", "#Majors", "BBB", "#Watching", "CCC", "AAA"])
    }

    func testDraggingAHeadingMovesItsSymbolsWithIt() throws {
        let seeded = try seedSections()
        let majorsRow = try XCTUnwrap(
            viewModel.rows.firstIndex { if case .section(let s, _) = $0 { return s.id == seeded.majors.id } else { return false } })

        viewModel.moveRows(fromOffsets: IndexSet(integer: majorsRow), toOffset: viewModel.rows.count)

        XCTAssertEqual(order(), ["ROOT", "#Watching", "CCC", "#Majors", "AAA", "BBB"])
    }

    func testAnEmptySectionIsAValidDropTarget() throws {
        let favorites = try XCTUnwrap(store.favorites)
        try store.addInstrument(item("AAA"), to: favorites.id)
        try store.addSection(title: "Ideas", in: favorites.id)
        // Rows: AAA, #Ideas, <placeholder>. Drop AAA onto the placeholder.
        XCTAssertEqual(viewModel.rows.count, 3)
        viewModel.moveRows(fromOffsets: IndexSet(integer: 0), toOffset: 2)

        XCTAssertEqual(order(), ["#Ideas", "AAA"])
    }

    func testDroppingWhereItAlreadyIsWritesNothing() throws {
        _ = try seedSections()
        let before = store.favorites?.updatedAt
        let aaa = try rowIndex(of: "AAA")

        viewModel.moveRows(fromOffsets: IndexSet(integer: aaa), toOffset: aaa)
        viewModel.moveRows(fromOffsets: IndexSet(integer: aaa), toOffset: aaa + 1)

        XCTAssertEqual(store.favorites?.updatedAt, before)
        XCTAssertEqual(order(), ["ROOT", "#Majors", "AAA", "BBB", "#Watching", "CCC"])
    }

    func testWhileSortedASymbolCanStillMoveToAnotherSection() throws {
        _ = try seedSections()
        viewModel.setSort(WatchlistSort(key: .symbol, ascending: false))
        let before = store.favorites?.display.sort
        // Rows (sorted within sections): ROOT, #Majors, BBB, AAA, #Watching, CCC. Drop AAA under CCC.
        viewModel.moveRows(fromOffsets: IndexSet(integer: try rowIndex(of: "AAA")), toOffset: viewModel.rows.count)

        XCTAssertEqual(order(), ["ROOT", "#Majors", "BBB", "#Watching", "CCC", "AAA"], "last in the stored order of Watching")
        XCTAssertEqual(store.favorites?.display.sort, before, "the sort is untouched")
        // It shows where the sort puts it: CCC then AAA is A..C descending, so CCC first.
        let shown = viewModel.rows.compactMap { row -> String? in
            if case .instrument(let item, _) = row { return item.instrument.symbol }
            return nil
        }
        XCTAssertEqual(shown, ["ROOT", "BBB", "CCC", "AAA"])
    }

    func testWhileSortedADropInsideTheSameSectionDoesNothing() throws {
        _ = try seedSections()
        viewModel.setSort(WatchlistSort(key: .symbol, ascending: false))
        let expected = order()
        let updated = store.favorites?.updatedAt

        viewModel.moveRows(fromOffsets: IndexSet(integer: try rowIndex(of: "AAA")), toOffset: try rowIndex(of: "BBB"))
        viewModel.moveRows(fromOffsets: IndexSet(integer: try rowIndex(of: "BBB")), toOffset: try rowIndex(of: "AAA") + 1)

        XCTAssertEqual(order(), expected)
        XCTAssertEqual(store.favorites?.updatedAt, updated, "nothing was written")
    }

    func testWhileSortedASymbolCanMoveToTheRootAndAHeadingStillReorders() throws {
        let seeded = try seedSections()
        viewModel.setSort(WatchlistSort(key: .symbol, ascending: true))

        viewModel.moveRows(fromOffsets: IndexSet(integer: try rowIndex(of: "AAA")), toOffset: 0)
        XCTAssertEqual(order(), ["ROOT", "AAA", "#Majors", "BBB", "#Watching", "CCC"])

        let majorsRow = try XCTUnwrap(
            viewModel.rows.firstIndex { if case .section(let s, _) = $0 { return s.id == seeded.majors.id } else { return false } })
        viewModel.moveRows(fromOffsets: IndexSet(integer: majorsRow), toOffset: viewModel.rows.count)
        XCTAssertEqual(order(), ["ROOT", "AAA", "#Watching", "CCC", "#Majors", "BBB"], "headings reorder while sorted")
    }

    func testWhileFilteredASymbolCanMoveBetweenVisibleSections() throws {
        _ = try seedSections()
        viewModel.filterText = "BBB"  // only BBB matches (the provider name "Binance" would match "B" everywhere)
        let favoritesBefore = store.favorites?.entries.count
        // Rows now: #Majors, BBB. Nothing below it is visible, so the drop target is Majors itself: no change.
        viewModel.moveRows(fromOffsets: IndexSet(integer: try rowIndex(of: "BBB")), toOffset: viewModel.rows.count)
        XCTAssertEqual(order(), ["ROOT", "#Majors", "AAA", "BBB", "#Watching", "CCC"])

        viewModel.filterText = "CCC"  // only CCC (in Watching) matches; drop it at the very top = the root
        viewModel.moveRows(fromOffsets: IndexSet(integer: try rowIndex(of: "CCC")), toOffset: 0)
        XCTAssertEqual(order(), ["ROOT", "CCC", "#Majors", "AAA", "BBB", "#Watching"])
        XCTAssertEqual(store.favorites?.entries.count, favoritesBefore, "nothing was added or lost")
        XCTAssertEqual(viewModel.filterText, "CCC", "the filter is left alone")
    }

    func testRowsAreDraggableWheneverAListIsShown() throws {
        XCTAssertTrue(viewModel.canDrag)
        viewModel.setSort(WatchlistSort(key: .last, ascending: true))
        viewModel.filterText = "x"
        XCTAssertTrue(viewModel.canDrag)
        XCTAssertFalse(viewModel.canReorder, "but positions only mean something in Manual order")
    }

    func testTheHighlightedRowFollowsTheFocusedChartsMarket() throws {
        let favorites = try XCTUnwrap(store.favorites)
        try store.addInstrument(item("BTCUSDT"), to: favorites.id)
        let entry = try XCTUnwrap(store.favorites?.instruments.first)

        viewModel.syncSelection(to: InstrumentID(source: .binance, symbol: "BTC"))
        XCTAssertEqual(viewModel.selectedEntryID, entry.id, "a bare BTC chart is the BTCUSDT row")

        viewModel.syncSelection(to: InstrumentID(source: .coinbase, symbol: "BTC-USD"))
        XCTAssertNil(viewModel.selectedEntryID, "another provider's market is not in the list")

        viewModel.syncSelection(to: nil)
        XCTAssertNil(viewModel.selectedEntryID)
    }

    func testErrorsSurfaceAsMessagesInsteadOfThrowing() throws {
        viewModel.createWatchlist(named: "   ")
        XCTAssertEqual(viewModel.errorMessage, WatchlistError.emptyName.localizedDescription)
    }

    func testDeletingTheCurrentListFallsBackToFavorites() throws {
        viewModel.createWatchlist(named: "Crypto")
        XCTAssertEqual(viewModel.list?.name, "Crypto")

        viewModel.deleteCurrent()

        XCTAssertEqual(viewModel.list?.isFavorites, true)
        XCTAssertEqual(store.lists.count, 1)
    }

    func testFilterIsLocalToTheWindowAndNeverEditsTheList() throws {
        let favorites = try XCTUnwrap(store.favorites)
        try store.addInstrument(item("BTCUSDT"), to: favorites.id)
        try store.addInstrument(item("ETHUSDT"), to: favorites.id)
        let before = store.favorites

        viewModel.filterText = "btc"

        XCTAssertEqual(viewModel.rows.count, 1)
        XCTAssertEqual(store.favorites, before)
        viewModel.clearFilter()
        XCTAssertEqual(viewModel.rows.count, 2)
    }
}
