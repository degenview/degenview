import SwiftUI

/// Title row: list selector, add symbol, filter, and the options menu.
struct WatchlistHeader: View {
    @ObservedObject var store: WatchlistStore
    @ObservedObject var viewModel: WatchlistSidebarViewModel
    let onAddSymbol: () -> Void
    let onPrompt: (WatchlistPrompt) -> Void
    let onDelete: () -> Void
    let onImport: () -> Void
    let onExport: () -> Void

    var body: some View {
        HStack(spacing: 8) {
            selector
            Spacer(minLength: 4)

            Button(action: onAddSymbol) { Image(systemName: "plus") }
                .help("Add Symbol")
                .accessibilityLabel("Add Symbol")
                .disabled(store.loadFailed || viewModel.list == nil)

            Button {
                withAnimation { viewModel.isFilterVisible.toggle() }
                if !viewModel.isFilterVisible { viewModel.clearFilter() }
            } label: {
                Image(systemName: "magnifyingglass")
                    .foregroundStyle(viewModel.isFilterVisible || viewModel.isFiltering ? Color.accentColor : .primary)
            }
            .help("Filter this watchlist")
            .accessibilityLabel("Filter")

            optionsMenu
        }
        .buttonStyle(.plain)
        .padding(.horizontal, 12)
        .padding(.vertical, 10)
    }

    private var selector: some View {
        Menu {
            ForEach(store.lists) { list in
                Toggle(
                    list.name,
                    isOn: Binding(
                        get: { viewModel.list?.id == list.id },
                        set: { _ in viewModel.select(list.id) }))
            }
            Divider()
            Button("Create New Watchlist…") { onPrompt(.newList) }
            Button("Rename Watchlist…") { onPrompt(.renameList(current: viewModel.list?.name ?? "")) }
                .disabled(viewModel.list == nil)
            Button("Make a Copy…") { onPrompt(.copyList(suggested: "\(viewModel.list?.name ?? "") copy")) }
                .disabled(viewModel.list == nil)
            Button("Delete Watchlist…", role: .destructive, action: onDelete)
                .disabled(viewModel.list == nil || viewModel.list?.isFavorites == true)
        } label: {
            HStack(spacing: 4) {
                Text(viewModel.list?.name ?? "Watchlists")
                    .font(.headline)
                    .lineLimit(1)
                Image(systemName: "chevron.down")
                    .font(.caption2.weight(.bold))
                    .foregroundStyle(.secondary)
            }
        }
        .menuStyle(.borderlessButton)
        .menuIndicator(.hidden)
        .fixedSize()
        .disabled(store.loadFailed)
        .accessibilityLabel("Watchlist: \(viewModel.list?.name ?? "none")")
    }

    private var optionsMenu: some View {
        Menu {
            Button("Add Section…") { onPrompt(.newSection) }

            Menu("Sort By") {
                ForEach(WatchlistSortKey.allCases) { key in
                    Toggle(
                        key.title,
                        isOn: Binding(
                            get: { viewModel.list?.display.sort.key == key },
                            set: { _ in
                                viewModel.setSort(
                                    key == .manual ? .manual : WatchlistSort(key: key, ascending: key == .symbol))
                            }))
                }
                Divider()
                Toggle(
                    "Descending",
                    isOn: Binding(
                        get: { !(viewModel.list?.display.sort.ascending ?? true) },
                        set: { descending in
                            guard let sort = viewModel.list?.display.sort, sort.key != .manual else { return }
                            viewModel.setSort(WatchlistSort(key: sort.key, ascending: !descending))
                        }))
                    .disabled(viewModel.list?.display.sort.key == .manual)
            }

            Menu("Columns") {
                ForEach(WatchlistColumn.allCases) { column in
                    Toggle(
                        column.accessibilityTitle,
                        isOn: Binding(
                            get: { viewModel.list?.display.columns.contains(column) ?? false },
                            set: { viewModel.setColumn(column, visible: $0) }))
                }
                Divider()
                Toggle(
                    "Show Symbol Subtitle",
                    isOn: Binding(
                        get: { viewModel.list?.display.showsDescription ?? true },
                        set: { viewModel.setShowsDescription($0) }))
            }

            Menu("Filter by Flag") {
                ForEach(WatchlistFlag.allCases) { flag in
                    Toggle(
                        flag.title,
                        isOn: Binding(
                            get: { viewModel.flagFilter == flag },
                            set: { viewModel.flagFilter = $0 ? flag : nil }))
                }
                Divider()
                Button("Show All") { viewModel.flagFilter = nil }.disabled(viewModel.flagFilter == nil)
            }

            Toggle("Show Details", isOn: $viewModel.showsDetail)

            Divider()

            Button("Import Symbols…", action: onImport)
            Button("Export to File…", action: onExport)
            Button("Copy as Text") { viewModel.exportText.map(WatchlistFileIO.copy) }
        } label: {
            Image(systemName: "ellipsis")
        }
        .menuStyle(.borderlessButton)
        .menuIndicator(.hidden)
        .fixedSize()
        .disabled(store.loadFailed || viewModel.list == nil)
        .accessibilityLabel("Watchlist options")
    }
}
