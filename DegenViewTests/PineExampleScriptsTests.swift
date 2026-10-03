import XCTest

@testable import DegenView

/// The scripts that ship in the app bundle must keep compiling and running as the engine changes.
final class PineExampleScriptsTests: XCTestCase {
    /// Four weeks of hourly candles: a trend with waves, and a volume spike every so often.
    private func bars() -> [KlineData] {
        (0..<700).map { i in
            let t = Double(i)
            let close = 100 + t * 0.05 + 8 * sin(t / 9) + 3 * sin(t / 3)
            let open = close - 1.2 * cos(t / 4)
            return KlineData(
                openTime: Date(timeIntervalSince1970: 1_700_000_000 + t * 3600), openPrice: open,
                highPrice: max(open, close) + 1.5, lowPrice: min(open, close) - 1.5, closePrice: close,
                volume: i % 50 == 49 ? 1_000 : 100 + 20 * abs(sin(t)), isClosed: true)
        }
    }

    private func output(of demo: DemoScriptLibrary.Demo) throws -> PineVisualOutput {
        let program = PineCompiler.compile(source: demo.source)
        XCTAssertTrue(program.diagnostics.isEmpty, "\(demo.name): \(program.diagnostics)")
        return try PineRuntimeSession(program: program).evaluate(bars: bars()).output
    }

    private func demo(_ name: String) throws -> DemoScriptLibrary.Demo {
        try XCTUnwrap(DemoScriptLibrary.bundled().first { $0.name == name }, name)
    }

    func testEveryBundledDemoCompilesAndRuns() throws {
        let demos = DemoScriptLibrary.bundled()
        XCTAssertGreaterThanOrEqual(demos.count, 9)
        for demo in demos {
            let output = try output(of: demo)
            XCTAssertEqual(output.barCount, 700, demo.name)
        }
    }

    func testDemoProducesWhatItShowsOff() throws {
        XCTAssertEqual(try output(of: demo("EMA Cross")).plots.count, 2)
        XCTAssertFalse(try output(of: demo("EMA Cross")).markers.isEmpty)
        XCTAssertEqual(try output(of: demo("RSI Zones")).hlines.count, 3)
        XCTAssertFalse(try output(of: demo("Bollinger Squeeze")).fills.isEmpty)
        XCTAssertFalse(try output(of: demo("MACD Histogram")).plots.isEmpty)
        let levels = try output(of: demo("Pivot Levels"))
        XCTAssertFalse(levels.lines.isEmpty)
        XCTAssertFalse(levels.labels.isEmpty)
        XCTAssertEqual(try output(of: demo("Stats Table")).tables.count, 1)
        XCTAssertFalse(try output(of: demo("Higher Timeframe Trend")).plots.isEmpty)
        XCTAssertFalse(try output(of: demo("Volume Spike Alert")).alerts.isEmpty)
        let strategy = try XCTUnwrap(try output(of: demo("EMA Cross Strategy")).strategy)
        XCTAssertFalse(strategy.trades.isEmpty)
    }

    func testDemosDeclareTheirType() throws {
        for demo in DemoScriptLibrary.bundled() {
            let type = PineCompiler.compile(source: demo.source).declaration.type
            XCTAssertEqual(type, demo.name.hasSuffix("Strategy") ? .strategy : .indicator, demo.name)
        }
    }

    // MARK: - Seeding

    private func makeStore() -> ScriptStore {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        addTeardownBlock { try? FileManager.default.removeItem(at: root) }
        return ScriptStore(
            scriptsDirectory: root.appendingPathComponent("Scripts"),
            metadataDirectory: root.appendingPathComponent("Metadata"))
    }

    private func makeDefaults() throws -> UserDefaults {
        let name = "PineExampleScriptsTests-\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: name))
        addTeardownBlock { defaults.removePersistentDomain(forName: name) }
        return defaults
    }

    func testSeedingRunsOnceAndRespectsDeletion() async throws {
        let store = makeStore()
        let defaults = try makeDefaults()
        let demos = DemoScriptLibrary.bundled()

        await DemoScriptLibrary.seedIfNeeded(in: store, defaults: defaults, demos: demos)
        let seeded = try await store.allScripts()
        XCTAssertEqual(Set(seeded.map(\.name)), Set(demos.map(\.name)))

        try await store.delete(id: try XCTUnwrap(seeded.first).id)
        await DemoScriptLibrary.seedIfNeeded(in: store, defaults: defaults, demos: demos)
        let afterSecondLaunch = try await store.allScripts()
        XCTAssertEqual(afterSecondLaunch.count, demos.count - 1)
    }

    func testAddMissingRestoresOnlyWhatIsAbsent() async throws {
        let store = makeStore()
        let demos = DemoScriptLibrary.bundled()
        let first = try await DemoScriptLibrary.addMissing(to: store, demos: demos)
        XCTAssertEqual(first, demos.count)

        let all = try await store.allScripts()
        let removed = try XCTUnwrap(all.first)
        try await store.delete(id: removed.id)
        let second = try await DemoScriptLibrary.addMissing(to: store, demos: demos)
        XCTAssertEqual(second, 1)
        let third = try await DemoScriptLibrary.addMissing(to: store, demos: demos)
        XCTAssertEqual(third, 0)
        let names = try await store.allScripts().map(\.name)
        XCTAssertEqual(Set(names), Set(demos.map(\.name)))
    }
}
