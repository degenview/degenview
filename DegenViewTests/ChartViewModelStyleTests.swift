import XCTest

@testable import DegenView

/// `setPineInstanceStyle` and the way `visiblePineOutputs` applies the choices, through the real
/// pipeline since `pineResults` is `private(set)`.
@MainActor
final class ChartViewModelStyleTests: XCTestCase {
    private final class StubSource: TickerDataSource {
        let type = DataSourceType.binance
        let bars: [KlineData]
        init(bars: [KlineData]) { self.bars = bars }
        func fetchKlines(symbol: String, interval: String, limit: Int) async throws -> [KlineData] { bars }
        func searchTickers(query: String) async throws -> [TickerSearchResult] { [] }
    }

    private func makeModel(count: Int = 4) async -> ChartViewModel {
        let bars = PineStyleFixtures.bars((0..<count).map { 10 + Double($0 % 3) })
        let model = ChartViewModel(ticker: "T", source: .binance, api: StubSource(bars: bars))
        await model.fetchData(for: .oneDay, count: bars.count)
        return model
    }

    private func waitUntil(_ condition: @autoclosure () -> Bool, timeout: Duration = .seconds(3)) async {
        let deadline = ContinuousClock.now + timeout
        while !condition(), ContinuousClock.now < deadline { try? await Task.sleep(for: .milliseconds(10)) }
    }

    private func addStyled(to model: ChartViewModel) async throws -> UUID {
        let id = try XCTUnwrap(
            model.addPineInstance(
                scriptID: UUID(), revisionID: UUID(), source: PineStyleFixtures.source, inputs: [:]))
        await waitUntil(model.pineResults[id]?.state == .ready)
        return id
    }

    func testStyleChoicesShapeTheRenderedOutputWithoutRebuilding() async throws {
        let model = await makeModel()
        let id = try await addStyled(to: model)
        let generation = model.pineGeneration(forInstance: id)
        let rows = PineStyleRows.rows(for: try XCTUnwrap(model.pineResults[id]).output)
        let level = try XCTUnwrap(rows.first { $0.kind == .hline })

        model.setPineInstanceStyle(
            id, overrides: [PineStyleOverrides.key(.hline, level.outputID, .visible): "false"])

        XCTAssertEqual(model.visiblePineOutputs.first?.hlines.count, 0)
        XCTAssertEqual(model.pineResults[id]?.output.hlines.count, 1, "the script's own output is untouched")
        XCTAssertEqual(model.pineGeneration(forInstance: id), generation)
    }

    func testChoicesForOutputsThatNoLongerExistAreDropped() async throws {
        let model = await makeModel()
        let id = try await addStyled(to: model)
        model.setPineInstanceStyle(id, overrides: ["plot.9999.visible": "false"])
        XCTAssertEqual(model.scriptInstances.first?.styleOverrides, [:])
    }

    func testChoicesSurviveAnInputChangeRebuild() async throws {
        let model = await makeModel()
        let id = try await addStyled(to: model)
        let rows = PineStyleRows.rows(for: try XCTUnwrap(model.pineResults[id]).output)
        let band = try XCTUnwrap(rows.first { $0.kind == .fill })
        let key = PineStyleOverrides.key(.fill, band.outputID, .visible)
        model.setPineInstanceStyle(id, overrides: [key: "false"])

        model.setPineInstanceInputs(id, inputs: [:])
        await waitUntil(model.pineResults[id]?.state == .ready)

        XCTAssertEqual(model.scriptInstances.first?.styleOverrides[key], "false")
        XCTAssertEqual(model.visiblePineOutputs.first?.fills.count, 0)
    }

    func testAnOverrideChangesTheLayoutFingerprint() {
        var instance = ChartScriptInstance(scriptID: UUID(), loadedRevisionID: UUID())
        let plain = TickerConfig(symbol: "T", source: .binance, scripts: [instance])
        instance.styleOverrides["plot.1.visible"] = "false"
        let styled = TickerConfig(symbol: "T", source: .binance, scripts: [instance])
        XCTAssertNotEqual(plain, styled)
    }

    func testStoredOverridesRoundTripThroughJSON() throws {
        var instance = ChartScriptInstance(scriptID: UUID(), loadedRevisionID: UUID())
        instance.styleOverrides = ["plot.1.color": "112233FF", "plot.1.width": "3"]
        let decoded = try JSONDecoder().decode(ChartScriptInstance.self, from: JSONEncoder().encode(instance))
        XCTAssertEqual(decoded.styleOverrides, instance.styleOverrides)
    }
}
