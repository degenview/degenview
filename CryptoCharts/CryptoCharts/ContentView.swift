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
    @State private var showSaveAlert = false
    @State private var saveViewName = ""
    @State private var showRenameAlert = false
    @State private var renameText = ""
    @AppStorage("appTheme") private var appTheme: AppTheme = .system

    init(tabID: UUID) {
        _contentViewModel = StateObject(wrappedValue: ContentViewModel(tabID: tabID))
    }

    var body: some View {
        NavigationStack {
            VStack(spacing: 0) {
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
            // The title is the tab label, and an empty tab still needs the
            // toolbar — both belong outside the empty/non-empty branch.
            .navigationTitle(contentViewModel.tabName)
            .toolbar { toolbarContent }
            .frame(minWidth: UI.windowMinWidth, idealWidth: UI.windowIdealWidth)
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
            AddTickerSheet(contentViewModel: contentViewModel)
        }
        .onChange(of: showAddSheet) { _, new in
            contentViewModel.isShowingSheet = new
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
            Picker("Theme", selection: $appTheme) {
                ForEach(AppTheme.allCases) { theme in
                    Label(theme.rawValue, systemImage: theme.icon).tag(theme)
                }
            }
            .pickerStyle(.menu)
            .accessibilityLabel("Theme")
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
            onZoom: { deltaY in
                let step = max(1, Int(Double(contentViewModel.candleCount) * Candle.zoomStepFraction))
                if deltaY > 0 {
                    contentViewModel.adjustCandleCount(by: step)
                } else if deltaY < 0 {
                    contentViewModel.adjustCandleCount(by: -step)
                }
            },
            onUpdateTicker: { symbol, source, displayName in
                contentViewModel.updateTicker(vm, symbol: symbol, source: source, displayName: displayName)
            },
            onStyleChanged: {
                contentViewModel.persistChartSettings()
            },
            onSettingsPresented: { shown in
                contentViewModel.isShowingSheet = shown
            }
        )
    }
}

#Preview {
    ContentView(tabID: UUID())
}
