import XCTest

@testable import DegenView

/// Features TradingView has and the engine accepts but ignores must say so with a warning.
final class PineIgnoredFeatureTests: XCTestCase {
    private func compile(_ body: String, header: String = "indicator(\"T\")") -> PineCompiledProgram {
        PineCompiler.compile(source: "//@version=6\n\(header)\n\(body)")
    }

    private func warnings(_ program: PineCompiledProgram) -> [PineDiagnostic] {
        program.diagnostics.filter { $0.severity == .warning }
    }

    /// The text a diagnostic's range covers in `source`.
    private func covered(_ diagnostic: PineDiagnostic, in source: String) -> String {
        let text = source as NSString
        let start = diagnostic.range.start.offset
        return text.substring(with: NSRange(location: start, length: diagnostic.range.end.offset - start))
    }

    func testIgnoredArgumentIsAWarningUnderlinedOnItsName() throws {
        let source = "//@version=6\nindicator(\"T\")\nhline(1, linestyle = hline.style_dashed)\nplot(close)\n"
        let program = PineCompiler.compile(source: source)
        XCTAssertTrue(program.isValid, "\(program.diagnostics)")
        let warning = try XCTUnwrap(warnings(program).first)
        XCTAssertEqual(program.diagnostics.count, 1, "\(program.diagnostics)")
        XCTAssertEqual(warning.code, "PINE7001")
        XCTAssertEqual(covered(warning, in: source), "linestyle")
        XCTAssertTrue(warning.message.contains("hline()"), warning.message)
        XCTAssertTrue(warning.message.contains("solid"), "it says what happens instead: \(warning.message)")
    }

    func testStrategyDeclarationAndOrderArgumentsWarn() {
        let program = compile(
            """
            strategy.entry("L", strategy.long, comment = "go")
            strategy.exit("X", from_entry = "L", stop = 1, comment_loss = "stop", alert_message = "a")
            strategy.close("L", comment = "bye")
            strategy.close_all(comment = "flat")
            """,
            header: "strategy(\"T\", margin_long = 10, margin_short = 10, slippage = 1)")
        XCTAssertTrue(program.isValid, "\(program.diagnostics)")
        let messages = warnings(program).map(\.message)
        for expected in [
            "strategy(): `margin_long`", "strategy(): `margin_short`", "strategy.entry(): `comment`",
            "strategy.exit(): `comment_loss`", "strategy.exit(): `alert_message`",
            "strategy.close(): `comment`", "strategy.close_all(): `comment`",
        ] {
            XCTAssertTrue(messages.contains { $0.hasPrefix(expected) }, "\(expected) in \(messages)")
        }
        XCTAssertEqual(messages.count, 7, "slippage and stop are honoured: \(messages)")
    }

    func testHonouredArgumentsGiveNoWarning() {
        let program = compile(
            """
            plot(close, "Close", color = color.red, linewidth = 2, style = plot.style_line, display = display.all)
            plotshape(close > open, "S", shape.triangleup, location.bottom, color.green, size = size.tiny)
            hline(0, "Zero", color = color.gray)
            bgcolor(na)
            """)
        XCTAssertEqual(program.diagnostics, [])
    }

    func testIgnoredFunctionAndValueAndVariableWarn() throws {
        let source = """
            //@version=6
            strategy("T")
            strategy.risk.max_drawdown(10, strategy.percent_of_equity)
            plot(close, style = plot.style_linebr)
            plot(syminfo.session == "regular" ? 1 : 0)
            """
        let program = PineCompiler.compile(source: source)
        XCTAssertTrue(program.isValid, "\(program.diagnostics)")
        let found = warnings(program)
        XCTAssertEqual(found.map(\.code), ["PINE7002", "PINE7004", "PINE7003"], "\(found)")
        XCTAssertEqual(covered(found[1], in: source), "plot.style_linebr")
        XCTAssertEqual(covered(found[2], in: source), "syminfo.session")
    }

    func testAUserFunctionWithACataloguedNameIsNotWarnedAbout() {
        let program = compile(
            """
            hline(x, linestyle) =>
                x + linestyle
            plot(hline(1, 2))
            """)
        XCTAssertEqual(program.diagnostics, [])
    }

    func testWarningsDoNotHideErrorsAndErrorsListFirst() {
        let program = compile("hline(1, linestyle = hline.style_dashed)\nplot(")
        XCTAssertFalse(program.isValid)
        let ordered = PineDiagnosticsListView.ordered(program.diagnostics)
        XCTAssertEqual(ordered.first?.severity, .error)
    }

    func testSeverityColorsAreRedForErrorsAndYellowForWarnings() {
        XCTAssertEqual(PineDiagnosticSeverity.error.nsColor, .systemRed)
        XCTAssertEqual(PineDiagnosticSeverity.warning.nsColor, .systemYellow)
    }
}
