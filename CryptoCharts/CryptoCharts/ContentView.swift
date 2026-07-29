import SwiftUI

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
                        // Global timeframe picker
                        Picker("Timeframe", selection: $contentViewModel.selectedTimeRange) {
                            ForEach(TimeRange.allCases) { range in
                                Text(range.rawValue).tag(range)
                            }
                        }
                        .pickerStyle(.segmented)
                        .labelsHidden()
                        .padding(.horizontal, 16)
                        .padding(.vertical, 8)
                        .onChange(of: contentViewModel.selectedTimeRange) {
                            contentViewModel.setTimeRange(contentViewModel.selectedTimeRange)
                        }

                        // Global loading indicator
                        if contentViewModel.isRefreshing {
                            ProgressView()
                                .progressViewStyle(.linear)
                                .scaleEffect(x: 1, y: 0.5)
                                .padding(.horizontal, 16)
                        }

                        // Chart list (vertical) or grid
                        if contentViewModel.layoutMode == .vertical {
                            List {
                                ForEach(contentViewModel.chartViewModels, id: \.ticker) { vm in
                                    chartCard(vm)
                                        .listRowSeparator(.hidden)
                                }
                                .onMove { from, to in
                                    contentViewModel.moveTicker(from: from, to: to)
                                }
                            }
                            .listStyle(.plain)
                            .scrollContentBackground(.hidden)
                        } else {
                            ScrollView {
                                LazyVGrid(
                                    columns: [GridItem(.flexible()), GridItem(.flexible())],
                                    spacing: 12
                                ) {
                                    ForEach(contentViewModel.chartViewModels, id: \.ticker) { vm in
                                        chartCard(vm)
                                    }
                                }
                                .padding(.vertical, 16)
                                .padding(.horizontal, 12)
                            }
                        }
                    }
                    .navigationTitle("CryptoCharts")
                    .toolbar {
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
                            Button {
                                contentViewModel.useLogScale.toggle()
                            } label: {
                                Image(systemName: contentViewModel.useLogScale
                                    ? "function" : "equal.square")
                            }
                            .accessibilityLabel(contentViewModel.useLogScale
                                ? "Linear scale" : "Log scale")
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
                                saveViewName = ""
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
            .frame(minWidth: 380, idealWidth: 440)
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

    private func chartCard(_ vm: ChartViewModel) -> some View {
        ChartCardView(
            viewModel: vm,
            intervalLabel: contentViewModel.selectedTimeRange.binanceInterval,
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
                let step = max(1, Int(Double(contentViewModel.candleCount) * 0.1))
                if deltaY > 0 {
                    contentViewModel.adjustCandleCount(by: step)
                } else if deltaY < 0 {
                    contentViewModel.adjustCandleCount(by: -step)
                }
            },
            useLogScale: contentViewModel.useLogScale
        )
    }
}

#Preview {
    ContentView()
}
