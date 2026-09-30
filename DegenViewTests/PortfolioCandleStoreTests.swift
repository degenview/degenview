import XCTest

@testable import DegenView

/// A fake exchange: daily bars from `firstDay` to `today` (the open one), close = day + 1.
private final class FakeMarket: @unchecked Sendable {
    static let day: TimeInterval = 86_400
    static let base = Date(timeIntervalSince1970: 1_700_000_000 - 1_700_000_000.truncatingRemainder(dividingBy: day))

    var firstDay = 0
    var today = 100
    var fails = false
    private(set) var limits: [Int] = []

    func time(_ day: Int) -> Date { Self.base.addingTimeInterval(Double(day) * Self.day) }
    /// Just after `day` started, so that day's bar is open.
    func now(_ day: Int, extra: TimeInterval = 3_600) -> Date { time(day).addingTimeInterval(extra) }

    func fetch(limit: Int) throws -> [KlineData] {
        limits.append(limit)
        if fails { throw URLError(.notConnectedToInternet) }
        return (max(firstDay, today - limit + 1)...today).map {
            KlineData(
                openTime: time($0), openPrice: Double($0), highPrice: Double($0 + 2), lowPrice: Double($0),
                closePrice: Double($0 + 1), volume: 1)
        }
    }
}

final class PortfolioCandleStoreTests: XCTestCase {
    private let asset = PortfolioAsset(key: "binance:BTCUSDT", symbol: "BTCUSDT", name: "BTC", source: .binance)
    private var market: FakeMarket!
    private var database: AppDatabase!
    private var store: PortfolioCandleStore!

    override func setUpWithError() throws {
        market = FakeMarket()
        database = try AppDatabase.makeInMemory()
        let market = market!
        store = PortfolioCandleStore(database: database) { _, limit in try market.fetch(limit: limit) }
    }

    private func storedCandles() -> [CandleRecord] {
        database.candles(source: asset.source.rawValue, symbol: "BTCUSDT", interval: "1d", from: 0)
    }

    func testColdFetchIsSizedToTheNeededWindowAndStoresOnlyClosedBars() async {
        let bars = await store.dailyBars(for: asset, from: market.time(90), now: market.now(100))

        // 13 days back to the lookback start, +2 slack.
        XCTAssertEqual(market.limits, [16])
        XCTAssertEqual(bars.last?.openTime, market.time(100))
        XCTAssertTrue(bars.contains { $0.openTime == market.time(87) })
        let stored = storedCandles()
        XCTAssertFalse(stored.contains { $0.openTime >= market.time(100).timeIntervalSince1970 })
        XCTAssertEqual(stored.last?.openTime, market.time(99).timeIntervalSince1970)
        // The oldest bar returned may be partial, so it isn't kept.
        XCTAssertEqual(stored.first?.openTime, market.time(86).timeIntervalSince1970)
    }

    func testNextDayFetchesOnlyTheTailAndKeepsTheStoredHistory() async {
        _ = await store.dailyBars(for: asset, from: market.time(90), now: market.now(100))
        market.today = 101

        let bars = await store.dailyBars(for: asset, from: market.time(90), now: market.now(101))

        XCTAssertEqual(market.limits, [16, 5])
        XCTAssertEqual(bars.map(\.openTime), (87...101).map { market.time($0) })
        XCTAssertEqual(storedCandles().last?.openTime, market.time(100).timeIntervalSince1970)
    }

    func testRebuildsInTheSameMinuteShareOneFetch() async {
        _ = await store.dailyBars(for: asset, from: market.time(90), now: market.now(100))
        let again = await store.dailyBars(for: asset, from: market.time(90), now: market.now(100, extra: 3_610))

        XCTAssertEqual(market.limits.count, 1)
        XCTAssertEqual(again.last?.openTime, market.time(100))
    }

    func testOlderStartRefetchesAndWidensCoverage() async {
        _ = await store.dailyBars(for: asset, from: market.time(90), now: market.now(100))

        let bars = await store.dailyBars(for: asset, from: market.time(60), now: market.now(100, extra: 7_200))

        // 43 days and 2 hours back to the new lookback start rounds up to 44, +2 slack.
        XCTAssertEqual(market.limits, [16, 44 + 2])
        XCTAssertTrue(bars.contains { $0.openTime == market.time(57) })
        let coverage = database.candleCoverage(source: asset.source.rawValue, symbol: "BTCUSDT", interval: "1d")
        XCTAssertEqual(coverage?.coveredFrom, market.time(57).timeIntervalSince1970)
    }

    func testSourceWithShortHistoryIsNotAskedForOlderDataAgain() async {
        market.firstDay = 95
        _ = await store.dailyBars(for: asset, from: market.time(90), now: market.now(100))
        market.today = 101

        _ = await store.dailyBars(for: asset, from: market.time(90), now: market.now(101))

        // The second request is a tail, not another walk back to day 87.
        XCTAssertEqual(market.limits.count, 2)
        XCTAssertLessThanOrEqual(market.limits[1], 5)
    }

    func testFailureFallsBackToStoredBars() async {
        _ = await store.dailyBars(for: asset, from: market.time(90), now: market.now(100))
        market.today = 101
        market.fails = true

        let bars = await store.dailyBars(for: asset, from: market.time(90), now: market.now(101))

        XCTAssertEqual(bars.last?.openTime, market.time(99))
        XCTAssertEqual(bars.first?.openTime, market.time(87))
    }

    // MARK: - buildSnapshots

    func testBuildSnapshotsPricesEachDayFromItsNearestPriorBar() {
        let portfolio = UUID()
        let bars = (0...10).map {
            KlineData(
                openTime: market.time($0), openPrice: 0, highPrice: 0, lowPrice: 0,
                closePrice: Double(100 + $0), volume: 0)
        }.filter { $0.openTime != market.time(5) }  // a gap, filled from the prior bar
        let buy = PortfolioTransaction(
            portfolioID: portfolio, asset: asset, type: .buy, quantity: 2, price: 50, fee: 0,
            timestamp: market.time(3), source: .manual, externalTransactionID: nil, createdAt: market.time(3))

        let points = PortfolioStore.buildSnapshots(
            portfolioID: portfolio, transactions: [buy], assets: [asset], histories: [asset.key: bars],
            start: market.time(2), end: market.time(6), calendar: utc)

        XCTAssertEqual(points.map(\.timestamp), (2...6).map { market.time($0) })
        // Day 2 predates the buy, so it is empty but complete.
        XCTAssertEqual(points[0].value, 0)
        XCTAssertTrue(points[0].isComplete)
        XCTAssertEqual(points[1].value, 2 * 103)
        XCTAssertEqual(points[1].netContributions, 100)
        XCTAssertEqual(points[1].unrealizedPnL, 2 * 103 - 100)
        // Day 5 has no bar, so day 4's close prices it.
        XCTAssertEqual(points[3].value, 2 * 104)
        XCTAssertEqual(points[4].value, 2 * 106)
        XCTAssertTrue(points.allSatisfy(\.isComplete))
    }

    func testBuildSnapshotsMarksDaysWithoutAFreshBarIncomplete() {
        let portfolio = UUID()
        let bars = [
            KlineData(
                openTime: market.time(0), openPrice: 0, highPrice: 0, lowPrice: 0, closePrice: 100, volume: 0)
        ]
        let buy = PortfolioTransaction(
            portfolioID: portfolio, asset: asset, type: .buy, quantity: 1, price: 50, fee: 0,
            timestamp: market.time(0), source: .manual, externalTransactionID: nil, createdAt: market.time(0))

        let points = PortfolioStore.buildSnapshots(
            portfolioID: portfolio, transactions: [buy], assets: [asset], histories: [asset.key: bars],
            start: market.time(0), end: market.time(5), calendar: utc)

        // The bar prices days 0 through 3; day 4 is more than three days past it.
        XCTAssertEqual(points.map(\.isComplete), [true, true, true, true, false, false])
    }

    private var utc: Calendar {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = TimeZone(identifier: "UTC")!
        return calendar
    }
}
