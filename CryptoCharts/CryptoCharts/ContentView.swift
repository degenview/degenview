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

                        ScrollView {
                            LazyVStack(spacing: 16) {
                                ForEach(contentViewModel.chartViewModels, id: \.ticker) { vm in
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
                            .padding(.vertical, 16)
                        }
                    }
                    .navigationTitle("CryptoCharts")
                    .toolbar {
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
                                Image(systemName: contentViewModel.useLogScale ? "function" : "equal.square")
                            }
                            .accessibilityLabel(contentViewModel.useLogScale ? "Linear scale" : "Log scale")
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
}

#Preview {
    ContentView()
}
