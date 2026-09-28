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

    // MARK: - Workspace

    func testFavoritesAndSavedViewsKeepOrder() {
        let favorites = [
            FavoriteItem(name: "Bitcoin", ticker: "BTC", config: TickerConfig(symbol: "BTCUSDT", source: .binance)),
            FavoriteItem(name: "Ether", ticker: "ETH", config: TickerConfig(symbol: "ETHUSDT", source: .binance)),
        ]
        database.replaceDocuments(favorites, in: .favorite)
        XCTAssertEqual(FavoritesStore(database: database).items, favorites)

        let views = ["B", "A"].map {
            SavedView(name: $0, tickers: ["BTCUSDT"], timeRange: .oneDay, layoutMode: .grid, createdAt: Date())
        }
        database.saveSavedViews(views)
        XCTAssertEqual(database.savedViews().map(\.name), ["B", "A"])
    }

    func testTabsPersistAcrossStores() throws {
        let store = TabsStore(database: database)
        let added = store.makeTab()
        store.setWindowGroups([store.tabs.map(\.id)])
        store.persist()

        let reloaded = TabsStore(database: database)
        XCTAssertEqual(reloaded.tabs, store.tabs)
        XCTAssertEqual(reloaded.windowIndex(of: added.id), 0)
    }

    func testDeletedDrawingsStayDeleted() {
        let store = DrawingStore(database: database)
        let line = TrendLine(
            start: TrendAnchor(date: Date(timeIntervalSince1970: 1), price: 1),
            end: TrendAnchor(date: Date(timeIntervalSince1970: 2), price: 2))
        store.save([line], ticker: "BTC", source: .binance)
        store.save([TrendLine](), ticker: "BTC", source: .binance)

        let reloaded = DrawingStore(database: database)
        XCTAssertEqual(reloaded.lines(ticker: "BTC", source: .binance), [])
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
}
