import XCTest

@testable import DegenView

/// `import user/Library/version as alias`: linking, scoping, exports and diagnostics.
final class PineLibraryImportTests: XCTestCase {
    private struct Stub: PineLibraryResolver {
        var sources: [String: String]
        func source(forLibrary path: String) -> String? { sources[path] }
    }

    private func bars(_ closes: [Double]) -> [KlineData] {
        closes.enumerated().map { i, c in
            .init(
                openTime: Date(timeIntervalSince1970: Double(i) * 60), openPrice: c, highPrice: c + 1,
                lowPrice: c - 1, closePrice: c, volume: 1)
        }
    }

    private func library(_ name: String, _ body: String) -> String {
        "//@version=6\nlibrary(\"\(name)\")\n\(body)\n"
    }

    private func script(_ body: String) -> String { "//@version=6\nindicator(\"T\")\n\(body)\n" }

    private func compile(_ source: String, _ libraries: [String: String]) -> PineCompiledProgram {
        PineCompiler.compile(source: source, limits: PineLimits(), libraries: Stub(sources: libraries))
    }

    private func codes(_ program: PineCompiledProgram) -> [String] { program.diagnostics.map(\.code) }

    private func plots(_ source: String, _ libraries: [String: String]) throws -> [[Double?]] {
        let program = compile(source, libraries)
        XCTAssertTrue(program.isValid, "\(program.diagnostics)")
        return try PineRuntimeSession(program: program).evaluate(bars: bars([1, 2, 3])).output.plots.map(\.values)
    }

    private let math = "me/MathLib/1"

    // MARK: - Functions

    func testAnImportedFunctionRunsThroughItsAlias() throws {
        let libs = [math: library("MathLib", "export twice(float x) =>\n    x * 2")]
        let result = try plots(script("import me/MathLib/1 as m\nplot(m.twice(close))"), libs)
        XCTAssertEqual(result, [[2, 4, 6]])
    }

    func testTheAliasDefaultsToTheLibraryName() throws {
        let libs = [math: library("MathLib", "export twice(float x) =>\n    x * 2")]
        let result = try plots(script("import me/MathLib/1\nplot(MathLib.twice(close))"), libs)
        XCTAssertEqual(result, [[2, 4, 6]])
    }

    func testALibraryFunctionCanCallItsPrivateHelpers() throws {
        let body = "inc(float x) =>\n    x + 1\n\nexport plusTwo(float x) =>\n    inc(inc(x))"
        let result = try plots(
            script("import me/MathLib/1 as m\nplot(m.plusTwo(close))"), [math: library("MathLib", body)])
        XCTAssertEqual(result, [[3, 4, 5]])
    }

    func testAPrivateFunctionCannotBeCalledByTheImporter() {
        let body = "inc(float x) =>\n    x + 1\n\nexport plusTwo(float x) =>\n    inc(inc(x))"
        let program = compile(
            script("import me/MathLib/1 as m\nplot(m.inc(close))"), [math: library("MathLib", body)])
        XCTAssertEqual(codes(program), ["PINE3044"])
    }

    func testAParameterDefaultBelongsToTheLibrary() throws {
        let body = "export scaled(float x, float k = FACTOR) =>\n    x * k\n\nFACTOR = 10.0"
        let result = try plots(
            script("import me/MathLib/1 as m\nplot(m.scaled(close))"), [math: library("MathLib", body)])
        XCTAssertEqual(result, [[10, 20, 30]])
    }

    func testALibraryAndTheScriptMayUseTheSameNames() throws {
        let libs = [math: library("MathLib", "export twice(float x) =>\n    x * 2")]
        let source = script(
            "import me/MathLib/1 as m\ntwice(float x) =>\n    x * 100\nplot(m.twice(close) + twice(close))")
        XCTAssertEqual(try plots(source, libs), [[102, 204, 306]])
    }

    func testALibraryLocalDoesNotTouchTheScriptsVariable() throws {
        let body = "export f(float v) =>\n    x = v + 1\n    x"
        let source = script("import me/MathLib/1 as m\nx = 10.0\nplot(m.f(close) + x)")
        XCTAssertEqual(try plots(source, [math: library("MathLib", body)]), [[12, 13, 14]])
    }

    // MARK: - Constants, types, enums, methods

    func testAnExportedConstantReadsThroughTheAlias() throws {
        let libs = [math: library("MathLib", "export const float RATE = 0.5")]
        let result = try plots(script("import me/MathLib/1 as m\nplot(close * m.RATE)"), libs)
        XCTAssertEqual(result, [[0.5, 1, 1.5]])
    }

    func testALibraryTypeIsConstructedAndReadAcrossTheBoundary() throws {
        let body = """
            export type Pt
                float x
                float y = 7

            export make(float a) =>
                Pt.new(a, a * 2)
            """
        let source = script(
            "import me/MathLib/1 as m\nm.Pt q = m.Pt.new(close)\np = m.make(close)\nplot(p.y + q.y)")
        XCTAssertEqual(try plots(source, [math: library("MathLib", body)]), [[9, 11, 13]])
    }

    func testALibraryTypeWorksInCollectionsAndFields() throws {
        let body = "export type Pt\n    float x\n    float y\n"
        let source = script(
            """
            import me/MathLib/1 as m
            type Holder
                m.Pt first
            pts = array.new<m.Pt>()
            pts.push(m.Pt.new(close, 2))
            h = Holder.new(pts.get(0))
            plot(h.first.x + pts.size())
            """)
        XCTAssertEqual(try plots(source, [math: library("MathLib", body)]), [[2, 3, 4]])
    }

    func testALibraryEnumCompares() throws {
        let body = """
            export enum Mode
                fast
                slow

            export isFast(Mode mode) =>
                mode == Mode.fast
            """
        let source = script(
            "import me/MathLib/1 as m\nplot(m.isFast(m.Mode.fast) ? 1 : 0)\nplot(m.isFast(m.Mode.slow) ? 1 : 0)")
        XCTAssertEqual(try plots(source, [math: library("MathLib", body)]), [[1, 1, 1], [0, 0, 0]])
    }

    func testALibraryMethodAppliesToItsType() throws {
        let body = """
            export type Pt
                float x
                float y

            export method scaled(Pt this, float k) =>
                Pt.new(this.x * k, this.y * k)
            """
        let source = script("import me/MathLib/1 as m\np = m.Pt.new(close, 1)\nplot(p.scaled(3).x)")
        XCTAssertEqual(try plots(source, [math: library("MathLib", body)]), [[3, 6, 9]])
    }

    func testAnImportedLibraryMayImportAnother() throws {
        let inner = library("Inner", "export one() =>\n    1.0")
        let outer = library("Outer", "import me/Inner/1 as i\nexport two() =>\n    i.one() + i.one()")
        let source = script("import me/Outer/1 as o\nplot(o.two())")
        XCTAssertEqual(try plots(source, ["me/Inner/1": inner, "me/Outer/1": outer]), [[2, 2, 2]])
    }

    // MARK: - Diagnostics

    func testAMissingLibraryIsReported() {
        XCTAssertEqual(codes(compile(script("import me/Nope/1 as n\nplot(close)"), [:])), ["PINE3040"])
    }

    func testACycleIsReported() {
        let a = library("A", "import me/B/1 as b\nexport f() =>\n    1.0")
        let b = library("B", "import me/A/1 as a\nexport g() =>\n    1.0")
        let program = compile(script("import me/A/1 as a\nplot(close)"), ["me/A/1": a, "me/B/1": b])
        XCTAssertTrue(codes(program).contains("PINE3041"), "\(codes(program))")
    }

    func testAMalformedPathIsReported() {
        XCTAssertEqual(codes(compile(script("import nope as n\nplot(close)"), [:])), ["PINE3042"])
    }

    func testAnIndicatorIsNotALibrary() {
        let libs = [math: script("plot(close)")]
        XCTAssertEqual(codes(compile(script("import me/MathLib/1 as m\nplot(close)"), libs)), ["PINE3043"])
    }

    func testALibraryWithErrorsIsRejected() {
        let libs = [math: library("MathLib", "export f(float x) =>\n    x *")]
        XCTAssertEqual(codes(compile(script("import me/MathLib/1 as m\nplot(close)"), libs)), ["PINE3043"])
    }

    func testADuplicateOrReservedAliasIsReported() {
        let libs = [
            math: library("MathLib", "export f() =>\n    1.0"),
            "me/Other/1": library("Other", "export g() =>\n    1.0"),
        ]
        let duplicate = compile(script("import me/MathLib/1 as m\nimport me/Other/1 as m\nplot(close)"), libs)
        XCTAssertEqual(codes(duplicate), ["PINE3045"])
        let reserved = compile(script("import me/MathLib/1 as math\nplot(close)"), libs)
        XCTAssertEqual(codes(reserved), ["PINE3045"])
    }

    func testAnImportWithoutAResolverIsNotFound() {
        let program = PineCompiler.compile(source: script("import me/MathLib/1 as m\nplot(close)"))
        XCTAssertEqual(codes(program), ["PINE3040"])
    }

    // MARK: - Registry

    private func local(_ name: String, _ type: ScriptType, _ source: String) -> LocalScript {
        LocalScript(
            id: UUID(), name: name, type: type, source: source, latestRevisionID: nil, createdAt: Date(),
            modifiedAt: Date(), lastOpenedAt: nil, isFavorite: false, compileRecord: nil)
    }

    func testTheRegistryFindsALibraryByNameAndIgnoresUserAndVersion() {
        let registry = PineLibraryRegistry()
        let source = library("MathLib", "export f() =>\n    1.0")
        registry.publish([local("MathLib", .library, source), local("Other", .indicator, script("plot(close)"))])
        XCTAssertEqual(registry.source(forLibrary: "anyone/MathLib/7"), source)
        XCTAssertNil(registry.source(forLibrary: "anyone/Other/1"))
        XCTAssertNil(registry.source(forLibrary: "MathLib"))
    }

    func testTheRegistryFallsBackToTheDeclaredTitle() {
        let registry = PineLibraryRegistry()
        let source = library("Declared Title", "export f() =>\n    1.0")
        registry.publish([local("renamed-file", .library, source)])
        XCTAssertEqual(registry.source(forLibrary: "me/Declared Title/1"), source)
        XCTAssertEqual(PineLibraryRegistry.title(of: "//@version=6\nlibrary('Single')\n"), "Single")
    }

    func testPublishingReplacesTheCatalog() {
        let registry = PineLibraryRegistry()
        registry.publish([local("MathLib", .library, library("MathLib", "export f() =>\n    1.0"))])
        registry.publish([])
        XCTAssertNil(registry.source(forLibrary: "me/MathLib/1"))
    }

    func testAScriptCompilesAgainstTheRegistry() throws {
        let registry = PineLibraryRegistry()
        registry.publish([local("MathLib", .library, library("MathLib", "export twice(float x) =>\n    x * 2"))])
        let program = PineCompiler.compile(
            source: script("import me/MathLib/1 as m\nplot(m.twice(close))"), libraries: registry)
        XCTAssertTrue(program.isValid, "\(program.diagnostics)")
    }
}
