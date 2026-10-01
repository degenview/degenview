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
}
