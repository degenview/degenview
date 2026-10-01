import XCTest

@testable import DegenView

/// `chart.point` objects and `polyline` drawings.
final class PinePolylineTests: XCTestCase {
    private func bars(_ count: Int) -> [KlineData] {
        (0..<count).map { i in
            let close = Double(i + 1)
            return .init(
                openTime: Date(timeIntervalSince1970: Double(i) * 60), openPrice: close, highPrice: close + 1,
                lowPrice: close - 1, closePrice: close, volume: 1)
        }
    }

    private func compile(_ body: String, header: String = "indicator(\"T\", overlay = true)") -> PineCompiledProgram {
        PineCompiler.compile(source: "//@version=6\n\(header)\n\(body)")
    }

    private func output(_ body: String, header: String = "indicator(\"T\", overlay = true)", count: Int = 6)
        throws -> PineVisualOutput
    {
        let program = compile(body, header: header)
        XCTAssertTrue(program.isValid, "\(program.diagnostics)")
        return try PineRuntimeSession(program: program).evaluate(bars: bars(count)).output
    }

    // MARK: - chart.point

    func testChartPointConstructorsFillIndexTimeAndPrice() throws {
        let result = try output(
            """
            chart.point a = chart.point.from_index(3, 10.0)
            b = chart.point.now(7.5)
            c = chart.point.from_time(time[1], 4.0)
            d = chart.point.new(time, bar_index, 2.0)
            plot(a.index)
            plot(na(a.time) ? 1 : 0)
            plot(a.price)
            plot(b.index)
            plot(b.time)
            plot(c.index)
            plot(d.price)
            """, count: 4)
        let last = result.plots.map { $0.values.last ?? nil }
        XCTAssertEqual(last, [3, 1, 10, 3, 180_000, 2, 2])
    }

    func testChartPointFieldsCanBeAssignedAndCopied() throws {
        let result = try output(
            """
            p = chart.point.from_index(1, 5.0)
            q = p.copy()
            q.price := 9.0
            p.index += 2
            plot(p.price)
            plot(q.price)
            plot(p.index)
            """, count: 1)
        XCTAssertEqual(result.plots.map(\.values), [[5], [9], [3]])
    }

    // MARK: - polyline

    func testPolylineKeepsPointsClosedFillStyleAndWidth() throws {
        let result = try output(
            """
            var array<chart.point> pts = array.new<chart.point>()
            if barstate.islast
                array.push(pts, chart.point.from_index(1, 10.0))
                array.push(pts, chart.point.from_index(4, 12.0))
                array.push(pts, chart.point.from_index(2, 8.0))
                polyline.new(pts, closed = true, line_color = color.red, fill_color = color.new(color.blue, 80),
                     line_style = line.style_dashed, line_width = 3)
            """)
        let polyline = try XCTUnwrap(result.polylines.first)
        XCTAssertEqual(
            polyline.points, [.init(index: 1, price: 10), .init(index: 4, price: 12), .init(index: 2, price: 8)])
        XCTAssertTrue(polyline.closed)
        XCTAssertEqual(polyline.lineColor, PineBuiltins.colors["color.red"])
        XCTAssertEqual(polyline.fillColor, PineBuiltins.withTransparency(PineBuiltins.colors["color.blue"] ?? 0, 80))
        XCTAssertEqual(polyline.style, .dashed)
        XCTAssertEqual(polyline.width, 3)
    }

    func testPolylineWithTimeAnchorsMapsTimesToBarIndexes() throws {
        let result = try output(
            """
            var array<chart.point> pts = array.new<chart.point>()
            if barstate.islast
                array.push(pts, chart.point.from_time(time[3], 1.0))
                array.push(pts, chart.point.from_time(time, 2.0))
                polyline.new(pts, xloc = xloc.bar_time)
            """)
        XCTAssertEqual(result.polylines.first?.points.map(\.index), [2, 5])
    }

    func testPolylinesAreDeletedAndLimitedByMaxPolylinesCount() throws {
        let result = try output(
            """
            var array<polyline> made = array.new<polyline>()
            pts = array.from(chart.point.from_index(0, 1.0), chart.point.from_index(1, 2.0))
            array.push(made, polyline.new(pts))
            if bar_index == 4
                polyline.delete(array.get(made, array.size(made) - 1))
            """,
            header: "indicator(\"T\", overlay = true, max_polylines_count = 3)", count: 5)
        // One polyline per bar, the oldest dropped past 3, and the last one deleted on bar 4.
        XCTAssertEqual(result.polylines.count, 2)
    }

    func testPolylineOverNaPointsIsNaAndSkipsInvalidPoints() throws {
        let result = try output(
            """
            var polyline gone = na
            gone := polyline.new(na)
            pts = array.from(chart.point.from_index(0, na), chart.point.from_index(1, 3.0), chart.point.from_index(2, 4.0))
            polyline.new(pts)
            plot(na(gone) ? 1 : 0)
            """, count: 1)
        XCTAssertEqual(result.plots[0].values, [1])
        XCTAssertEqual(result.polylines.first?.points.count, 2)
    }
}
