import XCTest

@testable import DegenView

/// Core isolation invariants for multiple applied Pine instances on one chart: independent
/// runtime state, independent inputs, visibility vs. removal, generation safety, and runtime
/// error isolation.
@MainActor
final class ChartViewModelMultiPineInstanceTests: XCTestCase {
    private final class StubSource: TickerDataSource {
        let type = DataSourceType.binance
        let bars: [KlineData]
        init(bars: [KlineData]) { self.bars = bars }
        func fetchKlines(symbol: String, interval: String, limit: Int) async throws -> [KlineData] { bars }
        func searchTickers(query: String) async throws -> [TickerSearchResult] { [] }
    }

    private static let plain = """
        //@version=6
        indicator("Plain")
        plot(close)
        """

    private static let plusLen = """
        //@version=6
        indicator("Plus")
        len = input.int(1, "Length")
        plot(close + len)
        """

    private static let arrayOutOfBounds = """
        //@version=6
        indicator("Bad")
        a = array.new_float(0)
        plot(array.get(a, 0))
        """

    private static let alerting = """
        //@version=6
        indicator("Alerting")
        if close > open
            alert("Up")
        plot(close)
        """

    private static let invalidSource = "//@version=6\nindicator(\"Bad\"\n"

    private static let strategy = """
        //@version=6
        strategy("Strat", overlay=true, initial_capital=10000, default_qty_type=strategy.fixed)
        qty = input.int(1, "Qty")
        if bar_index == 1
            strategy.entry("L", strategy.long, qty=qty)
        """

    private func bars(_ count: Int, start: Double = 100, rising: Double = 1) -> [KlineData] {
        (0..<count).map {
            let close = start + Double($0) * rising
            return KlineData(
                openTime: Date(timeIntervalSince1970: Double($0) * 60), openPrice: close - rising,
                highPrice: close + 1, lowPrice: close - 1, closePrice: close, volume: 1, isClosed: true)
        }
    }

    private func makeModel(bars: [KlineData]) async -> ChartViewModel {
        let model = ChartViewModel(ticker: "T", source: .binance, api: StubSource(bars: bars))
        await model.fetchData(for: .oneDay, count: bars.count)
        return model
    }

    private func waitUntil(_ condition: @autoclosure () -> Bool, timeout: Duration = .seconds(3)) async {
        let deadline = ContinuousClock.now + timeout
        while !condition(), ContinuousClock.now < deadline { try? await Task.sleep(for: .milliseconds(10)) }
    }

    func testAddingTwoInstancesKeepsBothRunningIndependently() async throws {
        let model = await makeModel(bars: bars(10))
        let a = try XCTUnwrap(model.addPineInstance(scriptID: UUID(), revisionID: UUID(), source: Self.plain))
        let b = try XCTUnwrap(
            model.addPineInstance(
                scriptID: UUID(), revisionID: UUID(), source: Self.plusLen, inputs: ["len": .int(5)]))

        await waitUntil(model.pineResults[a]?.output.barCount == 10)
        await waitUntil(model.pineResults[b]?.output.barCount == 10)

        XCTAssertEqual(model.scriptInstances.count, 2)
        XCTAssertEqual(model.pineResults[a]?.output.plots.first?.values.last, 109)
        XCTAssertEqual(model.pineResults[b]?.output.plots.first?.values.last, 114)
    }

    func testSameScriptAppliedTwiceWithDifferentInputsProducesDifferentOutputsAndInputChangeIsIsolated()
        async throws
    {
        let model = await makeModel(bars: bars(5))
        let scriptID = UUID()
        let a = try XCTUnwrap(
            model.addPineInstance(
                scriptID: scriptID, revisionID: UUID(), source: Self.plusLen, inputs: ["len": .int(20)]))
        let b = try XCTUnwrap(
            model.addPineInstance(
                scriptID: scriptID, revisionID: UUID(), source: Self.plusLen, inputs: ["len": .int(200)]))
        XCTAssertNotEqual(a, b, "same script, two distinct instance ids")

        await waitUntil(model.pineResults[a]?.output.barCount == 5)
        await waitUntil(model.pineResults[b]?.output.barCount == 5)
        XCTAssertEqual(model.pineResults[a]?.output.plots.first?.values.last, 124)
        XCTAssertEqual(model.pineResults[b]?.output.plots.first?.values.last, 304)

        model.setPineInstanceInputs(a, inputs: ["len": .int(1000)])
        await waitUntil(model.pineResults[a]?.output.plots.first?.values.last == 1104)
        XCTAssertEqual(model.pineResults[a]?.output.plots.first?.values.last, 1104)
        XCTAssertEqual(model.pineResults[b]?.output.plots.first?.values.last, 304, "B must not be affected")
    }

    func testChangingOneInstancesInputsDoesNotRebuildAnUnrelatedInstance() async throws {
        let model = await makeModel(bars: bars(5))
        let a = try XCTUnwrap(model.addPineInstance(scriptID: UUID(), revisionID: UUID(), source: Self.plusLen))
        let b = try XCTUnwrap(model.addPineInstance(scriptID: UUID(), revisionID: UUID(), source: Self.plain))
        await waitUntil(model.pineResults[a]?.output.barCount == 5)
        await waitUntil(model.pineResults[b]?.output.barCount == 5)

        let bGenerationBefore = model.pineGeneration(forInstance: b)
        model.setPineInstanceInputs(a, inputs: ["len": .int(9)])
        await waitUntil(model.pineResults[a]?.output.plots.first?.values.last == 113)

        XCTAssertEqual(model.pineResults[a]?.output.plots.first?.values.last, 113, "A's rebuild actually completed")
        XCTAssertEqual(model.pineGeneration(forInstance: b), bGenerationBefore, "B must not rebuild")
    }

    func testHidingAnInstanceKeepsRuntimeAndConfigButSuppressesItFromVisibleOutputs() async throws {
        let model = await makeModel(bars: bars(5))
        let id = try XCTUnwrap(model.addPineInstance(scriptID: UUID(), revisionID: UUID(), source: Self.plain))
        await waitUntil(model.pineResults[id]?.output.barCount == 5)

        model.setPineInstanceVisible(id, isVisible: false)
        XCTAssertTrue(model.scriptInstances.contains { $0.id == id })
        XCTAssertEqual(model.scriptInstances.first { $0.id == id }?.isVisible, false)
        XCTAssertTrue(model.visiblePineOutputs.isEmpty)

        // The runtime keeps ticking in the background while hidden.
        let next = bars(1, start: 999).first!
        let continued = KlineData(
            openTime: model.klineData.last!.openTime.addingTimeInterval(60), openPrice: next.openPrice,
            highPrice: next.highPrice, lowPrice: next.lowPrice, closePrice: next.closePrice, volume: 1,
            isClosed: true)
        model.applyKlineUpdate(continued)
        await waitUntil(model.pineResults[id]?.output.barCount == 6)

        XCTAssertEqual(model.pineResults[id]?.output.barCount, 6, "hidden instance still updates")
        model.setPineInstanceVisible(id, isVisible: true)
        XCTAssertEqual(model.visiblePineOutputs.count, 1)
        XCTAssertEqual(model.visiblePineOutputs.first?.barCount, 6, "visible output resumes immediately")
    }

    func testRemovingAnInstanceDropsItsStateImmediately() async throws {
        let model = await makeModel(bars: bars(5))
        let a = try XCTUnwrap(model.addPineInstance(scriptID: UUID(), revisionID: UUID(), source: Self.plain))
        let b = try XCTUnwrap(model.addPineInstance(scriptID: UUID(), revisionID: UUID(), source: Self.plain))
        await waitUntil(model.pineResults[a]?.output.barCount == 5)
        await waitUntil(model.pineResults[b]?.output.barCount == 5)

        model.removePineInstance(a)

        XCTAssertFalse(model.scriptInstances.contains { $0.id == a })
        XCTAssertNil(model.pineResults[a])
        XCTAssertNil(model.pineGeneration(forInstance: a))
        XCTAssertNotNil(model.pineResults[b], "B is unaffected by A's removal")
    }

    func testAddingAnInstanceThatFailsToCompileLeavesExistingInstancesUntouched() async throws {
        let model = await makeModel(bars: bars(3))
        let a = try XCTUnwrap(model.addPineInstance(scriptID: UUID(), revisionID: UUID(), source: Self.plain))
        await waitUntil(model.pineResults[a]?.output.barCount == 3)
        let generationBefore = model.pineGeneration(forInstance: a)

        let failed = model.addPineInstance(scriptID: UUID(), revisionID: UUID(), source: Self.invalidSource)

        XCTAssertNil(failed)
        XCTAssertEqual(model.scriptInstances.count, 1)
        XCTAssertEqual(model.pineGeneration(forInstance: a), generationBefore)
    }

    func testRuntimeErrorInOneInstanceDoesNotAffectAnother() async throws {
        let model = await makeModel(bars: bars(3))
        let good = try XCTUnwrap(model.addPineInstance(scriptID: UUID(), revisionID: UUID(), source: Self.plain))
        let bad = try XCTUnwrap(
            model.addPineInstance(scriptID: UUID(), revisionID: UUID(), source: Self.arrayOutOfBounds))

        await waitUntil(model.pineResults[good]?.output.barCount == 3)
        await waitUntil(!(model.pineResults[bad]?.diagnostics.isEmpty ?? true))

        XCTAssertEqual(model.pineResults[good]?.output.barCount, 3, "the healthy instance keeps running")
        XCTAssertTrue(model.pineResults[bad]?.diagnostics.contains { $0.severity == .error } ?? false)
    }

    func testBrokerStateIsIsolatedBetweenTwoApplicationsOfTheSameStrategyScript() async throws {
        let model = await makeModel(bars: bars(6, rising: 1))
        let scriptID = UUID()
        let a = try XCTUnwrap(
            model.addPineInstance(
                scriptID: scriptID, revisionID: UUID(), source: Self.strategy, inputs: ["qty": .int(1)]))
        let b = try XCTUnwrap(
            model.addPineInstance(
                scriptID: scriptID, revisionID: UUID(), source: Self.strategy, inputs: ["qty": .int(5)]))

        await waitUntil(model.pineResults[a]?.output.barCount == 6)
        await waitUntil(model.pineResults[b]?.output.barCount == 6)

        let profitA = try XCTUnwrap(model.pineResults[a]?.output.strategy?.openProfit)
        let profitB = try XCTUnwrap(model.pineResults[b]?.output.strategy?.openProfit)
        XCTAssertNotEqual(profitA, 0, "A has an open position")
        XCTAssertNotEqual(profitB, 0, "B has an open position")
        XCTAssertNotEqual(profitA, profitB, "independent broker state scaled by each instance's own qty")
    }

    func testHistoricalRebuildAcrossInstancesStillEmitsNoAlerts() async throws {
        let model = await makeModel(bars: bars(50))
        var received: [[PineAlertEvent]] = []
        model.pineAlertHandler = { events, _ in received.append(events) }

        let a = try XCTUnwrap(model.addPineInstance(scriptID: UUID(), revisionID: UUID(), source: Self.alerting))
        let b = try XCTUnwrap(model.addPineInstance(scriptID: UUID(), revisionID: UUID(), source: Self.alerting))

        await waitUntil(model.pineResults[a]?.output.barCount == 50)
        await waitUntil(model.pineResults[b]?.output.barCount == 50)

        XCTAssertTrue(received.isEmpty, "loading history must never dispatch an external alert")
    }
}
