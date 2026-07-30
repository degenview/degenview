import SwiftUI

// MARK: - ChartSettingsSheet

struct ChartSettingsSheet: View {
    @ObservedObject var viewModel: ChartViewModel
    let onUpdateTicker: (String, DataSourceType) -> Void
    let onRemove: () -> Void
    let onStyleChanged: () -> Void

    @State private var selectedTab: Tab = .ticker
    @State private var searchText = ""
    @State private var searchResults: [DataSourceType: [TickerSearchResult]] = [:]
    @State private var isSearching = false
    @State private var selectedResult: TickerSearchResult?
    @State private var errorMessage: String?
    @State private var searchTask: Task<Void, Never>?

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
                    searchTask?.cancel()
                    dismiss()
                }
                .keyboardShortcut(.return)
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 12)
        }
        .frame(width: 420, height: 420)
        .onDisappear {
            searchTask?.cancel()
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
                        scheduleSearch()
                    }
                    .onSubmit {
                        if let first = firstAvailableResult {
                            selectedResult = first
                        }
                    }

                if isSearching {
                    ProgressView()
                        .scaleEffect(0.7)
                        .frame(width: 20, height: 20)
                }
            }
            .padding(.horizontal, 16)

            // Search results
            if !searchResults.isEmpty {
                List {
                    ForEach(orderedSources, id: \.self) { source in
                        if let results = searchResults[source], !results.isEmpty {
                            Section {
                                ForEach(results) { result in
                                    searchResultRow(result)
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
                .frame(maxHeight: 180)
            }

            // Selected result
            if let selected = selectedResult {
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
                .disabled(selectedResult == nil)
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

    // MARK: - Search (reuses pattern from AddTickerSheet)

    private var orderedSources: [DataSourceType] {
        var sources = DataSourceType.allCases
        sources.sort { a, b in
            let aHas = !(searchResults[a]?.isEmpty ?? true)
            let bHas = !(searchResults[b]?.isEmpty ?? true)
            if aHas != bHas { return aHas }
            return a.rawValue < b.rawValue
        }
        return sources
    }

    private var firstAvailableResult: TickerSearchResult? {
        for source in orderedSources {
            if let results = searchResults[source], let first = results.first {
                return first
            }
        }
        return nil
    }

    private func scheduleSearch() {
        searchTask?.cancel()

        let text = searchText.trimmingCharacters(in: .whitespaces)
        guard !text.isEmpty else {
            searchResults = [:]
            selectedResult = nil
            return
        }

        let captured = text
        searchTask = Task { @MainActor in
            try? await Task.sleep(nanoseconds: 300_000_000)
            guard !Task.isCancelled, searchText.trimmingCharacters(in: .whitespaces) == captured else { return }

            isSearching = true
            defer { isSearching = false }

            let sources = DataSourceFactory.shared.allSources
            var newResults: [DataSourceType: [TickerSearchResult]] = [:]

            await withTaskGroup(of: (DataSourceType, [TickerSearchResult]?).self) { group in
                for source in sources {
                    group.addTask {
                        do {
                            let results = try await source.searchTickers(query: captured)
                            return (source.type, results)
                        } catch {
                            print("[ChartSettings] \(source.type.displayName) search failed: \(error.localizedDescription)")
                            return (source.type, nil)
                        }
                    }
                }

                for await (type, results) in group {
                    if let r = results, !r.isEmpty {
                        newResults[type] = r
                    }
                }
            }

            guard !Task.isCancelled else { return }
            searchResults = newResults

            if let selected = selectedResult,
               !newResults.values.flatMap({ $0 }).contains(selected) {
                selectedResult = nil
            }
        }
    }

    private func searchResultRow(_ result: TickerSearchResult) -> some View {
        HStack {
            VStack(alignment: .leading, spacing: 2) {
                Text(result.symbol)
                    .font(.body.weight(.medium))

                if let chain = result.chain, let dex = result.dex {
                    Text("\(chain.capitalized) · \(dex.capitalized)")
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                } else if result.source == .coingecko {
                    Text("via CoinGecko")
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                } else if result.source == .binance {
                    Text("via Binance")
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                }
            }

            Spacer()

            if let price = result.price {
                Text(price, format: .currency(code: "USD").precision(.fractionLength(2...6)))
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        }
        .padding(.vertical, 2)
        .contentShape(Rectangle())
        .onTapGesture {
            selectedResult = result
        }
        .background(selectedResult == result
            ? Color.accentColor.opacity(0.15)
            : Color.clear
        )
        .clipShape(RoundedRectangle(cornerRadius: 4))
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
