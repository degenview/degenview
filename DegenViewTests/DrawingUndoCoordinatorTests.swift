import Foundation
import XCTest

@testable import DegenView

@MainActor
final class DrawingUndoCoordinatorTests: XCTestCase {
    private var directory: URL!
    private var store: DrawingStore!

    override func setUpWithError() throws {
        directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        store = DrawingStore(directory: directory)
    }

    override func tearDownWithError() throws {
        try FileManager.default.removeItem(at: directory)
    }

    func testAddUndoRedoPreservesIDOrderAndMenuTitles() {
        let manager = UndoManager()
        let coordinator = DrawingUndoCoordinator(undoManager: manager, store: store)
        let instrument = store.key(ticker: "BTC", source: .binance)
        let first = line(price: 1)
        let added = line(price: 2)
        store.save([first, added], ticker: "BTC", source: .binance)

        coordinator.recordLine(
            instrument: instrument, before: nil, beforeIndex: 1, after: added, afterIndex: 1,
            actionName: "Add Trend Line")

        XCTAssertEqual(manager.undoMenuItemTitle, "Undo Add Trend Line")
        manager.undo()
        XCTAssertEqual(store.lines(ticker: "BTC", source: .binance), [first])
        XCTAssertEqual(manager.redoMenuItemTitle, "Redo Add Trend Line")
        manager.redo()
        XCTAssertEqual(store.lines(ticker: "BTC", source: .binance), [first, added])
    }

    func testNewActionAfterUndoClearsRedo() {
        let manager = UndoManager()
        let coordinator = DrawingUndoCoordinator(undoManager: manager, store: store)
        let instrument = store.key(ticker: "ETH", source: .binance)
        let first = line(price: 1)
        store.save([first], ticker: "ETH", source: .binance)
        coordinator.recordLine(
            instrument: instrument, before: nil, beforeIndex: 0, after: first, afterIndex: 0,
            actionName: "Add Trend Line")
        manager.undo()

        let second = line(price: 2)
        store.save([second], ticker: "ETH", source: .binance)
        coordinator.recordLine(
            instrument: instrument, before: nil, beforeIndex: 0, after: second, afterIndex: 0,
            actionName: "Add Trend Line")

        XCTAssertFalse(manager.canRedo)
    }

    func testWindowHistoriesAreIsolatedAndTargetedMutationKeepsOtherChanges() {
        let firstManager = UndoManager()
        let secondManager = UndoManager()
        let firstCoordinator = DrawingUndoCoordinator(undoManager: firstManager, store: store)
        let secondCoordinator = DrawingUndoCoordinator(undoManager: secondManager, store: store)
        let instrument = store.key(ticker: "SOL", source: .binance)
        let first = line(price: 1)
        let second = line(price: 2)

        store.save([first], ticker: "SOL", source: .binance)
        firstCoordinator.recordLine(
            instrument: instrument, before: nil, beforeIndex: 0, after: first, afterIndex: 0,
            actionName: "Add Trend Line")
        store.save([first, second], ticker: "SOL", source: .binance)
        secondCoordinator.recordLine(
            instrument: instrument, before: nil, beforeIndex: 1, after: second, afterIndex: 1,
            actionName: "Add Trend Line")

        firstManager.undo()
        XCTAssertEqual(store.lines(ticker: "SOL", source: .binance), [second])
        XCTAssertTrue(secondManager.canUndo)
        XCTAssertFalse(firstManager.canUndo)
    }

    func testFibonacciEditRestoresExactDrawingAndPersists() {
        let manager = UndoManager()
        let coordinator = DrawingUndoCoordinator(undoManager: manager, store: store)
        let instrument = store.key(ticker: "BTC", source: .coingecko)
        let original = FibonacciRetracementDrawing(
            point1: anchor(price: 10), point2: anchor(price: 20))
        var edited = original
        edited.style.reverse = true
        store.save([edited], ticker: "BTC", source: .coingecko)
        coordinator.recordFibonacci(
            instrument: instrument, before: original, beforeIndex: 0, after: edited,
            afterIndex: 0, actionName: "Edit Fibonacci Retracement")

        manager.undo()
        XCTAssertEqual(store.fibs(ticker: "BTC", source: .coingecko), [original])
        let reloaded = DrawingStore(directory: directory)
        XCTAssertEqual(reloaded.fibs(ticker: "BTC", source: .coingecko), [original])
    }

    private func line(price: Double) -> TrendLine {
        TrendLine(start: anchor(price: price), end: anchor(price: price + 1))
    }

    private func anchor(price: Double) -> TrendAnchor {
        TrendAnchor(date: Date(timeIntervalSince1970: price), price: price)
    }
}
