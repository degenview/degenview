import XCTest

@testable import DegenView

/// Script-defined `type`s: parsing, construction, fields, mutation and methods.
final class PineUserTypeTests: XCTestCase {
    private func bars(_ closes: [Double], spacing: TimeInterval = 60) -> [KlineData] {
        closes.enumerated().map { i, c in
            .init(
                openTime: Date(timeIntervalSince1970: Double(i) * spacing), openPrice: c,
                highPrice: c + 1, lowPrice: c - 1, closePrice: c, volume: Double(i + 1))
        }
    }

    private func compile(_ body: String, header: String = "indicator(\"T\")") -> PineCompiledProgram {
        PineCompiler.compile(source: "//@version=6\n\(header)\n\(body)")
    }

    private func codes(_ program: PineCompiledProgram) -> [String] { program.diagnostics.map(\.code) }

    // MARK: - Parsing

    func testTypeDeclarationParsesFieldsDefaultsAndNestedTypes() throws {
        let program = compile(
            """
            type Point
                float x = 0.0
                float y

            type Zone
                Point origin
                array<float> history
                string label = "zone"
                int bars
                Zone next

            plot(close)
            """)
        XCTAssertTrue(program.isValid, "\(program.diagnostics)")
        let declarations = program.statements.compactMap { statement -> (String, [PineTypeField])? in
            if case .typeDeclaration(let name, let fields, _) = statement { return (name, fields) }
            return nil
        }
        XCTAssertEqual(declarations.map(\.0), ["Point", "Zone"])
        XCTAssertEqual(declarations[1].1.map(\.name), ["origin", "history", "label", "bars", "next"])
        XCTAssertEqual(declarations[1].1.map(\.type), [.object, .array, .string, .int, .object])
        XCTAssertNotNil(declarations[1].1[2].defaultValue)
    }

    func testDeclaredTypeNameWorksInDeclarationsAndParameters() {
        let program = compile(
            """
            type Zone
                float top

            var Zone z = na
            f(Zone zone, array<Zone> all) =>
                zone.top
            plot(close)
            """)
        XCTAssertTrue(program.isValid, "\(program.diagnostics)")
    }

    func testTypeAndEnumRemainUsableAsVariableNames() {
        XCTAssertTrue(compile("type = 1\nenum = type + 1\nplot(enum)").isValid)
    }

    func testAnUndeclaredTypeNameIsNotADeclaration() {
        XCTAssertFalse(compile("Zone z = na\nplot(close)").isValid)
    }

    func testFieldWithoutANameIsADiagnostic() {
        let program = compile("type Zone\n    float\nplot(close)")
        XCTAssertTrue(codes(program).contains("PINE2015"), "\(codes(program))")
    }

    func testTypeWithoutFieldsBlockIsADiagnostic() {
        XCTAssertTrue(codes(compile("type Zone\nplot(close)")).contains("PINE2002"))
    }

    func testExportedTypeIsAcceptedInALibrary() {
        let source = "//@version=6\nlibrary(\"L\")\nexport type Zone\n    float top\n"
        XCTAssertTrue(PineCompiler.compile(source: source).isValid)
        XCTAssertTrue(codes(compile("export type Zone\n    float top\n")).contains("PINE3037"))
    }

    // MARK: - Objects

    private func values(_ body: String, bars series: [Double] = [1]) throws -> [[Double?]] {
        let program = compile(body)
        XCTAssertTrue(program.isValid, "\(program.diagnostics)")
        return try PineRuntimeSession(program: program).evaluate(bars: bars(series)).output.plots.map(\.values)
    }

    private func runtimeError(_ body: String) -> String? {
        let program = compile(body)
        XCTAssertTrue(program.isValid, "\(program.diagnostics)")
        do {
            _ = try PineRuntimeSession(program: program).evaluate(bars: bars([1]))
            return nil
        } catch {
            return (error as? PineDiagnostic)?.code
        }
    }

    func testConstructorTakesPositionalNamedAndDefaultFields() throws {
        let plots = try values(
            """
            type Zone
                float top
                float bottom = 2.0 * 3
                int bars = 4
                string label

            a = Zone.new(1.5, 2.5, 3)
            b = Zone.new(bars = 9, top = 7.0)
            c = Zone.new()
            plot(a.top)
            plot(a.bottom)
            plot(a.bars)
            plot(b.top)
            plot(b.bottom)
            plot(b.bars)
            plot(na(c.top) ? 1 : 0)
            plot(c.bars)
            plot(na(c.label) ? 1 : 0)
            """)
        XCTAssertEqual(plots.map { $0[0] }, [1.5, 2.5, 3, 7, 6, 9, 1, 4, 1])
    }

    func testVarObjectIsCreatedOnceAndPersists() throws {
        let plots = try values(
            """
            type Anchor
                int born

            var Anchor anchor = Anchor.new(bar_index)
            plot(anchor.born)
            """, bars: [1, 2, 3])
        XCTAssertEqual(plots, [[0, 0, 0]])
    }

    func testNestedObjectsAndHandlesAreReachableThroughFieldPaths() throws {
        let plots = try values(
            """
            type Point
                float x
                float y

            type Marker
                Point origin
                line ln

            m = Marker.new(Point.new(1.0, 2.0), line.new(0, 1.0, 1, 2.0))
            m.ln.set_y2(9.0)
            plot(m.origin.y)
            """)
        XCTAssertEqual(plots, [[2]])
        let program = compile(
            """
            type Marker
                line ln

            m = Marker.new(line.new(0, 1.0, 1, 2.0))
            m.ln.set_y2(9.0)
            """)
        let output = try PineRuntimeSession(program: program).evaluate(bars: bars([1, 2])).output
        XCTAssertEqual(output.lines.first?.y2, 9)
    }

    func testObjectsStoredInAnArrayComeBackAsObjects() throws {
        let plots = try values(
            """
            type Zone
                float top

            var array<Zone> zones = array.new<Zone>()
            array.push(zones, Zone.new(5.0))
            array.push(zones, Zone.new(8.0))
            Zone second = array.get(zones, 1)
            plot(second.top)
            plot(array.size(zones))
            """)
        XCTAssertEqual(plots, [[8], [2]])
    }

    func testReadingAFieldOfNaIsARuntimeError() {
        let error = runtimeError(
            """
            type Zone
                float top

            Zone z = na
            plot(z.top)
            """)
        XCTAssertEqual(error, "PINE4018")
    }

    func testUnknownFieldAndUnknownConstructorArgumentAreRuntimeErrors() {
        let header = "type Zone\n    float top\n\n"
        XCTAssertEqual(runtimeError(header + "z = Zone.new(1.0)\nplot(z.bottom)"), "PINE4019")
        XCTAssertEqual(runtimeError(header + "z = Zone.new(bottom = 1.0)\nplot(close)"), "PINE4019")
    }

    func testMethodsOnNonObjectsAndUnknownMembersStillFail() {
        let header = "type Zone\n    float top\n\n"
        XCTAssertEqual(runtimeError(header + "z = Zone.new(1.0)\nplot(z.nothing())"), "PINE4007")
    }
}
