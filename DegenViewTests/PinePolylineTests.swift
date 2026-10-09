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

    func testPolylineKeepsCurved() throws {
        let result = try output(
            """
            var array<chart.point> pts = array.new<chart.point>()
            if barstate.islast
                array.push(pts, chart.point.from_index(1, 10.0))
                array.push(pts, chart.point.from_index(4, 12.0))
                array.push(pts, chart.point.from_index(2, 8.0))
                polyline.new(pts, curved = true)
                polyline.new(pts)
            """)
        XCTAssertEqual(result.polylines.map(\.curved), [true, false])
    }

    func testCurvedPathPassesThroughEveryPointAndStraightDoesNot() {
        let points = [CGPoint(x: 0, y: 0), CGPoint(x: 10, y: 10), CGPoint(x: 20, y: 0), CGPoint(x: 30, y: 10)]
        for closed in [false, true] {
            let curved = PineDrawingGeometry.polylinePath(through: points, curved: true, closed: closed)
            var ends: [CGPoint] = []
            curved.forEach { element in
                switch element {
                case .move(let to), .line(let to), .curve(let to, _, _): ends.append(to)
                default: break
                }
            }
            XCTAssertEqual(Array(ends.prefix(points.count)), points)
            XCTAssertEqual(curved.currentPoint, closed ? points[0] : points.last)
        }
        let straight = PineDrawingGeometry.polylinePath(through: points, curved: false, closed: false)
        var curves = 0
        straight.forEach { if case .curve = $0 { curves += 1 } }
        XCTAssertEqual(curves, 0)
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

    // MARK: - Drawings made from points

    func testLineLabelAndBoxCanBeMadeFromChartPoints() throws {
        let result = try output(
            """
            a = chart.point.from_index(1, 10.0)
            b = chart.point.from_index(4, 12.0)
            line.new(a, b, color = color.red, width = 3)
            label.new(b, "tip", color = color.blue, style = label.style_label_down)
            box.new(a, b, border_color = color.green)
            """, count: 1)
        let line = try XCTUnwrap(result.lines.first)
        XCTAssertEqual([line.x1, line.x2, line.width], [1, 4, 3])
        XCTAssertEqual([line.y1, line.y2], [10, 12])
        XCTAssertEqual(line.color, PineBuiltins.colors["color.red"])
        let label = try XCTUnwrap(result.labels.first)
        XCTAssertEqual(label.x, 4)
        XCTAssertEqual(label.text, "tip")
        XCTAssertEqual(label.y, 12)
        XCTAssertEqual(label.color, PineBuiltins.colors["color.blue"])
        let box = try XCTUnwrap(result.boxes.first)
        XCTAssertEqual([box.left, box.right], [1, 4])
        XCTAssertEqual([box.top, box.bottom], [10, 12])
        XCTAssertEqual(box.borderColor, PineBuiltins.colors["color.green"])
    }

    func testPointDrawingsHonourTimeAnchors() throws {
        let result = try output(
            """
            if barstate.islast
                a = chart.point.from_time(time[3], 1.0)
                b = chart.point.from_time(time, 2.0)
                line.new(a, b, xloc = xloc.bar_time)
            """)
        XCTAssertEqual(result.lines.count, 1)
        XCTAssertEqual([result.lines[0].x1, result.lines[0].x2], [2, 5])
        XCTAssertTrue(result.lines[0].isComplete)
    }

    func testDrawingsMadeWithNaCoordinatesExistButStayHiddenUntilComplete() throws {
        let result = try output(
            """
            var line ln = line.new(na, na, na, na, color = color.red)
            var label lb = label.new(na, na, "t")
            var box bx = box.new(na, na, na, na)
            var line fromPoint = line.new(chart.point.from_index(0, na), chart.point.from_index(2, 3.0))
            plot(na(ln) ? 1 : 0)
            plot(na(ln.get_x1()) ? 1 : 0)
            if bar_index == 2
                ln.set_first_point(chart.point.from_index(1, 5.0))
                lb.set_point(chart.point.from_index(2, 6.0))
                bx.set_top_left_point(chart.point.from_index(0, 9.0))
            """, count: 3)
        XCTAssertEqual(result.plots[0].values, [0, 0, 0], "the line object exists")
        XCTAssertEqual(result.plots[1].values, [1, 1, 1], "a coordinate it was made without reads as na")
        let line = try XCTUnwrap(result.lines.first)
        XCTAssertFalse(line.isComplete, "x2 and y2 are still missing")
        XCTAssertEqual(line.x1, 1)
        XCTAssertEqual(line.y1, 5)
        XCTAssertTrue(try XCTUnwrap(result.labels.first).isComplete)
        XCTAssertFalse(try XCTUnwrap(result.boxes.first).isComplete, "the bottom right is still missing")
        XCTAssertFalse(result.lines[1].isComplete, "a point without a price leaves the line hidden")
    }

    func testPointSettersMoveEndsAndAnchors() throws {
        let result = try output(
            """
            var line ln = line.new(0, 1.0, 1, 2.0)
            var label lb = label.new(0, 1.0, "t")
            var box bx = box.new(0, 4.0, 1, 3.0)
            if bar_index == 2
                ln.set_first_point(chart.point.from_index(7, 9.0))
                ln.set_second_point(chart.point.from_index(8, 10.0))
                lb.set_point(chart.point.from_index(5, 6.0))
                bx.set_top_left_point(chart.point.from_index(2, 20.0))
                bx.set_bottom_right_point(chart.point.from_index(6, 15.0))
            """, count: 3)
        let line = try XCTUnwrap(result.lines.first)
        XCTAssertEqual([line.x1, line.x2], [7, 8])
        XCTAssertEqual([line.y1, line.y2], [9, 10])
        let label = try XCTUnwrap(result.labels.first)
        XCTAssertEqual([Double(label.x), label.y], [5, 6])
        let box = try XCTUnwrap(result.boxes.first)
        XCTAssertEqual([box.left, box.right], [2, 6])
        XCTAssertEqual([box.top, box.bottom], [20, 15])
    }
}
