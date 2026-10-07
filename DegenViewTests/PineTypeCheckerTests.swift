import XCTest

@testable import DegenView

final class PineTypeCheckerTests: XCTestCase {
    private func compile(_ body: String) -> PineCompiledProgram {
        PineCompiler.compile(source: "//@version=6\nindicator(\"T\", overlay=true)\n" + body)
    }

    private func codes(_ body: String) -> [String] { compile(body).diagnostics.map(\.code) }

    private func assertClean(_ body: String, file: StaticString = #filePath, line: UInt = #line) {
        let program = compile(body)
        XCTAssertTrue(program.diagnostics.isEmpty, "\(program.diagnostics)", file: file, line: line)
    }

    // MARK: - Declarations

    func testConstStringCannotHoldAnInteger() throws {
        let program = compile("const string g_sr = 222\n")
        XCTAssertFalse(program.isValid)
        let diagnostic = try XCTUnwrap(program.diagnostics.first)
        XCTAssertEqual(diagnostic.code, "PINE3030")
        XCTAssertEqual(diagnostic.message, "Cannot assign int to string variable 'g_sr'.")
        // Points at the `222`, on line 3 (after the version and indicator lines).
        XCTAssertEqual(diagnostic.range.start.line, 3)
        XCTAssertEqual(diagnostic.range.start.column, 21)
    }

    func testDeclaredTypeMismatches() {
        for body in [
            "int n = 1.5", "bool b = 1", "float f = \"x\"", "color c = 5", "string s = true",
            "int n = close", "float f = close > open",
        ] {
            XCTAssertEqual(codes(body + "\n"), ["PINE3030"], body)
        }
    }

    func testQualifiers() {
        XCTAssertEqual(codes("const int n = input.int(5)\n"), ["PINE3031"])
        XCTAssertEqual(codes("const string s = str.tostring(close)\n"), ["PINE3031"])
        XCTAssertEqual(codes("simple float f = close\n"), ["PINE3031"])
        assertClean("const int n = 5\nconst string g = \"a\" + \"b\"\nconst color c = color.new(color.red, 50)\n")
        assertClean("simple int len = input.int(5)\nseries float s = close\n")
    }

    // MARK: - Reassignment

    func testReassignmentMustKeepTheType() {
        XCTAssertEqual(codes("x = 0\nx := 1.5\n"), ["PINE3032"])
        XCTAssertEqual(codes("var s = \"a\"\ns := 1\n"), ["PINE3032"])
        XCTAssertEqual(codes("int n = 1\nn += 0.5\n"), ["PINE3032"])
        XCTAssertEqual(codes("float y = 1.0\ny += \"a\"\n"), ["PINE3034"])
        assertClean("float y = 1\ny := 2\ny += 0.5\nvar count = 0\ncount += 1\n")
    }

    // MARK: - Conditions and operators

    func testConditionsMustBeBool() {
        XCTAssertEqual(codes("if close\n    plot(1)\n"), ["PINE3033"])
        XCTAssertEqual(codes("v = close ? 1 : 2\n"), ["PINE3033"])
        XCTAssertEqual(codes("b = not 1\n"), ["PINE3033"])
        XCTAssertEqual(codes("b = close and true\n"), ["PINE3033"])
        XCTAssertEqual(codes("while 1\n    break\n"), ["PINE3033"])
        XCTAssertEqual(codes("s = switch\n    1 => 1\n    => 2\n"), ["PINE3033"])
    }

    func testOperatorOperandTypes() {
        for body in [
            "v = \"a\" - 1", "v = \"a\" + 1", "v = close < \"x\"", "v = true + 1", "v = -\"a\"",
            "v = close == \"x\"",
        ] {
            XCTAssertEqual(codes(body + "\n"), ["PINE3034"], body)
        }
    }

    func testInputDefaultsMustFitTheirFunction() {
        for body in [
            "a = input.int(\"x\")", "a = input.bool(1)", "a = input.string(5)", "a = input.color(1)",
            "a = input.time(1.5)", "a = input.int(2.5)",
        ] {
            XCTAssertEqual(codes(body + "\n"), ["PINE3035"], body)
        }
        assertClean("a = input.float(1)\nb = input.int(defval = 3)\n")
    }

    // MARK: - No false positives

    func testValidScriptsStayClean() {
        assertClean(
            """
            float a = na
            int b = na
            string s = na
            color c = na
            var float x = 0
            x := close
            grp = "Group"
            len = input.int(3, "Length", group = grp)
            float v = close > open ? 1 : 2.5
            color tint = close > open ? color.green : na
            int k = close > open ? 1 : 0
            int i = 2 * 3 % 4
            float d = 1 / 2
            int bars = bar_index + 1
            int t = time
            float m = math.max(1, 2.5)
            int r = math.round(close)
            float p = close[1]
            string label = "v" + str.tostring(close)
            bool ok = close > open and volume > 0 or not (close < open)
            bool gone = close == na
            """)
    }

    func testUnknownTypesAreNeverFlagged() {
        assertClean(
            """
            double(x) =>
                x * 2
            float u = double(1)
            string t = double(2)
            var int[] xs = array.new_int(0)
            var line l = na
            float first = array.get(xs, 0)
            [m, sig, hist] = ta.macd(close, 12, 26, 9)
            float macdLine = m
            for i = 0 to 2
                int j = i
            """)
    }

    func testLocalsShadowOuterNames() {
        assertClean(
            """
            f(text) =>
                text + "x"
            v = f("a")
            if close > open
                string close2 = "inner"
            string close2 = "outer"
            """)
    }

    func testStrategyScriptsStayClean() {
        assertClean(
            """
            x = strategy.position_size > 0 ? 1 : 0
            plot(x)
            """)
    }
}
