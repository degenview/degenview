import SwiftUI

// MARK: - ChartSettingsSheet

struct ChartSettingsSheet: View {
    @ObservedObject var viewModel: ChartViewModel
    let onUpdateTicker: (String, DataSourceType) -> Void
    let onRemove: () -> Void
    let onStyleChanged: () -> Void

    @StateObject private var searchVM = TickerSearchViewModel(logPrefix: "[ChartSettings]")

    @State private var selectedTab: Tab = .ticker
    @State private var searchText = ""
    @State private var errorMessage: String?

    // Appearance state — initialized from viewModel
    @State private var bullishColor: Color
    @State private var bearishColor: Color
    @State private var decimalPlacesMode: DecimalMode

    @Environment(\.dismiss) private var dismiss

    enum Tab: String, CaseIterable {
        case ticker = "Ticker"
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
         onUpdateTicker: @escaping (String, DataSourceType) -> Void,
         onRemove: @escaping () -> Void,
         onStyleChanged: @escaping () -> Void) {
        self.viewModel = viewModel
        self.onUpdateTicker = onUpdateTicker
        self.onRemove = onRemove
        self.onStyleChanged = onStyleChanged
        _bullishColor = State(initialValue: viewModel.bullishColor)
        _bearishColor = State(initialValue: viewModel.bearishColor)
        _decimalPlacesMode = State(initialValue: DecimalMode.from(viewModel.yAxisDecimalPlaces))
    }

    var body: some View {
        VStack(spacing: 0) {
            // Title
            Text("\(viewModel.ticker.uppercased()) Settings")
                .font(.headline)
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
                    searchVM.cancelSearch()
                    dismiss()
                }
                .keyboardShortcut(.return)
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 12)
        }
        .frame(width: UI.chartSettingsSheetWidth, height: UI.chartSettingsSheetHeight)
        .onDisappear {
            searchVM.cancelSearch()
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
    }

    // MARK: - Ticker Tab

    private var tickerTab: some View {
        VStack(spacing: 12) {
            // Current ticker
            HStack {
                Text("Current:")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                Label(viewModel.ticker.uppercased(), systemImage: viewModel.source.icon)
                    .font(.body.weight(.medium))
                Spacer()
            }
            .padding(.horizontal, 16)

            // Search input
            HStack(spacing: 8) {
                TextField("New ticker symbol (e.g. BTC or PEPE)", text: $searchText)
                    .textFieldStyle(.roundedBorder)
                    .font(.body)
                    .onChange(of: searchText) {
                        searchVM.scheduleSearch(query: searchText)
                    }
                    .onSubmit {
                        if let first = searchVM.firstAvailableResult {
                            searchVM.selectedResult = first
                        }
                    }

                if searchVM.isSearching {
                    ProgressView()
                        .scaleEffect(0.7)
                        .frame(width: 20, height: 20)
                }
            }
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
                HStack {
                    Label("New: \(selected.symbol)", systemImage: selected.source.icon)
                        .font(.callout.weight(.medium))
                    Spacer()
                }
                .padding(8)
                .padding(.horizontal, 8)
                .background(.quaternary, in: RoundedRectangle(cornerRadius: 6))
                .padding(.horizontal, 16)

                Button("Change Ticker") {
                    dismiss()
                    onUpdateTicker(selected.fullSymbol, selected.source)
                }
                .buttonStyle(.borderedProminent)
                .disabled(searchVM.selectedResult == nil)
            }

            if let error = errorMessage {
                HStack {
                    Image(systemName: "exclamationmark.triangle.fill")
                        .foregroundStyle(.orange)
                    Text(error)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
                .padding(.horizontal, 16)
            }

            Spacer()
        }
        .padding(.top, 8)
    }

    // MARK: - Appearance Tab

    private var appearanceTab: some View {
        VStack(alignment: .leading, spacing: 16) {
            HStack(spacing: 24) {
                VStack(alignment: .leading, spacing: 8) {
                    Text("Bullish Candle")
                        .font(.subheadline.weight(.medium))
                    ColorPicker("", selection: $bullishColor)
                        .labelsHidden()
                }

                VStack(alignment: .leading, spacing: 8) {
                    Text("Bearish Candle")
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

            Spacer()
        }
        .padding(16)
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
        onUpdateTicker: { _, _ in },
        onRemove: {},
        onStyleChanged: {}
    )
}
