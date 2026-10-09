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

    func testBehindChartAndLowerTimeframeCalcBarsCountAreHonouredButSecurityStillWarns() {
        let honoured = compile(
            """
            [c] = request.security_lower_tf(syminfo.tickerid, "1", [close], calc_bars_count = 10)
            plot(array.size(c))
            """, header: "indicator(\"T\", overlay = true, behind_chart = false)")
        XCTAssertEqual(warnings(honoured).count, 0, "\(honoured.diagnostics)")
        let ignored = compile(
            "plot(request.security(syminfo.tickerid, \"60\", close, calc_bars_count = 10))")
        XCTAssertEqual(warnings(ignored).map(\.code), ["PINE7001"])
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
            plot(close, style = plot.style_stepline_diamond)
            plot(syminfo.session == "regular" ? 1 : 0)
            """
        let program = PineCompiler.compile(source: source)
        XCTAssertTrue(program.isValid, "\(program.diagnostics)")
        let found = warnings(program)
        XCTAssertEqual(found.map(\.code), ["PINE7002", "PINE7004", "PINE7003"], "\(found)")
        XCTAssertEqual(covered(found[1], in: source), "plot.style_stepline_diamond")
        XCTAssertEqual(covered(found[2], in: source), "syminfo.session")
    }

    func testAUserFunctionWithACataloguedNameIsAnErrorNotAWarning() {
        let program = compile(
            """
            hline(x, linestyle) =>
                x + linestyle
            plot(hline(1, 2))
            """)
        XCTAssertEqual(program.diagnostics.map(\.code), ["CE10190"])
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

    func testDiagnosticsSectionCountsSeveritiesAndOpensOnlyForErrors() {
        let error = PineDiagnostic.error("PINE0001", .syntax, "Unexpected end", .zero)
        let warning = PineDiagnostic.warning("PINE9001", .semantic, "Ignored", .zero)
        let mixed = [warning, error, error]

        XCTAssertEqual(PineDiagnosticsSection.summary(for: mixed), .init(errors: 2, warnings: 1))
        XCTAssertTrue(PineDiagnosticsSection.startsExpanded(mixed))

        XCTAssertEqual(PineDiagnosticsSection.summary(for: [warning]), .init(errors: 0, warnings: 1))
        XCTAssertFalse(PineDiagnosticsSection.startsExpanded([warning]), "warnings alone wait to be opened")

        XCTAssertEqual(PineDiagnosticsSection.summary(for: []), .init())
        XCTAssertFalse(PineDiagnosticsSection.startsExpanded([]))
    }

    func testAlertsSectionSplitsLiveFromHistoricalAndListsNewestFirst() {
        func alert(_ id: Int, live: Bool) -> PineAlertEvent {
            PineAlertEvent(
                id: id, site: 1, bar: id, time: Date(timeIntervalSince1970: Double(id)), message: "m\(id)",
                isRealtime: live)
        }
        let alerts = (1...(PineAlertsSection.limit + 5)).map { alert($0, live: $0 % 10 == 0) }
        let summary = PineAlertsSection.summary(for: alerts)
        XCTAssertEqual(summary.live + summary.historical, alerts.count)
        XCTAssertEqual(summary.live, alerts.filter(\.isRealtime).count)

        let visible = PineAlertsSection.visible(alerts)
        XCTAssertEqual(visible.count, PineAlertsSection.limit)
        XCTAssertEqual(visible.first?.id, alerts.last?.id, "newest first")
        XCTAssertEqual(visible.last?.id, 6, "the oldest five are dropped")

        XCTAssertEqual(PineAlertsSection.summary(for: []), .init())
        XCTAssertEqual(PineCountChip.plural(1, "error"), "1 error")
        XCTAssertEqual(PineCountChip.plural(2, "error"), "2 errors")
    }
}
