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

    // MARK: - Portfolio

    func testLedgerRoundTripKeepsOrderPrecisionAndSelection() async throws {
        let asset = PortfolioAsset(key: "Binance:BTCUSDT", symbol: "BTC", name: "Bitcoin", source: .binance)
        let day = Date(timeIntervalSince1970: 1_735_689_600)
        let database = self.database!
        let ledger = PortfolioLedger(now: { day }, persist: { try database.savePortfolioLedger($0) })
        let id = try await ledger.createPortfolio(name: "Main", currency: .USD)
        _ = try await ledger.createPortfolio(name: "Other", currency: .EUR)
        try await ledger.select(id)
        try await ledger.add(
            PortfolioTransaction(
                portfolioID: id, asset: asset, type: .buy, quantity: Decimal(string: "0.123456789012345678")!,
                price: Decimal(string: "97123.45")!, timestamp: day.addingTimeInterval(86_400)))
        try await ledger.storeSnapshots(
            [
                PortfolioSnapshot(
                    portfolioID: id, timestamp: day, value: 1, netContributions: 1, realizedPnL: 0,
                    unrealizedPnL: 0, isComplete: true)
            ], for: id, from: day)
        try await ledger.add(
            PortfolioTransaction(
                portfolioID: id, asset: asset, type: .sell, quantity: Decimal(string: "0.1")!,
                price: 100_000, timestamp: day.addingTimeInterval(2 * 86_400)))

        let expected = await ledger.snapshot()
        XCTAssertFalse(expected.invalidatedAfter.isEmpty)
        XCTAssertEqual(try database.portfolioLedger(), expected)
    }

    func testLegacyPortfolioAndReportingCurrenciesImport() throws {
        let portfolio = Portfolio(name: "Legacy", baseCurrency: .USD, createdAt: Date(), updatedAt: Date())
        var legacy = PortfolioLedgerSnapshot()
        legacy.portfolios = [portfolio]
        legacy.selectedPortfolioID = portfolio.id
        try writeLegacy(legacy, to: "portfolios.json")
        try Data(#"{"currencies":{"all-portfolios":"EUR"}}"#.utf8)
            .write(to: directory.appendingPathComponent("portfolio_reporting_currencies.json"))

        let store = PortfolioStore(database: database, storageDirectory: directory)

        XCTAssertEqual(store.snapshot, legacy)
        XCTAssertTrue(exists("portfolios.migrated.json"))
        XCTAssertTrue(exists("portfolio_reporting_currencies.migrated.json"))
    }

    // MARK: - Paper trading

    func testPaperTradingRoundTripKeepsEveryCollection() async throws {
        let now = Date(timeIntervalSince1970: 1_700_000_000)
        let instrument = PaperInstrument(
            key: "test:XYZ", symbol: "XYZ", displayName: "XYZ", source: .alpaca, assetClass: .stock,
            quoteCurrency: .USD, tickSize: 1, minimumQuantity: 1, quantityIncrement: 1, contractMultiplier: 1,
            pointValue: 1)
        let database = self.database!
        let engine = PaperTradingEngine(
            quoteMaximumAge: 30, now: { now }, persist: { try database.savePaperTrading($0) })
        let account = try await engine.createAccount(name: "Paper", initialBalance: 10_000)
        _ = try await engine.createAccount(name: "Second", initialBalance: 5_000)
        try await engine.process(.init(instrumentKey: instrument.key, bid: 99, ask: 100, last: nil, timestamp: now))
        for side in [PaperOrderSide.buy, .sell] {
            _ = try await engine.submit(
                .init(
                    accountID: account, instrument: instrument, side: side, type: .market, quantity: 10,
                    limitPrice: nil, stopPrice: nil, takeProfit: nil, stopLoss: nil))
        }

        let expected = await engine.snapshot()
        XCTAssertFalse(expected.fills.isEmpty)
        XCTAssertFalse(expected.closedTrades.isEmpty)
        XCTAssertEqual(try database.paperTrading(), expected)
    }

    func testLegacyPaperTradingImport() throws {
        let account = PaperAccount(name: "Legacy", initialBalance: 1_000)
        var legacy = PaperTradingSnapshot()
        legacy.accounts = [account]
        legacy.selectedAccountID = account.id
        try writeLegacy(legacy, to: "paper_trading.json")

        let store = PaperTradingStore(database: database, legacyDirectory: directory)

        XCTAssertEqual(store.snapshot, legacy)
        XCTAssertTrue(exists("paper_trading.migrated.json"))
    }

    // MARK: - Alerts

    private struct PersistenceRepository: AlertSnapshotRepository {
        let persistence: AlertRuntimePersistence
        func load() async -> AlertPersistenceSnapshot? { persistence.loadSnapshot() }
        func save(_ snapshot: AlertPersistenceSnapshot) async { persistence.saveSnapshot(snapshot) }
    }

    private let btc = PortfolioAsset(key: "Binance:BTCUSDT", symbol: "BTC", name: "Bitcoin", source: .binance)

    func testAlertSnapshotRoundTripIncludingHistory() async throws {
        let persistence = AlertRuntimePersistence(database: database, directory: directory)
        XCTAssertNil(persistence.loadSnapshot())

        let engine = await LocalPriceAlertEngine(repository: PersistenceRepository(persistence: persistence))
        await engine.saveAlert(PriceAlert(asset: btc, condition: .crossesAbove(target: 100)), baseline: 90)
        let now = Date()
        _ = await engine.process(
            MarketQuote(
                asset: btc, price: 100, currency: .USD, sourceTimestamp: now, receivedAt: now, maximumAge: 60,
                fingerprint: "q1"))
        await engine.apply(AlertRuntimeCommand(payload: .updateSettings(AlertNotificationSettings(soundEnabled: false))))

        let expected = await engine.currentSnapshot()
        XCTAssertEqual(expected.history.count, 1)
        XCTAssertEqual(persistence.loadSnapshot(), expected)
    }

    /// Two databases on one file stand in for the GUI and the login-item agent.
    func testCommandsQueueAcrossConnectionsInOrder() throws {
        let path = directory.appendingPathComponent("test.sqlite").path
        let gui = AlertRuntimePersistence(database: database, directory: directory)
        let agent = AlertRuntimePersistence(database: try AppDatabase(path: path), directory: directory)
        let first = AlertRuntimeCommand(createdAt: Date(timeIntervalSince1970: 1), payload: .clearHistory)
        let second = AlertRuntimeCommand(createdAt: Date(timeIntervalSince1970: 2), payload: .delete(UUID()))

        try gui.enqueue(second)
        try gui.enqueue(first)
        try gui.enqueue(first)

        XCTAssertEqual(agent.pendingCommands(), [first, second])
        agent.acknowledge(first.id)
        XCTAssertEqual(gui.pendingCommands(), [second])
    }

    func testLegacyAlertFilesAndQueuedCommandsImport() throws {
        var legacy = AlertPersistenceSnapshot()
        legacy.revision = 7
        legacy.alerts = [PriceAlert(asset: btc, condition: .crossesBelow(target: 50))]
        try writeLegacy(legacy, to: "price_alerts.json")
        let commands = directory.appendingPathComponent("alert_commands", isDirectory: true)
        try FileManager.default.createDirectory(at: commands, withIntermediateDirectories: true)
        let queued = AlertRuntimeCommand(payload: .clearHistory)
        try JSONEncoder().encode(queued).write(to: commands.appendingPathComponent("1-\(queued.id).json"))

        let persistence = AlertRuntimePersistence(database: database, directory: directory)

        XCTAssertEqual(persistence.loadSnapshot(), legacy)
        XCTAssertEqual(persistence.pendingCommands(), [queued])
        XCTAssertTrue(exists("price_alerts.migrated.json"))
        XCTAssertFalse(exists("alert_commands"))
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
