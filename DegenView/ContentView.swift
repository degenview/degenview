import SwiftUI
import UniformTypeIdentifiers

enum AppTheme: String, CaseIterable, Identifiable {
    case system = "System"
    case light = "Light"
    case dark = "Dark"

    var id: String { rawValue }

    var colorScheme: ColorScheme? {
        switch self {
        case .system: return nil
        case .light: return .light
        case .dark: return .dark
        }
    }

    var icon: String {
        switch self {
        case .system: return "circle.lefthalf.filled"
        case .light: return "sun.max.fill"
        case .dark: return "moon.fill"
        }
    }
}

struct ContentView: View {
    @StateObject private var contentViewModel: ContentViewModel

    @State private var showAddSheet = false
    @State private var showAddFavoriteSheet = false
    @AppStorage("showFavoritesSidebar") private var showFavorites = false
    @StateObject private var watchlists = WatchlistStore.shared
    @ObservedObject private var recents = RecentMarketsStore.shared
    @State private var showLayoutPicker = false
    @State private var layoutPromptName = ""
    @State private var showReplayDatePicker = false
    @AppStorage("appTheme") private var appTheme: AppTheme = .system
    @StateObject private var paperTrading = PaperTradingStore.shared
    @AppStorage("showPaperTradingOnCharts") private var showPaperTradingOnCharts = true
    @State private var showTradingPanel = false
    @State private var paperManagerTab: PaperManagerTab = .positions
    @State private var orderTicket: PaperOrderTicketContext?
    @State private var draggedChartID: UUID?
    @State private var gridDropTarget: ChartGridDropTarget?
    @State private var previewedNewColumnChartID: UUID?

    init(tabID: UUID) {
        _contentViewModel = StateObject(wrappedValue: ContentViewModel(tabID: tabID))
    }

    var body: some View {
        NavigationStack {
            HStack(spacing: 0) {
                // Drawing tools. Outside the chart column, so the card-height math
                // below measures only what's left and needs no adjustment.
                ToolSidebar(
                    activeTool: contentViewModel.activeTool,
                    onSelect: { tool in contentViewModel.toggleTool(tool) }
                ) {
                    sidebarTradingControls
                }
                .zIndex(1)

                chartColumn

                if showFavorites {
                    Divider()
                    WatchlistSidebar(
                        store: watchlists,
                        onAdd: { showAddFavoriteSheet = true },
                        onSelect: contentViewModel.openWatchlistInstrument
                    )
                    .transition(.move(edge: .trailing).combined(with: .opacity))
                }
            }
            // The title is the tab label, and an empty tab still needs the
            // toolbar — both belong outside the empty/non-empty branch.
            .navigationTitle(contentViewModel.tabName)
            .toolbar {
                AppToolbar()
                toolbarContent
            }
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
        .alert(
            layoutPromptTitle,
            isPresented: Binding(
                get: { contentViewModel.layout.prompt != nil },
                set: { if !$0 { contentViewModel.layout.cancelPrompt() } })
        ) {
            TextField("Name", text: $layoutPromptName)
            Button(layoutPromptButton) {
                contentViewModel.layout.confirmPrompt(name: layoutPromptName)
            }
            Button("Cancel", role: .cancel) { contentViewModel.layout.cancelPrompt() }
        } message: {
            Text(layoutPromptMessage)
        }
        .onChange(of: contentViewModel.layout.prompt) { _, prompt in
            if prompt != nil { layoutPromptName = contentViewModel.layout.promptDefaultName }
        }
        .confirmationDialog(
            "Save changes to “\(contentViewModel.layout.displayName)”?",
            isPresented: Binding(
                get: { contentViewModel.layout.pendingTransition != nil },
                set: { if !$0 { contentViewModel.layout.resolve(.cancel) } }),
            titleVisibility: .visible
        ) {
            Button("Save") { contentViewModel.layout.resolve(.save) }
            Button("Don’t Save", role: .destructive) { contentViewModel.layout.resolve(.discard) }
            Button("Cancel", role: .cancel) { contentViewModel.layout.resolve(.cancel) }
        } message: {
            Text("Your changes will be lost if you switch layouts without saving.")
        }
        .alert(
            "Layout",
            isPresented: Binding(
                get: { contentViewModel.layout.lastError != nil },
                set: { if !$0 { contentViewModel.layout.clearError() } })
        ) {
            Button("OK") { contentViewModel.layout.clearError() }
        } message: {
            Text(contentViewModel.layout.lastError ?? "")
        }
        .sheet(isPresented: $showLayoutPicker) {
            SavedLayoutPickerSheet(layout: contentViewModel.layout, store: contentViewModel.layout.store)
        }
        .focusedSceneValue(\.savedLayout, contentViewModel.layout)
        .sheet(isPresented: $showAddSheet) {
            AddTickerSheet(
                onAddPortfolio: { config in
                    contentViewModel.addPortfolioChart(config)
                },
                onAddCoinMarketCap: { config in
                    contentViewModel.addCoinMarketCapChart(config)
                },
                onAddBitcoinPowerLaw: {
                    contentViewModel.addBitcoinPowerLawChart()
                }
            ) { selected in
                let displayName: String? = {
                    guard selected.source.isPredictionMarket else { return nil }
                    return selected.eventTitle ?? selected.question ?? selected.symbol
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
            AddTickerSheet(
                title: "Add Favorite", actionLabel: "Favorite",
                subtitle: "Save a market to your favorites sidebar.", systemImage: "star.fill"
            ) { selected in
                guard let favorites = watchlists.favorites else { throw WatchlistError.notFound }
                try watchlists.add(selected, to: favorites.id)
            }
        }
        .sheet(isPresented: $showReplayDatePicker) {
            ReplayStartSheet(
                range: contentViewModel.replayDateRange,
                initial: contentViewModel.replay.currentTimestamp,
                snap: contentViewModel.replayBarStart(containing:),
                onStart: { date in
                    contentViewModel.selectReplayDate(date)
                    showReplayDatePicker = false
                },
                onCancel: { showReplayDatePicker = false })
        }
        .sheet(item: $orderTicket) { ticket in
            PaperOrderTicketSheet(
                store: paperTrading, instrument: ticket.instrument,
                referencePrice: ticket.price, initialSide: ticket.side)
        }
        .alert(
            "Paper Trading",
            isPresented: Binding(
                get: { paperTrading.lastError != nil && orderTicket == nil },
                set: { if !$0 { paperTrading.clearError() } })
        ) {
            Button("OK") { paperTrading.clearError() }
        } message: {
            Text(paperTrading.lastError ?? "")
        }
        .onChange(of: showAddSheet) { _, _ in
            contentViewModel.isShowingSheet = showAddSheet || showAddFavoriteSheet || showLayoutPicker
        }
        .onChange(of: showAddFavoriteSheet) { _, _ in
            contentViewModel.isShowingSheet = showAddSheet || showAddFavoriteSheet || showLayoutPicker
        }
        .onChange(of: showLayoutPicker) { _, _ in
            contentViewModel.isShowingSheet = showAddSheet || showAddFavoriteSheet || showLayoutPicker
        }
    }

    // MARK: - Empty state

    /// Add a market picked on the empty state to this tab, and put it at the front of the recents.
    private func openMarket(_ result: TickerSearchResult) {
        Task { @MainActor in
            do {
                try await contentViewModel.addTicker(symbol: result.fullSymbol, source: result.source)
                recents.record(result)
            } catch {
                // Same as a favorite: a failed add leaves the tab as it was.
            }
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
                    .layoutPriority(1)
                PaperAccountManagerView(
                    store: paperTrading,
                    selectedTab: $paperManagerTab,
                    showTradingOnCharts: $showPaperTradingOnCharts
                ) {
                    showTradingPanel = false
                }
                .frame(minHeight: UI.paperPanelMinHeight, idealHeight: UI.paperPanelIdealHeight)
            }
        } else {
            chartsOnly
        }
    }

    @ViewBuilder
    private var chartsOnly: some View {
        VStack(spacing: 0) {
            if contentViewModel.replay.isActive {
                VStack(spacing: 0) {
                    ReplayControlBar(
                        engine: contentViewModel.replay,
                        onChangeStart: contentViewModel.beginReplaySelection,
                        onCancelSelection: contentViewModel.cancelReplaySelection,
                        onReturnToLive: contentViewModel.returnToLive,
                        availableIntervals: contentViewModel.availableReplayIntervals,
                        resolvedInterval: contentViewModel.resolvedReplayInterval,
                        onIntervalChanged: contentViewModel.setReplayInterval,
                        isPreparing: contentViewModel.isPreparingReplay,
                        notice: contentViewModel.replayNotice,
                        onDismissNotice: contentViewModel.dismissReplayNotice
                    )
                    // The grid's card inset shrinks to nothing in a crowded tab, so the replay
                    // outline would otherwise touch the bar.
                    Color.clear.frame(height: 6)
                }
                .transition(.move(edge: .top).combined(with: .opacity))
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
                    suggestions: EmptyStateSuggestions(
                        favorites: watchlists.favorites?.instruments ?? [], recents: recents.items,
                        savedViews: contentViewModel.layout.store.views),
                    offersStocks: AlpacaCredentialsStore.isConfigured,
                    onAddTapped: { showAddSheet = true },
                    onOpenFavorite: contentViewModel.openWatchlistInstrument,
                    onOpenRecent: { openMarket($0.result) },
                    onOpenView: { contentViewModel.openSavedView($0) },
                    onOpenSuggestion: openMarket,
                    onRemoveRecent: { recents.remove($0) },
                    onClearRecents: { recents.clear() },
                    onShowFavorites: { withAnimation { showFavorites = true } }
                )
                // Another tab may have saved a view since this one opened.
                .onAppear { contentViewModel.layout.store.reload() }
                .onReceive(NotificationCenter.default.publisher(for: NSWindow.didBecomeKeyNotification)) { _ in
                    contentViewModel.layout.store.reload()
                }
            } else {
                VStack(spacing: 0) {
                    // Chart grid — fills the remaining height
                    GeometryReader { geometry in
                        let available = geometry.size.height
                        let gridColumnCount =
                            contentViewModel.chartColumns.count
                            + (previewedNewColumnChartID == nil ? 0 : 1)
                        let maxGridRows = contentViewModel.chartColumns.map(\.chartIDs.count).max() ?? 0
                        let gridSizingCount = maxGridRows * max(1, gridColumnCount)

                        // Cards may shrink so every row remains visible.
                        let chartHeight = ChartLayout.gridPlotHeight(
                            available: available,
                            cardCount: gridSizingCount,
                            columnCount: max(1, gridColumnCount)
                        )

                        chartGrid(
                            availableHeight: available,
                            availableWidth: geometry.size.width,
                            chartHeight: chartHeight,
                            sizingCardCount: gridSizingCount,
                            columnCount: max(1, gridColumnCount)
                        )
                    }
                }
            }
        }
        .animation(.easeOut(duration: 0.2), value: contentViewModel.replay.isActive)
    }

    // MARK: - Toolbar

    @ViewBuilder
    private var sidebarTradingControls: some View {
        let isPanelOpen = showTradingPanel && paperTrading.isConnected
        SidebarIconButton(
            icon: "arrow.left.arrow.right",
            label: isPanelOpen ? "Hide Paper Trading panel" : "Show Paper Trading panel",
            tooltip: isPanelOpen ? "Hide Paper Trading" : "Paper Trading",
            isActive: isPanelOpen,
            activeStyle: .tinted
        ) {
            if isPanelOpen {
                showTradingPanel = false
            } else if paperTrading.isConnected {
                showTradingPanel = true
            } else {
                Task {
                    await paperTrading.connect()
                    showTradingPanel = true
                }
            }
        }

        let isReplaying = contentViewModel.replay.isActive
        let hasMarket = !contentViewModel.marketChartViewModels.isEmpty
        SidebarMenu(
            icon: "clock.arrow.circlepath",
            label: isReplaying ? "Historical bar replay, active" : "Historical bar replay",
            tooltip: replayTooltip(isReplaying: isReplaying, hasMarket: hasMarket),
            isActive: isReplaying,
            tint: ReplayStyle.accent,
            activeStyle: .tinted
        ) {
            Button {
                contentViewModel.beginReplaySelection()
            } label: {
                Label("Select Bar on Chart", systemImage: "cursorarrow.click.2")
            }
            Button {
                showReplayDatePicker = true
            } label: {
                Label("Choose Date & Time…", systemImage: "calendar")
            }
            Divider()
            Button {
                contentViewModel.selectRandomReplayBar()
            } label: {
                Label("Random Bar", systemImage: "dice")
            }
            Button {
                contentViewModel.selectFirstReplayBar()
            } label: {
                Label("First Available Bar", systemImage: "backward.end")
            }
            if isReplaying {
                Divider()
                Button {
                    contentViewModel.returnToLive()
                } label: {
                    Label("Return to Latest", systemImage: "dot.radiowaves.left.and.right")
                }
            }
        }
        .disabled(!hasMarket)
    }

    private func replayTooltip(isReplaying: Bool, hasMarket: Bool) -> String {
        if isReplaying { return "Replay Active" }
        return hasMarket ? "Historical Replay" : "Historical Replay — add a market chart first"
    }

    private var layoutPromptTitle: String {
        switch contentViewModel.layout.prompt {
        case .copy: return "Make a Copy"
        case .rename: return "Rename Layout"
        case .save, nil: return "Save Layout"
        }
    }

    private var layoutPromptButton: String {
        contentViewModel.layout.prompt == .rename ? "Rename" : "Save"
    }

    private var layoutPromptMessage: String {
        switch contentViewModel.layout.prompt {
        case .copy: return "The copy keeps what you see now and saves separately from the original."
        case .rename: return "The name is also the tab title."
        case .save, nil: return "Save the current charts, timeframe, and layout under a name."
        }
    }

    @ToolbarContentBuilder
    private var toolbarContent: some ToolbarContent {
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
            SavedLayoutToolbarButton(layout: contentViewModel.layout) { showLayoutPicker = true }
        }
        // Its own bubble, apart from the layout button, the title-bar buttons and Add Chart.
        if #available(macOS 26, *) {
            ToolbarSpacer(.fixed, placement: .primaryAction)
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
        // Its own bubble and a text label, so it isn't mistaken for the tab bar's `+`.
        if #available(macOS 26, *) {
            ToolbarSpacer(.fixed, placement: .primaryAction)
        }
        ToolbarItem(placement: .primaryAction) {
            Button {
                showAddSheet = true
            } label: {
                Label("Add Chart", systemImage: "plus")
                    .labelStyle(.titleAndIcon)
                    .padding(.horizontal, 6)
            }
            .help("Add Chart")
        }
    }

    // MARK: - Chart Grid

    @ViewBuilder
    private func chartGrid(
        availableHeight: CGFloat,
        availableWidth: CGFloat,
        chartHeight: CGFloat,
        sizingCardCount: Int,
        columnCount: Int
    ) -> some View {
        let cardHeight = ChartLayout.gridCardHeight(
            available: availableHeight,
            cardCount: sizingCardCount,
            columnCount: columnCount
        )
        let cardInset = ChartLayout.gridCardInset(
            available: availableHeight,
            cardCount: sizingCardCount,
            columnCount: columnCount
        )
        let canAddColumn = ChartLayout.canAddColumn(
            availableWidth: availableWidth,
            currentColumnCount: contentViewModel.chartColumns.count
        )

        ZStack(alignment: .trailing) {
            HStack(alignment: .top, spacing: 0) {
                ForEach(contentViewModel.chartColumns) { column in
                    VStack(spacing: 0) {
                        ForEach(contentViewModel.charts(in: column), id: \.chartID) { vm in
                            chartCard(vm, height: chartHeight, cardHeight: cardHeight)
                                .opacity(previewedNewColumnChartID == vm.chartID ? 0.35 : 1)
                                .padding(cardInset)
                                .overlay(alignment: .top) {
                                    if gridDropTarget
                                        == .existing(columnID: column.id, before: vm.chartID)
                                    {
                                        Capsule()
                                            .fill(Color.accentColor)
                                            .frame(height: 3)
                                            .padding(.horizontal, 8)
                                            .offset(y: -1.5)
                                    }
                                }
                                .onDrag {
                                    draggedChartID = vm.chartID
                                    return NSItemProvider(object: vm.chartID.uuidString as NSString)
                                }
                                .onDrop(
                                    of: [.utf8PlainText],
                                    delegate: ChartGridDropDelegate(
                                        destination: .existing(columnID: column.id, before: vm.chartID),
                                        viewModel: contentViewModel,
                                        draggedChartID: $draggedChartID,
                                        activeTarget: $gridDropTarget,
                                        previewedChartID: $previewedNewColumnChartID
                                    )
                                )
                        }

                        Color.clear
                            .frame(maxWidth: .infinity, maxHeight: .infinity)
                            .frame(minHeight: 20)
                            .contentShape(Rectangle())
                            .overlay(alignment: .top) {
                                if gridDropTarget == .existing(columnID: column.id, before: nil) {
                                    Capsule()
                                        .fill(Color.accentColor)
                                        .frame(height: 3)
                                        .padding(.horizontal, 8)
                                }
                            }
                            .onDrop(
                                of: [.utf8PlainText],
                                delegate: ChartGridDropDelegate(
                                    destination: .existing(columnID: column.id, before: nil),
                                    viewModel: contentViewModel,
                                    draggedChartID: $draggedChartID,
                                    activeTarget: $gridDropTarget,
                                    previewedChartID: $previewedNewColumnChartID
                                )
                            )
                    }
                    .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
                }

                if let previewID = previewedNewColumnChartID {
                    VStack {
                        RoundedRectangle(cornerRadius: 10)
                            .fill(Color.accentColor.opacity(0.08))
                            .overlay {
                                RoundedRectangle(cornerRadius: 10)
                                    .strokeBorder(
                                        Color.accentColor,
                                        style: StrokeStyle(lineWidth: 2, dash: [7, 5])
                                    )
                            }
                            .overlay(alignment: .top) {
                                Canvas { context, size in
                                    var line = Path()
                                    line.move(to: CGPoint(x: 10, y: 5))
                                    line.addLine(to: CGPoint(x: max(10, size.width - 10), y: 5))
                                    context.stroke(
                                        line,
                                        with: .color(Color.accentColor),
                                        style: StrokeStyle(lineWidth: 2, dash: [7, 5])
                                    )
                                }
                                .frame(height: 10)
                            }
                            .overlay {
                                VStack(spacing: 8) {
                                    Image(systemName: "rectangle.split.3x1")
                                        .font(.title2)
                                    Text("New column")
                                        .font(.headline)
                                }
                                .foregroundStyle(Color.accentColor)
                            }
                            .frame(height: cardHeight)
                            // Keep the dashed stroke inside the grid's clipping
                            // boundary even when constrained-height padding scales to zero.
                            .padding(.horizontal, cardInset)
                            .padding(.bottom, cardInset)
                            .padding(.top, max(4, cardInset))
                            .accessibilityLabel("Drop chart into new column")
                    }
                    .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
                    .onDrop(
                        of: [.utf8PlainText],
                        delegate: ChartGridDropDelegate(
                            destination: .newColumn,
                            viewModel: contentViewModel,
                            draggedChartID: $draggedChartID,
                            activeTarget: $gridDropTarget,
                            previewedChartID: $previewedNewColumnChartID
                        )
                    )
                    .transition(.move(edge: .trailing).combined(with: .opacity))
                    .id(previewID)
                }
            }
            .padding(
                ChartLayout.gridOuterInset(
                    available: availableHeight,
                    cardCount: sizingCardCount,
                    columnCount: columnCount
                )
            )
            .animation(.easeInOut(duration: 0.2), value: previewedNewColumnChartID)

            if draggedChartID != nil, canAddColumn {
                VStack(spacing: 6) {
                    if previewedNewColumnChartID == nil {
                        Image(systemName: "plus")
                            .font(.headline)
                        Image(systemName: "rectangle.split.3x1")
                            .font(.title3)
                    }
                }
                .foregroundStyle(Color.accentColor)
                .frame(
                    width: previewedNewColumnChartID == nil
                        ? ChartLayout.gridNewColumnDropWidth
                        : availableWidth / CGFloat(contentViewModel.chartColumns.count + 1)
                )
                .frame(maxHeight: .infinity)
                .background(
                    Color.accentColor.opacity(
                        previewedNewColumnChartID == nil
                            ? (gridDropTarget == .newColumn ? 0.16 : 0.07)
                            : 0
                    )
                )
                .overlay(alignment: .leading) {
                    if previewedNewColumnChartID == nil {
                        Rectangle()
                            .fill(Color.accentColor.opacity(0.75))
                            .frame(width: gridDropTarget == .newColumn ? 3 : 1)
                    }
                }
                .contentShape(Rectangle())
                .accessibilityLabel("Add chart column")
                .onDrop(
                    of: [.utf8PlainText],
                    delegate: ChartGridDropDelegate(
                        destination: .newColumn,
                        viewModel: contentViewModel,
                        draggedChartID: $draggedChartID,
                        activeTarget: $gridDropTarget,
                        previewedChartID: $previewedNewColumnChartID
                    )
                )
                .transition(.move(edge: .trailing).combined(with: .opacity))
            }
        }
        .frame(height: availableHeight)
    }

    // MARK: - Chart Card Builder

    private func chartCard(_ vm: ChartViewModel, height: CGFloat, cardHeight: CGFloat? = nil) -> some View {
        ChartCardView(
            viewModel: vm,
            chartHeight: height,
            timeRange: contentViewModel.selectedTimeRange,
            cardHeight: cardHeight,
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
            isFavorite: watchlists.isFavorite(InstrumentID(source: vm.source, symbol: vm.ticker)),
            onToggleFavorite: {
                try? watchlists.toggleFavorite(
                    WatchlistInstrument(
                        instrument: InstrumentID(source: vm.source, symbol: vm.ticker),
                        name: vm.title,
                        label: favoriteTicker(for: vm),
                        displayName: vm.displayName,
                        pmSeries: vm.pmSeries.isEmpty ? nil : vm.pmSeries))
            },
            onZoomRegion: {
                if vm.isBitcoinPowerLaw {
                    contentViewModel.registerPowerLawZoomRegion($0, for: vm)
                } else {
                    contentViewModel.registerZoomRegion($0)
                }
            },
            onAxisRegion: { contentViewModel.registerAxisRegion($0, for: vm) },
            onPlotRegion: { contentViewModel.registerPlotRegion($0, for: vm) },
            onLegendRegion: { contentViewModel.registerLegendRegion($0, for: vm) },
            isToolArmed: contentViewModel.activeTool != .none,
            showTrendHandles: contentViewModel.activeTool == .trendLine
                || contentViewModel.activeTool == .fibonacciRetracement,
            crosshair: contentViewModel.crosshair,
            onCrosshairExit: { contentViewModel.crosshair.clear(owner: vm.uniqueID) },
            onStyleChanged: {
                contentViewModel.persistChartSettings()
            },
            onSettingsPresented: { shown in
                contentViewModel.isShowingSheet = shown
            },
            onLineEditorPresented: { shown in
                contentViewModel.isShowingLineEditor = shown
            },
            onPaperBuy: { openTicket(for: vm, side: .buy) },
            onPaperSell: { openTicket(for: vm, side: .sell) },
            paperConnected: paperTrading.isConnected && showTradingPanel && showPaperTradingOnCharts,
            paperPositions: paperTrading.positions.filter {
                $0.instrument.key == "\(vm.source.rawValue):\(vm.apiSymbol)"
            },
            paperOrders: paperTrading.workingOrders.filter {
                $0.instrument.key == "\(vm.source.rawValue):\(vm.apiSymbol)"
            },
            paperAccountCurrency: paperTrading.selectedAccount?.baseCurrency ?? .USD,
            paperUnrealizedPnL: { position in paperTrading.unrealizedPnL(for: position) },
            onPaperModify: { order, price in
                let changes =
                    order.type == .stop || (order.type == .stopLimit && !order.stopTriggered)
                    ? PaperOrderChanges(stopPrice: price) : PaperOrderChanges(limitPrice: price)
                Task { await paperTrading.modify(order.id, changes: changes) }
            },
            onPaperCancel: { order in Task { await paperTrading.cancel(order.id) } },
            onPaperClose: { position in Task { await paperTrading.close(position) } }
        )
        // Behind the card, not on `ContentView`: this view does not observe its charts, so a watcher
        // here only ran when something unrelated redrew it. The feed observes the chart itself.
        .background {
            PaperQuoteFeed(viewModel: vm, quote: vm.liveQuote, store: paperTrading)
        }
    }

    private func favoriteTicker(for vm: ChartViewModel) -> String {
        if vm.source.isPredictionMarket {
            if vm.pmSeries.count > 1,
                let outcome = vm.pmSeries.first(where: {
                    $0.tokenID.caseInsensitiveCompare(vm.ticker) == .orderedSame
                })
            {
                return outcome.label
            }
            return vm.title
        }

        if vm.source == .dexscreener {
            return vm.baseSymbol
        }
        // Exchange pairs read BASE/QUOTE, like the card header they were starred from.
        return vm.marketPair?.display ?? vm.ticker.uppercased()
    }

    private func openTicket(for vm: ChartViewModel, side: PaperOrderSide) {
        let instrument = PaperInstrument.chart(symbol: vm.apiSymbol, displayName: vm.title, source: vm.source)
        orderTicket = .init(instrument: instrument, price: vm.displayedPrice.map { Decimal($0) }, side: side)
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
