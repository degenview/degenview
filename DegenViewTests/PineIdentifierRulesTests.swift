import XCTest

@testable import DegenView

final class PineIdentifierRulesTests: XCTestCase {
    private func compile(_ body: String) -> PineCompiledProgram {
        PineCompiler.compile(source: "//@version=6\nindicator(\"T\", overlay=true)\n" + body)
    }

    private func codes(_ body: String) -> [String] { compile(body).diagnostics.map(\.code) }

    // MARK: - Shadowing builtins (CE10190)

    func testShadowingBuiltinVariableIsAnError() throws {
        let program = compile("close = 5\n")
        XCTAssertFalse(program.isValid)
        let diagnostic = try XCTUnwrap(program.diagnostics.first)
        XCTAssertEqual(diagnostic.code, "CE10190")
        XCTAssertEqual(diagnostic.severity, .error)
        XCTAssertEqual(diagnostic.message, "Cannot shadow the built-in variable \"close\".")
        // Underlines the name on line 3, after the version and indicator lines.
        XCTAssertEqual(diagnostic.range.start.line, 3)
        XCTAssertEqual(diagnostic.range.start.column, 1)
        XCTAssertEqual(diagnostic.range.end.column, 6)
    }

    func testEveryDeclarationFormShadowsBuiltins() {
        for body in [
            "float volume = 1",
            "var float high = 1",
            "int bar_index = 1",
            "[open, hist] = ta.macd(close, 12, 26, 9)",
            "[macdLine, low] = ta.macd(close, 12, 26, 9)",
            "f(close) =>\n    close + 1",
            "f(x, hl2) =>\n    x + hl2",
            "for time = 0 to 2\n    int j = time",
            "for close in array.new_float(2, 1.0)\n    int j = 1",
            "for [i, open] in array.new_float(2, 1.0)\n    int j = i",
            "g() =>\n    float low = 1\n    low",
            "if close > open\n    float high = 1",
        ] {
            XCTAssertTrue(codes(body + "\n").contains("CE10190"), body)
        }
    }

    func testShadowingBuiltinFunctionIsAnError() throws {
        for body in ["plot = 3", "input = 1", "nz(x) =>\n    x", "bgcolor = 2"] {
            XCTAssertTrue(codes(body + "\n").contains("CE10190"), body)
        }
        let diagnostic = try XCTUnwrap(compile("plot = 3\n").diagnostics.first)
        XCTAssertEqual(diagnostic.message, "Cannot shadow the built-in function \"plot\".")
    }

    func testNamespaceOnlyNamesStayUsable() {
        for body in ["math = 44", "ta = 1", "str = \"x\"", "color = 44", "label = 1", "_ = 1"] {
            XCTAssertEqual(codes(body + "\n"), [], body)
        }
    }

    func testBuiltinsStillUsableWhenNotShadowed() {
        XCTAssertEqual(
            codes(
                """
                float m = math.max(1, 2)
                color c = color.red
                float s = ta.sma(close, 3)
                plot(s, color=c)
                """),
            [])
    }

    // MARK: - Dotted names (CE10090)

    func testDottedVariableNameIsAnError() throws {
        let program = compile("math.max = 44\n")
        XCTAssertFalse(program.isValid)
        XCTAssertEqual(program.diagnostics.map(\.code), ["CE10090"])
        let diagnostic = try XCTUnwrap(program.diagnostics.first)
        XCTAssertEqual(
            diagnostic.message, "User variable identifiers should not contain \".\" character: \"math.max\".")
        XCTAssertEqual(diagnostic.range.start.line, 3)
        XCTAssertEqual(diagnostic.range.start.column, 1)
        XCTAssertEqual(diagnostic.range.end.column, 9)
    }

    func testDottedNamesWithTypesAndModes() {
        for body in ["float ta.sma = 1", "var x.y = 2", "varip int a.b.c = 1", "foo.bar = close"] {
            XCTAssertEqual(codes(body + "\n"), ["CE10090"], body)
        }
    }

    func testDottedExpressionsAreUnaffected() {
        XCTAssertEqual(
            codes(
                """
                bool same = math.pi == math.pi
                float x = math.max(1, 2)
                """),
            [])
    }
}
