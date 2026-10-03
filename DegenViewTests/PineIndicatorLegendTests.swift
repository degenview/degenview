import XCTest

@testable import DegenView

/// `ChartViewModel.pineLegendRows`/`PineInputSchema.pineCompactSummary` — the data the
/// TradingView-style legend renders. Exercised through the real pipeline (not fabricated
/// state) since `pineResults` is `private(set)`.
@MainActor
final class PineIndicatorLegendTests: XCTestCase {
    private final class StubSource: TickerDataSource {
        let type = DataSourceType.binance
        let bars: [KlineData]
        init(bars: [KlineData]) { self.bars = bars }
        func fetchKlines(symbol: String, interval: String, limit: Int) async throws -> [KlineData] { bars }
        func searchTickers(query: String) async throws -> [TickerSearchResult] { [] }
    }

    private static let ema = """
        //@version=6
        indicator("EMA", shorttitle="EMA")
        len = input.int(20, "Length")
        plot(close + len)
        """

    private static let broken = """
        //@version=6
        indicator("Bad")
        a = array.new_float(0)
        plot(array.get(a, 0))
        """

    private func bars(_ count: Int) -> [KlineData] {
        (0..<count).map {
            KlineData(
                openTime: Date(timeIntervalSince1970: Double($0) * 60), openPrice: 100,
                highPrice: 101, lowPrice: 99, closePrice: 100, volume: 1, isClosed: true)
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

    func testLegendRowTitleIncludesCompactInputSummary() async throws {
        let model = await makeModel(bars: bars(3))
        let id = try XCTUnwrap(
            model.addPineInstance(scriptID: UUID(), revisionID: UUID(), source: Self.ema, inputs: ["len": .int(20)]))
        await waitUntil(model.pineResults[id]?.output.barCount == 3)

        let row = try XCTUnwrap(model.pineLegendRows.first { $0.id == id })
        XCTAssertEqual(row.title, "EMA 20")
        XCTAssertTrue(row.isVisible)
        XCTAssertFalse(row.hasError)
    }

    func testHiddenInstanceStillAppearsInLegendRowsMarkedNotVisible() async throws {
        let model = await makeModel(bars: bars(3))
        let id = try XCTUnwrap(model.addPineInstance(scriptID: UUID(), revisionID: UUID(), source: Self.ema))
        await waitUntil(model.pineResults[id]?.output.barCount == 3)

        model.setPineInstanceVisible(id, isVisible: false)

        let row = try XCTUnwrap(model.pineLegendRows.first { $0.id == id })
        XCTAssertFalse(row.isVisible)
    }

    func testInstanceWithRuntimeErrorFlagsHasErrorOnItsRow() async throws {
        let model = await makeModel(bars: bars(3))
        let id = try XCTUnwrap(model.addPineInstance(scriptID: UUID(), revisionID: UUID(), source: Self.broken))
        await waitUntil(!(model.pineResults[id]?.diagnostics.isEmpty ?? true))

        let row = try XCTUnwrap(model.pineLegendRows.first { $0.id == id })
        XCTAssertTrue(row.hasError)
    }

    func testDuplicateScriptApplicationsProduceDistinctRowsKeyedByInstanceNotTitle() async throws {
        let model = await makeModel(bars: bars(3))
        let scriptID = UUID()
        let a = try XCTUnwrap(
            model.addPineInstance(scriptID: scriptID, revisionID: UUID(), source: Self.ema, inputs: ["len": .int(20)]))
        let b = try XCTUnwrap(
            model.addPineInstance(scriptID: scriptID, revisionID: UUID(), source: Self.ema, inputs: ["len": .int(200)]))
        await waitUntil(model.pineResults[a]?.output.barCount == 3)
        await waitUntil(model.pineResults[b]?.output.barCount == 3)

        let rows = model.pineLegendRows
        XCTAssertEqual(rows.count, 2)
        XCTAssertEqual(Set(rows.map(\.id)), Set([a, b]))
        XCTAssertEqual(rows.first { $0.id == a }?.title, "EMA 20")
        XCTAssertEqual(rows.first { $0.id == b }?.title, "EMA 200")
    }
}
