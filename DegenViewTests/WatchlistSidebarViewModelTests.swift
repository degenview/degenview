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

    func testSidebarWidthGrowsWithExtraColumnsUpToACap() throws {
        let base = viewModel.sidebarWidth
        viewModel.setColumn(.volume, visible: true)
        XCTAssertGreaterThan(viewModel.sidebarWidth, base)
        for column in WatchlistColumn.allCases { viewModel.setColumn(column, visible: true) }
        XCTAssertLessThanOrEqual(viewModel.sidebarWidth, 470)
    }

    func testDroppingAnInstrumentOnASectionHeaderPutsItFirstInThatSection() throws {
        let favorites = try XCTUnwrap(store.favorites)
        let majors = try store.addSection(title: "Majors", in: favorites.id)
        try store.addInstrument(item("AAA"), to: favorites.id, section: majors.id)
        try store.addInstrument(item("BBB"), to: favorites.id)
        let bbb = try XCTUnwrap(store.favorites?.instruments.last)

        viewModel.moveToTop(bbb.id, of: majors)

        let list = try XCTUnwrap(store.favorites)
        XCTAssertEqual(list.instruments.map(\.instrument.symbol), ["BBB", "AAA"])
        XCTAssertEqual(list.owningSection(of: bbb.id)?.id, majors.id)
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
