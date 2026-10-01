import XCTest

@testable import DegenView

final class PineAlertValidationTests: XCTestCase {
    private typealias F = PineExecutionFixtures

    private func compile(_ body: String) -> PineCompiledProgram {
        PineCompiler.compile(source: "//@version=6\nindicator(\"T\")\n\(body)")
    }

    private func codes(_ body: String) -> [String] {
        compile(body).diagnostics.map(\.code)
    }

    func testValidAlertCallsCompile() {
        let body = """
            alert("plain")
            alert("freq", alert.freq_all)
            alert(message = "named", freq = alert.freq_once_per_bar_close)
            alert("Long " + syminfo.ticker + " @ " + str.tostring(close), alert.freq_once_per_bar)
            alertcondition(close > open, "Up", "Close above open")
            alertcondition(condition = close > open, message = "named")
            """
        XCTAssertEqual(codes(body), [])
    }

    func testAlertWithoutAMessageIsRejected() {
        XCTAssertTrue(codes("alert()").contains("PINE3025"))
    }

    func testAlertWithTooManyArgumentsIsRejected() {
        XCTAssertTrue(codes("alert(\"a\", alert.freq_all, 3)").contains("PINE3025"))
    }

    func testAlertRejectsUnknownArgumentNames() {
        XCTAssertTrue(codes("alert(\"a\", frequency = alert.freq_all)").contains("PINE3027"))
    }

    func testAlertFrequencyMustBeAnAlertConstant() {
        XCTAssertTrue(codes("alert(\"a\", \"sometimes\")").contains("PINE3028"))
        XCTAssertTrue(codes("alert(\"a\", 3)").contains("PINE3028"))
        XCTAssertTrue(codes("alert(\"a\", alert.freq_bogus)").contains("PINE3028"))
        XCTAssertEqual(codes("alert(\"a\", \"alert.freq_all\")"), [], "a literal naming the constant is accepted")
    }

    func testAlertMessageMustBeAString() {
        XCTAssertTrue(codes("alert(close)").contains("PINE3036"))
        XCTAssertTrue(codes("alertcondition(close > open, \"t\", close)").contains("PINE3036"))
        XCTAssertTrue(codes("alert(message = 5)").contains("PINE3036"))
    }

    func testAlertConditionArity() {
        XCTAssertTrue(codes("alertcondition()").contains("PINE3026"))
        XCTAssertTrue(codes("alertcondition(close > open, \"t\", \"m\", \"extra\")").contains("PINE3026"))
        XCTAssertTrue(codes("alertcondition(close > open, colour = \"x\")").contains("PINE3027"))
    }

    func testCallSitesListEveryAlertWithItsFrequency() {
        let program = compile(
            """
            string dynamic = alert.freq_all
            alert("default")
            if close > open
                alert("closed", alert.freq_once_per_bar_close)
            alert("computed", dynamic)
            alertcondition(close < open, "Down")
            """)
        XCTAssertTrue(program.isValid, "\(program.diagnostics)")
        let sites = program.alertCallSites
        XCTAssertEqual(sites.map(\.frequency), [.oncePerBar, .oncePerBarClose, nil, .all])
        XCTAssertEqual(sites.map(\.isCondition), [false, false, false, true])
    }

    func testProgramWithoutAlertsHasNoCallSites() {
        XCTAssertTrue(compile("plot(close)").alertCallSites.isEmpty)
    }

    func testUnresolvableFrequencyAtRuntimeFails() throws {
        let controller = F.controller("string f = \"oops\"\nalert(\"x\", f)")
        guard case .failed(let diagnostic) = controller.rebuild(bars: F.history([100, 101]), live: false) else {
            return XCTFail("a frequency that resolves to nothing must not silently default")
        }
        XCTAssertEqual(diagnostic.code, "PINE4009")
    }
}
