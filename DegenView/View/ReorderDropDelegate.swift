import SwiftUI
import UniformTypeIdentifiers

/// Drop delegate for drag-and-drop reorder in grid layout.
/// Handles UTF-8 plain text drop items containing unique chart IDs.
struct ReorderDropDelegate: DropDelegate {
    let targetTicker: String   // uniqueID of target
    let viewModel: ContentViewModel

    func performDrop(info: DropInfo) -> Bool {
        guard let provider = info.itemProviders(for: [.utf8PlainText]).first else {
            return false
        }

        provider.loadItem(forTypeIdentifier: UTType.utf8PlainText.identifier, options: nil) { item, _ in
            guard let data = item as? Data,
                  let sourceID = String(data: data, encoding: .utf8)
            else { return }

            DispatchQueue.main.async {
                guard let fromIdx = viewModel.chartViewModels.firstIndex(where: { $0.uniqueID == sourceID }),
                      let toIdx = viewModel.chartViewModels.firstIndex(where: { $0.uniqueID == targetTicker }),
                      fromIdx != toIdx
                else { return }

                viewModel.moveTicker(from: IndexSet(integer: fromIdx), to: toIdx)
            }
        }
        return true
    }
}
