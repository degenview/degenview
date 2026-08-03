import SwiftUI

// MARK: - ChartSettingsSheet

struct ChartSettingsSheet: View {
    @ObservedObject var viewModel: ChartViewModel
    /// (symbol, source, displayName) — `displayName` is nil for crypto sources,
    /// whose symbol already reads fine on the card.
    let onUpdateTicker: (String, DataSourceType, String?) -> Void
    let onRemove: () -> Void
    let onStyleChanged: () -> Void

    @StateObject private var searchVM = TickerSearchViewModel(logPrefix: "[ChartSettings]")
    @StateObject private var polymarketVM = PolymarketSearchViewModel(logPrefix: "[ChartSettings/Polymarket]")

    @State private var selectedTab: Tab
    @State private var searchText = ""
    @State private var polymarketText = ""

    // Appearance state — initialized from viewModel
    @State private var bullishColor: Color
    @State private var bearishColor: Color
    @State private var decimalPlacesMode: DecimalMode
    @State private var showVolume: Bool

    @Environment(\.dismiss) private var dismiss

    enum Tab: String, CaseIterable {
        case ticker = "Ticker"
        case polymarket = "Polymarket"
        case appearance = "Appearance"
    }

    enum DecimalMode: String, CaseIterable, Identifiable {
        case auto = "Auto"
        case zero = "0"
        case one = "1"
        case two = "2"
        case three = "3"
        case four = "4"
        case five = "5"
        case six = "6"
        case seven = "7"
        case eight = "8"

        var id: String { rawValue }

        var intValue: Int? {
            switch self {
            case .auto: return nil
            default: return Int(rawValue)!
            }
        }

        static func from(_ int: Int?) -> DecimalMode {
            guard let int else { return .auto }
            switch int {
            case 0: return .zero
            case 1: return .one
            case 2: return .two
            case 3: return .three
            case 4: return .four
            case 5: return .five
            case 6: return .six
            case 7: return .seven
            case 8: return .eight
            default: return .auto
            }
        }
    }

    init(viewModel: ChartViewModel,
         onUpdateTicker: @escaping (String, DataSourceType, String?) -> Void,
         onRemove: @escaping () -> Void,
         onStyleChanged: @escaping () -> Void) {
        self.viewModel = viewModel
        self.onUpdateTicker = onUpdateTicker
        self.onRemove = onRemove
        self.onStyleChanged = onStyleChanged
        _bullishColor = State(initialValue: viewModel.bullishColor)
        _bearishColor = State(initialValue: viewModel.bearishColor)
        _decimalPlacesMode = State(initialValue: DecimalMode.from(viewModel.yAxisDecimalPlaces))
        _showVolume = State(initialValue: viewModel.showVolume)
        // Open on the tab that matches what this chart already is.
        _selectedTab = State(initialValue: viewModel.source == .polymarket ? .polymarket : .ticker)
    }

    var body: some View {
        VStack(spacing: 0) {
            // Title
            Text("\(viewModel.title) Settings")
                .font(.headline)
                .lineLimit(1)
                .truncationMode(.tail)
                .padding(.horizontal, 16)
                .padding(.top, 16)
                .padding(.bottom, 8)

            // Tab picker
            Picker("", selection: $selectedTab) {
                ForEach(Tab.allCases, id: \.self) { tab in
                    Text(tab.rawValue).tag(tab)
                }
            }
            .pickerStyle(.segmented)
            .padding(.horizontal, 16)
            .padding(.bottom, 8)

            Divider()

            switch selectedTab {
            case .ticker:
                tickerTab
            case .polymarket:
                polymarketTab
            case .appearance:
                appearanceTab
            }

            Divider()

            // Bottom buttons
            HStack {
                Button(role: .destructive) {
                    dismiss()
                    onRemove()
                } label: {
                    Label("Remove", systemImage: "trash")
                }

                Spacer()

                Button("Save") {
                    cancelSearches()
                    dismiss()
                }
                .keyboardShortcut(.return)
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 12)
        }
        .frame(width: UI.chartSettingsSheetWidth, height: UI.chartSettingsSheetHeight)
        .onDisappear {
            cancelSearches()
        }
        .onChange(of: bullishColor) {
            viewModel.bullishColor = bullishColor
            onStyleChanged()
        }
        .onChange(of: bearishColor) {
            viewModel.bearishColor = bearishColor
            onStyleChanged()
        }
        .onChange(of: decimalPlacesMode) {
            viewModel.yAxisDecimalPlaces = decimalPlacesMode.intValue
            onStyleChanged()
        }
        .onChange(of: showVolume) {
            viewModel.showVolume = showVolume
            onStyleChanged()
        }
    }

    // MARK: - Ticker Tab

    private var tickerTab: some View {
        VStack(spacing: 12) {
            currentChartRow

            SearchFieldRow(
                placeholder: "New ticker symbol (e.g. BTC or PEPE)",
                text: $searchText,
                isSearching: searchVM.isSearching,
                onChange: { searchVM.scheduleSearch(query: $0) },
                onSubmit: {
                    if let first = searchVM.firstAvailableResult {
                        searchVM.selectedResult = first
                    }
                }
            )
            .padding(.horizontal, 16)

            // Search results
            if !searchVM.searchResults.isEmpty {
                List {
                    ForEach(searchVM.orderedSources, id: \.self) { source in
                        if let results = searchVM.searchResults[source], !results.isEmpty {
                            Section {
                                ForEach(results) { result in
                                    SearchResultRow(
                                        result: result,
                                        isSelected: searchVM.selectedResult == result,
                                        onSelect: { searchVM.selectedResult = result }
                                    )
                                }
                            } header: {
                                Label(source.displayName, systemImage: source.icon)
                                    .font(.caption)
                                    .foregroundStyle(.secondary)
                            }
                        }
                    }
                }
                .listStyle(.inset)
                .frame(maxHeight: UI.chartSettingsResultsMaxHeight)
            }

            // Selected result
            if let selected = searchVM.selectedResult {
                SelectedResultBanner(prefix: "New", result: selected)
                    .padding(.horizontal, 16)

                Button("Change Ticker") {
                    apply(selected)
                }
                .buttonStyle(.borderedProminent)
            }

            Spacer()
        }
        .padding(.top, 8)
    }

    // MARK: - Polymarket Tab

    private var polymarketTab: some View {
        VStack(spacing: 12) {
            currentChartRow

            PolymarketSearchPane(
                searchVM: polymarketVM,
                searchText: $polymarketText,
                resultsMinHeight: UI.chartSettingsResultsMinHeight,
                resultsMaxHeight: UI.chartSettingsResultsMaxHeight
            )
            .padding(.horizontal, 16)

            if let selected = polymarketVM.selectedResult {
                Button("Change Market") {
                    apply(selected)
                }
                .buttonStyle(.borderedProminent)
            }

            Spacer()
        }
        .padding(.top, 8)
    }

    /// What this chart currently tracks — shown above both search panes.
    private var currentChartRow: some View {
        HStack {
            Text("Current:")
                .font(.caption)
                .foregroundStyle(.secondary)
            Label(viewModel.title, systemImage: viewModel.source.icon)
                .font(.body.weight(.medium))
                .lineLimit(1)
                .truncationMode(.tail)
            Spacer()
        }
        .padding(.horizontal, 16)
    }

    // MARK: - Apply

    private func cancelSearches() {
        searchVM.cancelSearch()
        polymarketVM.cancelSearch()
    }

    private func apply(_ selected: TickerSearchResult) {
        // The search payload already carried the market artwork; seed the resolver so
        // the card repaints without another round trip.
        if selected.source == .polymarket {
            let ticker = selected.fullSymbol
            let url = selected.imageURL
            Task { await IconResolver.shared.remember(ticker: ticker, source: .polymarket, url: url) }
        }

        cancelSearches()
        dismiss()
        onUpdateTicker(
            selected.fullSymbol,
            selected.source,
            selected.source == .polymarket ? selected.symbol : nil
        )
    }

    // MARK: - Appearance Tab

    private var appearanceTab: some View {
        VStack(alignment: .leading, spacing: 16) {
            HStack(spacing: 24) {
                VStack(alignment: .leading, spacing: 8) {
                    Text(viewModel.usesLineChart ? "Line Up" : "Bullish Candle")
                        .font(.subheadline.weight(.medium))
                    ColorPicker("", selection: $bullishColor)
                        .labelsHidden()
                }

                VStack(alignment: .leading, spacing: 8) {
                    Text(viewModel.usesLineChart ? "Line Down" : "Bearish Candle")
                        .font(.subheadline.weight(.medium))
                    ColorPicker("", selection: $bearishColor)
                        .labelsHidden()
                }
            }

            Button("Reset Default Colors") {
                bullishColor = .green
                bearishColor = .red
            }
            .buttonStyle(.borderless)
            .font(.caption)
            .foregroundStyle(.secondary)

            Divider()

            VStack(alignment: .leading, spacing: 8) {
                Text("Y-Axis Decimal Places")
                    .font(.subheadline.weight(.medium))

                Picker("Decimals", selection: $decimalPlacesMode) {
                    ForEach(DecimalMode.allCases) { mode in
                        Text(mode.rawValue).tag(mode)
                    }
                }
                .pickerStyle(.menu)
                .frame(minWidth: 100)

                Text(decimalHint)
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }

            // Line sources report one price per timestamp and no turnover at all.
            if !viewModel.usesLineChart {
                Divider()

                VStack(alignment: .leading, spacing: 8) {
                    Toggle("Volume Bars", isOn: $showVolume)
                        .toggleStyle(.switch)
                        // Left visible rather than hidden: a greyed-out switch with a
                        // reason reads better than a setting that silently does nothing.
                        .disabled(!viewModel.source.providesVolume)

                    Text(volumeHint)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }

            Spacer()
        }
        .padding(16)
    }

    private var volumeHint: String {
        guard viewModel.source.providesVolume else {
            return "\(viewModel.source.displayName) doesn't report per-candle volume, so bars aren't available for this chart."
        }
        return "Turnover per candle, drawn along the bottom of the chart."
    }

    private var decimalHint: String {
        switch decimalPlacesMode {
        case .auto:
            "Automatically chooses precision based on price range."
        default:
            "Shows \(decimalPlacesMode.rawValue) decimal place\(decimalPlacesMode.rawValue == "1" ? "" : "s") on the Y-axis."
        }
    }
}

#Preview {
    let vm = ChartViewModel(ticker: "BTC")
    ChartSettingsSheet(
        viewModel: vm,
        onUpdateTicker: { _, _, _ in },
        onRemove: {},
        onStyleChanged: {}
    )
}
