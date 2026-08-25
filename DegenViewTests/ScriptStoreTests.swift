import XCTest
@testable import DegenView

final class ScriptStoreTests: XCTestCase {
    private func temporaryDirectory() throws -> URL {
        let url = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
        addTeardownBlock { try? FileManager.default.removeItem(at: url) }
        return url
    }

    func testCreatesEveryTypeAndDisambiguatesNames() async throws {
        let store = ScriptStore(rootDirectory: try temporaryDirectory())
        let indicator = try await store.create(name: "Alpha", type: .indicator)
        let strategy = try await store.create(name: "Alpha", type: .strategy)
        let library = try await store.create(name: "Tools", type: .library)
        XCTAssertEqual(indicator.name, "Alpha")
        XCTAssertEqual(strategy.name, "Alpha 2")
        XCTAssertTrue(indicator.source.contains("indicator("))
        XCTAssertTrue(strategy.source.contains("strategy("))
        XCTAssertTrue(library.source.contains("library("))
        XCTAssertNil(indicator.latestRevisionID)
    }

    func testBrokenSourceIsSavedExactlyAndRevisionIsNotDuplicated() async throws {
        let store = ScriptStore(rootDirectory: try temporaryDirectory())
        let script = try await store.create(name: "Broken", type: .indicator)
        let source = "//@version=6\nindicator(\"Broken\")\nplot("
        let saved = try await store.save(id: script.id, name: script.name, type: .indicator, source: source)
        XCTAssertEqual(saved.source, source)
        XCTAssertEqual(saved.compileRecord?.status, .error)
        var revisions = try await store.revisions(id: script.id)
        XCTAssertEqual(revisions.count, 1)
        _ = try await store.save(id: script.id, name: script.name, type: .indicator, source: source)
        revisions = try await store.revisions(id: script.id)
        XCTAssertEqual(revisions.count, 1)
    }

    func testDraftDoesNotOverwriteSavedSource() async throws {
        let store = ScriptStore(rootDirectory: try temporaryDirectory())
        let script = try await store.create(name: "Draft", type: .indicator)
        try await store.saveDraft(.init(scriptID: script.id, source: "unsaved", modifiedAt: Date(), basedOnRevisionID: nil))
        let persisted = try await store.script(id: script.id)
        let draft = try await store.draft(id: script.id)
        XCTAssertEqual(persisted?.source, script.source)
        XCTAssertEqual(draft?.source, "unsaved")
    }

    func testCompilerDetectsDeclarationsAndVersion() {
        for type in ScriptType.allCases {
            let result = PineCompiler.compile(source: ScriptStore.template(for: type, title: "Test"))
            XCTAssertEqual(result.declaration.type, type)
            XCTAssertEqual(result.declaration.pineVersion, 6)
            XCTAssertTrue(result.isValid, "\(type): \(result.diagnostics)")
        }
    }

    func testLegacyTickerConfigGainsChartIdentityAndEmptyInstances() throws {
        let data = Data(#"{"symbol":"BTCUSDT","source":"Binance"}"#.utf8)
        let config = try JSONDecoder().decode(TickerConfig.self, from: data)
        XCTAssertFalse(config.chartID.uuidString.isEmpty)
        XCTAssertTrue(config.scripts.isEmpty)
    }
}
