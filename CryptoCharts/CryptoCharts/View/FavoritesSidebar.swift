import SwiftUI

struct FavoritesSidebar: View {
    @ObservedObject var store: FavoritesStore
    let onAdd: () -> Void
    let onSelect: (FavoriteItem) -> Void

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
                            .contextMenu {
                                Button("Delete", role: .destructive) { store.remove(item) }
                            }
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

private struct FavoriteRow: View {
    let item: FavoriteItem
    let onSelect: () -> Void
    @StateObject private var viewModel: ChartViewModel
    @State private var iconURL: URL?

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
                    Text(item.ticker)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                }

                Spacer(minLength: 4)

                if let change = viewModel.priceChangePercent {
                    Text(String(format: "%+.2f%%", change))
                        .foregroundStyle(change >= 0 ? Color.green : Color.red)
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
    }

}
