import SwiftUI

/// The app-wide watchlist panel. Contents come from the shared `WatchlistStore`; which list is
/// shown, the filter and the highlighted row belong to this window's `WatchlistSidebarViewModel`.
struct WatchlistSidebar: View {
    @ObservedObject var store: WatchlistStore
    @ObservedObject var viewModel: WatchlistSidebarViewModel
    /// Observed so a value-sorted list reorders (at most once a second) as quotes arrive.
    @ObservedObject var quotes: WatchlistQuoteBook
    let actions: WatchlistInstrumentActions
    let onAddSymbol: () -> Void

    @State private var prompt: WatchlistPrompt?
    @State private var promptText = ""
    @State private var showsImport = false
    @State private var confirmsDelete = false
    @State private var alertAsset: PortfolioAsset?
    @State private var draggedID: UUID?
    @State private var hoverKey: String?

    init(
        viewModel: WatchlistSidebarViewModel, actions: WatchlistInstrumentActions,
        onAddSymbol: @escaping () -> Void
    ) {
        self.store = viewModel.store
        self.viewModel = viewModel
        self.quotes = viewModel.quotes
        self.actions = actions
        self.onAddSymbol = onAddSymbol
    }

    var body: some View {
        VStack(spacing: 0) {
            WatchlistHeader(
                store: store, viewModel: viewModel, onAddSymbol: onAddSymbol,
                onPrompt: present, onDelete: { confirmsDelete = true },
                onImport: { showsImport = true }, onExport: export)

            if viewModel.isFilterVisible { filterField }

            Divider()

            content

            if viewModel.showsDetail, let item = viewModel.selectedInstrument {
                Divider()
                WatchlistDetailPanel(
                    item: item, cell: quotes.cell(for: item.instrument),
                    flag: store.flag(for: item.instrument),
                    listNames: store.lists(containing: item.instrument).map(\.name))
            }
        }
        .frame(width: viewModel.sidebarWidth)
        .background(.bar)
        .animation(.easeInOut(duration: 0.15), value: viewModel.sidebarWidth)
        .alert(
            prompt?.title ?? "", isPresented: Binding(get: { prompt != nil }, set: { if !$0 { prompt = nil } }),
            presenting: prompt
        ) { current in
            TextField("Name", text: $promptText)
            Button(current.confirmTitle) { commit(current) }
            Button("Cancel", role: .cancel) {}
        } message: { current in
            Text(current.message)
        }
        .alert(
            "Delete “\(viewModel.list?.name ?? "")”?", isPresented: $confirmsDelete
        ) {
            Button("Delete", role: .destructive) { viewModel.deleteCurrent() }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text("Its symbols stay in your other watchlists. Charts, drawings and alerts are not affected.")
        }
        .alert(
            "Watchlist", isPresented: Binding(get: { viewModel.errorMessage != nil }, set: { if !$0 { viewModel.errorMessage = nil } })
        ) {
            Button("OK", role: .cancel) {}
        } message: {
            Text(viewModel.errorMessage ?? "")
        }
        .sheet(isPresented: $showsImport) {
            WatchlistImportSheet(listName: viewModel.list?.name ?? "watchlist", onImport: viewModel.importText)
        }
        .sheet(item: $alertAsset) { asset in PriceAlertEditor(asset: asset) }
    }

    // MARK: Content

    @ViewBuilder
    private var content: some View {
        if store.loadFailed {
            ContentUnavailableView(
                "Watchlists Unavailable", systemImage: "exclamationmark.triangle",
                description: Text(WatchlistError.persistenceUnavailable.localizedDescription)
            )
            .frame(maxHeight: .infinity)
        } else if let list = viewModel.list {
            let rows = viewModel.rows
            if rows.isEmpty {
                emptyState(for: list)
            } else {
                WatchlistColumnHeader(display: list.display, onSort: viewModel.toggleSort)
                rowList(rows, list: list)
            }
        } else {
            ContentUnavailableView(
                "No Watchlists", systemImage: "list.bullet.rectangle",
                description: Text("Create a watchlist from the menu above.")
            )
            .frame(maxHeight: .infinity)
        }
    }

    @ViewBuilder
    private func emptyState(for list: Watchlist) -> some View {
        if viewModel.isFiltering {
            ContentUnavailableView {
                Label("No Matches", systemImage: "magnifyingglass")
            } description: {
                Text("Nothing in \(list.name) matches this filter.")
            } actions: {
                Button("Clear Filter") { viewModel.clearFilter() }
            }
            .frame(maxHeight: .infinity)
        } else {
            ContentUnavailableView {
                Label("No Symbols", systemImage: "star")
            } description: {
                Text("Add a stock, crypto, or prediction-market item to follow it here.")
            } actions: {
                Button("Add Symbol", action: onAddSymbol)
            }
            .frame(maxHeight: .infinity)
        }
    }

    private func rowList(_ rows: [WatchlistLayoutEngine.Row], list: Watchlist) -> some View {
        List(selection: $viewModel.selectedEntryID) {
            ForEach(rows) { row in
                rowView(row, list: list)
                    .tag(row.entryID)
                    .listRowInsets(EdgeInsets(top: 1, leading: 12, bottom: 1, trailing: 12))
                    .listRowSeparator(.hidden)
            }

            if viewModel.canReorder {
                Color.clear
                    .frame(height: 28)
                    .overlay(alignment: .top) { insertionLine(visible: hoverKey == "end") }
                    .onDrop(of: [WatchlistDragPayload.type], delegate: dropDelegate(.end))
                    .listRowSeparator(.hidden)
                    .accessibilityHidden(true)
            }
        }
        .listStyle(.plain)
        .scrollContentBackground(.hidden)
        .onKeyPress(.return) {
            guard let item = viewModel.selectedInstrument else { return .ignored }
            actions.open(item)
            return .handled
        }
        .onDeleteCommand {
            if let item = viewModel.selectedInstrument { viewModel.remove(item) }
        }
    }

    @ViewBuilder
    private func rowView(_ row: WatchlistLayoutEngine.Row, list: Watchlist) -> some View {
        switch row {
        case .section(let section, let count):
            WatchlistSectionRow(
                section: section, count: count, isDropTarget: hoverKey == section.id.uuidString,
                onToggle: { withAnimation(.easeInOut(duration: 0.15)) { viewModel.toggleCollapsed(section) } }
            )
            .modifier(draggable(section.id))
            .onDrop(of: [WatchlistDragPayload.type], delegate: dropDelegate(.sectionHeader(section)))
            .contextMenu {
                Button("Rename Section…") { present(.renameSection(id: section.id, current: section.title)) }
                Button("Add Section…") { present(.newSection) }
                Divider()
                Button("Delete Section", role: .destructive) { viewModel.deleteSection(section) }
            }

        case .instrument(let item, let sectionID):
            WatchlistInstrumentRow(
                item: item, cell: quotes.cell(for: item.instrument), flag: store.flag(for: item.instrument),
                display: list.display, isIndented: sectionID != nil,
                menu: { menu(for: item) }
            )
            .overlay(alignment: .top) { insertionLine(visible: hoverKey == item.id.uuidString) }
            .onTapGesture {
                viewModel.selectedEntryID = item.id
                actions.open(item)
            }
            .modifier(draggable(item.id))
            .onDrop(of: [WatchlistDragPayload.type], delegate: dropDelegate(.row(item.id)))
            .contextMenu { menu(for: item) }
            .accessibilityAction(named: "Add as New Chart") { actions.addChart(item) }
            .accessibilityAction(named: "Remove from Watchlist") { viewModel.remove(item) }

        case .emptySection:
            Text("Empty section")
                .font(.caption)
                .foregroundStyle(.tertiary)
                .padding(.leading, 28)
                .padding(.vertical, 2)
                .accessibilityHidden(true)
        }
    }

    private func menu(for item: WatchlistInstrument) -> some View {
        WatchlistInstrumentMenu(
            store: store, viewModel: viewModel, item: item, actions: actions,
            onNewWatchlist: { present(.newListFor($0)) }, onCreateAlert: { alertAsset = $0.alertAsset })
    }

    // MARK: Pieces

    private var filterField: some View {
        HStack(spacing: 6) {
            Image(systemName: "magnifyingglass").foregroundStyle(.secondary)
            TextField("Filter symbols", text: $viewModel.filterText)
                .textFieldStyle(.plain)
                .onExitCommand { viewModel.clearFilter() }
            if let flag = viewModel.flagFilter {
                Label(flag.title, systemImage: "flag.fill")
                    .labelStyle(.iconOnly)
                    .foregroundStyle(flag.color)
                    .help("Showing only \(flag.title.lowercased())-flagged symbols")
            }
            if viewModel.isFiltering {
                Button {
                    viewModel.filterText = ""
                    viewModel.flagFilter = nil
                } label: {
                    Image(systemName: "xmark.circle.fill").foregroundStyle(.secondary)
                }
                .buttonStyle(.plain)
                .accessibilityLabel("Clear filter")
            }
        }
        .padding(.horizontal, 12)
        .padding(.bottom, 8)
    }

    private func insertionLine(visible: Bool) -> some View {
        Rectangle()
            .fill(Color.accentColor)
            .frame(height: 2)
            .opacity(visible ? 1 : 0)
            .allowsHitTesting(false)
    }

    private func dropDelegate(_ target: WatchlistDropDelegate.Target) -> WatchlistDropDelegate {
        WatchlistDropDelegate(target: target, viewModel: viewModel, draggedID: $draggedID, hoverKey: $hoverKey)
    }

    /// Dragging is offered only while the list is shown whole and in its stored order.
    private func draggable(_ id: UUID) -> some ViewModifier {
        WatchlistDraggable(enabled: viewModel.canReorder) {
            draggedID = id
            return WatchlistDragPayload.provider(for: id)
        }
    }

    // MARK: Actions

    private func present(_ next: WatchlistPrompt) {
        promptText = next.initialText
        prompt = next
    }

    private func commit(_ current: WatchlistPrompt) {
        let text = promptText
        switch current {
        case .newList: viewModel.createWatchlist(named: text)
        case .renameList: viewModel.renameCurrent(to: text)
        case .copyList: viewModel.duplicateCurrent(named: text)
        case .newSection: viewModel.addSection(named: text)
        case .renameSection(let id, _):
            if let list = viewModel.list { viewModel.perform { try store.renameSection(id, to: text, in: list.id) } }
        case .newListFor(let item):
            viewModel.perform {
                let created = try store.createWatchlist(name: text)
                try store.addInstrument(item, to: created.id)
            }
        }
    }

    private func export() {
        guard let text = viewModel.exportText, let name = viewModel.list?.name else { return }
        if let failure = WatchlistFileIO.save(text, suggestedName: name) { viewModel.errorMessage = failure }
    }
}

/// Attaches the drag only when reordering is allowed, so a sorted or filtered row does not
/// pick up a payload that could not be dropped anywhere meaningful.
private struct WatchlistDraggable: ViewModifier {
    let enabled: Bool
    let provider: () -> NSItemProvider

    func body(content: Content) -> some View {
        if enabled {
            content.onDrag(provider)
        } else {
            content
        }
    }
}
