import SwiftUI

enum ChartGridDropTarget: Equatable {
    case existing(columnID: UUID, before: UUID?)
    case newColumn
}

/// Synchronous internal-chart drop handling. The dragged chart ID is captured
/// when `.onDrag` starts, so hover previews do not wait for item-provider loading.
struct ChartGridDropDelegate: DropDelegate {
    let destination: ChartGridDropTarget
    let viewModel: ContentViewModel
    @Binding var draggedChartID: UUID?
    @Binding var activeTarget: ChartGridDropTarget?
    @Binding var previewedChartID: UUID?

    func dropEntered(info: DropInfo) {
        guard let chartID = draggedChartID else { return }
        withAnimation(.easeInOut(duration: 0.18)) {
            activeTarget = destination
        }

        guard destination == .newColumn else {
            previewedChartID = nil
            return
        }
        Task { @MainActor in
            try? await Task.sleep(nanoseconds: 300_000_000)
            guard activeTarget == .newColumn, draggedChartID == chartID else { return }
            withAnimation(.easeInOut(duration: 0.2)) {
                previewedChartID = chartID
            }
        }
    }

    func dropExited(info: DropInfo) {
        guard activeTarget == destination else { return }
        withAnimation(.easeInOut(duration: 0.18)) {
            activeTarget = nil
            if destination == .newColumn { previewedChartID = nil }
        }
    }

    func performDrop(info: DropInfo) -> Bool {
        guard let chartID = draggedChartID else { return false }
        switch destination {
        case .existing(let columnID, let targetID):
            viewModel.moveChart(chartID, toColumn: columnID, before: targetID)
        case .newColumn:
            viewModel.moveChartToNewTrailingColumn(chartID)
        }
        withAnimation(.easeInOut(duration: 0.18)) {
            activeTarget = nil
            previewedChartID = nil
            draggedChartID = nil
        }
        return true
    }
}
