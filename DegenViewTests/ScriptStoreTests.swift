import XCTest

@testable import DegenView

final class ScriptStoreTests: XCTestCase {
    private func temporaryDirectory() throws -> URL {
        let url = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
        addTeardownBlock { try? FileManager.default.removeItem(at: url) }
        return url
    }

    private func makeStore() throws -> (store: ScriptStore, scripts: URL, metadata: URL) {
        let root = try temporaryDirectory()
        let scripts = root.appendingPathComponent("Scripts", isDirectory: true)
        let metadata = root.appendingPathComponent("ScriptMetadata", isDirectory: true)
        return (ScriptStore(scriptsDirectory: scripts, metadataDirectory: metadata), scripts, metadata)
    }

    private let validSource = "//@version=6\nindicator(\"Valid\")\nplot(close)\n"

    func testCreateWritesPineFile() async throws {
        let (store, scripts, _) = try makeStore()
        let script = try await store.create(name: "Alpha", type: .indicator)
        let file = scripts.appendingPathComponent("Alpha.pine")
        XCTAssertEqual(try String(contentsOf: file, encoding: .utf8), script.source)
    }

    func testRenameMovesFileAndKeepsID() async throws {
        let (store, scripts, _) = try makeStore()
        let script = try await store.create(name: "Alpha", type: .indicator)
        let renamed = try await store.save(id: script.id, name: "Beta", type: .indicator, source: validSource)
        XCTAssertEqual(renamed.id, script.id)
        XCTAssertFalse(FileManager.default.fileExists(atPath: scripts.appendingPathComponent("Alpha.pine").path))
        XCTAssertEqual(
            try String(contentsOf: scripts.appendingPathComponent("Beta.pine"), encoding: .utf8), validSource)
        let caseOnly = try await store.save(id: script.id, name: "BETA", type: .indicator, source: validSource)
        XCTAssertEqual(caseOnly.name, "BETA")
        let all = try await store.allScripts()
        XCTAssertEqual(all.map(\.name), ["BETA"])
    }

    func testRenameOnDiskKeepsIdentityAndFavorite() async throws {
        let (store, scripts, _) = try makeStore()
        let script = try await store.create(name: "Alpha", type: .indicator)
        try await store.setFavorite(id: script.id, true)
        try FileManager.default.moveItem(
            at: scripts.appendingPathComponent("Alpha.pine"), to: scripts.appendingPathComponent("Gamma.pine"))
        let all = try await store.allScripts()
        XCTAssertEqual(all.count, 1)
        XCTAssertEqual(all.first?.id, script.id)
        XCTAssertEqual(all.first?.name, "Gamma")
        XCTAssertEqual(all.first?.isFavorite, true)
    }

    func testExternalEditWithoutAttributeKeepsIdentityByName() async throws {
        let (store, scripts, _) = try makeStore()
        let script = try await store.create(name: "Alpha", type: .indicator)
        // Replace the file the way an editor's atomic save does, dropping extended attributes.
        let file = scripts.appendingPathComponent("Alpha.pine")
        try FileManager.default.removeItem(at: file)
        try Data(validSource.utf8).write(to: file)
        let reloaded = try await store.script(id: script.id)
        XCTAssertEqual(reloaded?.source, validSource)
    }

    func testStrayFileAndDuplicateAreDiscovered() async throws {
        let (store, scripts, _) = try makeStore()
        let script = try await store.create(name: "Alpha", type: .indicator)
        try FileManager.default.createDirectory(at: scripts, withIntermediateDirectories: true)
        try Data("//@version=6\nstrategy(\"S\")\n".utf8).write(to: scripts.appendingPathComponent("Dropped.pine"))
        try FileManager.default.copyItem(
            at: scripts.appendingPathComponent("Alpha.pine"), to: scripts.appendingPathComponent("Alpha copy.pine"))
        let all = try await store.allScripts()
        XCTAssertEqual(Set(all.map(\.name)), ["Alpha", "Alpha copy", "Dropped"])
        XCTAssertEqual(Set(all.map(\.id)).count, 3)
        XCTAssertEqual(all.first { $0.name == "Alpha" }?.id, script.id)
        XCTAssertEqual(all.first { $0.name == "Dropped" }?.type, .strategy)
    }

    func testImportCopiesAndDisambiguates() async throws {
        let (store, scripts, _) = try makeStore()
        _ = try await store.create(name: "Imported", type: .indicator)
        let outside = try temporaryDirectory().appendingPathComponent("Imported.pine")
        try Data(validSource.utf8).write(to: outside)
        let imported = try await store.importFile(at: outside)
        XCTAssertEqual(imported.name, "Imported 2")
        XCTAssertTrue(FileManager.default.fileExists(atPath: outside.path))
        XCTAssertEqual(
            try String(contentsOf: scripts.appendingPathComponent("Imported 2.pine"), encoding: .utf8), validSource)
        let insideID = try await store.scriptID(forFileAt: scripts.appendingPathComponent("Imported 2.pine"))
        let outsideID = try await store.scriptID(forFileAt: outside)
        XCTAssertEqual(insideID, imported.id)
        XCTAssertNil(outsideID)
    }

    func testLegacyJSONLayoutMigratesWithSameID() async throws {
        let (store, scripts, metadata) = try makeStore()
        let id = UUID()
        let legacy = LocalScript(
            id: id, name: "Legacy", type: .indicator, source: validSource, latestRevisionID: nil,
            createdAt: Date(timeIntervalSince1970: 0), modifiedAt: Date(timeIntervalSince1970: 0),
            lastOpenedAt: nil, isFavorite: true, compileRecord: nil)
        let dir = scripts.appendingPathComponent(id.uuidString, isDirectory: true)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        let encoder = JSONEncoder()
        encoder.dateEncodingStrategy = .iso8601
        try encoder.encode(legacy).write(to: dir.appendingPathComponent("script.json"))

        let all = try await store.allScripts()
        XCTAssertEqual(all.map(\.id), [id])
        XCTAssertEqual(all.first?.source, validSource)
        XCTAssertEqual(all.first?.isFavorite, true)
        XCTAssertFalse(FileManager.default.fileExists(atPath: dir.path))
        XCTAssertTrue(FileManager.default.fileExists(atPath: scripts.appendingPathComponent("Legacy.pine").path))
        XCTAssertTrue(
            FileManager.default.fileExists(
                atPath: metadata.appendingPathComponent("\(id.uuidString)/script.json").path))
    }

    func testCreatesEveryTypeAndDisambiguatesNames() async throws {
        let store = try makeStore().store
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

    func testBrokenSourceIsSavedWithErrorStatus() async throws {
        let (store, scripts, _) = try makeStore()
        let script = try await store.create(name: "Broken", type: .indicator)
        let source = "//@version=6\nindicator(\"Broken\")\nplot("
        let saved = try await store.save(id: script.id, name: script.name, type: .indicator, source: source)
        XCTAssertEqual(saved.compileRecord?.status, .error)
        XCTAssertFalse(saved.compileRecord?.diagnostics.isEmpty ?? true)
        XCTAssertEqual(
            try String(contentsOf: scripts.appendingPathComponent("Broken.pine"), encoding: .utf8), source)
        let revisions = try await store.revisions(id: script.id)
        XCTAssertEqual(revisions.map(\.compileStatus), [.error])
        let reloaded = try await store.script(id: script.id)
        XCTAssertEqual(reloaded?.compileRecord?.status, .error)
    }

    func testDraftDoesNotOverwriteSavedSource() async throws {
        let store = try makeStore().store
        let script = try await store.create(name: "Draft", type: .indicator)
        try await store.saveDraft(
            .init(scriptID: script.id, source: "unsaved", modifiedAt: Date(), basedOnRevisionID: nil))
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
