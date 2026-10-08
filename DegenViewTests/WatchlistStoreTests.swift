import Combine
import GRDB
import XCTest

@testable import DegenView

@MainActor
final class WatchlistStoreTests: XCTestCase {
    private var database: AppDatabase!
    private var cancellables = Set<AnyCancellable>()

    override func setUpWithError() throws {
        database = try AppDatabase.makeInMemory()
    }

    private func makeStore() -> WatchlistStore {
        WatchlistStore(database: database, now: { Date(timeIntervalSince1970: 1_000) })
    }

    private func item(_ symbol: String, _ source: DataSourceType = .binance) -> WatchlistInstrument {
        WatchlistInstrument(instrument: InstrumentID(source: source, symbol: symbol), name: symbol, label: symbol)
    }

    /// The shape the old `favorite` table holds.
    private struct LegacyRow: Codable, Identifiable {
        let id: UUID
        let name: String
        let ticker: String
        let config: TickerConfig
    }

    private func legacy(_ symbol: String, _ source: DataSourceType = .binance, name: String? = nil) -> LegacyRow {
        LegacyRow(id: UUID(), name: name ?? symbol, ticker: symbol, config: TickerConfig(symbol: symbol, source: source))
    }

    // MARK: Migration

    func testFreshInstallGetsAnEmptyFavoritesList() throws {
        let store = makeStore()

        XCTAssertFalse(store.loadFailed)
        XCTAssertEqual(store.lists.count, 1)
        XCTAssertEqual(store.favorites?.name, "Favorites")
        XCTAssertTrue(store.favorites?.entries.isEmpty ?? false)
    }

    func testExistingFavoritesBecomeTheDefaultListInOrderWithTheirIds() throws {
        let rows = [legacy("BTCUSDT", name: "Bitcoin"), legacy("ETH-USD", .coinbase), legacy("btcusdt")]
        database.replaceDocuments(rows, in: .favorite)

        let store = makeStore()

        let favorites = try XCTUnwrap(store.favorites)
        XCTAssertEqual(favorites.instruments.map(\.instrument.key), ["Binance:BTCUSDT", "Coinbase:ETH-USD"])
        XCTAssertEqual(favorites.instruments.map(\.id), [rows[0].id, rows[1].id])
        XCTAssertEqual(favorites.instruments.first?.name, "Bitcoin")
    }

    func testMigrationRunsOnceAndLeavesTheLegacyRowsAlone() throws {
        let rows = [legacy("BTCUSDT")]
        database.replaceDocuments(rows, in: .favorite)

        _ = makeStore()
        let second = makeStore()

        XCTAssertEqual(second.lists.count, 1)
        XCTAssertEqual(second.favorites?.instruments.count, 1)
        XCTAssertEqual(database.documents(LegacyRow.self, in: .favorite).count, 1)
    }

    func testRemovingEveryFavoriteDoesNotBringTheLegacyOnesBack() throws {
        database.replaceDocuments([legacy("BTCUSDT")], in: .favorite)
        let store = makeStore()
        let favorites = try XCTUnwrap(store.favorites)
        try store.removeEntry(try XCTUnwrap(favorites.instruments.first).id, from: favorites.id)

        XCTAssertTrue(makeStore().favorites?.instruments.isEmpty ?? false)
    }

    // MARK: Lists

    func testRenameKeepsIdAndPersists() throws {
        let store = makeStore()
        let list = try store.createWatchlist(name: "Crypto")
        try store.renameWatchlist(id: list.id, name: "Crypto Majors")

        XCTAssertEqual(store.list(list.id)?.name, "Crypto Majors")
        XCTAssertEqual(makeStore().list(list.id)?.name, "Crypto Majors")
    }

    func testListOrderSurvivesReload() throws {
        let store = makeStore()
        let a = try store.createWatchlist(name: "A")
        let b = try store.createWatchlist(name: "B")
        try store.moveWatchlist(b.id, before: a.id)

        XCTAssertEqual(makeStore().lists.map(\.name), ["Favorites", "B", "A"])
    }

    func testDuplicateCopiesContentUnderNewIds() throws {
        let store = makeStore()
        let list = try store.createWatchlist(name: "Crypto")
        try store.addInstrument(item("BTCUSDT"), to: list.id)
        try store.addSection(title: "Majors", in: list.id)

        let copy = try store.duplicateWatchlist(id: list.id)

        XCTAssertEqual(copy.name, "Crypto copy")
        XCTAssertNotEqual(copy.id, list.id)
        XCTAssertEqual(copy.entries.count, 2)
        XCTAssertTrue(Set(copy.entries.map(\.id)).isDisjoint(with: list.entries.map(\.id)))
        XCTAssertEqual(store.lists.map(\.name), ["Favorites", "Crypto", "Crypto copy"])
    }

    func testDeletingAListKeepsItsMarketsInOthersAndFavoritesCannotBeDeleted() throws {
        let store = makeStore()
        let a = try store.createWatchlist(name: "A")
        let b = try store.createWatchlist(name: "B")
        try store.addInstrument(item("BTCUSDT"), to: a.id)
        try store.addInstrument(item("BTCUSDT"), to: b.id)

        try store.deleteWatchlist(id: a.id)

        XCTAssertNil(store.list(a.id))
        XCTAssertTrue(store.list(b.id)?.contains(InstrumentID(source: .binance, symbol: "BTCUSDT")) ?? false)
        let favorites = try XCTUnwrap(store.favorites)
        XCTAssertThrowsError(try store.deleteWatchlist(id: favorites.id)) {
            XCTAssertEqual($0 as? WatchlistError, .lastFavorites)
        }
    }

    func testSelectionFallsBackWhenTheListIsGone() throws {
        let store = makeStore()
        let gone = UUID()
        XCTAssertEqual(store.resolvedSelection(gone), store.favorites?.id)
    }

    // MARK: Instruments, sections, flags

    func testSameMarketInManyListsButOnceInEach() throws {
        let store = makeStore()
        let a = try store.createWatchlist(name: "A")
        let b = try store.createWatchlist(name: "B")
        try store.addInstrument(item("BTCUSDT"), to: a.id)
        try store.addInstrument(item("BTCUSDT"), to: b.id)

        XCTAssertThrowsError(try store.addInstrument(item("BTCUSDT"), to: a.id))
        XCTAssertEqual(store.lists(containing: InstrumentID(source: .binance, symbol: "BTCUSDT")).count, 2)
    }

    func testMovesAndSectionsPersist() throws {
        let store = makeStore()
        let list = try store.createWatchlist(name: "Crypto")
        let majors = try store.addSection(title: "Majors", in: list.id)
        try store.addInstrument(item("BTCUSDT"), to: list.id, section: majors.id)
        try store.addInstrument(item("ETHUSDT"), to: list.id, section: majors.id)
        let eth = try XCTUnwrap(store.list(list.id)?.entry(for: InstrumentID(source: .binance, symbol: "ETHUSDT")))
        let btc = try XCTUnwrap(store.list(list.id)?.entry(for: InstrumentID(source: .binance, symbol: "BTCUSDT")))
        try store.moveEntry(eth.id, before: btc.id, in: list.id)
        try store.setSectionCollapsed(majors.id, true, in: list.id)

        let reloaded = try XCTUnwrap(makeStore().list(list.id))
        XCTAssertEqual(reloaded.instruments.map(\.instrument.symbol), ["ETHUSDT", "BTCUSDT"])
        XCTAssertEqual(reloaded.sections.first?.isCollapsed, true)
        XCTAssertEqual(reloaded.sections.first?.id, majors.id)
    }

    func testFlagsAreGlobalAndPersist() throws {
        let store = makeStore()
        let btc = InstrumentID(source: .binance, symbol: "BTCUSDT")
        let other = InstrumentID(source: .coinbase, symbol: "BTC-USD")
        try store.setFlag(.red, for: btc)

        XCTAssertEqual(makeStore().flag(for: btc), .red)
        XCTAssertNil(makeStore().flag(for: other))
        try store.setFlag(nil, for: btc)
        XCTAssertNil(makeStore().flag(for: btc))
    }

    func testDisplaySettingsPersistPerList() throws {
        let store = makeStore()
        let a = try store.createWatchlist(name: "A")
        let b = try store.createWatchlist(name: "B")
        try store.updateDisplay(of: a.id) {
            $0.columns = [.last, .volume]
            $0.sort = WatchlistSort(key: .changePercent, ascending: false)
        }

        let reloaded = makeStore()
        XCTAssertEqual(reloaded.list(a.id)?.display.columns, [.last, .volume])
        XCTAssertEqual(reloaded.list(a.id)?.display.sort.key, .changePercent)
        XCTAssertEqual(reloaded.list(b.id)?.display, WatchlistDisplaySettings())
    }

    func testToggleFavoriteAddsAndRemoves() throws {
        let store = makeStore()
        let btc = item("BTCUSDT")

        try store.toggleFavorite(btc)
        XCTAssertTrue(store.isFavorite(btc.instrument))
        try store.toggleFavorite(btc)
        XCTAssertFalse(store.isFavorite(btc.instrument))
    }

    func testSetChainUpdatesEveryListHoldingThePair() throws {
        let store = makeStore()
        let a = try store.createWatchlist(name: "A")
        let b = try store.createWatchlist(name: "B")
        let pair = InstrumentID(source: .dexscreener, symbol: "PAIR")
        for list in [a, b] { try store.addInstrument(item("PAIR", .dexscreener), to: list.id) }

        try store.setChain("solana", for: pair)

        XCTAssertEqual(makeStore().list(a.id)?.entry(for: pair)?.instrument.chain, "solana")
        XCTAssertEqual(makeStore().list(b.id)?.entry(for: pair)?.instrument.chain, "solana")
    }

    // MARK: Safety

    func testUnreadableRowDisablesWritesInsteadOfBeingOverwritten() throws {
        let store = makeStore()
        let list = try store.createWatchlist(name: "Crypto")
        let corrupt = UUID()
        try database.writer.write {
            try $0.execute(
                sql: "INSERT INTO watchlist (id, position, payload) VALUES (?, 99, ?)",
                arguments: [corrupt.uuidString, "{not json"])
        }

        let broken = makeStore()

        XCTAssertTrue(broken.loadFailed)
        XCTAssertTrue(broken.lists.isEmpty)
        XCTAssertThrowsError(try broken.createWatchlist(name: "New")) {
            XCTAssertEqual($0 as? WatchlistError, .persistenceUnavailable)
        }
        XCTAssertThrowsError(try broken.renameWatchlist(id: list.id, name: "X"))
        let rows = try database.reader.read { try Int.fetchOne($0, sql: "SELECT COUNT(*) FROM watchlist") }
        XCTAssertEqual(rows, 3)
    }

    func testFailedWriteLeavesMemoryUnchanged() throws {
        let store = makeStore()
        let list = try store.createWatchlist(name: "Crypto")
        try database.writer.write { try $0.execute(sql: "DROP TABLE watchlist") }

        XCTAssertThrowsError(try store.renameWatchlist(id: list.id, name: "Renamed"))
        XCTAssertEqual(store.list(list.id)?.name, "Crypto")
    }

    // MARK: Windows

    func testEveryWindowObservingTheSharedStoreSeesAChange() throws {
        let store = makeStore()
        var windowA: [[String]] = []
        var windowB: [[String]] = []
        store.$lists.sink { windowA.append($0.map(\.name)) }.store(in: &cancellables)
        store.$lists.sink { windowB.append($0.map(\.name)) }.store(in: &cancellables)

        let list = try store.createWatchlist(name: "Crypto")
        try store.renameWatchlist(id: list.id, name: "Crypto Majors")

        XCTAssertEqual(windowA.last, ["Favorites", "Crypto Majors"])
        XCTAssertEqual(windowB, windowA)
    }

    // MARK: Import

    func testImportKeepsGoodRowsReusesSectionsAndCountsDuplicates() throws {
        let store = makeStore()
        let list = try store.createWatchlist(name: "Crypto")
        try store.addInstrument(item("BTCUSDT"), to: list.id)

        let report = try store.importText(
            "###Majors\nBINANCE:BTCUSDT\nBINANCE:ETHUSDT\nNONSENSE\n###majors\nCOINBASE:SOL-USD", into: list.id)

        XCTAssertEqual(report.added, 2)
        XCTAssertEqual(report.duplicates, 1)
        XCTAssertEqual(report.sectionsAdded, 1)
        XCTAssertEqual(report.skipped.map(\.line), [4])
        let result = try XCTUnwrap(store.list(list.id))
        XCTAssertEqual(result.sections.count, 1)
        XCTAssertEqual(result.instruments.map(\.instrument.symbol), ["BTCUSDT", "ETHUSDT", "SOL-USD"])
        XCTAssertEqual(result.owningSection(of: try XCTUnwrap(result.instruments.last).id)?.title, "Majors")
    }

    func testExportThenImportIntoANewListReproducesIt() throws {
        let store = makeStore()
        let source = try store.createWatchlist(name: "Source")
        let section = try store.addSection(title: "Majors", in: source.id)
        try store.addInstrument(item("BTCUSDT"), to: source.id, section: section.id)
        try store.addInstrument(item("ETH-USD", .coinbase), to: source.id, section: section.id)
        let target = try store.createWatchlist(name: "Target")

        _ = try store.importText(try XCTUnwrap(store.exportText(of: source.id)), into: target.id)

        let copy = try XCTUnwrap(store.list(target.id))
        XCTAssertEqual(copy.instruments.map(\.instrument), try XCTUnwrap(store.list(source.id)).instruments.map(\.instrument))
        XCTAssertEqual(copy.sections.map(\.title), ["Majors"])
    }

    func testMovingSeveralEntriesIsOneCommitAndSurvivesReload() throws {
        let store = makeStore()
        let list = try store.createWatchlist(name: "Crypto")
        for symbol in ["AAA", "BBB", "CCC"] { try store.addInstrument(item(symbol), to: list.id) }
        let ids = try XCTUnwrap(store.list(list.id)).instruments.map(\.id)

        try store.moveEntries([ids[2], ids[0]], before: ids[1], in: list.id)

        let symbols = { (store: WatchlistStore) in store.list(list.id)?.instruments.map(\.instrument.symbol) }
        XCTAssertEqual(symbols(store), ["CCC", "AAA", "BBB"])
        XCTAssertEqual(symbols(makeStore()), ["CCC", "AAA", "BBB"])
    }

    func testADropThatChangesNothingDoesNotWrite() throws {
        let store = makeStore()
        let list = try store.createWatchlist(name: "Crypto")
        for symbol in ["AAA", "BBB"] { try store.addInstrument(item(symbol), to: list.id) }
        let ids = try XCTUnwrap(store.list(list.id)).instruments.map(\.id)
        let before = store.list(list.id)

        try store.moveEntries([ids[0]], before: ids[1], in: list.id)

        XCTAssertEqual(store.list(list.id), before)
    }

    func testMovingSymbolsToASectionIsOneCommitAndSurvivesReload() throws {
        let store = makeStore()
        let list = try store.createWatchlist(name: "Crypto")
        let ideas = try store.addSection(title: "Ideas", in: list.id)
        for symbol in ["AAA", "BBB"] { try store.addInstrument(item(symbol), to: list.id) }
        let ids = try XCTUnwrap(store.list(list.id)).instruments.map(\.id)

        try store.moveInstruments(ids, toSection: ideas.id, in: list.id)

        let reloaded = try XCTUnwrap(makeStore().list(list.id))
        XCTAssertEqual(reloaded.instruments.map(\.instrument.symbol), ["AAA", "BBB"])
        XCTAssertEqual(reloaded.owningSection(of: ids[0])?.id, ideas.id)
        XCTAssertEqual(reloaded.owningSection(of: ids[1])?.id, ideas.id)
    }
}
