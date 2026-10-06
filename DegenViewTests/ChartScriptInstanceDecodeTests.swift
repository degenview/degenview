import XCTest

@testable import DegenView

/// Legacy-compatible decoding: an old single-script `TickerConfig` must migrate to exactly one
/// applied instance, a modern `scripts` array must pass through untouched, and `ChartScriptInstance`
/// itself must decode fine when `legacySource` is absent (today's shipped persisted shape).
final class ChartScriptInstanceDecodeTests: XCTestCase {
    func testLegacyTickerConfigWithOnlyPineFieldMigratesToOneLegacySourceInstance() {
        let config = TickerConfig(
            symbol: "BTCUSDT", source: .binance,
            pine: PineConfiguration(
                draftSource: "plot(close)", appliedSource: "plot(close)", inputs: ["len": .int(20)]),
            scripts: [])

        let migrated = ChartViewModel.migratedPineInstances(config)

        XCTAssertEqual(migrated.count, 1)
        XCTAssertEqual(migrated.first?.legacySource, "plot(close)")
        XCTAssertEqual(migrated.first?.inputs, ["len": .int(20)])
        XCTAssertTrue(migrated.first?.isVisible ?? false)
    }

    func testTickerConfigWithNoAppliedScriptMigratesToNoInstances() {
        let config = TickerConfig(symbol: "BTCUSDT", source: .binance, pine: nil, scripts: [])
        XCTAssertTrue(ChartViewModel.migratedPineInstances(config).isEmpty)
    }

    func testModernScriptsArrayDecodesUnchangedWhenPineIsAlsoPresent() {
        let instances = [
            ChartScriptInstance(scriptID: UUID(), loadedRevisionID: UUID()),
            ChartScriptInstance(scriptID: UUID(), loadedRevisionID: UUID(), isVisible: false),
        ]
        let config = TickerConfig(
            symbol: "BTCUSDT", source: .binance,
            pine: PineConfiguration(draftSource: "", appliedSource: "plot(close)", inputs: [:]),
            scripts: instances)

        let migrated = ChartViewModel.migratedPineInstances(config)

        XCTAssertEqual(migrated, instances, "the modern array wins; the legacy field is ignored")
    }

    func testChartScriptInstanceDecodesWhenLegacySourceKeyIsMissingEntirely() throws {
        let json = """
            {
                "id": "\(UUID().uuidString)",
                "scriptID": "\(UUID().uuidString)",
                "loadedRevisionID": "\(UUID().uuidString)",
                "inputs": {},
                "isVisible": true,
                "styleOverrides": {},
                "updateStatus": "current"
            }
            """
        let decoded = try JSONDecoder().decode(ChartScriptInstance.self, from: Data(json.utf8))
        XCTAssertNil(decoded.legacySource)
        XCTAssertTrue(decoded.isVisible)
    }
}
