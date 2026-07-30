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
    @StateObject private var contentViewModel = ContentViewModel()

    @State private var showAddSheet = false
    @State private var showSaveAlert = false
    @State private var saveViewName = ""
    @AppStorage("appTheme") private var appTheme: AppTheme = .system

    var body: some View {
        NavigationStack {
            Group {
                if contentViewModel.chartViewModels.isEmpty {
                    EmptyStateView {
                        showAddSheet = true
                    }
                } else {
                    VStack(spacing: 0) {
                        // Global loading indicator
                        if contentViewModel.isRefreshing {
                            ProgressView()
                                .progressViewStyle(.linear)
                                .scaleEffect(x: 1, y: 0.5)
                                .padding(.horizontal, 16)
                        }

                        // View name bar
                        HStack(spacing: 8) {
                            Text(contentViewModel.currentViewName)
                                .font(.subheadline.weight(.medium))
                                .foregroundStyle(.secondary)

                            if contentViewModel.hasUnsavedChanges {
                                Button("Save Changes") {
                                    if contentViewModel.currentViewName == UI.unnamedView {
                                        saveViewName = ""
                                        showSaveAlert = true
                                    } else {
                                        contentViewModel.saveChanges()
                                    }
                                }
                                .buttonStyle(.plain)
                                .font(.caption.weight(.medium))
                                .foregroundStyle(.blue)
                            }

                            Spacer()
                        }
                        .padding(.horizontal, 20)
                        .padding(.vertical, 2)

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

                            let chartHeight = min(ChartLayout.cardMaxHeight, max(ChartLayout.cardMinHeight, naturalHeight))
                            let needsScroll = chartHeight <= ChartLayout.cardMinHeight

                            if contentViewModel.layoutMode == .vertical {
                                List {
                                    ForEach(contentViewModel.chartViewModels, id: \.uniqueID) { vm in
                                        chartCard(vm, height: chartHeight)
                                            .listRowSeparator(.hidden)
                                            .padding(4)
                                    }
                                    .onMove { from, to in
                                        contentViewModel.moveTicker(from: from, to: to)
                                    }
                                }
                                .listStyle(.plain)
                                .scrollContentBackground(.hidden)
                                .scrollDisabled(!needsScroll)
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
                            }
                        }
                    }
                    .navigationTitle("CryptoCharts")
                    .toolbar {
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
                                ForEach(contentViewModel.savedViews) { view in
                                    Button(view.name) {
                                        contentViewModel.loadView(view)
                                    }
                                }
                                if !contentViewModel.savedViews.isEmpty {
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
                            .disabled(contentViewModel.savedViews.isEmpty)
                        }
                        ToolbarItem(placement: .automatic) {
                            Button {
                                saveViewName = contentViewModel.currentViewName == UI.unnamedView
                                    ? "" : contentViewModel.currentViewName
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
                }
            }
            .frame(minWidth: UI.windowMinWidth, idealWidth: UI.windowIdealWidth)
        }
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
        .sheet(isPresented: $showAddSheet) {
            AddTickerSheet(contentViewModel: contentViewModel)
        }
        .task {
            contentViewModel.loadTickers()
        }
        .onDisappear {
            contentViewModel.stopAutoRefresh()
        }
    }

    // MARK: - Chart Card Builder

    private func chartCard(_ vm: ChartViewModel, height: CGFloat = 220) -> some View {
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
            onUpdateTicker: { symbol, source in
                contentViewModel.updateTicker(vm, symbol: symbol, source: source)
            },
            onStyleChanged: {
                contentViewModel.persistChartSettings()
            }
        )
    }
}

#Preview {
    ContentView()
}
