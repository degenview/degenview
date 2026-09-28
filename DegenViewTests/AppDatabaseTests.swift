import XCTest

@testable import DegenView

@MainActor
final class AppDatabaseTests: XCTestCase {
    private var directory: URL!
    private var database: AppDatabase!

    override func setUpWithError() throws {
        directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("AppDatabaseTests-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        database = try AppDatabase(path: directory.appendingPathComponent("test.sqlite").path)
    }

    override func tearDownWithError() throws {
        database = nil
        try? FileManager.default.removeItem(at: directory)
    }

    // MARK: - Legacy import

    func testLegacyFavoritesAreImportedOnceAndKeptAsBackup() throws {
        let favorites = [
            FavoriteItem(name: "Bitcoin", ticker: "BTC", config: TickerConfig(symbol: "BTCUSDT", source: .binance)),
            FavoriteItem(name: "Ether", ticker: "ETH", config: TickerConfig(symbol: "ETHUSDT", source: .binance)),
        ]
        try writeLegacy(favorites, to: "favorites.json")

        let store = FavoritesStore(database: database, legacyDirectory: directory)

        XCTAssertEqual(store.items, favorites)
        XCTAssertFalse(exists("favorites.json"))
        XCTAssertTrue(exists("favorites.migrated.json"))
        XCTAssertEqual(FavoritesStore(database: database, legacyDirectory: directory).items, favorites)
    }

    func testCorruptLegacyFileDoesNotBlockOtherImports() throws {
        try Data("not-json".utf8).write(to: directory.appendingPathComponent("favorites.json"))
        let view = SavedView(
            name: "Majors", tickers: ["BTCUSDT"], timeRange: .oneDay, layoutMode: .grid, createdAt: Date())
        try writeLegacy([view], to: "views.json")

        XCTAssertEqual(FavoritesStore(database: database, legacyDirectory: directory).items, [])
        XCTAssertEqual(database.savedViews(legacyDirectory: directory).map(\.id), [view.id])
        let quarantined = try FileManager.default.contentsOfDirectory(atPath: directory.path)
            .filter { $0.hasPrefix("favorites.corrupt-") }
        XCTAssertEqual(quarantined.count, 1)
    }

    func testLegacyTabsKeepOrderAndWindowGroups() throws {
        let first = ChartTab(name: "One")
        let second = ChartTab(name: "Two")
        try writeLegacy(
            TabsSnapshot(tabs: [first, second], windowGroups: [[second.id], [first.id]]), to: "tabs.json")

        let store = TabsStore(database: database, userDefaults: defaults(), supportDirectory: directory)

        XCTAssertEqual(store.tabs, [first, second])
        XCTAssertEqual(store.windowIndex(of: first.id), 1)
        XCTAssertTrue(exists("tabs.migrated.json"))
    }

    func testCorruptTabsRecoverIntoOneFreshTab() throws {
        let original = Data("broken-session".utf8)
        try original.write(to: directory.appendingPathComponent("tabs.json"))

        let store = TabsStore(database: database, userDefaults: defaults(), supportDirectory: directory)

        XCTAssertEqual(store.tabs.count, 1)
        XCTAssertEqual(try database.tabsSnapshot()?.tabs, store.tabs)
        let backups = try FileManager.default.contentsOfDirectory(at: directory, includingPropertiesForKeys: nil)
            .filter { $0.lastPathComponent.hasPrefix("tabs.corrupt-") }
        XCTAssertEqual(backups.count, 1)
        XCTAssertEqual(try Data(contentsOf: backups[0]), original)
    }

    func testLegacyDrawingsKeepExplicitlyEmptyInstruments() throws {
        let line = TrendLine(
            start: TrendAnchor(date: Date(timeIntervalSince1970: 1), price: 1),
            end: TrendAnchor(date: Date(timeIntervalSince1970: 2), price: 2))
        let btc = "\(DataSourceType.binance.rawValue):btcusdt"
        let eth = "\(DataSourceType.binance.rawValue):ethusdt"
        try writeLegacy([btc: [line], eth: []], to: "drawings.json")

        let store = DrawingStore(database: database, legacyDirectory: directory)

        XCTAssertEqual(store.linesByInstrument, [btc: [line], eth: []])
        store.importLegacy([line], ticker: "ETHUSDT", source: .binance)
        XCTAssertEqual(store.lines(ticker: "ETHUSDT", source: .binance), [])
    }

    // MARK: - Round trips

    func testTabsPersistAcrossStores() throws {
        let store = TabsStore(database: database, userDefaults: defaults(), supportDirectory: directory)
        let added = store.makeTab()
        store.setWindowGroups([store.tabs.map(\.id)])
        store.persist()

        let reloaded = TabsStore(database: database, userDefaults: defaults(), supportDirectory: directory)
        XCTAssertEqual(reloaded.tabs, store.tabs)
        XCTAssertEqual(reloaded.windowIndex(of: added.id), 0)
    }

    func testDeletedDrawingsStayDeleted() {
        let store = DrawingStore(database: database, legacyDirectory: directory)
        let line = TrendLine(
            start: TrendAnchor(date: Date(timeIntervalSince1970: 1), price: 1),
            end: TrendAnchor(date: Date(timeIntervalSince1970: 2), price: 2))
        store.save([line], ticker: "BTC", source: .binance)
        store.save([TrendLine](), ticker: "BTC", source: .binance)

        let reloaded = DrawingStore(database: database, legacyDirectory: directory)
        XCTAssertEqual(reloaded.linesByInstrument[reloaded.key(ticker: "BTC", source: .binance)], [])
    }

    // MARK: - Helpers

    private func writeLegacy<T: Encodable>(_ value: T, to filename: String) throws {
        try JSONEncoder().encode(value).write(to: directory.appendingPathComponent(filename))
    }

    private func exists(_ filename: String) -> Bool {
        FileManager.default.fileExists(atPath: directory.appendingPathComponent(filename).path)
    }

    private func defaults() -> UserDefaults {
        let suiteName = "AppDatabaseTests-\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suiteName)!
        addTeardownBlock { defaults.removePersistentDomain(forName: suiteName) }
        return defaults
    }
}
