import SwiftUI

/// The app-wide watchlist panel. Contents come from the shared `WatchlistStore`; which list is
/// shown, the filter and the highlighted row belong to this window's `WatchlistSidebarViewModel`.
struct WatchlistSidebar: View {
    @ObservedObject var store: WatchlistStore
    @ObservedObject var viewModel: WatchlistSidebarViewModel
    /// Observed so a value-sorted list reorders (at most once a second) as quotes arrive.
    @ObservedObject var quotes: WatchlistQuoteBook
    let actions: WatchlistInstrumentActions
    /// Prices are only fetched while the sidebar is open and the window can be seen.
    let isWindowVisible: Bool
    /// The market the tab's focused chart shows; the matching row is highlighted.
    let focusedMarket: InstrumentID?
    let onAddSymbol: () -> Void

    @State private var prompt: WatchlistPrompt?
    @State private var promptText = ""
    @State private var showsImport = false
    @State private var confirmsDelete = false
    @State private var alertAsset: PortfolioAsset?
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    init(
        viewModel: WatchlistSidebarViewModel, actions: WatchlistInstrumentActions, isWindowVisible: Bool,
        focusedMarket: InstrumentID?, onAddSymbol: @escaping () -> Void
    ) {
        self.store = viewModel.store
        self.viewModel = viewModel
        self.quotes = viewModel.quotes
        self.actions = actions
        self.isWindowVisible = isWindowVisible
        self.focusedMarket = focusedMarket
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
        // The container sizes it; the divider on its leading edge resizes it.
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(.bar)
        .onAppear { viewModel.syncQuotes(isWindowVisible: isWindowVisible) }
        .onDisappear { viewModel.stopQuotes() }
        .onChange(of: isWindowVisible) { viewModel.syncQuotes(isWindowVisible: isWindowVisible) }
        .onChange(of: viewModel.quoteInstruments.map(\.key)) {
            viewModel.syncQuotes(isWindowVisible: isWindowVisible)
        }
        // Selecting a symbol opens it; the highlighted row follows the focused chart's market.
        .onChange(of: viewModel.selectedEntryID) {
            if let item = viewModel.selectedInstrument { actions.open(item) }
        }
        .onChange(of: focusedMarket, initial: true) { viewModel.syncSelection(to: focusedMarket) }
        .onChange(of: viewModel.list?.id) { viewModel.syncSelection(to: focusedMarket) }
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
                rowView(row, list: list, isFirst: row.id == rows.first?.id)
                    .listRowInsets(
                        EdgeInsets(
                            top: 1, leading: leadingInset(for: row), bottom: 1,
                            trailing: WatchlistMetrics.edgeInset)
                    )
                    .listRowSeparator(.hidden)
            }
            // The list's own row move: native lift, insertion line and auto-scroll, one write on drop.
            // Off while the list is sorted or filtered, since a drop could not mean a stored position.
            .onMove(perform: viewModel.canReorder ? { viewModel.moveRows(fromOffsets: $0, toOffset: $1) } : nil)
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
    private func rowView(_ row: WatchlistLayoutEngine.Row, list: Watchlist, isFirst: Bool) -> some View {
        switch row {
        case .section(let section, let count):
            WatchlistSectionRow(section: section, count: count, isFirst: isFirst, onToggle: { toggle(section) })
                // A heading folds on click; it is never "selected", so it never paints the row blue.
                .selectionDisabled()
                .contextMenu {
                    Button("Rename Section…") { present(.renameSection(id: section.id, current: section.title)) }
                    Button("Add Section…") { present(.newSection) }
                    Divider()
                    Button("Delete Section", role: .destructive) { viewModel.deleteSection(section) }
                }

        case .instrument(let item, _):
            WatchlistInstrumentRow(
                item: item, cell: quotes.cell(for: item.instrument), flag: store.flag(for: item.instrument),
                display: list.display, menu: { menu(for: item) }
            )
            .tag(item.id)
            .contextMenu { menu(for: item) }
            .accessibilityAction(named: "Add as New Chart") { actions.addChart(item) }
            .accessibilityAction(named: "Remove from Watchlist") { viewModel.remove(item) }

        case .emptySection:
            WatchlistEmptySectionRow()
                .selectionDisabled()
        }
    }

    /// Symbol rows take no leading inset from the list and add the standard one themselves, so a flag can be drawn
    /// in that gutter inside the row. Headings and placeholders use the list's inset; every visible edge is the same.
    private func leadingInset(for row: WatchlistLayoutEngine.Row) -> CGFloat {
        if case .instrument = row { return 0 }
        return WatchlistMetrics.edgeInset
    }

    private func toggle(_ section: WatchlistSection) {
        if reduceMotion {
            viewModel.toggleCollapsed(section)
        } else {
            withAnimation(.easeInOut(duration: 0.15)) { viewModel.toggleCollapsed(section) }
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
                WatchlistFlagMark(flag: flag)
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
        .padding(.horizontal, WatchlistMetrics.edgeInset)
        .padding(.bottom, 8)
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
