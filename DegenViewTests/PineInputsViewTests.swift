import XCTest

@testable import DegenView

final class PineInputsViewTests: XCTestCase {
    private func input(options: String, default value: String) throws -> PineInputDefinition {
        let source = """
            //@version=6
            indicator("T")
            mode = input.string("\(value)", "Mode", options=\(options))
            plot(close)
            """
        let program = PineCompiler.compile(source: source, libraries: PineLibraryRegistry.shared)
        return try XCTUnwrap(program.inputSchema.inputs.first)
    }

    func testShortOptionsKeepTheirNaturalWidth() throws {
        let definition = try input(options: "[\"SMA\", \"EMA\", \"WMA\"]", default: "EMA")
        let titles = PineInputsView.optionTitles(of: definition).map(\.title)
        XCTAssertEqual(titles, ["SMA", "EMA", "WMA"])
        XCTAssertNil(PineInputsView.menuWidth(forOptionTitles: titles))
    }

    func testLongOptionTitlesAreCapped() throws {
        let options =
            "[\"Strong Signals Only (Ultra Clean)\", \"All Confirmed Trend Signals\", \"All Signals (Unfiltered)\"]"
        let definition = try input(options: options, default: "Strong Signals Only (Ultra Clean)")
        let titles = PineInputsView.optionTitles(of: definition)
        XCTAssertEqual(titles.count, 3)
        XCTAssertEqual(titles.first?.value, "Strong Signals Only (Ultra Clean)")
        XCTAssertEqual(PineInputsView.menuWidth(forOptionTitles: titles.map(\.title)), 95)
    }

    func testNoOptionsMeansNoCap() {
        XCTAssertNil(PineInputsView.menuWidth(forOptionTitles: []))
    }
}
