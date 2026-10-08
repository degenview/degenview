import SwiftUI

/// The chart card header's indicator controls, right of the ticker symbol: a chip per applied
/// indicator (built-ins and scripts alike) and a "+" to add more. Folds into a count pill when the
/// card is too narrow, so the symbol and price are never pushed out.
struct ChartIndicatorStrip: View {
    @ObservedObject var viewModel: ChartViewModel
    let isHeaderHovered: Bool
    var onStyleChanged: () -> Void = {}
    let onOpenSettings: () -> Void

    @State private var showingOverflow = false
    /// The script awaiting a "remove" confirmation — only asked when removing it also deletes an alert.
    @State private var pendingRemoval: UUID?

    var body: some View {
        let items = viewModel.indicatorItems
        HStack(spacing: 4) {
            if !items.isEmpty {
                ViewThatFits(in: .horizontal) {
                    chips(items)
                    summaryPill(items)
                }
            }
            ChartIndicatorAddMenu(
                viewModel: viewModel, isEmpty: items.isEmpty, isHeaderHovered: isHeaderHovered,
                onStyleChanged: onStyleChanged, onOpenSettings: onOpenSettings)
        }
        .confirmationDialog(
            "Remove \(pendingRemoval.flatMap { id in items.first { $0.kind == .script(id) }?.title } ?? "script")?",
            isPresented: Binding(get: { pendingRemoval != nil }, set: { if !$0 { pendingRemoval = nil } }),
            titleVisibility: .visible
        ) {
            Button("Remove Script and Alert", role: .destructive) {
                if let id = pendingRemoval { removeScript(id) }
                pendingRemoval = nil
            }
        } message: {
            Text("This script has an alert on this chart. Removing the script deletes the alert too.")
        }
    }

    private func chips(_ items: [ChartIndicatorItem]) -> some View {
        let (inline, overflow) = ChartIndicatorItem.split(items)
        return HStack(spacing: 4) {
            ForEach(inline) { item in
                ChartIndicatorChip(
                    viewModel: viewModel, item: item, onStyleChanged: onStyleChanged,
                    onRemove: { requestRemoval(of: item) })
            }
            if !overflow.isEmpty {
                pill("+\(overflow.count)", items: overflow)
            }
        }
    }

    private func summaryPill(_ items: [ChartIndicatorItem]) -> some View {
        pill(count: items.count, items: items)
    }

    /// `systemImage` leads the count when the pill stands in for every chip.
    private func pill(_ title: String? = nil, count: Int? = nil, items: [ChartIndicatorItem]) -> some View {
        Button {
            showingOverflow.toggle()
        } label: {
            HStack(spacing: 4) {
                if let count {
                    Image(systemName: "chart.xyaxis.line").font(.system(size: 9, weight: .medium))
                    Text("\(count)").monospacedDigit()
                } else if let title {
                    Text(title)
                }
            }
            .font(.caption2.weight(.medium))
            .foregroundStyle(Color.primary.opacity(0.7))
            .padding(.horizontal, 8)
            .frame(height: 20)
            .background(Capsule().fill(Color.primary.opacity(0.06)))
            .fixedSize()
        }
        .buttonStyle(.plain)
        .help(count.map { "\($0) indicators" } ?? "More indicators")
        .accessibilityLabel(count.map { "\($0) indicators" } ?? "More indicators")
        .popover(isPresented: $showingOverflow) {
            ChartIndicatorListPopover(
                viewModel: viewModel, items: items, onStyleChanged: onStyleChanged,
                onRemove: { requestRemoval(of: $0) })
        }
    }

    private func requestRemoval(of item: ChartIndicatorItem) {
        switch item.kind {
        case .builtIn(let indicator):
            indicator.setOn(false, in: viewModel)
            onStyleChanged()
        case .script(let id):
            let alert = PineAlertStore.shared.subscription(
                forChart: viewModel.chartID, instanceID: id, dataset: viewModel.pineAlertDataset)
            if alert != nil {
                pendingRemoval = id
            } else {
                removeScript(id)
            }
        }
    }

    private func removeScript(_ id: UUID) {
        viewModel.removePineInstance(id)
        onStyleChanged()
    }
}
