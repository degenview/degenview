import XCTest

@testable import DegenView

final class PineLibraryExportsTests: XCTestCase {
    private let source = """
        //@version=6
        library("TestLib")
        helper(x) =>
            x * 2
        privateBase = 10
        export myEMA(float source, int length = 14) =>
            ta.ema(source, length)
        export method area(Shape this) =>
            this.w
        export type Shape
            float w
        export enum Mode
            fast
        export ratio = 2.0
        """

    private func registry(_ text: String? = nil) -> PineLibraryRegistry {
        let registry = PineLibraryRegistry()
        registry.publish([
            LocalScript(
                id: UUID(), name: "TestLib", type: .library, source: text ?? source, latestRevisionID: nil,
                createdAt: Date(), modifiedAt: Date(), lastOpenedAt: nil, isFavorite: false, compileRecord: nil)
        ])
        return registry
    }

    func testOnlyExportedNamesAreListed() throws {
        let directory = PineLibraryExportDirectory(registry: registry())
        let exports = try XCTUnwrap(directory.exports(forImportPath: "user/TestLib/1"))
        XCTAssertEqual(Set(exports.map(\.name)), ["myEMA", "area", "Shape", "Mode", "ratio"])
        XCTAssertFalse(exports.contains { $0.name == "helper" || $0.name == "privateBase" })
    }

    func testExportsCarryTheirKindAndParameters() throws {
        let directory = PineLibraryExportDirectory(registry: registry())
        let exports = try XCTUnwrap(directory.exports(forImportPath: "user/TestLib/1"))
        let ema = try XCTUnwrap(exports.first { $0.name == "myEMA" })
        XCTAssertEqual(ema.kind, .function)
        XCTAssertEqual(ema.signatureText, "myEMA(source, length)")
        XCTAssertEqual(ema.parameters?.last?.defaultText, "14")
        XCTAssertEqual(exports.first { $0.name == "area" }?.kind, .method)
        XCTAssertEqual(exports.first { $0.name == "Shape" }?.kind, .type)
        XCTAssertEqual(exports.first { $0.name == "Mode" }?.kind, .enumeration)
        XCTAssertEqual(exports.first { $0.name == "ratio" }?.kind, .variable)
        XCTAssertNil(exports.first { $0.name == "ratio" }?.parameters)
    }

    func testUserAndVersionAreIgnoredAndUnknownLibrariesAreNil() {
        let directory = PineLibraryExportDirectory(registry: registry())
        XCTAssertNotNil(directory.exports(forImportPath: "someone/TestLib/42"))
        XCTAssertNil(directory.exports(forImportPath: "user/Missing/1"))
        XCTAssertNil(directory.exports(forImportPath: "user/"))
    }

    func testNamesComeFromTheRegistry() {
        XCTAssertEqual(PineLibraryExportDirectory(registry: registry()).libraryNames(), ["TestLib"])
    }

    func testALookupIsRememberedUntilTheLibraryChanges() throws {
        let registry = registry()
        let directory = PineLibraryExportDirectory(registry: registry)
        let first = try XCTUnwrap(directory.exports(forImportPath: "user/TestLib/1"))
        let lexes = PineLexicalSnapshot.buildCount
        XCTAssertEqual(directory.exports(forImportPath: "user/TestLib/1"), first)
        XCTAssertEqual(PineLexicalSnapshot.buildCount, lexes, "a repeat lookup does no work")

        registry.publish([
            LocalScript(
                id: UUID(), name: "TestLib", type: .library, source: source + "\nexport extra() =>\n    1\n",
                latestRevisionID: nil, createdAt: Date(), modifiedAt: Date(), lastOpenedAt: nil, isFavorite: false,
                compileRecord: nil)
        ])
        XCTAssertTrue(try XCTUnwrap(directory.exports(forImportPath: "user/TestLib/1")).contains { $0.name == "extra" })
    }

    func testLookingUpALibraryDoesNotEvictTheEditedScriptsLex() {
        let directory = PineLibraryExportDirectory(registry: registry())
        let script = "x = 1\nplot(x)"
        let first = PineLexicalSnapshot.shared(for: script)
        _ = directory.exports(forImportPath: "user/TestLib/1")
        let lexes = PineLexicalSnapshot.buildCount
        let second = PineLexicalSnapshot.shared(for: script)
        XCTAssertEqual(PineLexicalSnapshot.buildCount, lexes)
        XCTAssertEqual(first.items, second.items)
    }

    func testCompletionAndSignatureHelpUseTheRealRegistry() throws {
        let directory = PineLibraryExportDirectory(registry: registry())
        let members = PineCompletionFixture("import user/TestLib/1 as lib\nx = lib.|").items(libraries: directory)
        XCTAssertEqual(Set(members.map(\.label)), ["myEMA", "area", "Shape", "Mode", "ratio"])

        let call = PineCompletionFixture("import user/TestLib/1 as lib\nx = lib.myEMA(close, |")
        let help = try XCTUnwrap(
            PineSignatureResolver.help(for: call.context, analysis: call.analysis, libraries: directory))
        XCTAssertEqual(help.callee, "lib.myEMA")
        XCTAssertEqual(help.activeParameter, 1)

        XCTAssertEqual(
            PineCompletionFixture("import user/|").labels(libraries: directory), ["TestLib"])
    }

    func testAMissingLibraryDegradesToNothing() {
        let directory = PineLibraryExportDirectory(registry: PineLibraryRegistry())
        XCTAssertEqual(PineCompletionFixture("import user/Gone/1 as g\nx = g.|").labels(libraries: directory), [])
        XCTAssertEqual(PineCompletionFixture("import user/|").labels(libraries: directory), [])
    }
}
