import XCTest

@testable import DegenView

final class PineStyleRowsTests: XCTestCase {
    fileprivate func rows() throws -> [PineStyleRow] {
        PineStyleRows.rows(for: try PineStyleFixtures.output())
    }

    func testEveryStyleableOutputGetsARowInScriptOrder() throws {
        let rows = try rows()
        XCTAssertEqual(
            rows.map(\.title), ["Fast", "Slow", "Level", "Band", "Up", "Zone", "Bars", "Candles"])
        XCTAssertEqual(
            rows.map(\.kind), [.plot, .plot, .hline, .fill, .marker, .bgcolor, .barcolor, .candle])
    }

    func testAPlotTheScriptKeepsOffThePaneIsNotListed() throws {
        let output = try PineStyleFixtures.output()
        XCTAssertEqual(output.plots.count, 3)
        XCTAssertEqual(PineStyleRows.rows(for: output).filter { $0.kind == .plot }.count, 2)
    }

    func testOnlyAFixedColorOffersASwatch() throws {
        let byTitle = Dictionary(uniqueKeysWithValues: try rows().map { ($0.title, $0) })
        XCTAssertNotNil(byTitle["Fast"]?.defaultColor, "one blue for every bar")
        XCTAssertNil(byTitle["Slow"]?.defaultColor, "green or red per bar")
        XCTAssertNotNil(byTitle["Level"]?.defaultColor)
        XCTAssertNotNil(byTitle["Band"]?.defaultColor)
        XCTAssertNotNil(byTitle["Zone"]?.defaultColor)
        XCTAssertNil(byTitle["Bars"]?.defaultColor)
        XCTAssertNil(byTitle["Candles"]?.defaultColor)
    }

    func testOnlyLinePlotsOfferAWidth() throws {
        let byTitle = Dictionary(uniqueKeysWithValues: try rows().map { ($0.title, $0) })
        XCTAssertEqual(byTitle["Fast"]?.defaultWidth, 2)
        XCTAssertEqual(byTitle["Slow"]?.defaultWidth, 1)
        XCTAssertNil(byTitle["Level"]?.defaultWidth)
        XCTAssertNil(byTitle["Band"]?.defaultWidth)
    }

    func testUntitledOutputsGetANumberedName() throws {
        let program = PineCompiler.compile(
            source: "//@version=6\nindicator(\"T\")\nplot(close)\nplot(open)\nbgcolor(color.red)\n")
        let output = try PineRuntimeSession(program: program).evaluate(bars: PineStyleFixtures.bars([1, 2])).output
        XCTAssertEqual(PineStyleRows.rows(for: output).map(\.title), ["Plot 1", "Plot 2", "Background 1"])
    }

    func testAGradientFillHasNoSwatch() throws {
        let source = """
            //@version=6
            indicator("T")
            a = plot(close)
            b = plot(open)
            fill(a, b, close, open, color.green, color.red, title = "Glow")
            """
        let program = PineCompiler.compile(source: source)
        XCTAssertTrue(program.isValid, "\(program.diagnostics)")
        let output = try PineRuntimeSession(program: program).evaluate(bars: PineStyleFixtures.bars([1, 2])).output
        let fill = try XCTUnwrap(PineStyleRows.rows(for: output).first { $0.kind == .fill })
        XCTAssertEqual(fill.title, "Glow")
        XCTAssertNil(fill.defaultColor)
    }

    func testPruningKeepsOnlyChoicesThatStillMatchARow() throws {
        let rows = try rows()
        let fast = try XCTUnwrap(rows.first { $0.title == "Fast" })
        let kept = PineStyleOverrides.pruned(
            [
                PineStyleOverrides.key(.plot, fast.outputID, .visible): "false",
                PineStyleOverrides.key(.plot, 9999, .color): "FF0000FF",
                "nonsense": "x",
            ], keeping: rows)
        XCTAssertEqual(Set(kept.keys), [PineStyleOverrides.key(.plot, fast.outputID, .visible)])
    }
}

extension PineStyleRowsTests {
    func testARowKnowsWhetherItWasChangedAndCanBeReverted() throws {
        let rows = try rows()
        let fast = try XCTUnwrap(rows.first { $0.title == "Fast" })
        let slow = try XCTUnwrap(rows.first { $0.title == "Slow" })
        let raw = [
            PineStyleOverrides.key(.plot, fast.outputID, .width): "4",
            PineStyleOverrides.key(.plot, fast.outputID, .visible): "false",
            PineStyleOverrides.key(.plot, slow.outputID, .visible): "false",
        ]
        let style = PineStyleOverrides(raw)
        XCTAssertTrue(style.hasChanges(fast))
        XCTAssertFalse(PineStyleOverrides(PineStyleOverrides.cleared(raw, fast)).hasChanges(fast))
        XCTAssertTrue(PineStyleOverrides(PineStyleOverrides.cleared(raw, fast)).hasChanges(slow))
    }

    func testTheFootnoteAppearsOnlyWhenAScriptColorsBarByBar() throws {
        XCTAssertTrue(PineStyleView.hasPerBarRows(try rows()))
        XCTAssertFalse(PineStyleView.hasPerBarRows(try rows().filter { $0.defaultColor != nil }))
    }
}
