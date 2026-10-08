import SwiftUI
import UniformTypeIdentifiers

/// The app-wide watchlist panel. Milestone 1 shows the Favorites list; the selector,
/// sections, columns and live quotes build on this.
struct WatchlistSidebar: View {
    @ObservedObject var store: WatchlistStore
    let onAdd: () -> Void
    let onSelect: (WatchlistInstrument) -> Void
    @State private var draggedID: UUID?

    private var list: Watchlist? { store.favorites }

    var body: some View {
        VStack(spacing: 0) {
            HStack {
                Text(list?.name ?? "Watchlist")
                    .font(.headline)
                Spacer()
                Button(action: onAdd) {
                    Image(systemName: "plus")
                }
                .buttonStyle(.plain)
                .disabled(store.loadFailed)
                .help("Add Symbol")
                .accessibilityLabel("Add Symbol")
            }
            .padding(12)

            Divider()

            if store.loadFailed {
                ContentUnavailableView(
                    "Watchlists Unavailable",
                    systemImage: "exclamationmark.triangle",
                    description: Text(WatchlistError.persistenceUnavailable.localizedDescription)
                )
                .frame(maxHeight: .infinity)
            } else if let list, !list.instruments.isEmpty {
                List {
                    ForEach(list.instruments) { item in
                        WatchlistRow(item: item) { onSelect(item) }
                            .padding(.bottom, 4)
                            .contextMenu {
                                Button("Remove from Watchlist", role: .destructive) {
                                    try? store.removeEntry(item.id, from: list.id)
                                }
                            }
                            .onDrag {
                                draggedID = item.id
                                return NSItemProvider(object: item.id.uuidString as NSString)
                            }
                            .onDrop(
                                of: [UTType.text],
                                delegate: WatchlistRowDropDelegate(
                                    targetID: item.id, listID: list.id, store: store, draggedID: $draggedID))
                    }
                }
                .listStyle(.sidebar)
            } else {
                ContentUnavailableView(
                    "No Symbols",
                    systemImage: "star",
                    description: Text("Save a stock, crypto, or prediction-market item for quick access.")
                )
                .frame(maxHeight: .infinity)
            }
        }
        .frame(width: UI.favoritesSidebarWidth)
        .background(.bar)
    }
}

/// Commits one move when the row is dropped, never while hovering.
private struct WatchlistRowDropDelegate: DropDelegate {
    let targetID: UUID
    let listID: UUID
    let store: WatchlistStore
    @Binding var draggedID: UUID?

    func dropUpdated(info: DropInfo) -> DropProposal? {
        DropProposal(operation: .move)
    }

    func performDrop(info: DropInfo) -> Bool {
        defer { draggedID = nil }
        guard let draggedID, draggedID != targetID else { return false }
        withAnimation { try? store.moveEntry(draggedID, before: targetID, in: listID) }
        return true
    }
}

private struct WatchlistRow: View {
    let item: WatchlistInstrument
    let onSelect: () -> Void
    @StateObject private var viewModel: ChartViewModel
    @State private var showTitleTooltip = false
    @State private var titleTooltipTask: Task<Void, Never>?

    init(item: WatchlistInstrument, onSelect: @escaping () -> Void) {
        self.item = item
        self.onSelect = onSelect
        let vm = ChartViewModel(
            ticker: item.instrument.symbol,
            source: item.instrument.source,
            displayName: item.displayName
        )
        if let series = item.pmSeries { vm.pmSeries = series }
        _viewModel = StateObject(wrappedValue: vm)
    }

    var body: some View {
        Button(action: onSelect) {
            HStack(spacing: 8) {
                ChartIconView(viewModel: viewModel, showsSource: false)

                VStack(alignment: .leading, spacing: 2) {
                    Text(item.name)
                        .font(.callout.weight(.medium))
                        .lineLimit(1)
                        .onHover { isHovering in
                            titleTooltipTask?.cancel()
                            if isHovering {
                                titleTooltipTask = Task {
                                    try? await Task.sleep(for: .milliseconds(250))
                                    guard !Task.isCancelled else { return }
                                    await MainActor.run { showTitleTooltip = true }
                                }
                            } else {
                                showTitleTooltip = false
                            }
                        }
                        .popover(isPresented: $showTitleTooltip, arrowEdge: .bottom) {
                            Text(item.name)
                                .font(.callout)
                                .padding(.horizontal, 10)
                                .padding(.vertical, 6)
                        }
                    Text(item.label)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                }

                Spacer(minLength: 4)

                if let percentChange = viewModel.priceChangePercent,
                    let amountChange = viewModel.priceChangeAmount
                {
                    VStack(alignment: .trailing, spacing: 1) {
                        Text(String(format: "%+.2f%%", percentChange))
                        Text(PriceFormatter.changeAmount(amountChange, scale: viewModel.priceScale))
                            .font(.caption)
                    }
                    .monospacedDigit()
                    .multilineTextAlignment(.trailing)
                    .foregroundStyle(percentChange >= 0 ? Color.green : Color.red)
                } else if viewModel.errorMessage == nil {
                    ProgressView().controlSize(.small)
                } else {
                    Text("—").foregroundStyle(.secondary)
                }
            }
            .foregroundStyle(.primary)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .task {
            await viewModel.fetchData(for: .oneDay, count: TimeRange.oneDay.dataPointLimit, silent: true)
        }
        .onDisappear {
            titleTooltipTask?.cancel()
        }
    }
}
