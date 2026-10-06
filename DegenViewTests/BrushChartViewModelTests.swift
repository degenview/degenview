import XCTest

@testable import DegenView

@MainActor
final class BrushChartViewModelTests: XCTestCase {
    private let t0 = Date(timeIntervalSince1970: 1_790_078_400)
    private let size = CGSize(width: 500, height: 300)
    private var store: DrawingStore!
    private var manager: UndoManager!
    private var chart: ChartViewModel!

    private final class EmptySource: TickerDataSource {
        let type = DataSourceType.binance
        func fetchKlines(symbol: String, interval: String, limit: Int) async throws -> [KlineData] { [] }
        func searchTickers(query: String) async throws -> [TickerSearchResult] { [] }
    }

    override func setUpWithError() throws {
        store = DrawingStore(database: try AppDatabase.makeInMemory())
        manager = UndoManager()
        chart = ChartViewModel(ticker: "BTC", api: EmptySource(), drawingStore: store)
        chart.klineData = (0..<20).map {
            KlineData(time: t0.addingTimeInterval(Double($0) * 3600), price: 100 + Double($0))
        }
        chart.drawingUndoCoordinator = DrawingUndoCoordinator(undoManager: manager, store: store)
    }

    private var plot: ChartPlot { chart.plot(in: size) }

    private func point(_ x: CGFloat, _ y: CGFloat) -> CGPoint {
        let rect = plot.plotRect
        return CGPoint(x: rect.minX + x * rect.width, y: rect.minY + y * rect.height)
    }

    private func stored() -> [BrushDrawing] { store.brushes(ticker: "BTC", source: .binance) }

    /// A run of plot-fraction positions from `from` to `to`, `steps` samples apart.
    private func ramp(_ from: CGPoint, _ to: CGPoint, steps: Int = 40) -> [CGPoint] {
        (0...steps).map { index in
            let t = CGFloat(index) / CGFloat(steps)
            return CGPoint(x: from.x + (to.x - from.x) * t, y: from.y + (to.y - from.y) * t)
        }
    }

    /// Draws a stroke through `fractions` with one drag sample per entry.
    @discardableResult
    private func draw(_ fractions: [CGPoint]) -> Bool {
        let plot = plot
        let screen = fractions.map { point($0.x, $0.y) }
        chart.beginBrushDraft(at: screen[0], in: plot)
        for p in screen.dropFirst() { chart.appendBrushPoint(at: p, in: plot) }
        return chart.commitBrushDraft(at: screen.last, in: plot)
    }

    func testSamplingIgnoresPointerJitter() {
        let plot = plot
        let origin = point(0.2, 0.5)
        chart.beginBrushDraft(at: origin, in: plot)
        chart.appendBrushPoint(at: CGPoint(x: origin.x + 1, y: origin.y), in: plot)
        XCTAssertEqual(chart.brushDraft?.points.count, 1)
        chart.appendBrushPoint(at: CGPoint(x: origin.x + 2, y: origin.y), in: plot)
        XCTAssertEqual(chart.brushDraft?.points.count, 2)
    }

    func testCommitSimplifiesPersistsOnceAndIsOneUndoStep() {
        XCTAssertTrue(draw(ramp(CGPoint(x: 0.1, y: 0.5), CGPoint(x: 0.9, y: 0.5))))
        XCTAssertEqual(stored().count, 1)
        XCTAssertEqual(stored()[0].points.count, 2, "a straight run collapses to its ends")
        XCTAssertNil(chart.brushDraft)
        XCTAssertEqual(manager.undoMenuItemTitle, "Undo Add Brush Stroke")

        manager.undo()
        XCTAssertTrue(stored().isEmpty)
        XCTAssertTrue(chart.brushes.isEmpty)
        XCTAssertFalse(manager.canUndo)
        manager.redo()
        XCTAssertEqual(stored().count, 1)
        XCTAssertEqual(stored()[0].points.count, 2)
    }

    func testCornersSurviveSimplification() {
        let down = ramp(CGPoint(x: 0.1, y: 0.2), CGPoint(x: 0.5, y: 0.6))
        let up = ramp(CGPoint(x: 0.5, y: 0.6), CGPoint(x: 0.9, y: 0.2))
        draw(down + up.dropFirst())
        XCTAssertEqual(stored()[0].points.count, 3)
    }

    func testClickMakesADotNotAnEmptyStroke() {
        let plot = plot
        let p = point(0.5, 0.5)
        chart.beginBrushDraft(at: p, in: plot)
        XCTAssertTrue(chart.commitBrushDraft(at: p, in: plot))
        XCTAssertEqual(stored().count, 1)
        XCTAssertEqual(stored()[0].points.count, 1)
        XCTAssertNotNil(chart.brushHit(at: p, in: plot))
    }

    func testCancelWritesNothingAndRegistersNoUndo() {
        let plot = plot
        chart.beginBrushDraft(at: point(0.2, 0.2), in: plot)
        chart.appendBrushPoint(at: point(0.4, 0.4), in: plot)
        chart.cancelBrushDraft()
        XCTAssertNil(chart.brushDraft)
        XCTAssertTrue(stored().isEmpty)
        XCTAssertFalse(manager.canUndo)
        XCTAssertFalse(chart.commitBrushDraft(at: point(0.5, 0.5), in: plot))
        XCTAssertTrue(stored().isEmpty)
    }

    func testHitFollowsTheStrokeNotItsBoundingBox() {
        draw(ramp(CGPoint(x: 0.1, y: 0.1), CGPoint(x: 0.7, y: 0.7)))
        let plot = plot
        XCTAssertNotNil(chart.brushHit(at: point(0.4, 0.4), in: plot))
        // Inside the bounding box, far from the diagonal.
        XCTAssertNil(chart.brushHit(at: point(0.65, 0.15), in: plot))
        XCTAssertNil(chart.brushHit(at: point(0.95, 0.95), in: plot))
    }

    func testTranslateIsAbsoluteFromTheOriginal() throws {
        draw(ramp(CGPoint(x: 0.2, y: 0.4), CGPoint(x: 0.5, y: 0.55)))
        let plot = plot
        let original = try XCTUnwrap(chart.brushes.first)
        let before = chart.projectedPoints(original.points, in: plot)

        // Many intermediate events, then the final position: same as one jump.
        for step in 1...50 {
            chart.translateBrush(
                original: original, by: CGSize(width: CGFloat(step), height: CGFloat(step) / 2), in: plot)
        }
        let after = chart.projectedPoints(chart.brushes[0].points, in: plot)
        for (a, b) in zip(before, after) {
            XCTAssertEqual(b.x - a.x, 50, accuracy: 0.001)
            XCTAssertEqual(b.y - a.y, 25, accuracy: 0.001)
        }

        chart.translateBrush(original: original, by: .zero, in: plot)
        for (a, b) in zip(before, chart.projectedPoints(chart.brushes[0].points, in: plot)) {
            XCTAssertEqual(a.x, b.x, accuracy: 0.001)
            XCTAssertEqual(a.y, b.y, accuracy: 0.001)
        }
    }

    func testMoveCommitsOnceAndUndoRestores() throws {
        draw(ramp(CGPoint(x: 0.2, y: 0.4), CGPoint(x: 0.5, y: 0.4)))
        manager.removeAllActions()
        let plot = plot
        let original = try XCTUnwrap(chart.brushes.first)
        chart.translateBrush(original: original, by: CGSize(width: 40, height: 30), in: plot)
        chart.commitBrushDrag(original: original)
        XCTAssertEqual(manager.undoMenuItemTitle, "Undo Move Brush Stroke")
        XCTAssertNotEqual(stored()[0].points, original.points)
        manager.undo()
        XCTAssertEqual(stored(), [original])
    }

    func testLockedStrokeDoesNotMove() throws {
        draw(ramp(CGPoint(x: 0.2, y: 0.4), CGPoint(x: 0.5, y: 0.4)))
        var locked = try XCTUnwrap(chart.brushes.first)
        locked.isLocked = true
        chart.updateBrush(locked)
        chart.translateBrush(original: locked, by: CGSize(width: 40, height: 30), in: plot)
        XCTAssertEqual(chart.brushes[0].points, locked.points)
    }

    func testStyleEditsAndDeleteUndo() throws {
        draw(ramp(CGPoint(x: 0.2, y: 0.4), CGPoint(x: 0.5, y: 0.4)))
        var edited = try XCTUnwrap(chart.brushes.first)
        edited.color = .red
        edited.lineWidth = 6
        chart.updateBrush(edited)
        XCTAssertEqual(stored()[0].color, .red)
        manager.undo()
        XCTAssertEqual(stored()[0].color, BrushStyle.default.color)

        chart.selectedBrushID = chart.brushes[0].id
        XCTAssertTrue(chart.removeSelectedBrush())
        XCTAssertTrue(stored().isEmpty)
        XCTAssertNil(chart.selectedBrushID)
        manager.undo()
        XCTAssertEqual(stored().count, 1)
    }

    func testStrokeSurvivesATimeframeLikeProjection() throws {
        draw(ramp(CGPoint(x: 0.2, y: 0.4), CGPoint(x: 0.5, y: 0.52)))
        let stroke = try XCTUnwrap(chart.brushes.first)
        // Re-project onto a chart with different candle spacing: the points keep their time and price.
        let coarse = (0..<10).map {
            KlineData(time: t0.addingTimeInterval(Double($0) * 7200), price: 100 + Double($0) * 2)
        }
        chart.klineData = coarse
        let projected = chart.projectedPoints(stroke.points, in: plot)
        XCTAssertEqual(projected.count, stroke.points.count)
        XCTAssertEqual(chart.brushes[0].points, stroke.points)
    }
}
