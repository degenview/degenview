import Foundation

@MainActor
final class DrawingUndoCoordinator {
    private let undoManager: UndoManager
    private let store: DrawingStore

    init(undoManager: UndoManager, store: DrawingStore? = nil) {
        self.undoManager = undoManager
        self.store = store ?? .shared
        undoManager.groupsByEvent = false
    }

    func recordLine(
        instrument: String, before: TrendLine?, beforeIndex: Int, after: TrendLine?, afterIndex: Int,
        actionName: String
    ) {
        guard before != after || beforeIndex != afterIndex else { return }
        undoManager.beginUndoGrouping()
        registerLine(
            instrument: instrument, current: after, currentIndex: afterIndex, replacement: before,
            replacementIndex: beforeIndex, actionName: actionName)
        undoManager.setActionName(actionName)
        undoManager.endUndoGrouping()
    }

    func recordFibonacci(
        instrument: String, before: FibonacciRetracementDrawing?, beforeIndex: Int,
        after: FibonacciRetracementDrawing?, afterIndex: Int, actionName: String
    ) {
        guard before != after || beforeIndex != afterIndex else { return }
        undoManager.beginUndoGrouping()
        registerFibonacci(
            instrument: instrument, current: after, currentIndex: afterIndex, replacement: before,
            replacementIndex: beforeIndex, actionName: actionName)
        undoManager.setActionName(actionName)
        undoManager.endUndoGrouping()
    }

    private func registerLine(
        instrument: String, current: TrendLine?, currentIndex: Int, replacement: TrendLine?,
        replacementIndex: Int, actionName: String
    ) {
        let id = (current ?? replacement)!.id
        undoManager.registerUndo(withTarget: self) { coordinator in
            coordinator.store.setLine(replacement, at: replacementIndex, instrument: instrument, id: id)
            coordinator.registerLine(
                instrument: instrument, current: replacement, currentIndex: replacementIndex,
                replacement: current, replacementIndex: currentIndex, actionName: actionName)
            coordinator.undoManager.setActionName(actionName)
        }
    }

    private func registerFibonacci(
        instrument: String, current: FibonacciRetracementDrawing?, currentIndex: Int,
        replacement: FibonacciRetracementDrawing?, replacementIndex: Int, actionName: String
    ) {
        let id = (current ?? replacement)!.id
        undoManager.registerUndo(withTarget: self) { coordinator in
            coordinator.store.setFibonacci(replacement, at: replacementIndex, instrument: instrument, id: id)
            coordinator.registerFibonacci(
                instrument: instrument, current: replacement, currentIndex: replacementIndex,
                replacement: current, replacementIndex: currentIndex, actionName: actionName)
            coordinator.undoManager.setActionName(actionName)
        }
    }
}
