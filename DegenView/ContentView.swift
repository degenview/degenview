import SwiftUI
import UniformTypeIdentifiers

enum AppTheme: String, CaseIterable, Identifiable {
    case system = "System"
    case light  = "Light"
    case dark   = "Dark"

    var id: String { rawValue }

    var colorScheme: ColorScheme? {
        switch self {
        case .system: return nil
        case .light:  return .light
        case .dark:   return .dark
        }
    }

    var icon: String {
        switch self {
        case .system: return "circle.lefthalf.filled"
        case .light:  return "sun.max.fill"
        case .dark:   return "moon.fill"
        }
    }
}

struct ContentView: View {
    @StateObject private var contentViewModel: ContentViewModel

    @State private var showAddSheet = false
    @State private var showAddFavoriteSheet = false
    @AppStorage("showFavoritesSidebar") private var showFavorites = false
    @StateObject private var favoritesStore = FavoritesStore.shared
    @State private var showSaveAlert = false
    @State private var saveViewName = ""
    @State private var showRenameAlert = false
    @State private var renameText = ""
    @State private var showReplayDatePicker = false
    @State private var replayDate = Date()
    @AppStorage("appTheme") private var appTheme: AppTheme = .system
    @StateObject private var paperTrading = PaperTradingStore.shared
    @AppStorage("showPaperTradingOnCharts") private var showPaperTradingOnCharts = true
    @AppStorage("hasChosenPaperTradingChartVisibility") private var hasChosenPaperTradingChartVisibility = false
    @State private var showTradingPanel = false
    @State private var paperManagerTab: PaperManagerTab = .positions
    @State private var orderTicket: PaperOrderTicketContext?

    init(tabID: UUID) {
        _contentViewModel = StateObject(wrappedValue: ContentViewModel(tabID: tabID))
    }

    var body: some View {
        NavigationStack {
            HStack(spacing: 0) {
                // Drawing tools. Outside the chart column, so the card-height math
                // below measures only what's left and needs no adjustment.
                ToolSidebar(activeTool: contentViewModel.activeTool) { tool in
                    contentViewModel.toggleTool(tool)
                }

                chartColumn

                if showFavorites {
                    Divider()
                    FavoritesSidebar(
                        store: favoritesStore,
                        onAdd: { showAddFavoriteSheet = true },
                        onSelect: contentViewModel.openFavorite
                    )
                    .transition(.move(edge: .trailing).combined(with: .opacity))
                }
            }
            // The title is the tab label, and an empty tab still needs the
            // toolbar — both belong outside the empty/non-empty branch.
            .navigationTitle(contentViewModel.tabName)
            .toolbar { toolbarContent }
            .frame(
                minWidth: UI.windowMinWidth + (showFavorites ? UI.favoritesSidebarWidth : 0),
                idealWidth: UI.windowIdealWidth + (showFavorites ? UI.favoritesSidebarWidth : 0),
                minHeight: UI.windowMinHeight,
                idealHeight: UI.windowIdealHeight
            )
        }
        .background(
            WindowAccessor { window in
                contentViewModel.attach(to: window)
            }
            .frame(width: 0, height: 0)
        )
        .preferredColorScheme(appTheme.colorScheme)
        .alert("Save View", isPresented: $showSaveAlert) {
            TextField("Name", text: $saveViewName)
            Button("Save") {
                let name = saveViewName.trimmingCharacters(in: .whitespaces)
                guard !name.isEmpty else { return }
                contentViewModel.saveCurrentView(name: name)
            }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text("Save the current charts, timeframe, and layout as a named view.")
        }
        .alert("Rename Tab", isPresented: $showRenameAlert) {
            TextField("Name", text: $renameText)
            Button("Rename") { contentViewModel.renameTab(to: renameText) }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text("The tab name is also the window title.")
        }
        .sheet(isPresented: $showAddSheet) {
            AddTickerSheet(onAddPortfolio: { config in
                contentViewModel.addPortfolioChart(config)
            }) { selected in
                let displayName: String? = {
                    guard selected.source == .polymarket else { return nil }
                    if let series = selected.pmSeries, series.count > 1 {
                        return selected.eventTitle ?? selected.symbol
                    }
                    return selected.symbol
                }()
                try await contentViewModel.addTicker(
                    symbol: selected.fullSymbol,
                    source: selected.source,
                    displayName: displayName,
                    pmSeries: selected.pmSeries
                )
            }
        }
        .sheet(isPresented: $showAddFavoriteSheet) {
            AddTickerSheet(title: "Add Favorite", actionLabel: "Favorite") { selected in
                try favoritesStore.add(selected)
            }
        }
        .sheet(isPresented: $showReplayDatePicker) {
            VStack(alignment: .leading, spacing: 16) {
                Text("Select Replay Date and Time").font(.headline)
                DatePicker("Replay starts at", selection: $replayDate)
                    .datePickerStyle(.field)
                HStack {
                    Spacer()
                    Button("Cancel", role: .cancel) { showReplayDatePicker = false }
                    Button("Start Replay") {
                        contentViewModel.selectReplayDate(replayDate)
                        showReplayDatePicker = false
                    }
                    .keyboardShortcut(.defaultAction)
                }
            }
            .padding(20)
            .frame(width: 380)
        }
        .sheet(item: $orderTicket) { ticket in
            PaperOrderTicketSheet(store: paperTrading, instrument: ticket.instrument,
                                  referencePrice: ticket.price, initialSide: ticket.side)
        }
        .alert("Paper Trading", isPresented: Binding(get: { paperTrading.lastError != nil && orderTicket == nil }, set: { if !$0 { paperTrading.clearError() } })) {
            Button("OK") { paperTrading.clearError() }
        } message: { Text(paperTrading.lastError ?? "") }
        .onChange(of: showAddSheet) { _, _ in
            contentViewModel.isShowingSheet = showAddSheet || showAddFavoriteSheet
        }
        .onChange(of: showAddFavoriteSheet) { _, _ in
            contentViewModel.isShowingSheet = showAddSheet || showAddFavoriteSheet
        }
    }

    // MARK: - Chart Column

    /// Everything right of the tool strip.
    @ViewBuilder
    private var chartColumn: some View {
        if showTradingPanel && paperTrading.isConnected {
            VSplitView {
                chartsOnly
                    .frame(minHeight: 260)
                PaperAccountManagerView(store: paperTrading, selectedTab: $paperManagerTab) {
                    showTradingPanel = false
                }
                .frame(minHeight: 180, idealHeight: 280)
            }
        } else {
            chartsOnly
        }
    }

    @ViewBuilder
    private var chartsOnly: some View {
        VStack(spacing: 0) {
            if contentViewModel.replay.isActive {
                ReplayControlBar(
                    engine: contentViewModel.replay,
                    onChangeStart: contentViewModel.beginReplaySelection,
                    onReturnToLive: contentViewModel.returnToLive,
                    onClose: contentViewModel.returnToLive,
                    availableIntervals: contentViewModel.availableReplayIntervals,
                    onIntervalChanged: contentViewModel.setReplayInterval,
                    isPreparing: contentViewModel.isPreparingReplay
                )
            }
            if let notice = contentViewModel.replayNotice, contentViewModel.replay.isActive {
                Text(notice)
                    .font(.caption2)
                    .foregroundStyle(.secondary)
                    .padding(.horizontal, 10)
                    .padding(.vertical, 3)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background(Color.orange.opacity(0.08))
            }
            // Global loading indicator
            if contentViewModel.isRefreshing {
                ProgressView()
                    .progressViewStyle(.linear)
                    .scaleEffect(x: 1, y: 0.5)
                    .padding(.horizontal, 16)
            }

            if contentViewModel.chartViewModels.isEmpty {
                EmptyStateView(
                    savedViews: contentViewModel.savedViews,
                    onAddTapped: { showAddSheet = true },
                    onOpenView: { contentViewModel.loadView($0) }
                )
            } else {
                VStack(spacing: 0) {
                    // Chart list (vertical) or grid — fills remaining height, scrolls if needed
                    GeometryReader { geometry in
                        let n = max(1, contentViewModel.chartViewModels.count)
                        let available = geometry.size.height

                        let naturalHeight: CGFloat = {
                            if contentViewModel.layoutMode == .vertical {
                                let gaps = CGFloat(max(0, n - 1)) * ChartLayout.cardGap
                                let chrome = CGFloat(n) * ChartLayout.cardChrome
                                return (available - gaps - chrome) / CGFloat(n)
                            } else {
                                let rows = max(1, Int(ceil(Double(n) / ChartLayout.gridColumnFraction)))
                                let gaps = CGFloat(max(0, rows - 1)) * ChartLayout.cardGap
                                let chrome = CGFloat(rows) * ChartLayout.cardChrome
                                return (available - gaps - chrome) / CGFloat(rows)
                            }
                        }()

                        // Charts fill available window height. Floor at chartMinHeight
                        // (the canvas won't render below 50 pt anyway).
                        let chartHeight = max(ChartLayout.chartMinHeight, naturalHeight)

                        if contentViewModel.layoutMode == .vertical {
                            ScrollView {
                                VStack(spacing: ChartLayout.cardGap) {
                                    ForEach(contentViewModel.chartViewModels, id: \.uniqueID) { vm in
                                        chartCard(vm, height: chartHeight)
                                            .onDrag {
                                                NSItemProvider(object: vm.uniqueID as NSString)
                                            }
                                            .onDrop(
                                                of: [.utf8PlainText],
                                                delegate: ReorderDropDelegate(
                                                    targetTicker: vm.uniqueID,
                                                    viewModel: contentViewModel
                                                )
                                            )
                                    }
                                }
                                .padding(4)
                            }
                            .frame(height: available)
                            .scrollIndicators(.never)
                        } else {
                            ScrollView {
                                LazyVGrid(
                                    columns: [GridItem(.flexible(), spacing: 0), GridItem(.flexible(), spacing: 0)],
                                    spacing: 0
                                ) {
                                    ForEach(contentViewModel.chartViewModels, id: \.uniqueID) { vm in
                                        chartCard(vm, height: chartHeight)
                                            .padding(4)
                                            .onDrag {
                                                NSItemProvider(object: vm.uniqueID as NSString)
                                            }
                                            .onDrop(
                                                of: [.utf8PlainText],
                                                delegate: ReorderDropDelegate(
                                                    targetTicker: vm.uniqueID,
                                                    viewModel: contentViewModel
                                                )
                                            )
                                    }
                                }
                                .padding(4)
                            }
                            .frame(height: available)
                            .scrollIndicators(.never)
                        }
                    }
                }
            }
        }
    }

    // MARK: - Toolbar

    /// Commit the tab's edits back to its saved view, prompting for a name the
    /// first time — an unnamed tab has no view to write to yet.
    private func saveChanges() {
        if contentViewModel.tabName == UI.unnamedView {
            saveViewName = ""
            showSaveAlert = true
        } else {
            contentViewModel.saveChanges()
        }
    }

    @ToolbarContentBuilder
    private var toolbarContent: some ToolbarContent {
        ToolbarItem(placement: .automatic) {
            Button {
                WindowCoordinator.shared.openPortfolio(beside: contentViewModel.tabID)
            } label: {
                Label("Portfolio", systemImage: "briefcase")
            }
            .help("Portfolio Tracker")
        }
        ToolbarItem(placement: .automatic) {
            Menu {
                if paperTrading.isConnected {
                    Button(showTradingPanel ? "Hide Account Manager" : "Open Account Manager") {
                        if showTradingPanel {
                            showTradingPanel = false
                        } else {
                            showTradingPanelWithChartControlsDefault()
                        }
                    }
                    Toggle("Show Trading on Charts", isOn: Binding(
                        get: { showPaperTradingOnCharts },
                        set: { value in
                            showPaperTradingOnCharts = value
                            hasChosenPaperTradingChartVisibility = true
                        }
                    ))
                    if let vm = contentViewModel.marketChartViewModels.first {
                        Divider()
                        Button("Buy \(vm.title)…") { openTicket(for: vm, side: .buy) }
                            .keyboardShortcut("b", modifiers: [.command, .shift])
                        Button("Sell \(vm.title)…") { openTicket(for: vm, side: .sell) }
                            .keyboardShortcut("s", modifiers: [.command, .shift])
                    }
                } else {
                    Button("Connect Paper Trading") {
                        Task {
                            await paperTrading.connect()
                            showTradingPanelWithChartControlsDefault()
                        }
                    }
                }
            } label: {
                Label(paperTrading.isConnected ? "PAPER" : "Trade", systemImage: "arrow.left.arrow.right")
            }
            .accessibilityLabel(paperTrading.isConnected ? "Paper Trading connected" : "Trade")
            .help("Paper Trading")
        }
        ToolbarItem(placement: .automatic) {
            Menu {
                Button("Select bar") { contentViewModel.beginReplaySelection() }
                Button("Select date/time…") {
                    replayDate = contentViewModel.replay.currentTimestamp ?? Date()
                    showReplayDatePicker = true
                }
                Button("Random bar") { contentViewModel.selectRandomReplayBar() }
                Button("First available bar") { contentViewModel.selectFirstReplayBar() }
                if contentViewModel.replay.isActive {
                    Divider()
                    Button("Return to Latest") { contentViewModel.returnToLive() }
                }
            } label: {
                Label("Replay", systemImage: "clock.arrow.circlepath")
            }
            .accessibilityLabel("Historical bar replay")
            .help("Start or control historical replay")
        }
        ToolbarItem(placement: .automatic) {
            Picker("Timeframe", selection: $contentViewModel.selectedTimeRange) {
                ForEach(TimeRange.allCases) { range in
                    Text(range.rawValue).tag(range)
                }
            }
            .pickerStyle(.segmented)
            .labelsHidden()
            .onChange(of: contentViewModel.selectedTimeRange) { _, newValue in
                contentViewModel.setTimeRange(newValue)
            }
        }
        ToolbarItem(placement: .automatic) {
            Button {
                contentViewModel.layoutMode = contentViewModel.layoutMode.next
            } label: {
                Image(systemName: contentViewModel.layoutMode.icon)
            }
            .accessibilityLabel(contentViewModel.layoutMode == .vertical
                ? "Grid layout" : "Vertical layout")
        }
        ToolbarItem(placement: .automatic) {
            Menu {
                // The name bar used to be the rename affordance; with it gone
                // this menu is the only place left to reach it.
                Button("Rename Tab…") {
                    renameText = contentViewModel.tabName == UI.unnamedView
                        ? "" : contentViewModel.tabName
                    showRenameAlert = true
                }

                if !contentViewModel.savedViews.isEmpty {
                    Divider()
                    ForEach(contentViewModel.savedViews) { view in
                        Button(view.name) {
                            contentViewModel.loadView(view)
                        }
                    }
                    Divider()
                    Menu("Delete…") {
                        ForEach(contentViewModel.savedViews) { view in
                            Button(role: .destructive) {
                                contentViewModel.deleteView(view)
                            } label: {
                                Text(view.name)
                            }
                        }
                    }
                }
            } label: {
                Image(systemName: "folder")
            }
            .accessibilityLabel("Load View")
        }
        // Only meaningful once the tab has drifted from what's on disk.
        if contentViewModel.hasUnsavedChanges {
            ToolbarItem(placement: .automatic) {
                Button(action: saveChanges) {
                    Image(systemName: "square.and.arrow.down.on.square")
                }
                .accessibilityLabel("Save Changes")
                .help("Save changes to \"\(contentViewModel.tabName)\"")
            }
        }
        ToolbarItem(placement: .automatic) {
            Button {
                saveViewName = contentViewModel.tabName == UI.unnamedView
                    ? "" : contentViewModel.tabName
                showSaveAlert = true
            } label: {
                Image(systemName: "square.and.arrow.down")
            }
            .accessibilityLabel("Save View")
        }
        ToolbarItem(placement: .primaryAction) {
            Button {
                withAnimation { showFavorites.toggle() }
            } label: {
                Image(systemName: showFavorites ? "sidebar.right" : "sidebar.right")
            }
            .accessibilityLabel(showFavorites ? "Hide Favorites" : "Show Favorites")
            .help(showFavorites ? "Hide Favorites" : "Show Favorites")
        }
        ToolbarItem(placement: .primaryAction) {
            Button {
                showAddSheet = true
            } label: {
                Image(systemName: "plus")
            }
            .accessibilityLabel("Add Ticker")
        }
    }

    // MARK: - Chart Card Builder

    private func chartCard(_ vm: ChartViewModel, height: CGFloat) -> some View {
        ChartCardView(
            viewModel: vm,
            chartHeight: height,
            onRemove: {
                withAnimation {
                    contentViewModel.removeTicker(vm)
                }
            },
            onRetry: {
                Task {
                    await vm.fetchData(
                        for: contentViewModel.selectedTimeRange,
                        count: contentViewModel.candleCount
                    )
                }
            },
            isFavorite: favoritesStore.contains(source: vm.source, symbol: vm.ticker),
            onToggleFavorite: {
                favoritesStore.toggle(
                    config: TickerConfig(
                        symbol: vm.ticker,
                        source: vm.source,
                        displayName: vm.displayName,
                        pmSeries: vm.pmSeries.isEmpty ? nil : vm.pmSeries
                    ),
                    name: vm.title,
                    ticker: favoriteTicker(for: vm)
                )
            },
            onZoomRegion: { contentViewModel.registerZoomRegion($0) },
            onAxisRegion: { contentViewModel.registerAxisRegion($0, for: vm) },
            onPlotRegion: { contentViewModel.registerPlotRegion($0, for: vm) },
            isToolArmed: contentViewModel.activeTool != .none,
            showTrendHandles: contentViewModel.activeTool == .trendLine,
            crosshair: contentViewModel.crosshair,
            onCrosshairExit: { contentViewModel.crosshair.clear(owner: vm.uniqueID) },
            onUpdateTicker: { symbol, source, displayName, pmSeries in
                contentViewModel.updateTicker(vm, symbol: symbol, source: source, displayName: displayName, pmSeries: pmSeries)
            },
            onStyleChanged: {
                contentViewModel.persistChartSettings()
            },
            onSettingsPresented: { shown in
                contentViewModel.isShowingSheet = shown
            },
            onPaperBuy: { openTicket(for: vm, side: .buy) },
            onPaperSell: { openTicket(for: vm, side: .sell) },
            paperConnected: paperTrading.isConnected && showPaperTradingOnCharts,
            paperPositions: paperTrading.positions.filter { $0.instrument.key == "\(vm.source.rawValue):\(vm.apiSymbol)" },
            paperOrders: paperTrading.workingOrders.filter { $0.instrument.key == "\(vm.source.rawValue):\(vm.apiSymbol)" },
            paperAccountCurrency: paperTrading.selectedAccount?.baseCurrency ?? .USD,
            paperUnrealizedPnL: { position in
                guard let quote = paperTrading.snapshot.quotes[position.instrument.key],
                      let mark = position.signedQuantity >= 0 ? (quote.bid ?? quote.last) : (quote.ask ?? quote.last)
                else { return 0 }
                return (position.signedQuantity >= 0 ? mark - position.averageEntryPrice : position.averageEntryPrice - mark) * position.quantity * position.instrument.pointValue
            },
            onPaperModify: { order, price in
                let changes = order.type == .stop || (order.type == .stopLimit && !order.stopTriggered)
                    ? PaperOrderChanges(stopPrice: price) : PaperOrderChanges(limitPrice: price)
                Task { await paperTrading.modify(order.id, changes: changes) }
            },
            onPaperCancel: { order in Task { await paperTrading.cancel(order.id) } },
            onPaperClose: { position in Task { await paperTrading.close(position) } }
        )
        .onChange(of: vm.currentPrice) { _, price in
            guard let price else { return }
            let instrument = PaperInstrument.chart(symbol: vm.apiSymbol, displayName: vm.title, source: vm.source)
            Task { await paperTrading.process(instrument: instrument, last: Decimal(price), timestamp: Date()) }
        }
    }

    private func favoriteTicker(for vm: ChartViewModel) -> String {
        if vm.source == .polymarket {
            if vm.pmSeries.count > 1,
               let outcome = vm.pmSeries.first(where: {
                   $0.tokenID.caseInsensitiveCompare(vm.ticker) == .orderedSame
               }) {
                return outcome.label
            }
            return vm.title
        }

        if vm.source == .dexscreener {
            return vm.baseSymbol
        }
        return vm.ticker.uppercased()
    }

    private func openTicket(for vm: ChartViewModel, side: PaperOrderSide) {
        let instrument = PaperInstrument.chart(symbol: vm.apiSymbol, displayName: vm.title, source: vm.source)
        orderTicket = .init(instrument: instrument, price: vm.displayedPrice.map { Decimal($0) }, side: side)
    }

    /// Existing installs may carry a hidden value from the retired eye button. Treat
    /// visibility as default-on until the user makes a choice in the PAPER menu.
    private func showTradingPanelWithChartControlsDefault() {
        if !hasChosenPaperTradingChartVisibility {
            showPaperTradingOnCharts = true
        }
        showTradingPanel = true
    }
}

private struct PaperOrderTicketContext: Identifiable {
    let id = UUID()
    let instrument: PaperInstrument
    let price: Decimal?
    let side: PaperOrderSide
}

#Preview {
    ContentView(tabID: UUID())
}
