import XCTest

@testable import DegenView

/// Persistence invariants for applied Pine instances: `applyConfig` round trip, legacy
/// single-script charts still execute (not just decode), and a saved tab with several
/// instances round-trips exactly through the real SQLite-backed `AppDatabase`.
@MainActor
final class ChartViewModelPineInstancePersistenceTests: XCTestCase {
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

    private func bars(_ count: Int) -> [KlineData] {
        (0..<count).map {
            KlineData(
                openTime: Date(timeIntervalSince1970: Double($0) * 60), openPrice: 100,
                highPrice: 101, lowPrice: 99, closePrice: 100, volume: 1, isClosed: true)
        }
    }

    private func waitUntil(_ condition: @autoclosure () -> Bool, timeout: Duration = .seconds(3)) async {
        let deadline = ContinuousClock.now + timeout
        while !condition(), ContinuousClock.now < deadline { try? await Task.sleep(for: .milliseconds(10)) }
    }

    func testApplyConfigPreservesInstanceOrderVisibilityAndInputs() {
        let instances = [
            ChartScriptInstance(scriptID: UUID(), loadedRevisionID: UUID(), inputs: ["len": .int(20)]),
            ChartScriptInstance(
                scriptID: UUID(), loadedRevisionID: UUID(), inputs: ["len": .int(200)], isVisible: false),
        ]
        let config = TickerConfig(symbol: "BTCUSDT", source: .binance, scripts: instances)
        let model = ChartViewModel(ticker: "BTCUSDT", source: .binance)

        model.applyConfig(config)

        XCTAssertEqual(model.scriptInstances, instances, "same order, same ids, same inputs, same visibility")
    }

    func testLegacyTickerConfigStillExecutesAfterDecode() async throws {
        let config = TickerConfig(
            symbol: "BTCUSDT", source: .binance,
            pine: PineConfiguration(draftSource: Self.plain, appliedSource: Self.plain, inputs: [:]),
            scripts: [])
        let model = ChartViewModel(ticker: "BTCUSDT", source: .binance, api: StubSource(bars: bars(5)))

        model.applyConfig(config)
        XCTAssertEqual(model.scriptInstances.count, 1)
        let id = try XCTUnwrap(model.scriptInstances.first?.id)

        await model.fetchData(for: .oneDay, count: 5)
        await waitUntil(model.pineResults[id]?.output.barCount == 5)

        XCTAssertEqual(model.pineResults[id]?.output.barCount, 5, "the migrated instance actually ran")
    }

    func testAppDatabaseRoundTripsATabWithMultiplePineInstances() throws {
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("PineInstancePersistenceTests-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let database = try AppDatabase(path: directory.appendingPathComponent("test.sqlite").path)

        let instances = [
            ChartScriptInstance(scriptID: UUID(), loadedRevisionID: UUID(), inputs: ["len": .int(20)]),
            ChartScriptInstance(
                scriptID: UUID(), loadedRevisionID: UUID(), inputs: ["len": .int(200)], isVisible: false),
            ChartScriptInstance(scriptID: UUID(), loadedRevisionID: UUID()),
        ]
        let config = TickerConfig(symbol: "BTCUSDT", source: .binance, scripts: instances)

        let store = TabsStore(database: database)
        let tab = store.makeTab(name: "T", tickerConfig: config)
        store.persist()

        let reloaded = TabsStore(database: database)
        let reloadedTab = try XCTUnwrap(reloaded.tabs.first { $0.id == tab.id })

        XCTAssertEqual(reloadedTab.tickerConfigs.first?.scripts, instances)
    }
}
