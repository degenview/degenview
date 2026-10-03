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

    func testSavingAnImporterCompilesAgainstTheStoresLibraries() async throws {
        let root = try temporaryDirectory()
        let registry = PineLibraryRegistry()
        let store = ScriptStore(
            scriptsDirectory: root.appendingPathComponent("Scripts", isDirectory: true),
            metadataDirectory: root.appendingPathComponent("ScriptMetadata", isDirectory: true),
            libraries: registry)
        let lib = try await store.create(
            name: "MathLib", type: .library,
            source: "//@version=6\nlibrary(\"MathLib\")\nexport twice(float x) =>\n    x * 2\n")
        let user = try await store.create(
            name: "User", type: .indicator,
            source: "//@version=6\nindicator(\"U\")\nimport me/MathLib/1 as m\nplot(m.twice(close))\n")
        let saved = try await store.save(id: user.id, source: user.source)
        XCTAssertEqual(saved.compileRecord?.status, .valid)
        XCTAssertNotNil(registry.source(forLibrary: "me/\(lib.name)/1"))
    }

    func testRenameMovesFileAndKeepsID() async throws {
        let (store, scripts, _) = try makeStore()
        let script = try await store.create(name: "Alpha", type: .indicator)
        _ = try await store.save(id: script.id, source: validSource)
        let renamed = try await store.rename(id: script.id, to: "Beta")
        XCTAssertEqual(renamed.id, script.id)
        XCTAssertFalse(FileManager.default.fileExists(atPath: scripts.appendingPathComponent("Alpha.pine").path))
        XCTAssertEqual(
            try String(contentsOf: scripts.appendingPathComponent("Beta.pine"), encoding: .utf8), validSource)
        let caseOnly = try await store.rename(id: script.id, to: "BETA")
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
        let saved = try await store.save(id: script.id, source: source)
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

    // MARK: - Names

    func testValidatorAcceptsAndNormalizes() {
        for (raw, expected) in [
            ("Alpha", "Alpha"), ("My Script v2", "My Script v2"), ("  Padded  ", "Padded"),
            ("foo.pine", "foo"), ("Foo.PINE", "Foo"), ("v1.2", "v1.2"),
        ] {
            XCTAssertEqual(try? ScriptNameValidator.validate(raw).get(), expected, raw)
        }
    }

    func testValidatorRejectsUnsafeNames() {
        let rejected = [
            "", "   ", ".", "..", ".hidden", ".pine", "../x", "..\\x", "a/b", "a\\b", "a:b",
            "/etc/passwd", "nul\u{0}byte", "line\nbreak", String(repeating: "a", count: 251),
        ]
        for raw in rejected {
            XCTAssertThrowsError(try ScriptNameValidator.validate(raw).get(), raw.debugDescription)
        }
        XCTAssertNoThrow(try ScriptNameValidator.validate(String(repeating: "a", count: 250)).get())
    }

    func testCreateRejectsInvalidNameWithoutWritingFiles() async throws {
        let (store, scripts, _) = try makeStore()
        do {
            _ = try await store.create(name: "../escape", type: .indicator)
            XCTFail("expected failure")
        } catch {}
        let files = (try? FileManager.default.contentsOfDirectory(atPath: scripts.path)) ?? []
        XCTAssertTrue(files.isEmpty)
        let escaped = scripts.deletingLastPathComponent().appendingPathComponent("escape.pine")
        XCTAssertFalse(FileManager.default.fileExists(atPath: escaped.path))
    }

    func testCreateWithoutDisambiguationThrowsOnConflict() async throws {
        let (store, _, _) = try makeStore()
        _ = try await store.create(name: "Alpha", type: .indicator)
        do {
            _ = try await store.create(name: "alpha", type: .indicator, disambiguating: false)
            XCTFail("expected nameConflict")
        } catch {
            XCTAssertEqual(error as? ScriptStoreError, .nameConflict)
        }
        let all = try await store.allScripts()
        XCTAssertEqual(all.count, 1)
    }

    func testRenameRejectsInvalidAndConflictingNamesLeavingFilesAlone() async throws {
        let (store, scripts, _) = try makeStore()
        let alpha = try await store.create(name: "Alpha", type: .indicator)
        _ = try await store.create(name: "Beta", type: .indicator)
        for bad in ["../Alpha2", "a/b", "", ".x"] {
            do {
                _ = try await store.rename(id: alpha.id, to: bad)
                XCTFail("expected failure for \(bad)")
            } catch {}
        }
        do {
            _ = try await store.rename(id: alpha.id, to: "beta")
            XCTFail("expected nameConflict")
        } catch {
            XCTAssertEqual(error as? ScriptStoreError, .nameConflict)
        }
        XCTAssertTrue(FileManager.default.fileExists(atPath: scripts.appendingPathComponent("Alpha.pine").path))
        let names = try await store.allScripts().map(\.name).sorted()
        XCTAssertEqual(names, ["Alpha", "Beta"])
    }

    func testRenameKeepsSourceAndRevisions() async throws {
        let (store, _, _) = try makeStore()
        let script = try await store.create(name: "Alpha", type: .indicator)
        _ = try await store.save(id: script.id, source: validSource)
        let before = try await store.revisions(id: script.id).count
        let renamed = try await store.rename(id: script.id, to: "Gamma.pine")
        XCTAssertEqual(renamed.name, "Gamma")
        XCTAssertEqual(renamed.source, validSource)
        let after = try await store.revisions(id: script.id).count
        XCTAssertEqual(before, after)
    }

    func testResolvedSourcePinsToAnOlderRevisionAndFlagsItNotLatest() async throws {
        let (store, _, _) = try makeStore()
        let script = try await store.create(name: "Alpha", type: .indicator)
        let first = try await store.save(id: script.id, source: validSource)
        let second = try await store.save(
            id: script.id, source: "//@version=6\nindicator(\"Valid\")\nplot(close + 1)\n")
        let firstRevisionID = try XCTUnwrap(first.latestRevisionID)
        let secondRevisionID = try XCTUnwrap(second.latestRevisionID)
        XCTAssertNotEqual(firstRevisionID, secondRevisionID)

        let old = try await store.resolvedSource(scriptID: script.id, revisionID: firstRevisionID)
        XCTAssertEqual(old?.source, validSource)
        XCTAssertEqual(old?.isLatest, false)

        let latest = try await store.resolvedSource(scriptID: script.id, revisionID: secondRevisionID)
        XCTAssertEqual(latest?.source, second.source)
        XCTAssertEqual(latest?.isLatest, true)
    }

    func testResolvedSourceReturnsNilWhenScriptNoLongerExists() async throws {
        let (store, _, _) = try makeStore()
        let script = try await store.create(name: "Alpha", type: .indicator)
        let saved = try await store.save(id: script.id, source: validSource)
        try await store.delete(id: script.id)

        let result = try await store.resolvedSource(
            scriptID: script.id, revisionID: try XCTUnwrap(saved.latestRevisionID))
        XCTAssertNil(result)
    }

    // MARK: - Type detection

    func testSaveDetectsTypeFromSource() async throws {
        let (store, _, _) = try makeStore()
        let script = try await store.create(name: "Alpha", type: .indicator)
        let strategy = try await store.save(
            id: script.id, source: "//@version=6\nstrategy(\"S\", overlay=true)\n")
        XCTAssertEqual(strategy.type, .strategy)
        let library = try await store.save(id: script.id, source: "//@version=6\nlibrary(\"L\")\n")
        XCTAssertEqual(library.type, .library)
        let back = try await store.save(id: script.id, source: validSource)
        XCTAssertEqual(back.type, .indicator)
    }

    func testSaveKeepsTypeWhenDeclarationIsMissing() async throws {
        let (store, _, _) = try makeStore()
        let script = try await store.create(name: "Alpha", type: .indicator)
        _ = try await store.save(id: script.id, source: "//@version=6\nstrategy(\"S\")\n")
        let broken = try await store.save(id: script.id, source: "//@version=6\nplot(close)\n")
        XCTAssertEqual(broken.type, .strategy)
    }
}
