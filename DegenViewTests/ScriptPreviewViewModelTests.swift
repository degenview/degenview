import XCTest

@testable import DegenView

@MainActor
final class ScriptPreviewViewModelTests: XCTestCase {
    private final class StubSource: TickerDataSource {
        let type = DataSourceType.coingecko
        private(set) var fetches = 0

        func fetchKlines(symbol: String, interval: String, limit: Int) async throws -> [KlineData] {
            fetches += 1
            return (0..<60).map {
                KlineData(time: Date(timeIntervalSince1970: Double($0) * 86_400), price: 100 + Double($0))
            }
        }

        func searchTickers(query: String) async throws -> [TickerSearchResult] { [] }
    }

    private static let source = """
        //@version=6
        indicator("Test", overlay=true)
        len = input.int(14, "Length")
        avg = ta.sma(close, len)
        plot(avg)
        """

    private static let sourceWithoutInputs = """
        //@version=6
        indicator("Test", overlay=true)
        plot(close)
        """

    private var directory: URL!
    private var defaults: UserDefaults!
    private var suite: String!
    private var store: ScriptPreviewInputsStore!
    private var stub: StubSource!

    override func setUpWithError() throws {
        directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("ScriptPreviewViewModelTests-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        suite = "ScriptPreviewViewModelTests-\(UUID().uuidString)"
        defaults = UserDefaults(suiteName: suite)
        store = ScriptPreviewInputsStore(store: JSONStore(filename: "inputs.json", directory: directory))
        stub = StubSource()
    }

    override func tearDownWithError() throws {
        defaults.removePersistentDomain(forName: suite)
        try? FileManager.default.removeItem(at: directory)
    }

    private func makeModel(refreshInterval: TimeInterval = 60) -> ScriptPreviewViewModel {
        ScriptPreviewViewModel(
            chart: ChartViewModel(ticker: "bitcoin", source: .coingecko, api: stub),
            defaults: defaults, inputsStore: store, refreshInterval: refreshInterval, applyDelay: .zero)
    }

    private func waitUntil(_ condition: @autoclosure () -> Bool, timeout: Duration = .seconds(3)) async {
        let deadline = ContinuousClock.now + timeout
        while !condition(), ContinuousClock.now < deadline { try? await Task.sleep(for: .milliseconds(10)) }
    }

    // MARK: - Inputs

    func testPrunedDropsUnknownAndMismatchedInputs() {
        let schema = PineCompiler.compile(source: Self.source).inputSchema
        let values: [String: PineInputValue] = ["len": .int(5), "gone": .int(1)]
        XCTAssertEqual(ScriptPreviewViewModel.pruned(values, to: schema), ["len": .int(5)])
        XCTAssertEqual(
            ScriptPreviewViewModel.pruned(["len": .float(5)], to: schema), [:], "an int input can't take a float")
    }

    func testSavedInputsReachTheRunningScript() async {
        let script = UUID()
        store.setInputs(["len": .int(5)], for: script)
        let model = makeModel()

        model.bind(scriptID: script, type: .indicator)
        model.sourceChanged(Self.source)
        await waitUntil(model.chart.pineConfiguration?.appliedSource != nil)

        XCTAssertEqual(model.chart.pineConfiguration?.inputs["len"], .int(5))
        XCTAssertEqual(model.inputSchema.inputs.map(\.id), ["len"])
    }

    func testSetInputIsSavedPerScript() async {
        let script = UUID()
        let model = makeModel()
        model.bind(scriptID: script, type: .indicator)
        model.sourceChanged(Self.source)
        await waitUntil(model.chart.pineConfiguration?.appliedSource != nil)

        model.setInput(.int(21), id: "len")
        await waitUntil(store.inputs(for: script)["len"] != nil)

        XCTAssertEqual(store.inputs(for: script), ["len": .int(21)])
        XCTAssertEqual(model.chart.pineConfiguration?.inputs["len"], .int(21))
    }

    func testRemovingAnInputFromTheSourceDropsItsValue() async {
        let model = makeModel()
        model.bind(scriptID: UUID(), type: .indicator)
        model.sourceChanged(Self.source)
        await waitUntil(model.chart.pineConfiguration?.appliedSource != nil)
        model.setInput(.int(21), id: "len")

        model.sourceChanged(Self.sourceWithoutInputs)
        await waitUntil(model.inputSchema.inputs.isEmpty)

        XCTAssertEqual(model.inputValues, [:])
    }

    func testResetInputsRestoresDefaults() async {
        let model = makeModel()
        model.bind(scriptID: UUID(), type: .indicator)
        model.sourceChanged(Self.source)
        await waitUntil(model.chart.pineConfiguration?.appliedSource != nil)
        model.setInput(.int(21), id: "len")

        model.resetInputs()

        XCTAssertEqual(model.inputValues, [:])
        XCTAssertEqual(model.chart.pineConfiguration?.inputs, [:])
    }

    // MARK: - Source

    func testSourceThatDoesNotCompileKeepsTheLastWorkingOne() async {
        let model = makeModel()
        model.bind(scriptID: UUID(), type: .indicator)
        model.sourceChanged(Self.source)
        await waitUntil(model.chart.pineConfiguration?.appliedSource != nil)

        model.sourceChanged("indicator(")
        await waitUntil(model.isShowingStaleOutput)

        XCTAssertTrue(model.isShowingStaleOutput)
        XCTAssertEqual(model.chart.pineConfiguration?.appliedSource, Self.source)

        model.sourceChanged(Self.sourceWithoutInputs)
        await waitUntil(!model.isShowingStaleOutput)
        XCTAssertFalse(model.isShowingStaleOutput)
    }

    func testUndoingTheErrorClearsTheStaleBanner() async {
        let model = makeModel()
        model.bind(scriptID: UUID(), type: .indicator)
        model.sourceChanged(Self.source)
        await waitUntil(model.chart.pineConfiguration?.appliedSource != nil)

        model.sourceChanged("indicator(")
        await waitUntil(model.isShowingStaleOutput)
        XCTAssertTrue(model.isShowingStaleOutput)

        // Fixing it by restoring the text the chart already shows is not a new source to apply.
        model.sourceChanged(Self.source)
        XCTAssertFalse(model.isShowingStaleOutput)
        XCTAssertEqual(model.chart.pineConfiguration?.appliedSource, Self.source)
    }

    func testLibrariesNeverRun() async {
        let model = makeModel()
        model.bind(scriptID: UUID(), type: .library)
        model.sourceChanged(Self.source)
        try? await Task.sleep(for: .milliseconds(100))

        XCTAssertNotNil(model.unsupportedReason)
        XCTAssertNil(model.chart.pineConfiguration?.appliedSource)

        model.setPaneVisible(true)
        XCTAssertFalse(model.isActive)
    }

    func testSwitchingScriptsDoesNotLeakInputs() async {
        let model = makeModel()
        model.bind(scriptID: UUID(), type: .indicator)
        model.sourceChanged(Self.source)
        await waitUntil(model.chart.pineConfiguration?.appliedSource != nil)
        model.setInput(.int(21), id: "len")

        model.bind(scriptID: UUID(), type: .indicator)

        XCTAssertEqual(model.inputValues, [:])
        XCTAssertNil(model.chart.pineConfiguration)
    }

    // MARK: - Market, timeframe, zoom

    func testPredictionMarketsCannotBePicked() {
        let model = makeModel()
        model.selectMarket(PreviewMarket(ticker: "123", source: .polymarket))
        model.selectMarket(PreviewMarket(ticker: "ABC", source: .coinMarketCap))
        XCTAssertEqual(model.market, .default)
        XCTAssertTrue(PreviewMarket.isSupported(.alpaca))
        XCTAssertTrue(PreviewMarket.isSupported(.dexscreener))
        XCTAssertFalse(PreviewMarket.isSupported(.kalshi))
    }

    func testMarketAndTimeframeAreRemembered() {
        let model = makeModel()
        let eth = PreviewMarket(ticker: "ETH", source: .coinbase)
        model.selectMarket(eth)
        model.setTimeRange(.oneWeek)

        let reopened = makeModel()
        XCTAssertEqual(reopened.market, eth)
        XCTAssertEqual(reopened.recents, [eth])
        XCTAssertEqual(reopened.timeRange, .oneWeek)
        XCTAssertEqual(reopened.candleCount, TimeRange.oneWeek.dataPointLimit)
    }

    func testRecentsAreDeduplicatedAndCapped() {
        let model = makeModel()
        let markets = (0..<7).map { PreviewMarket(ticker: "T\($0)", source: .coinbase) }
        markets.forEach(model.selectMarket)
        model.selectMarket(markets[3])

        XCTAssertEqual(model.recents.count, ScriptPreviewViewModel.recentsLimit)
        XCTAssertEqual(model.recents.first, markets[3])
        XCTAssertEqual(Set(model.recents).count, model.recents.count)
    }

    func testZoomStaysWithinLimits() {
        let model = makeModel()
        model.zoom(steps: 1_000)
        XCTAssertEqual(model.candleCount, Candle.minCandles)
        model.zoom(steps: -1_000)
        XCTAssertEqual(model.candleCount, Candle.maxCandles)
    }

    // MARK: - Lifecycle

    func testPollsOnlyWhileActive() async {
        let model = makeModel(refreshInterval: 0.05)
        model.bind(scriptID: UUID(), type: .indicator)
        XCTAssertFalse(model.isActive, "nothing runs until the pane is on screen")

        model.setPaneVisible(true)
        await waitUntil(stub.fetches >= 3)
        XCTAssertGreaterThanOrEqual(stub.fetches, 3)
        XCTAssertFalse(model.chart.klineData.isEmpty)

        model.setWindowVisible(false)
        try? await Task.sleep(for: .milliseconds(100))
        let settled = stub.fetches
        try? await Task.sleep(for: .milliseconds(200))
        XCTAssertEqual(stub.fetches, settled, "a hidden window stops polling")
    }
}
