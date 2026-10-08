import XCTest

@testable import DegenView

/// A provider that answers each symbol with its own price, and can hold a request until released.
private final class StubSource: TickerDataSource, @unchecked Sendable {
    let type: DataSourceType
    private let prices: [String: Double]
    private let lock = NSLock()
    private var gates: [String: CheckedContinuation<Void, Never>] = [:]
    private var held: Set<String> = []
    private(set) var requested: [String] = []

    init(_ type: DataSourceType = .binance, prices: [String: Double]) {
        self.type = type
        self.prices = prices
    }

    /// Requests for `symbol` wait until `release(_:)`.
    func hold(_ symbol: String) { lock.withLock { _ = held.insert(symbol) } }

    func release(_ symbol: String) {
        let continuation = lock.withLock { () -> CheckedContinuation<Void, Never>? in
            held.remove(symbol)
            return gates.removeValue(forKey: symbol)
        }
        continuation?.resume()
    }

    func fetchKlines(symbol: String, interval: String, limit: Int) async throws -> [KlineData] {
        lock.withLock { requested.append(symbol) }
        if lock.withLock({ held.contains(symbol) }) {
            await withCheckedContinuation { continuation in
                lock.withLock { gates[symbol] = continuation }
            }
        }
        return [KlineData(time: Date(), price: prices[symbol] ?? 1)]
    }

    func searchTickers(query: String) async throws -> [TickerSearchResult] { [] }
}

@MainActor
final class ChartSwitchTests: XCTestCase {
    private var database: AppDatabase!
    private var drawings: DrawingStore!

    override func setUpWithError() throws {
        database = try AppDatabase.makeInMemory()
        drawings = DrawingStore(database: database)
    }

    private func chart(_ ticker: String, _ source: DataSourceType = .binance, api: TickerDataSource? = nil)
        -> ChartViewModel
    {
        let vm = ChartViewModel(ticker: ticker, source: source, api: api, drawingStore: drawings)
        vm.serviceResolver = { [source] in StubSource($0 == source ? source : $0, prices: [:]) }
        return vm
    }

    private func content() throws -> ContentViewModel {
        let db = try AppDatabase.makeInMemory()
        return ContentViewModel(
            tabID: UUID(), savedViews: SavedViewStore(database: db), tabs: TabsStore(database: db))
    }

    private func item(_ symbol: String, _ source: DataSourceType = .binance) -> WatchlistInstrument {
        WatchlistInstrument(instrument: InstrumentID(source: source, symbol: symbol), name: symbol, label: symbol)
    }

    private func waitUntil(_ condition: @autoclosure () -> Bool, timeout: Double = 3) async {
        let deadline = Date().addingTimeInterval(timeout)
        while !condition(), Date() < deadline { try? await Task.sleep(for: .milliseconds(10)) }
    }

    // MARK: Stale responses

    func testAResponseForTheOldMarketNeverLandsOnTheNewOne() async throws {
        let old = StubSource(prices: ["BTCUSDT": 100])
        old.hold("BTCUSDT")
        let vm = chart("BTCUSDT", api: old)

        let fetch = Task { await vm.fetchData(for: .oneDay, count: 10) }
        await waitUntil(old.requested.contains("BTCUSDT"))
        vm.updateTicker(symbol: "ETHUSDT", source: .binance)
        old.release("BTCUSDT")
        await fetch.value

        XCTAssertEqual(vm.ticker, "ETHUSDT")
        XCTAssertTrue(vm.klineData.isEmpty, "BTC candles must not appear on the ETH chart")
        XCTAssertNil(vm.currentPrice)
        XCTAssertNil(vm.lastUpdated)
        XCTAssertFalse(vm.isFetching)
    }

    func testSwitchingResetsLoadingAndErrorState() {
        let vm = chart("BTCUSDT")
        vm.errorMessage = "boom"
        vm.lastUpdated = Date()

        vm.updateTicker(symbol: "ETHUSDT", source: .binance)

        XCTAssertNil(vm.errorMessage)
        XCTAssertNil(vm.lastUpdated)
        XCTAssertFalse(vm.isFetching)
    }

    func testRapidSwitchesEndOnTheLastMarketOnly() async throws {
        let source = StubSource(prices: ["AAAUSDT": 1, "BBBUSDT": 2, "CCCUSDT": 3])
        let model = try content()
        let vm = chart("AAAUSDT", api: source)
        vm.serviceResolver = { _ in source }
        model.chartViewModels = [vm]

        model.switchChart(vm, to: item("BBBUSDT"))
        model.switchChart(vm, to: item("CCCUSDT"))
        await waitUntil(vm.currentPrice == 3)
        // Give any late BBB response the chance to misbehave.
        try? await Task.sleep(for: .milliseconds(100))

        XCTAssertEqual(vm.ticker, "CCCUSDT")
        XCTAssertEqual(vm.currentPrice, 3)
        XCTAssertEqual(vm.klineData.count, 1)
    }

    // MARK: Identity

    func testUniqueIDFollowsTheMarketButChartIDDoesNot() {
        let vm = chart("BTCUSDT")
        let chartID = vm.chartID
        let before = vm.uniqueID

        vm.updateTicker(symbol: "ETHUSDT", source: .binance)

        XCTAssertEqual(vm.chartID, chartID)
        XCTAssertNotEqual(vm.uniqueID, before)
        XCTAssertEqual(vm.uniqueID, "ETHUSDT_Binance")
        XCTAssertEqual(vm.iconKey, "Binance:ETHUSDT")
    }

    func testRemovingAChartRemovesOnlyThatChartEvenIfTwoShowTheSameMarket() throws {
        let model = try content()
        let first = chart("BTCUSDT")
        let second = chart("BTCUSDT")
        model.chartViewModels = [first, second]

        model.removeTicker(first)

        XCTAssertEqual(model.chartViewModels.map(\.chartID), [second.chartID])
    }

    func testAChartRecognisesTheSameMarketWhateverWayItIsWritten() {
        XCTAssertTrue(chart("BTC").shows(InstrumentID(source: .binance, symbol: "BTCUSDT")))
        XCTAssertTrue(chart("btcusdt").shows(InstrumentID(source: .binance, symbol: "BTCUSDT")))
        XCTAssertFalse(chart("BTCUSDT").shows(InstrumentID(source: .coinbase, symbol: "BTC-USD")))
    }

    // MARK: Drawings

    func testDrawingsStayWithTheirOwnMarket() throws {
        let line = TrendLine(
            start: TrendAnchor(date: Date(timeIntervalSince1970: 1_000), price: 10),
            end: TrendAnchor(date: Date(timeIntervalSince1970: 2_000), price: 20))
        drawings.save([line], ticker: "BTCUSDT", source: .binance)
        let vm = chart("BTCUSDT")
        XCTAssertEqual(vm.trendLines, [line])

        vm.updateTicker(symbol: "ETHUSDT", source: .binance)
        XCTAssertTrue(vm.trendLines.isEmpty, "BTC's line is not carried to ETH")

        vm.updateTicker(symbol: "BTCUSDT", source: .binance)
        XCTAssertEqual(vm.trendLines, [line], "and is back, untouched, on BTC")
        XCTAssertEqual(drawings.lines(ticker: "ETHUSDT", source: .binance), [])
    }

    func testSelectionsAndDraftsDoNotFollowTheChartToAnotherMarket() {
        let vm = chart("BTCUSDT")
        vm.selectedLineID = UUID()
        vm.editingLineID = UUID()
        vm.selectedFibonacciID = UUID()
        vm.editingFibonacciID = UUID()
        vm.beginFibonacciDraft(at: TrendAnchor(date: Date(), price: 1))
        vm.selectedBrushID = UUID()

        vm.updateTicker(symbol: "ETHUSDT", source: .binance)

        XCTAssertNil(vm.selectedLineID)
        XCTAssertNil(vm.editingLineID)
        XCTAssertNil(vm.selectedFibonacciID)
        XCTAssertNil(vm.editingFibonacciID)
        XCTAssertFalse(vm.hasFibonacciDraft)
        XCTAssertNil(vm.selectedBrushID)
    }

    // MARK: Capabilities

    func testIncompatibleSettingsFallBackExplicitly() {
        let vm = chart("BTCUSDT")
        vm.showVolume = true
        vm.showRSI = true
        vm.yAxisDecimalPlaces = 2

        vm.updateTicker(symbol: "bitcoin", source: .coingecko)

        XCTAssertFalse(vm.showVolume, "CoinGecko has no volume to draw")
        XCTAssertTrue(vm.showRSI, "indicators every source supports stay on")
        XCTAssertNil(vm.yAxisDecimalPlaces, "a precision picked for BTC is not carried to another asset")

        vm.showVolume = true
        vm.updateTicker(symbol: "market-token", source: .polymarket)
        XCTAssertFalse(vm.showVolume, "line charts have no volume bars")
    }

    func testSettingsThatStillApplyAreKept() {
        let vm = chart("BTCUSDT")
        vm.showVolume = true

        vm.updateTicker(symbol: "ETHUSDT", source: .binance)

        XCTAssertTrue(vm.showVolume)
    }

    // MARK: Pine

    func testPineStateIsClearedAndTheAlertStaysBoundToItsOwnMarket() throws {
        let vm = chart("BTCUSDT")
        let coordinator = PineAlertCoordinator(store: PineAlertStore(database: database))
        let oldDataset = vm.pineAlertDataset
        let subscription = PineAlertSubscription(
            chartID: vm.chartID, instanceID: UUID(), scriptID: nil, scriptName: "Script",
            symbolKey: oldDataset.symbolKey, timeframe: oldDataset.timeframe, sourceHash: "hash")
        coordinator.store.add(subscription)
        XCTAssertTrue(subscription.watches(vm.pineAlertDataset))

        vm.updateTicker(symbol: "ETHUSDT", source: .binance)
        coordinator.contextChanged(chartID: vm.chartID, dataset: vm.pineAlertDataset, hashes: [:])

        XCTAssertTrue(vm.pineOutput.plots.isEmpty)
        XCTAssertTrue(vm.pineOutput.markers.isEmpty)
        XCTAssertTrue(vm.pineResults.isEmpty)
        let stored = try XCTUnwrap(coordinator.store.subscription(id: subscription.id))
        XCTAssertEqual(stored.symbolKey, oldDataset.symbolKey, "never silently retargeted")
        XCTAssertEqual(stored.state, .active)
        XCTAssertFalse(stored.watches(vm.pineAlertDataset), "so it cannot fire for the new market")

        vm.updateTicker(symbol: "BTCUSDT", source: .binance)
        XCTAssertTrue(stored.watches(vm.pineAlertDataset), "and resumes if the chart returns")
    }
}
