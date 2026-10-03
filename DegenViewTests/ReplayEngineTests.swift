import XCTest

@testable import DegenView

@MainActor
final class ReplayEngineTests: XCTestCase {
    private let base = Date(timeIntervalSince1970: 1_700_000_000)

    private func dates(_ count: Int) -> [Date] {
        (0..<count).map { base.addingTimeInterval(Double($0 * 60)) }
    }

    private func changeCandles(_ prices: [Double]) -> [KlineData] {
        prices.enumerated().map { index, price in
            KlineData(time: base.addingTimeInterval(Double(index * 60)), price: price)
        }
    }

    func testPriceChangesUseFirstAndLastClose() {
        XCTAssertEqual(changeCandles([100, 110, 125]).priceChangeAmount, 25)
        XCTAssertEqual(changeCandles([100, 110, 125]).priceChangePercent, 25)
        XCTAssertEqual(changeCandles([100, 90, 75]).priceChangeAmount, -25)
        XCTAssertEqual(changeCandles([100, 90, 75]).priceChangePercent, -25)
        XCTAssertEqual(changeCandles([100, 90, 100]).priceChangeAmount, 0)
        XCTAssertEqual(changeCandles([100, 90, 100]).priceChangePercent, 0)
    }

    func testPriceChangesRequireTwoCandles() {
        XCTAssertNil(changeCandles([]).priceChangeAmount)
        XCTAssertNil(changeCandles([]).priceChangePercent)
        XCTAssertNil(changeCandles([100]).priceChangeAmount)
        XCTAssertNil(changeCandles([100]).priceChangePercent)
    }

    func testZeroStartingPriceHasAbsoluteButNotPercentageChange() {
        XCTAssertEqual(changeCandles([0, 5]).priceChangeAmount, 5)
        XCTAssertNil(changeCandles([0, 5]).priceChangePercent)
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

    func testStepBackwardFloorsAtStartAndLeavesCompleted() {
        let engine = ReplayEngine()
        let timeline = dates(3)
        engine.start(at: timeline[0], symbol: "BTCUSDT", timeframe: .oneHour, timeline: timeline)
        XCTAssertFalse(engine.canStepBackward)
        XCTAssertFalse(engine.stepBackward())
        XCTAssertEqual(engine.currentTimestamp, timeline[0])

        engine.seek(to: timeline[2])
        XCTAssertEqual(engine.status, .completed)
        XCTAssertTrue(engine.stepBackward())
        XCTAssertEqual(engine.status, .paused)
        XCTAssertEqual(engine.currentTimestamp, timeline[1])
        XCTAssertTrue(engine.stepForward())
    }

    func testSeekToFractionClampsAndCompletesAtTheEnd() {
        let engine = ReplayEngine()
        let timeline = dates(11)
        engine.start(at: timeline[0], symbol: "BTCUSDT", timeframe: .oneHour, timeline: timeline)
        engine.seek(toFraction: 0.5)
        XCTAssertEqual(engine.currentTimestamp, timeline[5])
        XCTAssertEqual(engine.status, .paused)
        engine.seek(toFraction: 7)
        XCTAssertEqual(engine.currentTimestamp, timeline[10])
        XCTAssertEqual(engine.status, .completed)
        engine.seek(toFraction: -3)
        XCTAssertEqual(engine.currentTimestamp, timeline[0])
        engine.seek(toFraction: .nan)
        XCTAssertEqual(engine.currentTimestamp, timeline[0])
    }

    func testProgressAndBarNumberFollowTheCursor() {
        let engine = ReplayEngine()
        XCTAssertEqual(engine.progress, 0)
        XCTAssertEqual(engine.barNumber, 0)
        XCTAssertNil(engine.timelineBounds)

        let timeline = dates(5)
        engine.start(at: timeline[1], symbol: "BTCUSDT", timeframe: .oneHour, timeline: timeline)
        XCTAssertEqual(engine.barCount, 5)
        XCTAssertEqual(engine.barNumber, 2)
        XCTAssertEqual(engine.progress, 0.25, accuracy: 1e-9)
        XCTAssertEqual(engine.startFraction, 0.25, accuracy: 1e-9)
        XCTAssertEqual(engine.timelineBounds, timeline[0]...timeline[4])

        engine.stepForward()
        engine.stepForward()
        XCTAssertEqual(engine.progress, 0.75, accuracy: 1e-9)
        XCTAssertEqual(engine.startFraction, 0.25, accuracy: 1e-9)
    }

    func testSingleBarTimelineHasZeroProgress() {
        let engine = ReplayEngine()
        let timeline = dates(1)
        engine.start(at: timeline[0], symbol: "BTCUSDT", timeframe: .oneHour, timeline: timeline)
        XCTAssertEqual(engine.progress, 0)
        XCTAssertEqual(engine.startFraction, 0)
    }

    func testRestoreKeepsProgress() {
        let timeline = dates(5)
        let saved = ReplaySession(
            status: .paused, symbol: "BTCUSDT", chartTimeframe: .oneHour,
            startTimestamp: timeline[1], currentTimestamp: timeline[3], currentBarIndex: 3,
            replayInterval: .automatic, playbackSpeed: .normal, sessionStartedAt: base
        )
        let engine = ReplayEngine()
        engine.restore(saved, timeline: timeline)
        XCTAssertEqual(engine.progress, 0.75, accuracy: 1e-9)
        XCTAssertEqual(engine.startFraction, 0.25, accuracy: 1e-9)
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
            KlineData(
                openTime: base.addingTimeInterval(60), openPrice: 101, highPrice: 103, lowPrice: 100, closePrice: 102,
                volume: 20),
            KlineData(
                openTime: base.addingTimeInterval(120), openPrice: 102, highPrice: 104, lowPrice: 98, closePrice: 99,
                volume: 30),
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
            KlineData(
                openTime: base.addingTimeInterval(60), openPrice: 101, highPrice: 103, lowPrice: 100, closePrice: 102,
                volume: 20),
            KlineData(
                openTime: base.addingTimeInterval(120), openPrice: 102, highPrice: 104, lowPrice: 98, closePrice: 99,
                volume: 30),
            KlineData(
                openTime: base.addingTimeInterval(180), openPrice: 99, highPrice: 200, lowPrice: 1, closePrice: 150,
                volume: 1),
        ]
        let source = MockGranularSource(candles: minuteBars)
        let vm = ChartViewModel(ticker: "BTC", api: source)
        vm.klineData = [
            KlineData(openTime: base, openPrice: 100, highPrice: 200, lowPrice: 1, closePrice: 150, volume: 61),
            KlineData(
                openTime: base.addingTimeInterval(300), openPrice: 150, highPrice: 151, lowPrice: 149, closePrice: 150,
                volume: 1),
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
            KlineData(
                openTime: base.addingTimeInterval(3_600), openPrice: 1, highPrice: 1, lowPrice: 1, closePrice: 1,
                volume: 1),
        ]
        _ = try await vm.loadGranularReplayData(interval: .oneMinute)
        XCTAssertEqual(vm.replayTimeline(), [base.addingTimeInterval(60)])
    }

    func testNoFutureDataLeakageAtChartBoundary() {
        let vm = ChartViewModel(ticker: "BTC")
        vm.klineData = dates(100).enumerated().map { index, date in
            KlineData(
                openTime: date, openPrice: Double(index), highPrice: Double(index), lowPrice: Double(index),
                closePrice: Double(index), volume: 1)
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

    func fetchReplayKlines(symbol: String, interval: ReplayInterval, start: Date, end: Date, maximumCount: Int)
        async throws -> [KlineData]
    {
        Array(candles.prefix(maximumCount))
    }

    func fetchKlines(symbol: String, interval: String, limit: Int) async throws -> [KlineData] { candles }
    func searchTickers(query: String) async throws -> [TickerSearchResult] { [] }
}
