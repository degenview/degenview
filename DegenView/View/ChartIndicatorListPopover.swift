import SwiftUI

/// Indicators that did not fit in the header strip, one chip per row with its controls always shown.
struct ChartIndicatorListPopover: View {
    @ObservedObject var viewModel: ChartViewModel
    let items: [ChartIndicatorItem]
    var onStyleChanged: () -> Void = {}
    let onRemove: (ChartIndicatorItem) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text("Indicators")
                .font(.caption.weight(.semibold))
                .foregroundStyle(.secondary)
            ForEach(items) { item in
                ChartIndicatorChip(
                    viewModel: viewModel, item: item, alwaysShowControls: true, onStyleChanged: onStyleChanged,
                    onRemove: { onRemove(item) })
            }
        }
        .padding(12)
    }
}
