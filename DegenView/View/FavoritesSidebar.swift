import SwiftUI
import UniformTypeIdentifiers

struct FavoritesSidebar: View {
    @ObservedObject var store: FavoritesStore
    let onAdd: () -> Void
    let onSelect: (FavoriteItem) -> Void
    @State private var draggedFavoriteID: UUID?

    var body: some View {
        VStack(spacing: 0) {
            HStack {
                Text("Favorites")
                    .font(.headline)
                Spacer()
                Button(action: onAdd) {
                    Image(systemName: "plus")
                }
                .buttonStyle(.plain)
                .help("Add Favorite")
                .accessibilityLabel("Add Favorite")
            }
            .padding(12)

            Divider()

            if store.items.isEmpty {
                ContentUnavailableView(
                    "No Favorites",
                    systemImage: "star",
                    description: Text("Save a stock, crypto, or Polymarket item for quick access.")
                )
                .frame(maxHeight: .infinity)
            } else {
                List {
                    ForEach(store.items) { item in
                        FavoriteRow(item: item) { onSelect(item) }
                            .padding(.bottom, 4)
                            .overlay(alignment: .bottom) {
                                if item.id != store.items.last?.id {
                                    Divider()
                                        .opacity(0.45)
                                }
                            }
                            .contextMenu {
                                Button("Delete", role: .destructive) { store.remove(item) }
                            }
                            .onDrag {
                                draggedFavoriteID = item.id
                                return NSItemProvider(object: item.id.uuidString as NSString)
                            }
                            .onDrop(
                                of: [UTType.text],
                                delegate: FavoriteDropDelegate(
                                    targetID: item.id,
                                    store: store,
                                    draggedID: $draggedFavoriteID
                                )
                            )
                    }
                    .onDelete { offsets in
                        let removed = offsets.map { store.items[$0] }
                        removed.forEach(store.remove)
                    }
                }
                .listStyle(.sidebar)
            }
        }
        .frame(width: UI.favoritesSidebarWidth)
        .background(.bar)
    }
}

private struct FavoriteDropDelegate: DropDelegate {
    let targetID: UUID
    let store: FavoritesStore
    @Binding var draggedID: UUID?

    func dropEntered(info: DropInfo) {
        guard let draggedID, draggedID != targetID else { return }
        withAnimation { store.move(draggedID, to: targetID) }
    }

    func dropUpdated(info: DropInfo) -> DropProposal? {
        DropProposal(operation: .move)
    }

    func performDrop(info: DropInfo) -> Bool {
        draggedID = nil
        return true
    }

    func dropExited(info: DropInfo) {
        // Keep the identity while crossing the small gaps between rows. The next
        // target's `dropEntered` needs it to continue the reorder.
    }
}

private struct FavoriteRow: View {
    let item: FavoriteItem
    let onSelect: () -> Void
    @StateObject private var viewModel: ChartViewModel
    @State private var iconURL: URL?
    @State private var showTitleTooltip = false
    @State private var titleTooltipTask: Task<Void, Never>?

    init(item: FavoriteItem, onSelect: @escaping () -> Void) {
        self.item = item
        self.onSelect = onSelect
        let vm = ChartViewModel(
            ticker: item.config.symbol,
            source: item.config.source,
            displayName: item.config.displayName
        )
        if let series = item.config.pmSeries { vm.pmSeries = series }
        _viewModel = StateObject(wrappedValue: vm)
    }

    var body: some View {
        Button(action: onSelect) {
            HStack(spacing: 8) {
                TickerIconView(symbol: viewModel.baseSymbol, url: iconURL)

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
                    Text(item.ticker)
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
        .task(id: viewModel.iconKey) {
            iconURL = nil
            iconURL = await IconResolver.shared.iconURL(
                ticker: viewModel.ticker,
                source: viewModel.source,
                baseSymbol: viewModel.baseSymbol
            )
        }
        .onDisappear {
            titleTooltipTask?.cancel()
        }
    }

}
