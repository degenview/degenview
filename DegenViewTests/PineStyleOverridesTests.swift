import XCTest

@testable import DegenView

final class PineStyleOverridesTests: XCTestCase {
    private func row(_ title: String, in output: PineVisualOutput) throws -> PineStyleRow {
        try XCTUnwrap(PineStyleRows.rows(for: output).first { $0.title == title }, title)
    }

    private func key(_ row: PineStyleRow, _ attribute: PineStyleAttribute) -> String {
        PineStyleOverrides.key(row.kind, row.outputID, attribute)
    }

    func testNoOverridesLeavesTheOutputAlone() throws {
        let output = try PineStyleFixtures.output()
        let applied = output.applying(styleOverrides: [:])
        XCTAssertEqual(applied.plots.map(\.colors), output.plots.map(\.colors))
        XCTAssertEqual(applied.fills.count, output.fills.count)
    }

    func testHidingAPlotClearsItsPaneDisplayButKeepsItForAFill() throws {
        let output = try PineStyleFixtures.output()
        let fast = try row("Fast", in: output)
        let applied = output.applying(styleOverrides: [key(fast, .visible): "false"])
        let plot = try XCTUnwrap(applied.plots.first { $0.id == fast.outputID })
        XCTAssertFalse(plot.display.contains(.pane))
        XCTAssertEqual(applied.fills.count, 1, "the fill still references the plot")
        XCTAssertEqual(applied.plots.count, output.plots.count)
    }

    func testHidingAHorizontalLineFillBackgroundOrBarColorDropsIt() throws {
        let output = try PineStyleFixtures.output()
        var overrides: [String: String] = [:]
        for title in ["Level", "Band", "Zone", "Bars"] {
            overrides[key(try row(title, in: output), .visible)] = "false"
        }
        let applied = output.applying(styleOverrides: overrides)
        XCTAssertTrue(applied.hlines.isEmpty)
        XCTAssertTrue(applied.fills.isEmpty)
        XCTAssertTrue(applied.backgrounds.isEmpty)
        XCTAssertTrue(applied.barColors.isEmpty)
    }

    func testHidingMarkersAndCandlesClearsTheirPaneDisplay() throws {
        let output = try PineStyleFixtures.output()
        let applied = output.applying(styleOverrides: [
            key(try row("Up", in: output), .visible): "false",
            key(try row("Candles", in: output), .visible): "false",
        ])
        XCTAssertFalse(try XCTUnwrap(applied.markers.first).display.contains(.pane))
        XCTAssertFalse(try XCTUnwrap(applied.candles.first).display.contains(.pane))
    }

    func testAColorReplacesVisibleColorsAndLeavesNaBarsAlone() throws {
        let program = PineCompiler.compile(
            source: "//@version=6\nindicator(\"T\")\nplot(close, \"P\", color = close > 11 ? color.red : na)\n")
        let output = try PineRuntimeSession(program: program)
            .evaluate(bars: PineStyleFixtures.bars([10, 12, 10, 13])).output
        let plot = try row("P", in: output)
        let applied = output.applying(styleOverrides: [key(plot, .color): "0000FFFF"])
        let colors = try XCTUnwrap(applied.plots.first).colors
        let shown = colors.compactMap { $0 }.filter { $0 & 0xFF != 0 }
        XCTAssertEqual(shown, [0x0000_FFFF, 0x0000_FFFF])
        XCTAssertEqual(colors.filter { ($0 ?? 0) & 0xFF == 0 }.count, 2, "the na bars stay hidden")
    }

    func testAColorRecolorsAFillAHorizontalLineAndABackground() throws {
        let output = try PineStyleFixtures.output()
        let applied = output.applying(styleOverrides: [
            key(try row("Band", in: output), .color): "112233FF",
            key(try row("Level", in: output), .color): "445566FF",
            key(try row("Zone", in: output), .color): "778899FF",
        ])
        XCTAssertEqual(applied.fills.first?.colors.compactMap { $0 }, Array(repeating: 0x1122_33FF, count: 4))
        XCTAssertEqual(applied.hlines.first?.color, 0x4455_66FF)
        XCTAssertEqual(applied.backgrounds.first?.colors.compactMap { $0 }, Array(repeating: 0x7788_99FF, count: 4))
    }

    func testAWidthIsClampedAndOnlyChangesThatPlot() throws {
        let output = try PineStyleFixtures.output()
        let fast = try row("Fast", in: output)
        let slow = try row("Slow", in: output)
        let applied = output.applying(styleOverrides: [key(fast, .width): "99"])
        XCTAssertEqual(applied.plots.first { $0.id == fast.outputID }?.lineWidth, 20)
        XCTAssertEqual(applied.plots.first { $0.id == slow.outputID }?.lineWidth, 1)
    }

    func testGarbageAndStaleKeysAreIgnored() throws {
        let output = try PineStyleFixtures.output()
        let applied = output.applying(styleOverrides: [
            "plot.9999.visible": "false", "plot.x.color": "zz", "plot.0.color": "not hex", "": "",
        ])
        XCTAssertEqual(applied.plots.map(\.colors), output.plots.map(\.colors))
        XCTAssertEqual(applied.hlines.count, output.hlines.count)
    }
}
