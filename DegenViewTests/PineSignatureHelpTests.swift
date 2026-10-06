import XCTest

@testable import DegenView

final class PineSignatureHelpTests: XCTestCase {
    private let library = StubLibraryExports(exports: [
        "user/MathLib/1": [
            PineLibraryExport(
                name: "myEMA", kind: .function,
                parameters: [
                    .init(name: "source", typeText: "float", defaultText: nil),
                    .init(name: "length", typeText: "int", defaultText: "14"),
                ])
        ]
    ])

    private func help(_ marked: String) -> PineSignatureHelp? {
        let fixture = PineCompletionFixture(marked)
        return PineSignatureResolver.help(for: fixture.context, analysis: fixture.analysis, libraries: library)
    }

    private func activeName(_ help: PineSignatureHelp?) -> String? {
        guard let help, let index = help.activeParameter else { return nil }
        return help.signature.parameters[index].name
    }

    func testSecondArgumentOfSma() throws {
        let result = try XCTUnwrap(help("ta.sma(close, |"))
        XCTAssertEqual(result.callee, "ta.sma")
        XCTAssertEqual(result.signature.label, "ta.sma(source, length) → series float")
        XCTAssertEqual(activeName(result), "length")
    }

    func testFirstArgumentRightAfterTheParenthesis() {
        XCTAssertEqual(activeName(help("ta.rsi(|")), "source")
        XCTAssertEqual(activeName(help("ta.rsi(|)")), "source")
    }

    func testNestedCallsReportTheOuterOne() {
        let result = help("ta.ema(ta.sma(close, 20), |")
        XCTAssertEqual(result?.callee, "ta.ema")
        XCTAssertEqual(activeName(result), "length")
        XCTAssertEqual(help("ta.ema(ta.sma(close, |20), 5)")?.callee, "ta.sma")
        XCTAssertEqual(activeName(help("ta.ema(\n    ta.sma(close, 20),\n    |\n)")), "length")
    }

    func testCommasInStringsAndCommentsDoNotAdvanceTheArgument() {
        let source = "foo(a, b) =>\n    a\n"
        XCTAssertEqual(activeName(help(source + "foo(\"a,b\", |")), "b")
        XCTAssertEqual(activeName(help(source + "foo(\"a,b\"|")), "a")
        XCTAssertEqual(activeName(help(source + "foo(1, // x, y, z\n    |")), "b")
        XCTAssertEqual(activeName(help(source + "foo(bar(1, 2, 3), |")), "b")
    }

    func testNamedArgumentSetsTheActiveParameter() {
        XCTAssertEqual(activeName(help("plot(close, title = |")), "title")
        XCTAssertEqual(activeName(help("plot(close, linewidth = 2, co|")), "color")
    }

    func testGroupingParenthesesAreSkipped() {
        XCTAssertEqual(help("plot((1 + |")?.callee, "plot")
        XCTAssertNil(help("x = (1 + |"))
        XCTAssertNil(help("if (a > |"))
    }

    func testUserFunctionsAndTheirDefaults() throws {
        let result = try XCTUnwrap(help("myAverage(source, float length = 14) =>\n    source\nmyAverage(close, |"))
        XCTAssertEqual(result.signature.label, "myAverage(source, length)")
        XCTAssertEqual(result.signature.parameters.last?.defaultValue, "14")
        XCTAssertEqual(activeName(result), "length")
    }

    func testAUserNameHidesTheBuiltinSignature() {
        XCTAssertEqual(help("sma(a) =>\n    a\nsma(|")?.signature.label, "sma(a)")
        XCTAssertNil(help("close = 1\nclose(|"), "a variable is not callable")
    }

    func testLibraryFunctions() throws {
        let result = try XCTUnwrap(help("import user/MathLib/1 as m\nx = m.myEMA(close, |"))
        XCTAssertEqual(result.callee, "m.myEMA")
        XCTAssertEqual(activeName(result), "length")
    }

    func testConstructorsAndMethods() {
        XCTAssertEqual(activeName(help("type Point\n    float x\n    float y\nPoint.new(1, |")), "y")
        let method = "type Box\n    float w\nmethod grow(Box this, float by) =>\n    this.w\nb = Box.new(1)\nb.grow(|"
        XCTAssertEqual(help(method)?.signature.label, "b.grow(by)")
    }

    func testOverloadsAreKeptAndTheBestFitComesFirst() throws {
        let result = try XCTUnwrap(help("ta.highest(close, |"))
        XCTAssertEqual(result.signatures.count, 2)
        XCTAssertEqual(result.signature.parameters.map(\.name), ["source", "length"])
        let single = try XCTUnwrap(help("math.round(close, |"))
        XCTAssertEqual(single.signature.returns, "series float", "the precision form fits a second argument")
    }

    func testUnknownCalleesAndSuppressedPlacesHaveNoHelp() {
        XCTAssertNil(help("nothing.known(|"))
        XCTAssertNil(help("// ta.sma(|"))
        XCTAssertNil(help("x = \"ta.sma(|\""))
        XCTAssertNil(help("x = |"))
    }

    func testTooManyArgumentsHaveNoActiveParameter() {
        let result = help("ta.rsi(close, 14, 3, |")
        XCTAssertNotNil(result)
        XCTAssertNil(result?.activeParameter)
    }

    func testUnclosedCallsStayResolvable() {
        XCTAssertEqual(help("x = ta.sma(close,\n  |")?.callee, "ta.sma")
        XCTAssertEqual(help("plot(\nx = 1\n|")?.callee, nil)
    }
}
