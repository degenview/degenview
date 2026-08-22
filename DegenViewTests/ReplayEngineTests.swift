import XCTest
@testable import DegenView

@MainActor
final class ReplayEngineTests: XCTestCase {
    private let base = Date(timeIntervalSince1970: 1_700_000_000)

    private func dates(_ count: Int) -> [Date] {
        (0..<count).map { base.addingTimeInterval(Double($0 * 60)) }
    }

    func testStartSelectionAndInitialCursor() {
        let engine = ReplayEngine()
        let timeline = dates(100)
        engine.start(at: timeline[49], symbol: "BTCUSDT", timeframe: .oneHour, timeline: timeline)
        XCTAssertEqual(engine.status, .paused)
        XCTAssertEqual(engine.session?.currentBarIndex, 49)
        XCTAssertEqual(engine.currentTimestamp, timeline[49])
    }

    func testStepPauseAndEndOfDataset() {
        let engine = ReplayEngine()
        let timeline = dates(3)
        engine.start(at: timeline[1], symbol: "BTCUSDT", timeframe: .oneHour, timeline: timeline)
        XCTAssertTrue(engine.stepForward())
        XCTAssertEqual(engine.currentTimestamp, timeline[2])
        XCTAssertFalse(engine.stepForward())
        XCTAssertEqual(engine.status, .completed)
    }

    func testSeekClampsToAvailableBarAndJumpToLatestStops() {
        let engine = ReplayEngine()
        let timeline = dates(5)
        engine.start(at: timeline[0], symbol: "BTCUSDT", timeframe: .oneHour, timeline: timeline)
        engine.seek(to: timeline[2].addingTimeInterval(30))
        XCTAssertEqual(engine.currentTimestamp, timeline[2])
        engine.jumpToLatest()
        XCTAssertEqual(engine.status, .inactive)
        XCTAssertNil(engine.session)
    }

    func testRestoreAlwaysPausesPlayingSession() {
        let timeline = dates(5)
        let saved = ReplaySession(
            status: .playing, symbol: "BTCUSDT", chartTimeframe: .oneHour,
            startTimestamp: timeline[0], currentTimestamp: timeline[2], currentBarIndex: 2,
            replayInterval: .automatic, playbackSpeed: .ten, sessionStartedAt: base
        )
        let engine = ReplayEngine()
        engine.restore(saved, timeline: timeline)
        XCTAssertEqual(engine.status, .paused)
        XCTAssertEqual(engine.session?.playbackSpeed, .ten)
    }

    func testDuplicateAndMissingTimestampsAreDeterministic() {
        let timeline = [dates(4)[0], dates(4)[2], dates(4)[2], dates(4)[3]]
        XCTAssertEqual(ReplayEngine.normalized(timeline), [dates(4)[0], dates(4)[2], dates(4)[3]])
        XCTAssertEqual(ReplayEngine.index(atOrBefore: dates(4)[1], in: ReplayEngine.normalized(timeline)), 0)
    }

    func testPartialCandleAggregation() throws {
        let candles = [
            KlineData(openTime: base, openPrice: 100, highPrice: 102, lowPrice: 99, closePrice: 101, volume: 10),
            KlineData(openTime: base.addingTimeInterval(60), openPrice: 101, highPrice: 103, lowPrice: 100, closePrice: 102, volume: 20),
            KlineData(openTime: base.addingTimeInterval(120), openPrice: 102, highPrice: 104, lowPrice: 98, closePrice: 99, volume: 30)
        ]
        let result = try XCTUnwrap(ReplayEngine.aggregate(candles[...], bucketStart: base))
        XCTAssertEqual(result.openPrice, 100)
        XCTAssertEqual(result.highPrice, 104)
        XCTAssertEqual(result.lowPrice, 98)
        XCTAssertEqual(result.closePrice, 99)
        XCTAssertEqual(result.volume, 60)
    }

    func testGranularSourceBuildsOnlyObservedPartialChartCandle() async throws {
        let minuteBars = [
            KlineData(openTime: base, openPrice: 100, highPrice: 102, lowPrice: 99, closePrice: 101, volume: 10),
            KlineData(openTime: base.addingTimeInterval(60), openPrice: 101, highPrice: 103, lowPrice: 100, closePrice: 102, volume: 20),
            KlineData(openTime: base.addingTimeInterval(120), openPrice: 102, highPrice: 104, lowPrice: 98, closePrice: 99, volume: 30),
            KlineData(openTime: base.addingTimeInterval(180), openPrice: 99, highPrice: 200, lowPrice: 1, closePrice: 150, volume: 1)
        ]
        let source = MockGranularSource(candles: minuteBars)
        let vm = ChartViewModel(ticker: "BTC", api: source)
        vm.klineData = [
            KlineData(openTime: base, openPrice: 100, highPrice: 200, lowPrice: 1, closePrice: 150, volume: 61),
            KlineData(openTime: base.addingTimeInterval(300), openPrice: 150, highPrice: 151, lowPrice: 149, closePrice: 150, volume: 1)
        ]
        _ = try await vm.loadGranularReplayData(interval: .oneMinute)
        vm.applyReplayTimestamp(base.addingTimeInterval(180))

        let partial = try XCTUnwrap(vm.replayKlines.last)
        XCTAssertEqual(partial.openPrice, 100)
        XCTAssertEqual(partial.highPrice, 104)
        XCTAssertEqual(partial.lowPrice, 98)
        XCTAssertEqual(partial.closePrice, 99)
        XCTAssertEqual(partial.volume, 60)
    }

    func testGranularTimelineUsesSourceCloseTimes() async throws {
        let source = MockGranularSource(candles: [
            KlineData(openTime: base, openPrice: 1, highPrice: 1, lowPrice: 1, closePrice: 1, volume: 1)
        ])
        let vm = ChartViewModel(ticker: "BTC", api: source)
        vm.klineData = [
            KlineData(openTime: base, openPrice: 1, highPrice: 1, lowPrice: 1, closePrice: 1, volume: 1),
            KlineData(openTime: base.addingTimeInterval(3_600), openPrice: 1, highPrice: 1, lowPrice: 1, closePrice: 1, volume: 1)
        ]
        _ = try await vm.loadGranularReplayData(interval: .oneMinute)
        XCTAssertEqual(vm.replayTimeline(), [base.addingTimeInterval(60)])
    }

    func testNoFutureDataLeakageAtChartBoundary() {
        let vm = ChartViewModel(ticker: "BTC")
        vm.klineData = dates(100).enumerated().map { index, date in
            KlineData(openTime: date, openPrice: Double(index), highPrice: Double(index), lowPrice: Double(index), closePrice: Double(index), volume: 1)
        }
        vm.setVisibleCount(100)
        vm.showEMA = true
        vm.emaPeriod = 3
        vm.applyReplayTimestamp(dates(100)[49])
        XCTAssertEqual(vm.replayKlines.count, 50)
        XCTAssertEqual(vm.visibleKlines.count, 50)
        XCTAssertEqual(vm.visibleKlines.last?.openTime, dates(100)[49])
        XCTAssertEqual(vm.indicators.ema.count, 50)
    }
}

private final class MockGranularSource: GranularReplayDataSource {
    let type: DataSourceType = .binance
    let candles: [KlineData]

    init(candles: [KlineData]) { self.candles = candles }

    func supportedReplayIntervals(chartInterval: String) -> [ReplayInterval] {
        [.automatic, .oneMinute, .chartBar]
    }

    func fetchReplayKlines(symbol: String, interval: ReplayInterval, start: Date, end: Date, maximumCount: Int) async throws -> [KlineData] {
        Array(candles.prefix(maximumCount))
    }

    func fetchKlines(symbol: String, interval: String, limit: Int) async throws -> [KlineData] { candles }
    func searchTickers(query: String) async throws -> [TickerSearchResult] { [] }
}
