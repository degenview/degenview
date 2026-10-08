import SwiftUI

/// One drop zone of the watchlist: before a row, onto a section header, or the end of the list.
/// The move is committed once, in `performDrop`; hovering only moves the insertion indicator.
struct WatchlistDropDelegate: DropDelegate {
    enum Target {
        case row(UUID)
        case sectionHeader(WatchlistSection)
        case end

        var key: String {
            switch self {
            case .row(let id): return id.uuidString
            case .sectionHeader(let section): return section.id.uuidString
            case .end: return "end"
            }
        }
    }

    let target: Target
    let viewModel: WatchlistSidebarViewModel
    @Binding var draggedID: UUID?
    @Binding var hoverKey: String?

    func validateDrop(info: DropInfo) -> Bool {
        draggedID != nil && viewModel.canReorder
    }

    func dropEntered(info: DropInfo) {
        if validateDrop(info: info) { hoverKey = target.key }
    }

    func dropUpdated(info: DropInfo) -> DropProposal? {
        DropProposal(operation: validateDrop(info: info) ? .move : .forbidden)
    }

    func dropExited(info: DropInfo) {
        if hoverKey == target.key { hoverKey = nil }
    }

    func performDrop(info: DropInfo) -> Bool {
        defer {
            draggedID = nil
            hoverKey = nil
        }
        guard let draggedID, viewModel.canReorder else { return false }
        let movesSection = viewModel.list?.sections.contains { $0.id == draggedID } ?? false
        withAnimation {
            switch target {
            case .row(let id): viewModel.move(draggedID, before: id)
            case .sectionHeader(let section):
                if movesSection {
                    viewModel.move(draggedID, before: section.id)
                } else {
                    viewModel.moveToTop(draggedID, of: section)
                }
            case .end: viewModel.move(draggedID, before: nil)
            }
        }
        return true
    }
}
