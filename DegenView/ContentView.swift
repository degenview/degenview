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
                    .layoutPriority(1)
                PaperAccountManagerView(
                    store: paperTrading,
                    selectedTab: $paperManagerTab,
                    showTradingOnCharts: $showPaperTradingOnCharts
                ) {
                    showTradingPanel = false
                }
                .frame(minHeight: 270, idealHeight: 270)
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
                        let available = geometry.size.height
                        let cardCount = contentViewModel.chartViewModels.count
                        let gridColumnCount =
                            contentViewModel.chartColumns.count
                            + (previewedNewColumnChartID == nil ? 0 : 1)
                        let maxGridRows = contentViewModel.chartColumns.map(\.chartIDs.count).max() ?? 0
                        let gridSizingCount = maxGridRows * max(1, gridColumnCount)

                        let naturalHeight: CGFloat = {
                            if contentViewModel.layoutMode == .vertical {
                                return ChartLayout.verticalPlotHeight(
                                    available: available, cardCount: cardCount
                                )
                            } else {
                                return ChartLayout.gridPlotHeight(
                                    available: available,
                                    cardCount: gridSizingCount,
                                    columnCount: max(1, gridColumnCount)
                                )
                            }
                        }()

                        // Vertical cards retain their readable minimum and scroll.
                        // Grid cards may shrink further so every row remains visible.
                        let chartHeight =
                            contentViewModel.layoutMode == .vertical
                            ? max(ChartLayout.chartMinHeight, naturalHeight)
                            : naturalHeight

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
        }
    }

    // MARK: - Toolbar

    @ViewBuilder
    private var sidebarTradingControls: some View {
        Button {
            if paperTrading.isConnected {
                showTradingPanel = true
            } else {
                Task {
                    await paperTrading.connect()
                    showTradingPanel = true
                }
            }
        } label: {
            Image(systemName: "arrow.left.arrow.right")
                .font(.system(size: 13, weight: .medium))
                .frame(width: 26, height: 26)
                .contentShape(RoundedRectangle(cornerRadius: 5))
        }
        .buttonStyle(.plain)
        .accessibilityLabel(paperTrading.isConnected ? "Paper Trading connected" : "Trade")
        .sidebarTooltip("Paper Trading")

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
            Image(systemName: "clock.arrow.circlepath")
                .font(.system(size: 13, weight: .medium))
                .frame(width: 26, height: 26)
                .contentShape(RoundedRectangle(cornerRadius: 5))
        }
        .menuStyle(.borderlessButton)
        .menuIndicator(.hidden)
        .fixedSize()
        .accessibilityLabel("Historical bar replay")
        .sidebarTooltip("Historical Replay")
    }

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
            .accessibilityLabel(
                contentViewModel.layoutMode == .vertical
                    ? "Grid layout" : "Vertical layout")
        }
        ToolbarItem(placement: .automatic) {
            Menu {
                // The name bar used to be the rename affordance; with it gone
                // this menu is the only place left to reach it.
                Button("Rename Tab…") {
                    renameText =
                        contentViewModel.tabName == UI.unnamedView
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
                saveViewName =
                    contentViewModel.tabName == UI.unnamedView
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
            onZoomRegion: {
                if vm.isBitcoinPowerLaw {
                    contentViewModel.registerPowerLawZoomRegion($0, for: vm)
                } else {
                    contentViewModel.registerZoomRegion($0)
                }
            },
            onAxisRegion: { contentViewModel.registerAxisRegion($0, for: vm) },
            onPlotRegion: { contentViewModel.registerPlotRegion($0, for: vm) },
            isToolArmed: contentViewModel.activeTool != .none,
            showTrendHandles: contentViewModel.activeTool == .trendLine
                || contentViewModel.activeTool == .fibonacciRetracement,
            crosshair: contentViewModel.crosshair,
            onCrosshairExit: { contentViewModel.crosshair.clear(owner: vm.uniqueID) },
            onUpdateTicker: { symbol, source, displayName, pmSeries in
                contentViewModel.updateTicker(
                    vm, symbol: symbol, source: source, displayName: displayName, pmSeries: pmSeries)
            },
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
            paperUnrealizedPnL: { position in
                guard let quote = paperTrading.snapshot.quotes[position.instrument.key],
                    let mark = position.signedQuantity >= 0 ? (quote.bid ?? quote.last) : (quote.ask ?? quote.last)
                else { return 0 }
                return
                    (position.signedQuantity >= 0
                    ? mark - position.averageEntryPrice : position.averageEntryPrice - mark) * position.quantity
                    * position.instrument.pointValue
            },
            onPaperModify: { order, price in
                let changes =
                    order.type == .stop || (order.type == .stopLimit && !order.stopTriggered)
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
                })
            {
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
