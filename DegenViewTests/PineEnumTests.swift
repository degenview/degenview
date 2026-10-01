import XCTest

@testable import DegenView

/// Script-defined `enum`s: declarations, values, comparisons, inputs and titles.
final class PineEnumTests: XCTestCase {
    private func bars(_ closes: [Double]) -> [KlineData] {
        closes.enumerated().map { i, c in
            .init(
                openTime: Date(timeIntervalSince1970: Double(i) * 60), openPrice: c, highPrice: c + 1,
                lowPrice: c - 1, closePrice: c, volume: 1)
        }
    }

    private func compile(_ body: String, header: String = "indicator(\"T\")") -> PineCompiledProgram {
        PineCompiler.compile(source: "//@version=6\n\(header)\n\(body)")
    }

    private func codes(_ program: PineCompiledProgram) -> [String] { program.diagnostics.map(\.code) }

    private func plots(_ body: String, inputs: [String: PineInputValue] = [:]) throws -> [[Double?]] {
        let program = compile(body)
        XCTAssertTrue(program.isValid, "\(program.diagnostics)")
        return try PineRuntimeSession(program: program, inputs: inputs).evaluate(bars: bars([1, 2])).output.plots
            .map(\.values)
    }

    private let mode = "enum Mode\n    fast = \"Fast\"\n    slow\n    off = \"Switched off\"\n\n"

    // MARK: - Parsing

    func testEnumParsesMembersAndTitles() throws {
        let program = compile(mode + "plot(close)")
        XCTAssertTrue(program.isValid, "\(program.diagnostics)")
        let members = program.statements.compactMap { statement -> [PineEnumMember]? in
            if case .enumDeclaration("Mode", let members, _) = statement { return members }
            return nil
        }.first
        XCTAssertEqual(
            members,
            [
                PineEnumMember(name: "fast", title: "Fast"), PineEnumMember(name: "slow", title: "slow"),
                PineEnumMember(name: "off", title: "Switched off"),
            ])
    }

    func testAnEnumTitleMustBeAStringLiteral() {
        XCTAssertTrue(codes(compile("enum Mode\n    fast = 1\nplot(close)")).contains("PINE2017"))
    }

    func testAnEnumWithoutAMembersBlockIsADiagnostic() {
        XCTAssertTrue(codes(compile("enum Mode\nplot(close)")).contains("PINE2002"))
    }

    func testEnumRemainsUsableAsAVariableName() {
        XCTAssertTrue(compile("enum = 1\nplot(enum + 1)").isValid)
    }

    func testExportedEnumIsAcceptedInALibraryAndRejectedElsewhere() {
        let body = "export enum Mode\n    fast\n    slow\n"
        XCTAssertTrue(PineCompiler.compile(source: "//@version=6\nlibrary(\"L\")\n\(body)").isValid)
        XCTAssertTrue(codes(compile(body)).contains("PINE3037"))
    }

    func testDeclaredEnumNameAnnotatesVariablesAndParameters() {
        let program = compile(mode + "Mode current = Mode.fast\nf(Mode m) => m == Mode.slow\nplot(f(current) ? 1 : 0)")
        XCTAssertTrue(program.isValid, "\(program.diagnostics)")
    }

    // MARK: - Values

    func testMembersCompareAndSwitch() throws {
        let result = try plots(
            mode + """
                Mode current = Mode.slow
                plot(current == Mode.slow ? 1 : 0)
                plot(current == Mode.fast ? 1 : 0)
                picked = switch current
                    Mode.fast => 10
                    Mode.slow => 20
                    => 30
                plot(picked)
                """)
        XCTAssertEqual(result.map { $0[0] }, [1, 0, 20])
    }

    func testAMemberTheEnumDoesNotHaveIsARuntimeError() {
        let program = compile(mode + "plot(Mode.fats == Mode.fast ? 1 : 0)")
        XCTAssertTrue(program.isValid)
        XCTAssertThrowsError(try PineRuntimeSession(program: program).evaluate(bars: bars([1]))) {
            XCTAssertEqual(($0 as? PineDiagnostic)?.code, "PINE4025")
        }
    }

    func testToStringReturnsTheTitleOrTheNameWhenThereIsNone() throws {
        let result = try plots(
            mode + """
                plot(str.length(str.tostring(Mode.off)))
                plot(str.length(str.tostring(Mode.slow)))
                plot(str.length(str.tostring("plain")))
                """)
        XCTAssertEqual(result.map { $0[0] }, [12, 4, 5])
    }

    // MARK: - input.enum

    func testInputEnumBecomesAStringInputWithTitledOptions() throws {
        let program = compile(
            mode
                + "pick = input.enum(Mode.slow, \"Mode\", group = \"G\", tooltip = \"t\")\nplot(pick == Mode.slow ? 1 : 0)"
        )
        XCTAssertTrue(program.isValid, "\(program.diagnostics)")
        let input = try XCTUnwrap(program.inputSchema.inputs.first)
        XCTAssertEqual(input.type, .string)
        XCTAssertEqual(input.defaultValue, .string("Mode.slow"))
        XCTAssertEqual(input.options, [.string("Mode.fast"), .string("Mode.slow"), .string("Mode.off")])
        XCTAssertEqual(input.optionTitles, ["Fast", "slow", "Switched off"])
        XCTAssertEqual([input.title, input.group, input.tooltip], ["Mode", "G", "t"])
    }

    func testInputEnumDefaultAndOverrideReachTheScript() throws {
        let body =
            mode
            + "pick = input.enum(Mode.slow, \"Mode\")\nplot(pick == Mode.off ? 1 : 0)\nplot(pick == Mode.slow ? 1 : 0)"
        XCTAssertEqual(try plots(body).map { $0[0] }, [0, 1])
        XCTAssertEqual(try plots(body, inputs: ["pick": .string("Mode.off")]).map { $0[0] }, [1, 0])
    }

    func testInputEnumOptionsCanBeRestrictedToAList() throws {
        let program = compile(
            mode + "pick = input.enum(Mode.fast, \"Mode\", options = [Mode.fast, Mode.off])\nplot(close)")
        XCTAssertTrue(program.isValid, "\(program.diagnostics)")
        let input = try XCTUnwrap(program.inputSchema.inputs.first)
        XCTAssertEqual(input.options, [.string("Mode.fast"), .string("Mode.off")])
        XCTAssertEqual(input.optionTitles, ["Fast", "Switched off"])
    }

    func testInputEnumNeedsAMemberOfADeclaredEnum() {
        XCTAssertTrue(codes(compile(mode + "pick = input.enum(Mode.nope, \"Mode\")\nplot(close)")).contains("PINE3010"))
        XCTAssertTrue(codes(compile("pick = input.enum(Other.a, \"Mode\")\nplot(close)")).contains("PINE3010"))
        XCTAssertTrue(codes(compile("pick = input.enum(5, \"Mode\")\nplot(close)")).contains("PINE3010"))
    }

    func testInputEnumInsideAnIfBlockStillFindsTheEnum() throws {
        let program = compile(mode + "if barstate.isfirst\n    pick = input.enum(Mode.fast, \"Mode\")\nplot(close)")
        XCTAssertEqual(program.inputSchema.inputs.first?.optionTitles, ["Fast", "slow", "Switched off"])
    }
}
