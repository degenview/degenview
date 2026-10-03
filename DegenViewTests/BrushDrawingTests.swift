import XCTest

@testable import DegenView

@MainActor
final class BrushDrawingTests: XCTestCase {
    private var directory: URL!
    private var database: AppDatabase!

    override func setUpWithError() throws {
        directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        database = try AppDatabase(path: directory.appendingPathComponent("test.sqlite").path)
    }

    override func tearDownWithError() throws {
        try FileManager.default.removeItem(at: directory)
    }

    private func stroke(_ seed: Double) -> BrushDrawing {
        let base = 1_700_000_000 + seed * 100
        let points = (0..<5).map {
            TrendAnchor(date: Date(timeIntervalSince1970: base + Double($0) * 37.5), price: seed + Double($0) / 3)
        }
        return BrushDrawing(points: points, style: BrushStyle(color: .purple, lineWidth: 4, opacity: 0.5))
    }

    func testCodableRoundTripKeepsSubCandleTimeAndStyle() throws {
        var original = stroke(1)
        original.isLocked = true
        let decoded = try JSONDecoder().decode(BrushDrawing.self, from: JSONEncoder().encode(original))
        XCTAssertEqual(decoded, original)
        XCTAssertEqual(decoded.style, BrushStyle(color: .purple, lineWidth: 4, opacity: 0.5))
        XCTAssertEqual(decoded.schemaVersion, BrushDrawing.schemaVersion)
    }

    func testStrokesPersistPerInstrumentAndNextToOtherKinds() {
        let store = DrawingStore(database: database)
        let line = TrendLine(
            start: TrendAnchor(date: Date(timeIntervalSince1970: 1), price: 1),
            end: TrendAnchor(date: Date(timeIntervalSince1970: 2), price: 2))
        let brush = stroke(1)
        store.save([line], ticker: "BTC", source: .binance)
        store.save([brush], ticker: "BTC", source: .binance)

        let reloaded = DrawingStore(database: database)
        XCTAssertEqual(reloaded.brushes(ticker: "BTC", source: .binance), [brush])
        XCTAssertEqual(reloaded.lines(ticker: "BTC", source: .binance), [line])
        // Same symbol on another source is a different instrument.
        XCTAssertTrue(reloaded.brushes(ticker: "BTC", source: .coinbase).isEmpty)
        XCTAssertTrue(reloaded.brushes(ticker: "ETH", source: .binance).isEmpty)
    }

    func testUndoCoordinatorAddMoveDelete() {
        let store = DrawingStore(database: database)
        let manager = UndoManager()
        let coordinator = DrawingUndoCoordinator(undoManager: manager, store: store)
        let instrument = store.key(ticker: "BTC", source: .binance)
        let brush = stroke(2)

        store.save([brush], ticker: "BTC", source: .binance)
        coordinator.recordBrush(
            instrument: instrument, before: nil, beforeIndex: 0, after: brush, afterIndex: 0,
            actionName: "Add Brush Stroke")
        XCTAssertEqual(manager.undoMenuItemTitle, "Undo Add Brush Stroke")

        manager.undo()
        XCTAssertTrue(store.brushes(ticker: "BTC", source: .binance).isEmpty)
        manager.redo()
        XCTAssertEqual(store.brushes(ticker: "BTC", source: .binance), [brush])
        XCTAssertEqual(DrawingStore(database: database).brushes(ticker: "BTC", source: .binance), [brush])

        var moved = brush
        moved.points[0].price += 10
        store.save([moved], ticker: "BTC", source: .binance)
        coordinator.recordBrush(
            instrument: instrument, before: brush, beforeIndex: 0, after: moved, afterIndex: 0,
            actionName: "Move Brush Stroke")
        manager.undo()
        XCTAssertEqual(store.brushes(ticker: "BTC", source: .binance), [brush])
    }
}
